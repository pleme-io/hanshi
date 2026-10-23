//! SNMP port — query printer state from the LEF2-300.
//!
//! The trait is deliberately coarse: callers ask for a `PrinterObservation`
//! (what hanshi needs) rather than raw OIDs. The implementation maps the
//! standard Printer-MIB (RFC 3805) + whatever private Roland OIDs apply
//! to a single structured snapshot.

use async_trait::async_trait;

use hanshi_domain::PrinterObservation;

use crate::error::PortResult;

#[async_trait]
pub trait SnmpClient: Send + Sync {
    /// Take a single snapshot of the printer's observable state.
    async fn snapshot(&self) -> PortResult<PrinterObservation>;

    /// Is the printer reachable at all? Quick health check — may be
    /// answered by anything from a single ping to a trivial OID read
    /// depending on the implementation.
    async fn is_reachable(&self) -> PortResult<bool>;
}
