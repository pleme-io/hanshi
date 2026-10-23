//! In-memory mock implementations.
//!
//! These let every downstream crate develop and test without needing a
//! real printer or Windows box. Each mock is configurable at
//! construction so tests can inject specific scenarios (printer offline,
//! VW stalled, ink low, etc.).

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;

use hanshi_domain::{now, InkChannel, InkLevel, JobId, PrinterObservation, PrinterState};
use sha2::Digest;

use crate::error::{PortError, PortResult};
use crate::hot_folder::{HotFolderWriter, SubmittedFile};
use crate::snmp::SnmpClient;
use crate::vw_log::{VwLogEntry, VwLogKind, VwLogReader};

// ── Mock SNMP ─────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct MockSnmpClient {
    state: Arc<Mutex<MockPrinterState>>,
}

struct MockPrinterState {
    printer: PrinterState,
    inks: Vec<InkLevel>,
    lamp_hours: Option<f64>,
    error_codes: Vec<String>,
    reachable: bool,
}

impl MockSnmpClient {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(MockPrinterState {
                printer: PrinterState::Idle,
                inks: default_inks(),
                lamp_hours: Some(0.0),
                error_codes: Vec::new(),
                reachable: true,
            })),
        }
    }

    /// Set the printer state. Used by test scenarios.
    pub async fn set_state(&self, s: PrinterState) {
        self.state.lock().await.printer = s;
    }

    pub async fn set_reachable(&self, v: bool) {
        self.state.lock().await.reachable = v;
    }

    pub async fn set_ink(&self, channel: InkChannel, percent: u8) {
        let mut s = self.state.lock().await;
        if let Some(slot) = s.inks.iter_mut().find(|i| i.channel == channel) {
            slot.percent = percent.min(100);
        } else {
            s.inks.push(InkLevel::new(channel, percent));
        }
    }

    pub async fn push_error_code(&self, code: impl Into<String>) {
        self.state.lock().await.error_codes.push(code.into());
    }

    pub async fn clear_errors(&self) {
        self.state.lock().await.error_codes.clear();
    }
}

impl Default for MockSnmpClient {
    fn default() -> Self {
        Self::new()
    }
}

fn default_inks() -> Vec<InkLevel> {
    vec![
        InkLevel::new(InkChannel::Cyan, 100),
        InkLevel::new(InkChannel::Magenta, 100),
        InkLevel::new(InkChannel::Yellow, 100),
        InkLevel::new(InkChannel::Black, 100),
        InkLevel::new(InkChannel::White, 100),
        InkLevel::new(InkChannel::Gloss, 100),
    ]
}

#[async_trait]
impl SnmpClient for MockSnmpClient {
    async fn snapshot(&self) -> PortResult<PrinterObservation> {
        let s = self.state.lock().await;
        if !s.reachable {
            return Err(PortError::unavailable("mock snmp unreachable"));
        }
        let mut obs = PrinterObservation::at(now(), s.printer.clone());
        obs.inks = s.inks.clone();
        obs.lamp_hours = s.lamp_hours;
        obs.error_codes = s.error_codes.clone();
        Ok(obs)
    }

    async fn is_reachable(&self) -> PortResult<bool> {
        Ok(self.state.lock().await.reachable)
    }
}

// ── Mock hot folder ───────────────────────────────────────────────────

#[derive(Clone)]
pub struct MockHotFolderWriter {
    inner: Arc<Mutex<HotFolderInner>>,
}

struct HotFolderInner {
    /// Files staged and not yet "consumed" by a simulated VW.
    present: Vec<SubmittedFile>,
    /// Files that have been consumed (cleared from the folder).
    consumed: Vec<SubmittedFile>,
}

impl MockHotFolderWriter {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HotFolderInner {
                present: Vec::new(),
                consumed: Vec::new(),
            })),
        }
    }

    /// Simulate VW consuming a staged file. Moves it from `present` to
    /// `consumed`. Returns `true` iff the file was present.
    pub async fn simulate_vw_consume(&self, file: &SubmittedFile) -> bool {
        let mut inner = self.inner.lock().await;
        if let Some(idx) = inner.present.iter().position(|f| f == file) {
            let taken = inner.present.remove(idx);
            inner.consumed.push(taken);
            true
        } else {
            false
        }
    }

    /// Consume all presently-staged files.
    pub async fn simulate_vw_consume_all(&self) -> usize {
        let mut inner = self.inner.lock().await;
        let count = inner.present.len();
        let taken = std::mem::take(&mut inner.present);
        inner.consumed.extend(taken);
        count
    }

    pub async fn consumed_count(&self) -> usize {
        self.inner.lock().await.consumed.len()
    }
}

