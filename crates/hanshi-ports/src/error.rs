//! Errors surfaced by port traits. Kept thin — each port-specific
//! error flattens into a common type so callers don't care which
//! backend produced a failure.

use thiserror::Error;

pub type PortResult<T> = std::result::Result<T, PortError>;

#[derive(Debug, Error)]
pub enum PortError {
    #[error("transport: {0}")]
    Transport(String),

    #[error("not available: {0}")]
    Unavailable(String),

    #[error("timed out after {ms}ms: {what}")]
    Timeout { what: String, ms: u64 },

    #[error("decode: {0}")]
    Decode(String),

    #[error("configuration: {0}")]
    Config(String),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

impl PortError {
    pub fn transport(msg: impl Into<String>) -> Self {
        Self::Transport(msg.into())
    }

    pub fn decode(msg: impl Into<String>) -> Self {
        Self::Decode(msg.into())
    }

    pub fn unavailable(msg: impl Into<String>) -> Self {
        Self::Unavailable(msg.into())
    }
}
