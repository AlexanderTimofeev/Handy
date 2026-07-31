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
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const CONNECT_TIMEOUT_SECS: u64 = 5;
const REQUEST_TIMEOUT_SECS: u64 = 45;
const RETRY_DELAY: Duration = Duration::from_millis(500);
const CHILD_POLL_INTERVAL: Duration = Duration::from_millis(50);
const MAX_ATTEMPTS: usize = 2;

/// Incrementing generation used to cancel currently running remote requests.
/// A generation avoids a stale cancellation affecting the next transcription.
static REMOTE_CANCEL_GENERATION: AtomicU64 = AtomicU64::new(0);

#[derive(Deserialize)]
struct TranscribeResponse {
    text: String,
}

enum AttemptOutcome {
    Success(String),
    Retryable(anyhow::Error),
    Fatal(anyhow::Error),
    Cancelled,
}

enum WaitOutcome {
    Output(Output),
    Cancelled,
    TimedOut,
    Failed(anyhow::Error),
}

/// Cancel all currently running cloud transcription requests.
///
/// Handy only runs one normal transcription pipeline at a time, but using a
/// generation keeps this safe if a history retry happens to overlap.
pub fn cancel_active_requests() {
    REMOTE_CANCEL_GENERATION.fetch_add(1, Ordering::AcqRel);
}

fn cancellation_generation() -> u64 {
    REMOTE_CANCEL_GENERATION.load(Ordering::Acquire)
}

fn was_cancelled_since(generation: u64) -> bool {
    REMOTE_CANCEL_GENERATION.load(Ordering::Acquire) != generation
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
    // A short connect timeout catches a dead proxy/network quickly. The full
    // request timeout still leaves enough time to upload and process long audio.
    cfg.push_str("http1.1\nsilent\nshow-error\n");
    cfg.push_str(&format!("connect-timeout = {}\n", CONNECT_TIMEOUT_SECS));
    cfg.push_str(&format!("max-time = {}\n", REQUEST_TIMEOUT_SECS));
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

fn is_retryable_http_status(code: &str) -> bool {
    matches!(code, "408" | "425" | "429")
        || code
            .parse::<u16>()
            .map(|status| (500..=599).contains(&status))
            .unwrap_or(false)
}

fn wait_for_child(mut child: Child, generation: u64) -> WaitOutcome {
    let started = Instant::now();
    // curl has its own max-time. This slightly larger guard also protects us if
    // a platform-specific curl build fails to honour the config option.
    let hard_limit = Duration::from_secs(REQUEST_TIMEOUT_SECS + 5);

    loop {
        if was_cancelled_since(generation) {
            let _ = child.kill();
            let _ = child.wait();
            return WaitOutcome::Cancelled;
        }

        if started.elapsed() >= hard_limit {
            let _ = child.kill();
            let _ = child.wait();
            return WaitOutcome::TimedOut;
        }

        match child.try_wait() {
            Ok(Some(_)) => {
                return match child.wait_with_output() {
                    Ok(output) => WaitOutcome::Output(output),
                    Err(err) => WaitOutcome::Failed(anyhow!(
                        "Failed to collect curl output: {}",
                        err
                    )),
                };
            }
            Ok(None) => thread::sleep(CHILD_POLL_INTERVAL),
            Err(err) => {
                let _ = child.kill();
                let _ = child.wait();
                return WaitOutcome::Failed(anyhow!("Failed to poll curl process: {}", err));
            }
        }
    }
}

fn run_attempt(config: &str, generation: u64) -> AttemptOutcome {
    if was_cancelled_since(generation) {
        return AttemptOutcome::Cancelled;
    }

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

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            return AttemptOutcome::Fatal(anyhow!(
                "Failed to run curl (is it installed?): {}",
                err
            ))
        }
    };

    let write_result = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("Failed to open curl stdin"))
        .and_then(|mut stdin| {
            stdin
                .write_all(config.as_bytes())
                .map_err(|err| anyhow!("Failed to configure curl request: {}", err))
        });

    if let Err(err) = write_result {
        let _ = child.kill();
        let _ = child.wait();
        return AttemptOutcome::Retryable(err);
    }

    let attempt_started = Instant::now();
    let output = match wait_for_child(child, generation) {
        WaitOutcome::Output(output) => output,
        WaitOutcome::Cancelled => return AttemptOutcome::Cancelled,
        WaitOutcome::TimedOut => {
            return AttemptOutcome::Fatal(anyhow!(
                "Transcription request timed out after {} seconds",
                REQUEST_TIMEOUT_SECS
            ))
        }
        WaitOutcome::Failed(err) => return AttemptOutcome::Retryable(err),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let (body, code) = split_response(&stdout);

    if code != "200" {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = if body.is_empty() {
            stderr.trim().to_string()
        } else {
            body.chars().take(300).collect()
        };
        let display_code = if code.is_empty() { "(no status)" } else { code };
        let err = anyhow!(
            "Transcription request failed: HTTP {} - {}",
            display_code,
            detail
        );

        // HTTP 408/425/429/5xx are explicitly transient. A missing/000 status
        // is retried only when it failed near the short connect-timeout; a full
        // request timeout is not repeated and therefore cannot block for 90s.
        let failed_during_connect = (code.is_empty() || code == "000")
            && attempt_started.elapsed()
                <= Duration::from_secs(CONNECT_TIMEOUT_SECS.saturating_add(5));
        if is_retryable_http_status(code) || failed_during_connect {
            return AttemptOutcome::Retryable(err);
        }
        return AttemptOutcome::Fatal(err);
    }

    let parsed: TranscribeResponse = match serde_json::from_str(body) {
        Ok(parsed) => parsed,
        Err(err) => {
            return AttemptOutcome::Fatal(anyhow!(
                "Failed to parse transcription response: {} (body: {})",
                err,
                body.chars().take(200).collect::<String>()
            ))
        }
    };

    AttemptOutcome::Success(parsed.text)
}