impl Default for MockHotFolderWriter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl HotFolderWriter for MockHotFolderWriter {
    async fn stage_pdf(
        &self,
        job_id: JobId,
        bytes: &[u8],
        filename_hint: Option<&str>,
    ) -> PortResult<SubmittedFile> {
        let mut hasher = sha2::Sha256::new();
        hasher.update(bytes);
        let sha = hex::encode(hasher.finalize());
        let relative_path = match filename_hint {
            Some(h) => format!("{job_id}-{h}"),
            None => format!("{job_id}.pdf"),
        };
        let file = SubmittedFile {
            job_id,
            relative_path,
            bytes: bytes.len() as u64,
            sha256_hex: sha,
            written_at: now(),
        };
        self.inner.lock().await.present.push(file.clone());
        Ok(file)
    }

    async fn is_present(&self, submitted: &SubmittedFile) -> PortResult<bool> {
        Ok(self.inner.lock().await.present.iter().any(|f| f == submitted))
    }

    async fn queue_depth(&self) -> PortResult<usize> {
        Ok(self.inner.lock().await.present.len())
    }
}

// ── Mock VW log ───────────────────────────────────────────────────────

#[derive(Clone)]
pub struct MockVwLogReader {
    inner: Arc<Mutex<Vec<VwLogEntry>>>,
}

impl MockVwLogReader {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Queue an entry for the next `drain()` call.
    pub async fn push(&self, kind: VwLogKind, raw: impl Into<String>) {
        self.inner.lock().await.push(VwLogEntry {
            observed_at: now(),
            raw: raw.into(),
            kind,
        });
    }
}

impl Default for MockVwLogReader {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl VwLogReader for MockVwLogReader {
    async fn drain(&self) -> PortResult<Vec<VwLogEntry>> {
        let mut inner = self.inner.lock().await;
        Ok(std::mem::take(&mut *inner))
    }

    async fn is_reachable(&self) -> PortResult<bool> {
        Ok(true)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn snmp_mock_defaults_to_idle_and_reachable() {
        let c = MockSnmpClient::new();
        assert!(c.is_reachable().await.unwrap());
        let obs = c.snapshot().await.unwrap();
        assert_eq!(obs.state, PrinterState::Idle);
        assert!(obs.inks.iter().all(|i| i.percent == 100));
    }

    #[tokio::test]
    async fn snmp_mock_reports_unreachable_when_flagged() {
        let c = MockSnmpClient::new();
        c.set_reachable(false).await;
        assert!(!c.is_reachable().await.unwrap());
        assert!(c.snapshot().await.is_err());
    }

    #[tokio::test]
    async fn snmp_mock_state_transition_visible() {
        let c = MockSnmpClient::new();
        c.set_state(PrinterState::Printing).await;
        assert_eq!(c.snapshot().await.unwrap().state, PrinterState::Printing);
    }

    #[tokio::test]
    async fn snmp_mock_ink_override_visible() {
        let c = MockSnmpClient::new();
        c.set_ink(InkChannel::White, 5).await;
        let obs = c.snapshot().await.unwrap();
        let w = obs.inks.iter().find(|i| i.channel == InkChannel::White).unwrap();
        assert_eq!(w.percent, 5);
    }

    #[tokio::test]
    async fn hot_folder_mock_stages_and_consumes() {
        let w = MockHotFolderWriter::new();
        let j = JobId::new();
        let f = w.stage_pdf(j, b"pdf bytes", None).await.unwrap();
        assert_eq!(w.queue_depth().await.unwrap(), 1);
        assert!(w.is_present(&f).await.unwrap());
        assert!(w.simulate_vw_consume(&f).await);
        assert_eq!(w.queue_depth().await.unwrap(), 0);
        assert!(!w.is_present(&f).await.unwrap());
        assert_eq!(w.consumed_count().await, 1);
    }

    #[tokio::test]
    async fn hot_folder_mock_sha_matches_content() {
        let w = MockHotFolderWriter::new();
        let j = JobId::new();
        let f = w.stage_pdf(j, b"abc", None).await.unwrap();
        // sha256("abc") = ba7816bf...
        assert!(f.sha256_hex.starts_with("ba7816bf"));
    }

    #[tokio::test]
    async fn vw_log_mock_drain_returns_queued_entries() {
        let r = MockVwLogReader::new();
        r.push(
            VwLogKind::JobIngested {
                filename: "x.pdf".into(),
            },
            "VW: ingested x.pdf",
        )
        .await;
        let drained = r.drain().await.unwrap();
        assert_eq!(drained.len(), 1);
        // Subsequent drain returns nothing.
        assert!(r.drain().await.unwrap().is_empty());
    }
}
