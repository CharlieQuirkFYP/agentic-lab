use serde::{Deserialize, Serialize};
use va_core::{DictionaryHints, EngineConfig};

use crate::{BridgeError, INVALID_ARGUMENT, UNSUPPORTED};

pub const SCHEMA_VERSION: u32 = 1;
pub const MAX_CONFIG_BYTES: usize = 16 * 1024;
pub const MAX_AUDIO_SAMPLES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    pub schema_version: u32,
    pub model_path: String,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub dictionary: Vec<String>,
    #[serde(default = "default_threads")]
    pub threads: i32,
    #[serde(default = "default_duration")]
    pub max_audio_seconds: u32,
    #[serde(default = "default_metrics")]
    pub metrics_enabled: bool,
}

fn default_threads() -> i32 {
    4
}
fn default_duration() -> u32 {
    120
}
fn default_metrics() -> bool {
    true
}

impl Configuration {
    #[cfg(any(feature = "whisper", test, pheme_ffi_test))]
    pub fn for_model(model_path: String) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            model_path,
            language: None,
            dictionary: Vec::new(),
            threads: default_threads(),
            max_audio_seconds: default_duration(),
            metrics_enabled: true,
        }
    }

    pub fn validate(mut self) -> Result<Self, BridgeError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(BridgeError::new(
                UNSUPPORTED,
                "unsupported configuration schema_version",
            ));
        }
        if self.model_path.is_empty()
            || self.model_path.len() > 4096
            || self.model_path.contains('\0')
        {
            return Err(BridgeError::new(
                INVALID_ARGUMENT,
                "model_path must contain 1–4096 UTF-8 bytes without NUL",
            ));
        }
        if !(1..=16).contains(&self.threads) || !(1..=120).contains(&self.max_audio_seconds) {
            return Err(BridgeError::new(
                INVALID_ARGUMENT,
                "threads must be 1–16 and max_audio_seconds must be 1–120",
            ));
        }
        if let Some(language) = &self.language {
            if language.is_empty()
                || language.len() > 16
                || !language
                    .bytes()
                    .all(|b| b.is_ascii_alphabetic() || b == b'-')
            {
                return Err(BridgeError::new(
                    INVALID_ARGUMENT,
                    "language must be a language code or null for automatic detection",
                ));
            }
        }
        if self.dictionary.len() > 128
            || self
                .dictionary
                .iter()
                .any(|term| term.contains('\0') || term.chars().count() > 128)
        {
            return Err(BridgeError::new(
                INVALID_ARGUMENT,
                "dictionary accepts at most 128 terms of 128 characters without NUL",
            ));
        }
        let mut terms: Vec<String> = Vec::new();
        for term in &self.dictionary {
            let term = term.split_whitespace().collect::<Vec<_>>().join(" ");
            if !term.is_empty()
                && !terms
                    .iter()
                    .any(|v| v.to_lowercase() == term.to_lowercase())
            {
                terms.push(term);
            }
        }
        if terms.join(", ").chars().count() > 2000 {
            return Err(BridgeError::new(
                INVALID_ARGUMENT,
                "dictionary prompt exceeds 2000 characters",
            ));
        }
        self.dictionary = terms;
        Ok(self)
    }

    pub fn engine_config(&self) -> EngineConfig {
        let mut config = EngineConfig {
            max_audio_seconds: Some(self.max_audio_seconds),
            dictionary: DictionaryHints::with_terms(self.dictionary.clone()),
            language: self.language.clone(),
            cleanup_enabled: false,
            ..EngineConfig::default()
        };
        config.decoder.threads = self.threads;
        config
    }
}
