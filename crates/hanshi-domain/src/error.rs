//! Domain-layer errors.
//!
//! Anything in `hanshi-domain` that can fail surfaces an error here.
//! IO failures live in other crates' error types — this one is pure
//! domain-logic (invalid transitions, invariant violations).

use thiserror::Error;

use crate::job::JobState;

pub type Result<T> = std::result::Result<T, DomainError>;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DomainError {
    #[error("invalid job state transition: {from:?} → {to:?} ({reason})")]
    InvalidTransition {
        from: JobState,
        to: JobState,
        reason: &'static str,
    },

    #[error("invariant violated: {0}")]
    Invariant(&'static str),

    #[error("validation failed for {field}: {reason}")]
    Validation {
        field: &'static str,
        reason: String,
    },

    #[error("unknown value for {kind}: {value}")]
    UnknownEnum {
        kind: &'static str,
        value: String,
    },
}

impl DomainError {
    pub fn invalid_transition(from: JobState, to: JobState, reason: &'static str) -> Self {
        Self::InvalidTransition { from, to, reason }
    }

    pub fn validation(field: &'static str, reason: impl Into<String>) -> Self {
        Self::Validation {
            field,
            reason: reason.into(),
        }
    }
}
