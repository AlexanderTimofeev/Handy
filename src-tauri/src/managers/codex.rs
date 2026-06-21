use anyhow::{anyhow, Result};
use log::debug;
use serde::Deserialize;
use std::io::{Cursor, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

/// ChatGPT backend transcription endpoint used by Codex.
const TRANSCRIBE_URL: &str = "https://chatgpt.com/backend-api/transcribe";
/// Identifies the request as coming from the Codex desktop client.
const ORIGINATOR: &str = "codex_desktop";
/// User-Agent the Codex desktop client sends.
const CODEX_USER_AGENT: &str = "Codex Desktop/26.611.62324";

/// Remote transcription engine backed by the ChatGPT backend transcribe API,
/// authenticated with the local Codex CLI credentials (`~/.codex/auth.json`).
///
/// Unlike the local inference engines, this engine holds no model state — each
/// transcription is an HTTP request. It exists as a unit struct so it can live
/// in the same `LoadedEngine` enum as the local engines.
pub struct CodexEngine;

#[derive(Debug, Clone, PartialEq)]
struct CodexAuth {
    access_token: String,
    account_id: Option<String>,
}

#[derive(Deserialize)]
struct TranscribeResponse {
    text: String,
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

    /// Map Handy's selected language to the API `language` parameter.
    ///
    /// Returns `None` when no language should be sent (auto-detect). `auto` is
    /// never forwarded; `zh-Hans`/`zh-Hant` collapse to `zh`.
    fn map_language(selected: &str) -> Option<String> {
        match selected {
            "" | "auto" => None,
            "zh-Hans" | "zh-Hant" => Some("zh".to_string()),
            other => Some(other.to_string()),
        }
    }

    /// Encode 16 kHz mono f32 samples as an in-memory 16-bit PCM WAV buffer,
    /// matching the format Handy uses elsewhere (see `save_wav_file`).
    fn encode_wav(samples: &[f32]) -> Result<Vec<u8>> {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut writer = hound::WavWriter::new(&mut cursor, spec)?;
            for &sample in samples {
                let clamped = sample.clamp(-1.0, 1.0);
                writer.write_sample((clamped * i16::MAX as f32) as i16)?;
            }
            writer.finalize()?;
        }
        Ok(cursor.into_inner())
    }

    /// Build the `curl -K -` config (one option per line) for a transcribe
    /// request. Returned as a string fed to curl via stdin, so the bearer token
    /// never appears in the process argument list or on disk.
    fn build_curl_config(auth: &CodexAuth, wav_path: &str, language: &Option<String>) -> String {
        let mut cfg = String::new();
        cfg.push_str(&format!("url = \"{}\"\n", TRANSCRIBE_URL));
        cfg.push_str("request = \"POST\"\n");
        cfg.push_str(&format!("header = \"originator: {}\"\n", ORIGINATOR));
        cfg.push_str(&format!("header = \"User-Agent: {}\"\n", CODEX_USER_AGENT));
        cfg.push_str(&format!(
            "header = \"Authorization: Bearer {}\"\n",
            auth.access_token
        ));
        if let Some(account_id) = &auth.account_id {
            cfg.push_str(&format!("header = \"ChatGPT-Account-Id: {}\"\n", account_id));
        }
        cfg.push_str(&format!(
            "form = \"file=@{};type=audio/wav;filename=handy.wav\"\n",
            wav_path
        ));
        if let Some(lang) = language {
            cfg.push_str(&format!("form = \"language={}\"\n", lang));
        }
        // HTTP/1.1 + curl's TLS handshake is what the real Codex desktop client
        // uses and what Cloudflare lets through; reqwest's ClientHello (any TLS
        // backend) gets a managed-challenge 403 on this endpoint.
        cfg.push_str("http1.1\nsilent\nshow-error\n");
        cfg.push_str("write-out = \"\\n%{http_code}\"\n");
        cfg
    }

    /// Split curl's combined output into (body, status_code). The config sets
    /// `write-out = "\n%{http_code}"`, so the status is the final line.
    fn split_response(stdout: &str) -> (&str, &str) {
        let trimmed = stdout.trim_end_matches(['\r', '\n']);
        match trimmed.rfind('\n') {
            Some(idx) => (trimmed[..idx].trim_end(), trimmed[idx + 1..].trim()),
            None => ("", trimmed.trim()),
        }
    }

    /// Transcribe audio samples via the remote Codex endpoint.
    ///
    /// The request is sent with the system `curl` rather than reqwest: Cloudflare
    /// fingerprints reqwest's TLS ClientHello and returns a managed-challenge 403,
    /// while curl's handshake (matching the real Codex client) is allowed through.
    pub fn transcribe(&self, samples: &[f32], selected_language: &str) -> Result<String> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);

        let wav = Self::encode_wav(samples)?;
        let language = Self::map_language(selected_language);
        let auth = Self::load_auth()?;

        debug!(
            "Codex transcribe: {} bytes wav, language={:?}, account_id={}",
            wav.len(),
            language,
            auth.account_id.is_some()
        );

        // Write the WAV to a unique temp file (curl's form upload needs a file
        // path). The audio is not sensitive; the bearer token goes via stdin.
        let unique = format!(
            "{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let wav_path = std::env::temp_dir().join(format!("handy-codex-{}.wav", unique));
        std::fs::write(&wav_path, &wav)
            .map_err(|e| anyhow!("Failed to write temp WAV for Codex request: {}", e))?;

        let wav_arg = wav_path.to_string_lossy().replace('\\', "/");
        let config = Self::build_curl_config(&auth, &wav_arg, &language);

        let result = (|| -> Result<String> {
            let mut child = Command::new("curl")
                .arg("-K")
                .arg("-")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| anyhow!("Failed to run curl (is it installed?): {}", e))?;

            child
                .stdin
                .take()
                .ok_or_else(|| anyhow!("Failed to open curl stdin"))?
                .write_all(config.as_bytes())?;

            let output = child.wait_with_output()?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            let (body, code) = Self::split_response(&stdout);

            if code != "200" {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let detail = if body.is_empty() {
                    stderr.trim().to_string()
                } else {
                    body.chars().take(300).collect()
                };
                return Err(anyhow!(
                    "Codex transcribe request failed: HTTP {} - {}",
                    if code.is_empty() { "(no status)" } else { code },
                    detail
                ));
            }

            let parsed: TranscribeResponse = serde_json::from_str(body).map_err(|e| {
                anyhow!(
                    "Failed to parse Codex transcribe response: {} (body: {})",
                    e,
                    body.chars().take(200).collect::<String>()
                )
            })?;
            Ok(parsed.text)
        })();

        let _ = std::fs::remove_file(&wav_path);
        result
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

    #[test]
    fn map_language_auto_and_empty_are_none() {
        assert_eq!(CodexEngine::map_language("auto"), None);
        assert_eq!(CodexEngine::map_language(""), None);
    }

    #[test]
    fn map_language_chinese_variants_collapse_to_zh() {
        assert_eq!(CodexEngine::map_language("zh-Hans"), Some("zh".to_string()));
        assert_eq!(CodexEngine::map_language("zh-Hant"), Some("zh".to_string()));
    }

    #[test]
    fn map_language_passes_through_other_codes() {
        assert_eq!(CodexEngine::map_language("en"), Some("en".to_string()));
        assert_eq!(CodexEngine::map_language("ru"), Some("ru".to_string()));
    }

    #[test]
    fn encode_wav_roundtrips_through_hound() {
        let samples = vec![0.0f32, 0.5, -0.5, 1.0, -1.0];
        let bytes = CodexEngine::encode_wav(&samples).unwrap();

        let reader = hound::WavReader::new(Cursor::new(bytes)).unwrap();
        let spec = reader.spec();
        assert_eq!(spec.channels, 1);
        assert_eq!(spec.sample_rate, 16000);
        assert_eq!(spec.bits_per_sample, 16);
        assert_eq!(reader.len() as usize, samples.len());
    }

    #[test]
    fn encode_wav_empty_samples_produces_valid_header() {
        let bytes = CodexEngine::encode_wav(&[]).unwrap();
        let reader = hound::WavReader::new(Cursor::new(bytes)).unwrap();
        assert_eq!(reader.len(), 0);
    }

    #[test]
    fn transcribe_response_parses_text_field() {
        let parsed: TranscribeResponse = serde_json::from_str(r#"{ "text": "hello world" }"#).unwrap();
        assert_eq!(parsed.text, "hello world");
    }

    #[test]
    fn split_response_separates_body_and_status() {
        let (body, code) = CodexEngine::split_response("{\"text\":\"hi\"}\n200");
        assert_eq!(body, "{\"text\":\"hi\"}");
        assert_eq!(code, "200");
    }

    #[test]
    fn split_response_handles_multiline_body_and_crlf() {
        let (body, code) = CodexEngine::split_response("{\n  \"text\": \"hi\"\n}\r\n403\r\n");
        assert_eq!(body, "{\n  \"text\": \"hi\"\n}");
        assert_eq!(code, "403");
    }

    #[test]
    fn build_curl_config_includes_auth_form_and_language() {
        let auth = CodexAuth {
            access_token: "TOK".to_string(),
            account_id: Some("ACC".to_string()),
        };
        let cfg = CodexEngine::build_curl_config(&auth, "C:/tmp/handy.wav", &Some("ru".to_string()));
        assert!(cfg.contains("url = \"https://chatgpt.com/backend-api/transcribe\""));
        assert!(cfg.contains("header = \"originator: codex_desktop\""));
        assert!(cfg.contains("header = \"Authorization: Bearer TOK\""));
        assert!(cfg.contains("header = \"ChatGPT-Account-Id: ACC\""));
        assert!(cfg.contains("form = \"file=@C:/tmp/handy.wav;type=audio/wav;filename=handy.wav\""));
        assert!(cfg.contains("form = \"language=ru\""));
    }

    #[test]
    fn build_curl_config_omits_account_and_language_when_absent() {
        let auth = CodexAuth {
            access_token: "TOK".to_string(),
            account_id: None,
        };
        let cfg = CodexEngine::build_curl_config(&auth, "/tmp/h.wav", &None);
        assert!(!cfg.contains("ChatGPT-Account-Id"));
        assert!(!cfg.contains("language="));
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
