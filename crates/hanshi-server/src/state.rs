//! Shared application state handed to every axum handler.

use std::sync::Arc;

use hanshi_core::Coordinator;
use hanshi_ports::{HotFolderWriter, SnmpClient, VwLogReader};

pub struct AppState<S, L, N, H, V>
where
    S: hanshi_core::JobStore + 'static,
    L: hanshi_core::EventLog + 'static,
    N: SnmpClient + 'static,
    H: HotFolderWriter + 'static,
    V: VwLogReader + 'static,
{
    pub coordinator: Arc<Coordinator<S, L, N, H, V>>,
}

impl<S, L, N, H, V> Clone for AppState<S, L, N, H, V>
where
    S: hanshi_core::JobStore + 'static,
    L: hanshi_core::EventLog + 'static,
    N: SnmpClient + 'static,
    H: HotFolderWriter + 'static,
    V: VwLogReader + 'static,
{
    fn clone(&self) -> Self {
        Self {
            coordinator: Arc::clone(&self.coordinator),
        }
    }
}
