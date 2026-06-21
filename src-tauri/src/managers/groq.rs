use crate::managers::remote;
use anyhow::Result;

/// Groq's OpenAI-compatible audio transcription endpoint.
const GROQ_TRANSCRIBE_URL: &str = "https://api.groq.com/openai/v1/audio/transcriptions";

/// Remote transcription engine backed by Groq's hosted Whisper models.
///
/// Holds the Groq model name (e.g. `whisper-large-v3-turbo`) and the user's
/// per-model API key. Each transcription is an HTTP request; no local state.
pub struct GroqEngine {
    model: String,
    api_key: String,
}

impl GroqEngine {
    pub fn new(model: String, api_key: String) -> Self {
        GroqEngine { model, api_key }
    }

    /// Transcribe audio samples via Groq.
    pub fn transcribe(&self, samples: &[f32], selected_language: &str) -> Result<String> {
        let mut form_fields = vec![
            ("model".to_string(), self.model.clone()),
            ("response_format".to_string(), "json".to_string()),
        ];
        if let Some(lang) = remote::normalize_language(selected_language) {
            form_fields.push(("language".to_string(), lang));
        }

        remote::curl_transcribe(
            GROQ_TRANSCRIBE_URL,
            &self.api_key,
            &[],
            &form_fields,
            samples,
        )
    }
}
