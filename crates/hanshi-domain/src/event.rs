//! Job events — the structured log that makes hanshi better than VW7.
//!
//! Every thing that happens to a job becomes an `Event` with a
//! timestamp + correlation-id. Together these events are the complete
//! audit trail of a job's lifecycle; they power the UI timeline and
//! the rule-engine advisor input stream.

use serde::{Deserialize, Serialize};

use crate::advisory::AdvisoryRef;
use crate::ids::{CorrelationId, EventId, JobId};
use crate::job::JobState;
use crate::printer::PrinterState;
use crate::Timestamp;

/// One event in a job's audit log.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JobEvent {
    pub id: EventId,
    pub job_id: JobId,
    pub correlation: CorrelationId,
    pub timestamp: Timestamp,
    pub kind: JobEventKind,
}

impl JobEvent {
    pub fn new(job_id: JobId, timestamp: Timestamp, kind: JobEventKind) -> Self {
        Self {
            id: EventId::new(),
            job_id,
            correlation: CorrelationId::from(job_id),
            timestamp,
            kind,
        }
    }
}

/// Kinds of things that can happen to a job. Additive — new kinds
/// shipped in later versions are ignored by older consumers that
/// don't recognize the tag (via `#[serde(other)]` on a future catch-all
/// if we need it).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize,
         gen_platform::TypedDispatcher,
         gen_platform::Discriminant,
         gen_platform::IsVariant)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum JobEventKind {
    /// State transition. `reason` is optional operator/system note.
    StateTransition {
        from: JobState,
        to: JobState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// File landed in the VW hot folder, with its sha256 for integrity.
    FileWritten {
        path: String,
        bytes: u64,
        sha256_hex: String,
    },
    /// VW picked up the file from the hot folder. Detected by the file
    /// disappearing or by a VW log entry we parsed.
    VwConsumed,
    /// Parsed line from VW's log file — carried forward verbatim.
    VwLogLine { text: String },
    /// One SNMP printer-state snapshot.
    PrinterObserved {
        state: PrinterState,
        /// Optional summary text — "printing, ink all above 30%"
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
    },
    /// An advisory rule fired relating to this job.
    AdvisoryFired { advisory: AdvisoryRef },
    /// Free-form note from the operator or another subsystem.
    OperatorNote { text: String },
    /// QC pass/fail recorded by the operator.
    QcRecorded { passed: bool, notes: Option<String> },
}

// Fleet-wide dispatcher-catalog registration. ELEVENTH consumer
// class adopting gen-platform's typed-dispatcher catamorphism
// (after gen / caixa / wasm-platform / cofre / shigoto / engenho /
// magma / kura / pangea / tatara). See
// theory/UNIFIED-COMPUTING-MODEL.md §VI.
gen_platform::register_dispatcher!("hanshi.job-event-kind", JobEventKind);

impl JobEventKind {
    /// Operator-facing short label. Distinct from `discriminant()`
    /// (which auto-derives the kebab variant name) — short_label is
    /// human-curated shorthand for UI display, e.g. "advisory"
    /// vs the stable wire kind "advisory-fired".
    pub fn short_label(&self) -> &'static str {
        match self {
            Self::StateTransition { .. } => "state",
            Self::FileWritten { .. } => "file-written",
            Self::VwConsumed => "vw-consumed",
            Self::VwLogLine { .. } => "vw-log",
            Self::PrinterObserved { .. } => "printer-observed",
            Self::AdvisoryFired { .. } => "advisory",
            Self::OperatorNote { .. } => "note",
            Self::QcRecorded { .. } => "qc",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::advisory::{AdvisoryRef, Severity};

    #[test]
    fn event_new_pins_correlation_to_job() {
        let j = JobId::new();
        let t = crate::now();
        let e = JobEvent::new(
            j,
            t,
            JobEventKind::StateTransition {
                from: JobState::Submitted,
                to: JobState::Staged,
                reason: None,
            },
        );
        assert_eq!(e.correlation.as_uuid(), j.as_uuid());
        assert_eq!(e.job_id, j);
        assert_eq!(e.timestamp, t);
    }

    #[test]
    fn short_labels() {
        assert_eq!(
            JobEventKind::VwConsumed.short_label(),
            "vw-consumed",
        );
        assert_eq!(
            JobEventKind::FileWritten {
                path: "/x".into(),
                bytes: 0,
                sha256_hex: "".into(),
            }
            .short_label(),
            "file-written",
        );
    }

    #[test]
    fn event_serde_round_trip() {
        let j = JobId::new();
        let t = crate::now();
        let e = JobEvent::new(
            j,
            t,
            JobEventKind::AdvisoryFired {
                advisory: AdvisoryRef {
                    id: crate::ids::AdvisoryId::new(),
                    severity: Severity::Warn,
                    title: "stalled".into(),
                },
            },
        );
        let s = serde_json::to_string(&e).unwrap();
        let back: JobEvent = serde_json::from_str(&s).unwrap();
        assert_eq!(e, back);
    }
}
