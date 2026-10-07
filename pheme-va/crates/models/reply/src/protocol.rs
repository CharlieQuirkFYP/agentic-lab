//! Private-process NDJSON protocol, versioned and bounded independently of HTTP.

use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use va_core::{ConversationConfig, ConversationError, ConversationMessage};

pub const VERSION: u32 = 1;
pub const MAX_FRAME_BYTES: usize = 1_048_576;
// Leave room for JSON escaping, role/field overhead, and terminal full text.
pub const MAX_TEXT_CHARS: usize = MAX_FRAME_BYTES / 8;

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Init {
        protocol: u32,
        path: PathBuf,
        config: ConversationConfig,
        threads: i32,
        gpu_layers: u32,
    },
    Generate {
        id: u64,
        messages: Vec<ConversationMessage>,
        config: ConversationConfig,
    },
    Cancel {
        id: u64,
    },
    Shutdown,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Event {
    Ready {
        protocol: u32,
        name: String,
        load_time_ms: u64,
    },
    StartupError {
        message: String,
    },
    Delta {
        id: u64,
        text: String,
    },
    Done {
        id: u64,
        text: String,
    },
    Error {
        id: u64,
        error: ReplyError,
    },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReplyError {
    InvalidConfig { message: String },
    InvalidInput { message: String },
    InvalidHistory { message: String },
    InputTooLong { actual: usize, limit: usize },
    OutputTooLong { limit: usize },
    EmptyResponse,
    ContextExceeded,
    Cancelled,
    Backend { message: String },
}

impl From<ConversationError> for ReplyError {
    fn from(error: ConversationError) -> Self {
        match error {
            ConversationError::InvalidConfig(message) => Self::InvalidConfig { message },
            ConversationError::InvalidInput(message) => Self::InvalidInput { message },
            ConversationError::InvalidHistory(message) => Self::InvalidHistory { message },
            ConversationError::InputTooLong { actual, limit } => {
                Self::InputTooLong { actual, limit }
            }
            ConversationError::OutputTooLong { limit } => Self::OutputTooLong { limit },
            ConversationError::EmptyResponse => Self::EmptyResponse,
            ConversationError::ContextExceeded => Self::ContextExceeded,
            ConversationError::Cancelled => Self::Cancelled,
            ConversationError::Backend(message) => Self::Backend { message },
        }
    }
}
impl From<ReplyError> for ConversationError {
    fn from(error: ReplyError) -> Self {
        match error {
            ReplyError::InvalidConfig { message } => Self::InvalidConfig(message),
            ReplyError::InvalidInput { message } => Self::InvalidInput(message),
            ReplyError::InvalidHistory { message } => Self::InvalidHistory(message),
            ReplyError::InputTooLong { actual, limit } => Self::InputTooLong { actual, limit },
            ReplyError::OutputTooLong { limit } => Self::OutputTooLong { limit },
            ReplyError::EmptyResponse => Self::EmptyResponse,
            ReplyError::ContextExceeded => Self::ContextExceeded,
            ReplyError::Cancelled => Self::Cancelled,
            ReplyError::Backend { message } => Self::Backend(message),
        }
    }
}

pub fn validate_config(config: &ConversationConfig) -> Result<(), ConversationError> {
    config.validate()?;
    if config.max_input_chars > MAX_TEXT_CHARS || config.max_output_chars > MAX_TEXT_CHARS {
        return Err(ConversationError::InvalidConfig(format!(
            "stdio reply input/output budgets may not exceed {MAX_TEXT_CHARS} characters"
        )));
    }
    Ok(())
}

pub fn encode_frame(value: &impl Serialize) -> io::Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(value).map_err(invalid_data)?;
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(invalid_data("reply IPC frame exceeds byte limit"));
    }
    bytes.push(b'\n');
    Ok(bytes)
}

pub fn write_frame(writer: &mut impl Write, value: &impl Serialize) -> io::Result<()> {
    writer.write_all(&encode_frame(value)?)?;
    writer.flush()
}

/// Never allocate an unbounded line, accept invalid UTF-8, or treat truncated
/// output without its delimiter as a successful terminal event.
pub fn read_frame<T: DeserializeOwned>(reader: &mut impl BufRead) -> io::Result<Option<T>> {
    let mut bytes = Vec::new();
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return if bytes.is_empty() {
                Ok(None)
            } else {
                Err(invalid_data("truncated reply IPC frame"))
            };
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let size = newline.unwrap_or(available.len());
        if size > MAX_FRAME_BYTES.saturating_sub(bytes.len()) {
            return Err(invalid_data("reply IPC frame exceeds byte limit"));
        }
        bytes.extend_from_slice(&available[..size]);
        reader.consume(size + usize::from(newline.is_some()));
        if newline.is_some() {
            return serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(invalid_data);
        }
    }
}

fn invalid_data(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Cursor};

    #[test]
    fn utf8_and_json_escaping_survive_single_byte_reads() {
        let event = Event::Delta {
            id: 7,
            text: "café 世界 🧯\n\"quoted\"".into(),
        };
        let mut reader = BufReader::with_capacity(1, Cursor::new(encode_frame(&event).unwrap()));
        let Some(Event::Delta { id, text }) = read_frame(&mut reader).unwrap() else {
            panic!("missing delta")
        };
        assert_eq!(id, 7);
        assert_eq!(text, "café 世界 🧯\n\"quoted\"");
        assert!(read_frame::<Event>(&mut reader).unwrap().is_none());
    }

    #[test]
    fn bounds_and_truncated_frames_are_rejected() {
        for bytes in [
            b"{\"type\":\"shutdown\"}".to_vec(),
            vec![b'x'; MAX_FRAME_BYTES + 1],
            vec![0xff, b'\n'],
        ] {
            assert!(read_frame::<Command>(&mut Cursor::new(bytes)).is_err());
        }
        assert!(encode_frame(&Event::Delta {
            id: 1,
            text: "x".repeat(MAX_FRAME_BYTES)
        })
        .is_err());
    }

    #[test]
    fn errors_keep_the_core_contract_across_ipc() {
        for error in [
            ConversationError::Cancelled,
            ConversationError::ContextExceeded,
            ConversationError::OutputTooLong { limit: 5 },
            ConversationError::Backend("native failed".into()),
        ] {
            let expected = error.to_string();
            let encoded = encode_frame(&ReplyError::from(error)).unwrap();
            let roundtrip: ReplyError = read_frame(&mut Cursor::new(encoded)).unwrap().unwrap();
            assert_eq!(ConversationError::from(roundtrip).to_string(), expected);
        }
    }
}
