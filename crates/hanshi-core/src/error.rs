//! Core-layer errors.

use thiserror::Error;

pub type CoreResult<T> = std::result::Result<T, CoreError>;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("domain: {0}")]
    Domain(#[from] hanshi_domain::DomainError),

    #[error("port: {0}")]
    Port(#[from] hanshi_ports::PortError),

    #[error("not found: {kind} {id}")]
    NotFound { kind: &'static str, id: String },

    #[error("rule engine: {0}")]
    Rule(#[from] blue_lang_host::Error),

    #[error("busy: {0}")]
    Busy(&'static str),
}

impl CoreError {
    pub fn not_found(kind: &'static str, id: impl Into<String>) -> Self {
        Self::NotFound {
            kind,
            id: id.into(),
        }
    }
}
