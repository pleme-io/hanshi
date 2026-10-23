//! hanshi-ports — traits for every external surface hanshi talks to,
//! plus in-memory mock implementations.
//!
//! Real implementations live in their own crates (future
//! `hanshi-snmp`, `hanshi-hotfolder`, `hanshi-vwlog`). The mocks here
//! are the dev-time counterpart: they let the rest of hanshi develop
//! and test without a real printer or Windows box.
//!
//! Each port trait is `async` via `async-trait` so real impls can do
//! network / disk IO without blocking the tokio runtime. Mocks
//! implement the same trait, just fulfilling it from in-memory
//! state.
//!
//! # Ports
//!
//! - `SnmpClient` — query printer status via SNMP
//! - `HotFolderWriter` — drop files into VW's watched folder
//! - `VwLogReader` — stream new lines from VW's log file

pub mod error;
pub mod hot_folder;
pub mod mocks;
pub mod snmp;
pub mod vw_log;

pub use error::{PortError, PortResult};
pub use hot_folder::{HotFolderWriter, SubmittedFile};
pub use snmp::SnmpClient;
pub use vw_log::{VwLogEntry, VwLogKind, VwLogReader};

pub use mocks::{MockHotFolderWriter, MockSnmpClient, MockVwLogReader};
