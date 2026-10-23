//! Admin HTTP API.
//!
//! Routes:
//!   GET    /health                     — liveness
//!   GET    /status                     — printer + queue + advisory summary
//!   POST   /jobs                       — submit a job
//!   GET    /jobs                       — list jobs
//!   GET    /jobs/:id                   — single job + event timeline
//!   POST   /jobs/:id/stage             — record hot-folder stage
//!   POST   /jobs/:id/qc                — record QC pass/fail
//!   GET    /advisories                 — all fired advisories
//!   GET    /events/recent?limit=       — recent events across jobs

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tower_http::trace::TraceLayer;
use tracing::instrument;

use hanshi_domain::{
    now, Advisory, Job, JobEvent, JobId, JobState, MaterialId, PrinterObservation, ProfileId,
};
use hanshi_ports::{HotFolderWriter, SnmpClient, SubmittedFile, VwLogReader};

use crate::state::AppState;

pub fn make_router<S, L, N, H, V>(state: AppState<S, L, N, H, V>) -> Router
where
    S: hanshi_core::JobStore + 'static,
    L: hanshi_core::EventLog + 'static,
    N: SnmpClient + 'static,
    H: HotFolderWriter + 'static,
    V: VwLogReader + 'static,
{
    Router::new()
        .route("/health", get(health::<S, L, N, H, V>))
        .route("/status", get(status::<S, L, N, H, V>))
        .route("/jobs", get(list_jobs::<S, L, N, H, V>).post(submit_job::<S, L, N, H, V>))
        .route("/jobs/:id", get(get_job::<S, L, N, H, V>))
        .route("/jobs/:id/stage", post(stage_job::<S, L, N, H, V>))
        .route("/jobs/:id/qc", post(record_qc::<S, L, N, H, V>))
        .route("/advisories", get(advisories::<S, L, N, H, V>))
        .route("/events/recent", get(recent_events::<S, L, N, H, V>))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

// ── Response types ────────────────────────────────────────────────────

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

#[derive(Serialize)]
struct StatusResponse {
    printer: Option<PrinterObservation>,
    queue_depth: Option<usize>,
    jobs_in_flight: usize,
    advisories: Vec<Advisory>,
}

#[derive(Serialize)]
struct JobDetail {
    job: Job,
    events: Vec<JobEvent>,
}

#[derive(Deserialize)]
struct SubmitJobRequest {
    name: String,
    material: MaterialId,
    profile: ProfileId,
    #[serde(default = "default_copies")]
    copies: u32,
    #[serde(default)]
    notes: Option<String>,
}

fn default_copies() -> u32 {
    1
}

#[derive(Deserialize)]
struct StageRequest {
    /// Base64-encoded PDF content. Decoded server-side before staging.
    #[serde(with = "base64_bytes")]
    pdf: Vec<u8>,
    #[serde(default)]
    filename_hint: Option<String>,
}

#[derive(Deserialize)]
struct QcRequest {
    passed: bool,
    #[serde(default)]
    notes: Option<String>,
}

#[derive(Deserialize)]
struct RecentEventsQuery {
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    50
}

// ── Handlers ──────────────────────────────────────────────────────────

#[instrument(skip_all)]
async fn health<S, L, N, H, V>() -> impl IntoResponse
where
    S: hanshi_core::JobStore + 'static,
    L: hanshi_core::EventLog + 'static,
    N: SnmpClient + 'static,
    H: HotFolderWriter + 'static,
    V: VwLogReader + 'static,
{
    Json(HealthResponse { status: "ok" })
}

#[instrument(skip_all)]
async fn status<S, L, N, H, V>(State(s): State<AppState<S, L, N, H, V>>) -> impl IntoResponse
where
    S: hanshi_core::JobStore + 'static,
    L: hanshi_core::EventLog + 'static,
    N: SnmpClient + 'static,
    H: HotFolderWriter + 'static,
    V: VwLogReader + 'static,
{
    let printer = s.coordinator.snmp.snapshot().await.ok();
    let queue_depth = s.coordinator.hot.queue_depth().await.ok();
    let in_flight = s.coordinator.jobs.list_in_flight().await.map(|v| v.len()).unwrap_or(0);
    let advisories = s.coordinator.log.advisories().await.unwrap_or_default();
    Json(StatusResponse {
        printer,
        queue_depth,
        jobs_in_flight: in_flight,
        advisories,
    })
}

#[instrument(skip_all)]
async fn submit_job<S, L, N, H, V>(
    State(s): State<AppState<S, L, N, H, V>>,
    Json(req): Json<SubmitJobRequest>,
) -> Result<Json<Job>, ApiError>
where
    S: hanshi_core::JobStore + 'static,
    L: hanshi_core::EventLog + 'static,
    N: SnmpClient + 'static,
    H: HotFolderWriter + 'static,
    V: VwLogReader + 'static,
{
    let mut job = Job::new(req.name, req.material, req.profile, req.copies, now())
        .map_err(ApiError::from_domain)?;
    job.notes = req.notes;
    let submitted = s.coordinator.submit(job).await.map_err(ApiError::from_core)?;
    Ok(Json(submitted))
}

#[instrument(skip_all)]
async fn list_jobs<S, L, N, H, V>(
    State(s): State<AppState<S, L, N, H, V>>,
) -> Result<Json<Vec<Job>>, ApiError>
where
    S: hanshi_core::JobStore + 'static,
    L: hanshi_core::EventLog + 'static,
    N: SnmpClient + 'static,
    H: HotFolderWriter + 'static,
    V: VwLogReader + 'static,
{
    Ok(Json(
        s.coordinator.jobs.list().await.map_err(ApiError::from_core)?,
    ))
}

#[instrument(skip_all, fields(job_id = %id))]
async fn get_job<S, L, N, H, V>(
    State(s): State<AppState<S, L, N, H, V>>,
    Path(id): Path<JobId>,
) -> Result<Json<JobDetail>, ApiError>
where
    S: hanshi_core::JobStore + 'static,
    L: hanshi_core::EventLog + 'static,
    N: SnmpClient + 'static,
    H: HotFolderWriter + 'static,
    V: VwLogReader + 'static,
{
    let job = s.coordinator.jobs.get(id).await.map_err(ApiError::from_core)?;
    let events = s
        .coordinator
        .log
        .for_job(id)
        .await
        .map_err(ApiError::from_core)?;
    Ok(Json(JobDetail { job, events }))
}

#[instrument(skip_all, fields(job_id = %id))]
async fn stage_job<S, L, N, H, V>(
    State(s): State<AppState<S, L, N, H, V>>,
    Path(id): Path<JobId>,
    Json(req): Json<StageRequest>,
) -> Result<Json<SubmittedFile>, ApiError>
where
    S: hanshi_core::JobStore + 'static,
    L: hanshi_core::EventLog + 'static,
    N: SnmpClient + 'static,
    H: HotFolderWriter + 'static,
    V: VwLogReader + 'static,
{
    let file = s
        .coordinator
        .hot
        .stage_pdf(id, &req.pdf, req.filename_hint.as_deref())
        .await
        .map_err(ApiError::from_port)?;
    s.coordinator
        .mark_staged(id, &file)
        .await
        .map_err(ApiError::from_core)?;
    Ok(Json(file))
}

#[instrument(skip_all, fields(job_id = %id))]
async fn record_qc<S, L, N, H, V>(
    State(s): State<AppState<S, L, N, H, V>>,
    Path(id): Path<JobId>,
    Json(req): Json<QcRequest>,
) -> Result<StatusCode, ApiError>
where
    S: hanshi_core::JobStore + 'static,
    L: hanshi_core::EventLog + 'static,
    N: SnmpClient + 'static,
    H: HotFolderWriter + 'static,
    V: VwLogReader + 'static,
{
    let mut job = s
        .coordinator
        .jobs
        .get(id)
        .await
        .map_err(ApiError::from_core)?;
    let next = if req.passed {
        JobState::PassedQc
    } else {
        JobState::FailedQc
    };
    if !job.state.can_transition_to(next) {
        return Err(ApiError::status(
            StatusCode::CONFLICT,
            format!("job is in state {:?}, cannot record QC", job.state),
        ));
    }
    let from = job.state;
    job.advance(next, now()).map_err(ApiError::from_domain)?;
    s.coordinator
        .jobs
        .update(job.clone())
        .await
        .map_err(ApiError::from_core)?;
    s.coordinator
        .log
        .append(JobEvent::new(
            job.id,
            now(),
            hanshi_domain::JobEventKind::QcRecorded {
                passed: req.passed,
                notes: req.notes.clone(),
            },
        ))
        .await
        .map_err(ApiError::from_core)?;
    s.coordinator
        .log
        .append(JobEvent::new(
            job.id,
            now(),
            hanshi_domain::JobEventKind::StateTransition {
                from,
                to: next,
                reason: req.notes,
            },
        ))
        .await
        .map_err(ApiError::from_core)?;
    Ok(StatusCode::NO_CONTENT)
}

#[instrument(skip_all)]
async fn advisories<S, L, N, H, V>(
    State(s): State<AppState<S, L, N, H, V>>,
) -> Result<Json<Vec<Advisory>>, ApiError>
where
    S: hanshi_core::JobStore + 'static,
    L: hanshi_core::EventLog + 'static,
    N: SnmpClient + 'static,
    H: HotFolderWriter + 'static,
    V: VwLogReader + 'static,
{
    Ok(Json(
        s.coordinator.log.advisories().await.map_err(ApiError::from_core)?,
    ))
}

#[instrument(skip_all)]
async fn recent_events<S, L, N, H, V>(
    State(s): State<AppState<S, L, N, H, V>>,
    Query(q): Query<RecentEventsQuery>,
) -> Result<Json<Vec<JobEvent>>, ApiError>
where
    S: hanshi_core::JobStore + 'static,
    L: hanshi_core::EventLog + 'static,
    N: SnmpClient + 'static,
    H: HotFolderWriter + 'static,
    V: VwLogReader + 'static,
{
    Ok(Json(
        s.coordinator
            .log
            .recent(q.limit)
            .await
            .map_err(ApiError::from_core)?,
    ))
}

// ── Error → response mapping ──────────────────────────────────────────

struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn status(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    fn from_domain(e: hanshi_domain::DomainError) -> Self {
        Self::status(StatusCode::BAD_REQUEST, e.to_string())
    }

    fn from_core(e: hanshi_core::CoreError) -> Self {
        let status = match &e {
            hanshi_core::CoreError::NotFound { .. } => StatusCode::NOT_FOUND,
            hanshi_core::CoreError::Domain(_) => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        Self::status(status, e.to_string())
    }

    fn from_port(e: hanshi_ports::PortError) -> Self {
        Self::status(StatusCode::BAD_GATEWAY, e.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (
            self.status,
            Json(serde_json::json!({ "error": self.message })),
        )
            .into_response()
    }
}

// ── Minimal base64 codec for the stage endpoint ──────────────────────
//
// Kept inline to avoid a dependency; the hot-folder stage body is the
// only place the API accepts binary blobs.
mod base64_bytes {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    const CHARSET: &[u8] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        encode(bytes).serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        decode(&s).map_err(serde::de::Error::custom)
    }

    fn encode(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
        for chunk in bytes.chunks(3) {
            let b0 = chunk[0];
            let b1 = chunk.get(1).copied().unwrap_or(0);
            let b2 = chunk.get(2).copied().unwrap_or(0);
            out.push(CHARSET[(b0 >> 2) as usize] as char);
            out.push(CHARSET[((b0 & 0x03) << 4 | b1 >> 4) as usize] as char);
            if chunk.len() >= 2 {
                out.push(CHARSET[((b1 & 0x0f) << 2 | b2 >> 6) as usize] as char);
            } else {
                out.push('=');
            }
            if chunk.len() >= 3 {
                out.push(CHARSET[(b2 & 0x3f) as usize] as char);
            } else {
                out.push('=');
            }
        }
        out
    }

    fn decode(s: &str) -> Result<Vec<u8>, String> {
        let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
        if !s.len().is_multiple_of(4) {
            return Err(format!("base64 length {} is not a multiple of 4", s.len()));
        }
        let mut out = Vec::with_capacity(s.len() / 4 * 3);
        for chunk in s.as_bytes().chunks(4) {
            let vals: [i16; 4] = [
                decode_char(chunk[0])?,
                decode_char(chunk[1])?,
                decode_char(chunk[2])?,
                decode_char(chunk[3])?,
            ];
            out.push(((vals[0] as u16) << 2 | ((vals[1] as u16) >> 4)) as u8);
            if vals[2] >= 0 {
                out.push((((vals[1] as u16) & 0x0f) << 4 | ((vals[2] as u16) >> 2)) as u8);
            }
            if vals[3] >= 0 {
                out.push((((vals[2] as u16) & 0x03) << 6 | (vals[3] as u16)) as u8);
            }
        }
        Ok(out)
    }

    fn decode_char(c: u8) -> Result<i16, String> {
        if c == b'=' {
            return Ok(-1);
        }
        match c {
            b'A'..=b'Z' => Ok((c - b'A') as i16),
            b'a'..=b'z' => Ok((c - b'a' + 26) as i16),
            b'0'..=b'9' => Ok((c - b'0' + 52) as i16),
            b'+' => Ok(62),
            b'/' => Ok(63),
            other => Err(format!("invalid base64 char: {:?}", other as char)),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn round_trip() {
            for input in [&b""[..], b"a", b"ab", b"abc", b"hello world"] {
                let encoded = encode(input);
                let decoded = decode(&encoded).unwrap();
                assert_eq!(decoded, input);
            }
        }
    }
}
