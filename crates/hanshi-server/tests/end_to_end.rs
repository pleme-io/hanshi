//! End-to-end test: wire real Coordinator + mock ports + axum router
//! and drive a synthetic job through the full lifecycle, verifying
//! every state transition and advisory firing.
//!
//! This is the Phase 9 dry-run harness — it proves the pipeline works
//! end to end against in-memory ports so only the real IO layer needs
//! hardware validation.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;

use hanshi_core::{
    Coordinator, CoordinatorConfig, JobStore, MemoryEventLog, MemoryJobStore, RuleEngine,
};
use hanshi_domain::{InkChannel, Job, JobState, MaterialId, PrinterState, ProfileId};
use hanshi_ports::{
    HotFolderWriter, MockHotFolderWriter, MockSnmpClient, MockVwLogReader, VwLogKind,
};
use hanshi_server::{make_router, AppState};

type Coord = Coordinator<
    MemoryJobStore,
    MemoryEventLog,
    MockSnmpClient,
    MockHotFolderWriter,
    MockVwLogReader,
>;

fn build() -> (Arc<Coord>, axum::Router) {
    let coord = Arc::new(Coordinator {
        jobs: Arc::new(MemoryJobStore::new()),
        log: Arc::new(MemoryEventLog::new()),
        snmp: Arc::new(MockSnmpClient::new()),
        hot: Arc::new(MockHotFolderWriter::new()),
        vw_log: Arc::new(MockVwLogReader::new()),
        rules: Arc::new(RuleEngine::new()),
        config: CoordinatorConfig::default(),
    });
    let router = make_router(AppState {
        coordinator: Arc::clone(&coord),
    });
    (coord, router)
}

async fn send_json(
    router: &axum::Router,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(path);
    let req = match body {
        Some(v) => {
            builder = builder.header("content-type", "application/json");
            builder.body(Body::from(serde_json::to_vec(&v).unwrap())).unwrap()
        }
        None => builder.body(Body::empty()).unwrap(),
    };
    let res = router.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let body_bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let parsed: Value = if body_bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body_bytes).unwrap_or(Value::Null)
    };
    (status, parsed)
}

