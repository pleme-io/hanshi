//! hanshi-server — HTTP admin API + coordinator tick loop.
//!
//! The router is generic over the concrete port types so the binary
//! can wire real impls (SNMP, SMB hot folder, VW log tailer) while
//! tests can wire mocks. Every admin endpoint is idempotent where
//! possible and emits tracing events with the incoming request's
//! correlation fields.

pub mod api;
pub mod state;
pub mod tick;

pub use api::make_router;
pub use state::AppState;
pub use tick::{spawn_tick_loop, TickConfig};