fn sleep_before_retry(generation: u64) -> bool {
    let started = Instant::now();
    while started.elapsed() < RETRY_DELAY {
        if was_cancelled_since(generation) {
            return false;
        }
        let remaining = RETRY_DELAY.saturating_sub(started.elapsed());
        thread::sleep(remaining.min(CHILD_POLL_INTERVAL));
    }
    true
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

    let generation = cancellation_generation();
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
        for attempt in 0..MAX_ATTEMPTS {
            match run_attempt(&config, generation) {
                AttemptOutcome::Success(text) => return Ok(text),
                AttemptOutcome::Cancelled => {
                    return Err(anyhow!("Transcription request cancelled"))
                }
                AttemptOutcome::Fatal(err) => return Err(err),
                AttemptOutcome::Retryable(err) => {
                    if attempt + 1 >= MAX_ATTEMPTS {
                        return Err(err);
                    }
                    debug!(
                        "Remote transcription attempt {} failed transiently: {}. Retrying once...",
                        attempt + 1,
                        err
                    );
                    if !sleep_before_retry(generation) {
                        return Err(anyhow!("Transcription request cancelled"));
                    }
                }
            }
        }

        Err(anyhow!("Remote transcription failed without a result"))
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
    fn build_curl_config_includes_auth_headers_form_and_timeouts() {
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
        assert!(cfg.contains("connect-timeout = 5"));
        assert!(cfg.contains("max-time = 45"));
    }

    #[test]
    fn retryable_http_statuses_are_classified() {
        for status in ["408", "425", "429", "500", "502", "503", "504"] {
            assert!(is_retryable_http_status(status), "status {status}");
        }
        for status in ["200", "400", "401", "403", "404"] {
            assert!(!is_retryable_http_status(status), "status {status}");
        }
    }

    #[test]
    fn transcribe_response_parses_text_field() {
        let parsed: TranscribeResponse =
            serde_json::from_str(r#"{ "text": "hello world" }"#).unwrap();
        assert_eq!(parsed.text, "hello world");
    }
}
