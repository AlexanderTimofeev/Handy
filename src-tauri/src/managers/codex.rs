use crate::managers::remote;
use anyhow::{anyhow, Result};
use std::path::PathBuf;

/// ChatGPT backend transcription endpoint used by Codex.
const TRANSCRIBE_URL: &str = "https://chatgpt.com/backend-api/transcribe";
/// Identifies the request as coming from the Codex desktop client.
const ORIGINATOR: &str = "codex_desktop";
/// User-Agent the Codex desktop client sends.
const CODEX_USER_AGENT: &str = "Codex Desktop/26.611.62324";

/// Remote transcription engine backed by the ChatGPT backend transcribe API,
/// authenticated with the local Codex CLI credentials (`~/.codex/auth.json`).
///
/// Holds no model state — each transcription is an HTTP request. It is a unit
/// struct so it can live in the same `LoadedEngine` enum as the local engines.
pub struct CodexEngine;

#[derive(Debug, Clone, PartialEq)]
struct CodexAuth {
    access_token: String,
    account_id: Option<String>,
}

impl CodexEngine {
    pub fn new() -> Self {
        CodexEngine
    }

    /// Path to the Codex auth file (`~/.codex/auth.json`).
    fn auth_path() -> Result<PathBuf> {
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .ok_or_else(|| {
                anyhow!("Could not determine home directory (HOME/USERPROFILE unset)")
            })?;
        Ok(PathBuf::from(home).join(".codex").join("auth.json"))
    }

    /// Parse Codex credentials from `auth.json` contents.
    ///
    /// Uses `tokens.access_token` (required) and `tokens.account_id` (optional).
    /// `refresh_token` and `id_token` are intentionally ignored.
    fn parse_auth(contents: &str) -> Result<CodexAuth> {
        let json: serde_json::Value = serde_json::from_str(contents)
            .map_err(|e| anyhow!("Failed to parse Codex auth.json: {}", e))?;
        let tokens = json
            .get("tokens")
            .ok_or_else(|| anyhow!("Codex auth.json missing 'tokens' object"))?;
        let access_token = tokens
            .get("access_token")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("Codex auth.json missing tokens.access_token"))?
            .to_string();
        let account_id = tokens
            .get("account_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        Ok(CodexAuth {
            access_token,
            account_id,
        })
    }

    /// Read and parse the Codex auth file from disk.
    fn load_auth() -> Result<CodexAuth> {
        let path = Self::auth_path()?;
        let contents = std::fs::read_to_string(&path).map_err(|e| {
            anyhow!(
                "Failed to read Codex auth file {}: {}. Is the Codex CLI logged in?",
                path.display(),
                e
            )
        })?;
        Self::parse_auth(&contents)
    }

    /// Transcribe audio samples via the remote Codex endpoint.
    pub fn transcribe(&self, samples: &[f32], selected_language: &str) -> Result<String> {
        let auth = Self::load_auth()?;

        let mut headers = vec![
            format!("originator: {}", ORIGINATOR),
            format!("User-Agent: {}", CODEX_USER_AGENT),
        ];
        if let Some(account_id) = &auth.account_id {
            headers.push(format!("ChatGPT-Account-Id: {}", account_id));
        }

        let mut form_fields = Vec::new();
        if let Some(lang) = remote::normalize_language(selected_language) {
            form_fields.push(("language".to_string(), lang));
        }

        remote::curl_transcribe(
            TRANSCRIBE_URL,
            &auth.access_token,
            &headers,
            &form_fields,
            samples,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_auth_reads_access_token_and_account_id() {
        let json = r#"{
            "tokens": {
                "access_token": "abc123",
                "account_id": "acct-xyz",
                "refresh_token": "should-ignore",
                "id_token": "should-ignore"
            }
        }"#;
        let auth = CodexEngine::parse_auth(json).unwrap();
        assert_eq!(auth.access_token, "abc123");
        assert_eq!(auth.account_id, Some("acct-xyz".to_string()));
    }

    #[test]
    fn parse_auth_account_id_optional() {
        let json = r#"{ "tokens": { "access_token": "tok" } }"#;
        let auth = CodexEngine::parse_auth(json).unwrap();
        assert_eq!(auth.access_token, "tok");
        assert_eq!(auth.account_id, None);
    }

    #[test]
    fn parse_auth_empty_account_id_treated_as_absent() {
        let json = r#"{ "tokens": { "access_token": "tok", "account_id": "" } }"#;
        let auth = CodexEngine::parse_auth(json).unwrap();
        assert_eq!(auth.account_id, None);
    }

    #[test]
    fn parse_auth_missing_access_token_errors() {
        let json = r#"{ "tokens": { "account_id": "acct" } }"#;
        assert!(CodexEngine::parse_auth(json).is_err());
    }

    #[test]
    fn parse_auth_missing_tokens_object_errors() {
        let json = r#"{ "other": {} }"#;
        assert!(CodexEngine::parse_auth(json).is_err());
    }

    #[test]
    fn parse_auth_invalid_json_errors() {
        assert!(CodexEngine::parse_auth("not json").is_err());
    }

    /// Live diagnostic: decodes a WAV from disk and runs the real `transcribe()`
    /// (curl → ChatGPT backend) with the local Codex auth. Ignored by default.
    /// Run with:
    ///   set HANDY_CODEX_WAV=<path to a 16kHz mono .wav>
    ///   cargo test --release --lib codex::tests::live_codex_request -- --ignored --nocapture
    #[test]
    #[ignore]
    fn live_codex_request() {
        let wav_path = std::env::var("HANDY_CODEX_WAV").expect("set HANDY_CODEX_WAV");
        let reader = hound::WavReader::open(&wav_path).expect("open wav");
        let samples: Vec<f32> = reader
            .into_samples::<i16>()
            .map(|s| s.unwrap() as f32 / i16::MAX as f32)
            .collect();
        eprintln!("samples={}", samples.len());
        match CodexEngine::new().transcribe(&samples, "auto") {
            Ok(text) => eprintln!("OK text={:?}", text),
            Err(e) => eprintln!("ERR {}", e),
        }
    }
}
