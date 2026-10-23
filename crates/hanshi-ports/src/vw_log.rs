//! VW log port — stream log lines out of VersaWorks' runtime log
//! directory. Real impl tails a file over SMB; mock replays a canned
//! sequence.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use hanshi_domain::Timestamp;

use crate::error::PortResult;

/// One parsed entry from VW's log. `raw` preserves the line verbatim
/// for audit; `kind` is hanshi's best-effort classification.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VwLogEntry {
    pub observed_at: Timestamp,
    pub raw: String,
    pub kind: VwLogKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum VwLogKind {
    /// VW picked up a file from its input folder.
    JobIngested { filename: String },
    /// VW started ripping a job.
    RipStarted { filename: String },
    /// VW finished ripping; data is ready to send.
    RipFinished { filename: String },
    /// VW sent data to the printer.
    SentToPrinter { filename: String },
    /// VW reports the job completed.
    JobCompleted { filename: String },
    /// VW reports an error relating to a job.
    Error { filename: Option<String>, message: String },
    /// An entry we couldn't classify.
    Unknown,
}

#[async_trait]
pub trait VwLogReader: Send + Sync {
    /// Return any entries observed since the previous call. Entries
    /// accumulate in implementation-defined storage between polls; the
    /// caller drains them.
    async fn drain(&self) -> PortResult<Vec<VwLogEntry>>;

    /// Whether the underlying log source is reachable (e.g., SMB mount
    /// is healthy, log file exists).
    async fn is_reachable(&self) -> PortResult<bool>;
}
