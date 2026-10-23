//! Background tick loop — drives `Coordinator::step` on a fixed interval.

use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinHandle;
use tracing::{debug, error, info};

use hanshi_core::Coordinator;
use hanshi_ports::{HotFolderWriter, SnmpClient, VwLogReader};

#[derive(Clone, Debug)]
pub struct TickConfig {
    pub interval: Duration,
}

impl Default for TickConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(3),
        }
    }
}

/// Spawn the background loop on the current tokio runtime. Returns the
/// handle so the caller can join or abort on shutdown.
pub fn spawn_tick_loop<S, L, N, H, V>(
    coordinator: Arc<Coordinator<S, L, N, H, V>>,
    config: TickConfig,
) -> JoinHandle<()>
where
    S: hanshi_core::JobStore + 'static,
    L: hanshi_core::EventLog + 'static,
    N: SnmpClient + 'static,
    H: HotFolderWriter + 'static,
    V: VwLogReader + 'static,
{
    tokio::spawn(async move {
        info!(interval_ms = ?config.interval.as_millis(), "hanshi tick loop running");
        let mut interval = tokio::time::interval(config.interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            match coordinator.step().await {
                Ok(report) => {
                    debug!(
                        advisories = report.advisories_fired,
                        vw_lines = report.vw_lines_drained,
                        "tick complete"
                    );
                }
                Err(e) => {
                    error!(error = %e, "tick error");
                }
            }
        }
    })
}
