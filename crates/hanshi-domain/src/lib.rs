//! hanshi-domain — typed domain model for the hanshi UV-print orchestrator.
//!
//! Pure Rust, no IO. Every other hanshi crate depends on these types but
//! nothing in this crate touches the filesystem, network, or SQLite.
//! Invariants that can be enforced at construction time are enforced
//! at construction time.
//!
//! # Layering
//!
//! ```text
//!          hanshi-bin
//!             │
//!          hanshi-server  ← axum + web UI
//!             │
//!          hanshi-core    ← business logic, state machine runtime
//!             │
//!          hanshi-ports   ← traits (SnmpClient, HotFolderWriter, …)
//!             │       ├── hanshi-snmp      (real SNMP impl)
//!             │       ├── hanshi-hotfolder (SMB writer impl)
//!             │       └── hanshi-vwlog     (VW log tailer impl)
//!             │
//!          hanshi-domain  ← THIS CRATE — types only
//! ```

pub mod advisory;
pub mod error;
pub mod event;
pub mod ids;
pub mod job;
pub mod material;
pub mod printer;
pub mod profile;

pub use advisory::{Advisory, AdvisoryRef, Severity};
pub use error::{DomainError, Result};
pub use event::{JobEvent, JobEventKind};
pub use ids::{AdvisoryId, CorrelationId, EventId, JobId, MaterialId, ProfileId};
pub use job::{Job, JobState};
pub use material::{InkChannel, Material, Substrate};
pub use printer::{InkLevel, PrinterObservation, PrinterState};
pub use profile::{GlossMode, PassPlan, PrintProfile, WhiteInkMode};

/// Timestamp type used throughout the domain — UTC, serde-compatible.
pub type Timestamp = chrono::DateTime<chrono::Utc>;

/// Current time helper — thin wrapper so tests can mock by monkey-patching
/// at a higher layer if needed.
pub fn now() -> Timestamp {
    chrono::Utc::now()
}
