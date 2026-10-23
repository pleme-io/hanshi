//! hanshi-core — business logic.
//!
//! Owns:
//!   - the job queue and event log (in-memory; pluggable for SQLite later)
//!   - the state-machine runtime that drives jobs forward from port
//!     observations (SNMP snapshots, VW log entries, hot folder state)
//!   - the rule engine / advisor (blue rule bundle → advisories)
//!
//! Does not own:
//!   - the HTTP surface (`hanshi-server`)
//!   - any external IO impls (`hanshi-ports` traits + mocks handle that)
//!   - persistence beyond in-memory (future `hanshi-storage` crate)

pub mod coordinator;
pub mod blue_rules;
pub mod error;
pub mod rules;
pub mod store;

pub use coordinator::{Coordinator, CoordinatorConfig, StepReport};
pub use error::{CoreError, CoreResult};
pub use blue_rules::{BlueRules, RuleHost, UnknownSeverity};
pub use rules::{RuleEngine, RuleOutcome};
pub use store::{EventLog, JobStore, MemoryEventLog, MemoryJobStore};
