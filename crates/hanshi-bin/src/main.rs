//! hanshi CLI entry point.
//!
//! Reads a small TOML config (or falls back to env vars) and starts:
//!   * the coordinator tick loop
//!   * the axum HTTP admin API (bound to the configured address)
//!
//! In v1 all ports are wired to mocks by default — the binary is useful
//! for development without hardware. A future revision swaps real
//! impls (SNMP client, SMB hot folder writer, VW log tailer) in based
//! on the config.

use std::net::SocketAddr;
use std::sync::Arc;

use serde::Deserialize;
use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use hanshi_core::{Coordinator, CoordinatorConfig, MemoryEventLog, MemoryJobStore, RuleEngine};
use hanshi_ports::{MockHotFolderWriter, MockSnmpClient, MockVwLogReader};
use hanshi_server::{make_router, spawn_tick_loop, AppState, TickConfig};

#[derive(Debug, Deserialize)]
struct Config {
    #[serde(default = "default_listen")]
    listen: String,
    #[serde(default)]
    tick_interval_ms: Option<u64>,
    #[serde(default)]
    stuck_threshold_secs: Option<u64>,
    #[serde(default)]
    ink_low_threshold_percent: Option<u8>,
}

fn default_listen() -> String {
    "127.0.0.1:8085".into()
}

fn load_config() -> Config {
    if let Ok(path) = std::env::var("HANSHI_CONFIG") {
        match std::fs::read_to_string(&path) {
            Ok(text) => match toml::from_str(&text) {
                Ok(cfg) => return cfg,
                Err(e) => eprintln!("HANSHI_CONFIG invalid: {e}"),
            },
            Err(e) => eprintln!("HANSHI_CONFIG unreadable ({path}): {e}"),
        }
    }
    Config {
        listen: std::env::var("HANSHI_LISTEN").unwrap_or_else(|_| default_listen()),
        tick_interval_ms: None,
        stuck_threshold_secs: None,
        ink_low_threshold_percent: None,
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_target(true).with_thread_ids(false))
        .init();
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing();
    let cfg = load_config();
    info!(?cfg, "hanshi starting");

    let rules = RuleEngine::new();
    let rules = match cfg.ink_low_threshold_percent {
        Some(p) => rules.with_ink_threshold(p),
        None => rules,
    };
    let rules = match cfg.stuck_threshold_secs {
        Some(s) => rules.with_stuck_threshold(s),
        None => rules,
    };

    let coordinator = Arc::new(Coordinator {
        jobs: Arc::new(MemoryJobStore::new()),
        log: Arc::new(MemoryEventLog::new()),
        snmp: Arc::new(MockSnmpClient::new()),
        hot: Arc::new(MockHotFolderWriter::new()),
        vw_log: Arc::new(MockVwLogReader::new()),
        rules: Arc::new(rules),
        config: CoordinatorConfig::default(),
    });

    let tick_config = TickConfig {
        interval: std::time::Duration::from_millis(cfg.tick_interval_ms.unwrap_or(3000)),
    };
    let _tick_handle = spawn_tick_loop(Arc::clone(&coordinator), tick_config);

    let app = make_router(AppState {
        coordinator: Arc::clone(&coordinator),
    });

    let addr: SocketAddr = cfg.listen.parse()?;
    info!(%addr, "hanshi listening");
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
