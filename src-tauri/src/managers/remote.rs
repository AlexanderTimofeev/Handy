//! Shared plumbing for cloud transcription engines (Codex, Groq, …).
//!
//! Requests go through the system `curl` rather than reqwest: some endpoints
//! (notably ChatGPT's) sit behind Cloudflare bot-management that fingerprints
//! reqwest's TLS ClientHello and returns a managed-challenge 403, while curl's
//! handshake is allowed through. curl ships on Windows 10+/macOS/Linux.

use anyhow::{anyhow, Result};
use log::debug;
use serde::Deserialize;
use std::io::{Cursor, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Deserialize)]
struct TranscribeResponse {
    text: String,
}

/// Map Handy's selected language to an API `language` parameter.
///
/// Returns `None` when no language should be sent (auto-detect). `auto` is
/// never forwarded; `zh-Hans`/`zh-Hant` collapse to `zh`.
pub fn normalize_language(selected: &str) -> Option<String> {
    match selected {
        "" | "auto" => None,
        "zh-Hans" | "zh-Hant" => Some("zh".to_string()),
        other => Some(other.to_string()),
    }
}

/// Encode 16 kHz mono f32 samples as an in-memory 16-bit PCM WAV buffer,
/// matching the format Handy uses elsewhere (see `save_wav_file`).
pub fn encode_wav(samples: &[f32]) -> Result<Vec<u8>> {
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

/// Build the `curl -K -` config (one option per line). Fed to curl on stdin so
/// the bearer token never appears in the process argument list or on disk.
/// `extra_headers` are full `Name: value` strings; `form_fields` are additional
/// multipart text parts (e.g. `model`, `language`).
fn build_curl_config(
    url: &str,
    bearer: &str,
    extra_headers: &[String],
    form_fields: &[(String, String)],
    wav_path: &str,
) -> String {
    let mut cfg = String::new();
    cfg.push_str(&format!("url = \"{}\"\n", url));
    cfg.push_str("request = \"POST\"\n");
    cfg.push_str(&format!("header = \"Authorization: Bearer {}\"\n", bearer));
    for header in extra_headers {
        cfg.push_str(&format!("header = \"{}\"\n", header));
    }
    cfg.push_str(&format!(
        "form = \"file=@{};type=audio/wav;filename=handy.wav\"\n",
        wav_path
    ));
    for (name, value) in form_fields {
        cfg.push_str(&format!("form = \"{}={}\"\n", name, value));
    }
    // HTTP/1.1 + curl's TLS handshake is what passes Cloudflare here.
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

/// POST a multipart transcription request and return the parsed `text`.
///
/// The audio is written to a unique temp WAV (curl's form upload needs a file
/// path) which is deleted afterwards; the bearer token goes via stdin only.
pub fn curl_transcribe(
    url: &str,
    bearer: &str,
    extra_headers: &[String],
    form_fields: &[(String, String)],
    samples: &[f32],
) -> Result<String> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let wav = encode_wav(samples)?;
    debug!(
        "remote transcribe: url={} {} bytes wav, {} form fields",
        url,
        wav.len(),
        form_fields.len()
    );

    let unique = format!(
        "{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let wav_path = std::env::temp_dir().join(format!("handy-remote-{}.wav", unique));
    std::fs::write(&wav_path, &wav)
        .map_err(|e| anyhow!("Failed to write temp WAV for remote request: {}", e))?;

    let wav_arg = wav_path.to_string_lossy().replace('\\', "/");
    let config = build_curl_config(url, bearer, extra_headers, form_fields, &wav_arg);

    let result = (|| -> Result<String> {
        let mut command = Command::new("curl");
        command
            .arg("-K")
            .arg("-")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Hide the curl console window on Windows. Without CREATE_NO_WINDOW a
        // console flashes on every transcription and steals focus from the app
        // the user is dictating into.
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = command
            .spawn()
            .map_err(|e| anyhow!("Failed to run curl (is it installed?): {}", e))?;

        child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("Failed to open curl stdin"))?
            .write_all(config.as_bytes())?;

        let output = child.wait_with_output()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let (body, code) = split_response(&stdout);

        if code != "200" {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let detail = if body.is_empty() {
                stderr.trim().to_string()
            } else {
                body.chars().take(300).collect()
            };
            return Err(anyhow!(
                "Transcription request failed: HTTP {} - {}",
                if code.is_empty() { "(no status)" } else { code },
                detail
            ));
        }

        let parsed: TranscribeResponse = serde_json::from_str(body).map_err(|e| {
            anyhow!(
                "Failed to parse transcription response: {} (body: {})",
                e,
                body.chars().take(200).collect::<String>()
            )
        })?;
        Ok(parsed.text)
    })();

    let _ = std::fs::remove_file(&wav_path);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_language_auto_and_empty_are_none() {
        assert_eq!(normalize_language("auto"), None);
        assert_eq!(normalize_language(""), None);
    }

    #[test]
    fn normalize_language_chinese_variants_collapse_to_zh() {
        assert_eq!(normalize_language("zh-Hans"), Some("zh".to_string()));
        assert_eq!(normalize_language("zh-Hant"), Some("zh".to_string()));
    }

    #[test]
    fn normalize_language_passes_through_other_codes() {
        assert_eq!(normalize_language("en"), Some("en".to_string()));
        assert_eq!(normalize_language("ru"), Some("ru".to_string()));
    }

    #[test]
    fn encode_wav_roundtrips_through_hound() {
        let samples = vec![0.0f32, 0.5, -0.5, 1.0, -1.0];
        let bytes = encode_wav(&samples).unwrap();
        let reader = hound::WavReader::new(Cursor::new(bytes)).unwrap();
        let spec = reader.spec();
        assert_eq!(spec.channels, 1);
        assert_eq!(spec.sample_rate, 16000);
        assert_eq!(spec.bits_per_sample, 16);
        assert_eq!(reader.len() as usize, samples.len());
    }

    #[test]
    fn encode_wav_empty_samples_produces_valid_header() {
        let bytes = encode_wav(&[]).unwrap();
        let reader = hound::WavReader::new(Cursor::new(bytes)).unwrap();
        assert_eq!(reader.len(), 0);
    }

    #[test]
    fn split_response_separates_body_and_status() {
        let (body, code) = split_response("{\"text\":\"hi\"}\n200");
        assert_eq!(body, "{\"text\":\"hi\"}");
        assert_eq!(code, "200");
    }

    #[test]
    fn split_response_handles_multiline_body_and_crlf() {
        let (body, code) = split_response("{\n  \"text\": \"hi\"\n}\r\n403\r\n");
        assert_eq!(body, "{\n  \"text\": \"hi\"\n}");
        assert_eq!(code, "403");
    }

    #[test]
    fn build_curl_config_includes_auth_headers_and_form() {
        let cfg = build_curl_config(
            "https://api.example.com/transcribe",
            "TOK",
            &["originator: codex_desktop".to_string()],
            &[("model".to_string(), "whisper-large-v3".to_string())],
            "C:/tmp/handy.wav",
        );
        assert!(cfg.contains("url = \"https://api.example.com/transcribe\""));
        assert!(cfg.contains("header = \"Authorization: Bearer TOK\""));
        assert!(cfg.contains("header = \"originator: codex_desktop\""));
        assert!(cfg.contains("form = \"file=@C:/tmp/handy.wav;type=audio/wav;filename=handy.wav\""));
        assert!(cfg.contains("form = \"model=whisper-large-v3\""));
    }

    #[test]
    fn transcribe_response_parses_text_field() {
        let parsed: TranscribeResponse =
            serde_json::from_str(r#"{ "text": "hello world" }"#).unwrap();
        assert_eq!(parsed.text, "hello world");
    }
}