#[tokio::test]
async fn health_endpoint_responds_ok() {
    let (_coord, router) = build();
    let (status, body) = send_json(&router, "GET", "/health", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
async fn submit_and_list_jobs_via_http() {
    let (_coord, router) = build();
    let (status, created) = send_json(
        &router,
        "POST",
        "/jobs",
        Some(json!({
            "name": "banner-42",
            "material": MaterialId::new(),
            "profile": ProfileId::new(),
            "copies": 1,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let job_id = created["id"].clone();
    assert!(!job_id.is_null());

    let (status, listed) = send_json(&router, "GET", "/jobs", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["id"], job_id);
}

#[tokio::test]
async fn submit_rejects_empty_name() {
    let (_coord, router) = build();
    let (status, _) = send_json(
        &router,
        "POST",
        "/jobs",
        Some(json!({
            "name": "",
            "material": MaterialId::new(),
            "profile": ProfileId::new(),
            "copies": 1,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn status_endpoint_reports_printer_state() {
    let (coord, router) = build();
    coord.snmp.set_state(PrinterState::Printing).await;
    let (status, body) = send_json(&router, "GET", "/status", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["printer"]["state"]["kind"], "printing");
}

#[tokio::test]
async fn full_lifecycle_end_to_end_via_coordinator_and_router() {
    let (coord, router) = build();

    // 1) Submit
    let job = Job::new(
        "end-to-end",
        MaterialId::new(),
        ProfileId::new(),
        1,
        hanshi_domain::now(),
    )
    .unwrap();
    let id = job.id;
    coord.submit(job).await.unwrap();

    // 2) Stage a PDF (hot folder)
    let file = coord.hot.stage_pdf(id, b"pdf", None).await.unwrap();
    coord.mark_staged(id, &file).await.unwrap();
    assert_eq!(coord.jobs.get(id).await.unwrap().state, JobState::Staged);

    // 3) VW ingests -> AcceptedByVw
    coord
        .vw_log
        .push(
            VwLogKind::JobIngested {
                filename: format!("{id}-file.pdf"),
            },
            "VW: ingested",
        )
        .await;
    coord.step().await.unwrap();
    assert_eq!(
        coord.jobs.get(id).await.unwrap().state,
        JobState::AcceptedByVw
    );

    // 4) Printer transitions Waiting → Ripping, then Printing → Sent/Printing
    coord.snmp.set_state(PrinterState::Waiting).await;
    coord.step().await.unwrap();
    assert_eq!(coord.jobs.get(id).await.unwrap().state, JobState::Ripping);

    // 5) Sent happens on next waiting/printing SNMP
    coord.snmp.set_state(PrinterState::Printing).await;
    coord.step().await.unwrap();
    let s = coord.jobs.get(id).await.unwrap().state;
    assert!(matches!(s, JobState::Sent | JobState::Printing));
    // Force on if still Sent
    if s == JobState::Sent {
        coord.step().await.unwrap();
    }
    assert_eq!(coord.jobs.get(id).await.unwrap().state, JobState::Printing);

    // 6) Printer returns to Idle without errors → Completed
    coord.snmp.set_state(PrinterState::Idle).await;
    coord.step().await.unwrap();
    assert_eq!(
        coord.jobs.get(id).await.unwrap().state,
        JobState::Completed
    );

    // 7) Record QC pass via HTTP
    let (status, _) = send_json(
        &router,
        "POST",
        &format!("/jobs/{id}/qc"),
        Some(json!({ "passed": true })),
    )
    .await;
    // qc_record: must be QcPending before PassedQc; our pipeline went
    // Completed -> (not QcPending) so the endpoint rightly refuses.
    assert_eq!(status, StatusCode::CONFLICT);

    // Advance through QcPending via advance() directly to prove the
    // transition works end-to-end.
    let mut job = coord.jobs.get(id).await.unwrap();
    job.advance(JobState::QcPending, hanshi_domain::now())
        .unwrap();
    coord.jobs.update(job).await.unwrap();
    let (status, _) = send_json(
        &router,
        "POST",
        &format!("/jobs/{id}/qc"),
        Some(json!({ "passed": true })),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(coord.jobs.get(id).await.unwrap().state, JobState::PassedQc);
}

#[tokio::test]
async fn ink_low_observation_surfaces_as_advisory() {
    let (coord, router) = build();
    coord.snmp.set_ink(InkChannel::Cyan, 3).await;
    coord.step().await.unwrap();
    let (status, advisories) = send_json(&router, "GET", "/advisories", None).await;
    assert_eq!(status, StatusCode::OK);
    let arr = advisories.as_array().unwrap();
    assert!(arr.iter().any(|a| a["rule"] == "ink-low"));
}

#[tokio::test]
async fn printer_error_surfaces_as_advisory() {
    let (coord, router) = build();
    coord
        .snmp
        .set_state(PrinterState::Error {
            reason: "cover open".into(),
        })
        .await;
    coord.step().await.unwrap();
    let (_, advisories) = send_json(&router, "GET", "/advisories", None).await;
    let arr = advisories.as_array().unwrap();
    assert!(arr.iter().any(|a| a["rule"] == "printer-error"));
}

#[tokio::test]
async fn recent_events_respects_limit() {
    let (coord, router) = build();
    // submit two jobs; each generates 1 event
    for i in 0..2 {
        coord
            .submit(
                Job::new(
                    format!("job-{i}"),
                    MaterialId::new(),
                    ProfileId::new(),
                    1,
                    hanshi_domain::now(),
                )
                .unwrap(),
            )
            .await
            .unwrap();
    }
    let (_, body) = send_json(&router, "GET", "/events/recent?limit=1", None).await;
    assert_eq!(body.as_array().unwrap().len(), 1);
}
