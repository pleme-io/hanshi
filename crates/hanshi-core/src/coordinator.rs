//! State-machine runtime — correlates external signals (SNMP, VW log,
//! hot folder depth) with in-flight jobs and drives their state
//! transitions forward.
//!
//! The coordinator is *stateless between steps*: each `step()` reads
//! fresh observations, compares them to the current job store, writes
//! events + advisories, and returns a report. Wrap it in a tokio loop
//! in the binary to drive continuously.

use std::sync::Arc;
use std::time::Duration;

use tracing::{debug, info, instrument, warn};

use hanshi_domain::{
    now, Job, JobEvent, JobEventKind, JobId, JobState, PrinterObservation, PrinterState,
    Timestamp,
};
use hanshi_ports::{HotFolderWriter, SnmpClient, SubmittedFile, VwLogReader};

use crate::error::CoreResult;
use crate::rules::{RuleEngine, RuleInputs};
use crate::store::{EventLog, JobStore};

/// Summary of what one coordinator step did.
#[derive(Clone, Debug, Default)]
pub struct StepReport {
    pub observation: Option<PrinterObservation>,
    pub vw_lines_drained: usize,
    pub advisories_fired: usize,
    pub transitions_applied: usize,
}

pub struct Coordinator<S, L, N, H, V>
where
    S: JobStore,
    L: EventLog,
    N: SnmpClient,
    H: HotFolderWriter,
    V: VwLogReader,
{
    pub jobs: Arc<S>,
    pub log: Arc<L>,
    pub snmp: Arc<N>,
    pub hot: Arc<H>,
    pub vw_log: Arc<V>,
    pub rules: Arc<RuleEngine>,
    pub config: CoordinatorConfig,
}

#[derive(Clone, Debug)]
pub struct CoordinatorConfig {
    /// After this duration in a single state, a job is considered stuck.
    pub stuck_threshold: Duration,
}

impl Default for CoordinatorConfig {
    fn default() -> Self {
        Self {
            stuck_threshold: Duration::from_secs(120),
        }
    }
}

impl<S, L, N, H, V> Coordinator<S, L, N, H, V>
where
    S: JobStore,
    L: EventLog,
    N: SnmpClient,
    H: HotFolderWriter,
    V: VwLogReader,
{
    /// Submit a new job into the queue. Writes the initial event and
    /// returns the inserted job.
    #[instrument(skip_all, fields(job_id = %job.id))]
    pub async fn submit(&self, job: Job) -> CoreResult<Job> {
        info!("submitting job");
        self.jobs.insert(job.clone()).await?;
        self.log
            .append(JobEvent::new(
                job.id,
                job.submitted_at,
                JobEventKind::StateTransition {
                    from: JobState::Submitted,
                    to: JobState::Submitted,
                    reason: Some("submitted".into()),
                },
            ))
            .await?;
        Ok(job)
    }

    /// Mark a job as having had its PDF written to the hot folder —
    /// transitions Submitted → Staged and records a FileWritten event.
    #[instrument(skip_all, fields(job_id = %id))]
    pub async fn mark_staged(&self, id: JobId, file: &SubmittedFile) -> CoreResult<()> {
        let mut job = self.jobs.get(id).await?;
        if job.state != JobState::Submitted {
            return Ok(());
        }
        job.advance(JobState::Staged, now())?;
        self.jobs.update(job.clone()).await?;
        self.log
            .append(JobEvent::new(
                id,
                now(),
                JobEventKind::FileWritten {
                    path: file.relative_path.clone(),
                    bytes: file.bytes,
                    sha256_hex: file.sha256_hex.clone(),
                },
            ))
            .await?;
        self.log
            .append(JobEvent::new(
                id,
                now(),
                JobEventKind::StateTransition {
                    from: JobState::Submitted,
                    to: JobState::Staged,
                    reason: None,
                },
            ))
            .await?;
        Ok(())
    }

    /// Run one cycle: poll ports, correlate, apply transitions, fire
    /// advisories.
    #[instrument(skip_all)]
    pub async fn step(&self) -> CoreResult<StepReport> {
        let mut report = StepReport::default();

        // 1. Poll printer.
        match self.snmp.snapshot().await {
            Ok(obs) => {
                debug!(state = ?obs.state, "snmp snapshot");
                report.observation = Some(obs.clone());
                self.log.record_observation(obs.clone()).await?;
                self.correlate_printer(&obs).await?;
            }
            Err(e) => {
                warn!(error = %e, "snmp snapshot failed");
            }
        }

        // 2. Drain VW log.
        match self.vw_log.drain().await {
            Ok(lines) => {
                report.vw_lines_drained = lines.len();
                for line in lines {
                    self.handle_vw_line(line).await?;
                }
            }
            Err(e) => {
                warn!(error = %e, "vw log drain failed");
            }
        }

        // 3. Evaluate rules.
        let in_flight = self.jobs.list_in_flight().await?;
        let oldest = oldest_age_in_flight(&in_flight, now());
        let depth = self.hot.queue_depth().await.ok();
        let advisories = self.rules.evaluate(&RuleInputs {
            observation: report.observation.as_ref(),
            in_flight_jobs: &in_flight,
            oldest_stuck_age: oldest,
            vw_queue_depth: depth,
        });
        report.advisories_fired = advisories.len();
        for a in advisories {
            self.log.record_advisory(a.clone()).await?;
            if let Some(job_id) = a.related_job {
                self.log
                    .append(JobEvent::new(
                        job_id,
                        a.fired_at,
                        JobEventKind::AdvisoryFired {
                            advisory: hanshi_domain::AdvisoryRef::from(&a),
                        },
                    ))
                    .await?;
            }
        }

        // 4. Count transitions that happened this step. (Approximate —
        // correlate_printer + handle_vw_line can each cause 0..N.)
        // We expose it for reporting without tracking exact deltas; a
        // downstream Prometheus counter can be wired later.
        Ok(report)
    }

    /// Correlate a fresh printer observation with in-flight jobs.
    async fn correlate_printer(&self, obs: &PrinterObservation) -> CoreResult<()> {
        let in_flight = self.jobs.list_in_flight().await?;
        for mut job in in_flight {
            let next = match (&job.state, &obs.state) {
                (JobState::Sent, PrinterState::Printing) => Some(JobState::Printing),
                (JobState::Printing, PrinterState::Idle) => {
                    if obs.error_codes.is_empty() {
                        Some(JobState::Completed)
                    } else {
                        Some(JobState::Failed)
                    }
                }
                (JobState::AcceptedByVw, PrinterState::Waiting | PrinterState::Printing) => {
                    Some(JobState::Ripping)
                }
                (JobState::Ripping, PrinterState::Waiting | PrinterState::Printing) => {
                    Some(JobState::Sent)
                }
                _ => None,
            };
            if let Some(to) = next {
                let from = job.state;
                job.advance(to, now())?;
                self.jobs.update(job.clone()).await?;
                self.log
                    .append(JobEvent::new(
                        job.id,
                        now(),
                        JobEventKind::StateTransition {
                            from,
                            to,
                            reason: Some(format!(
                                "printer observed: {}",
                                printer_state_label(&obs.state)
                            )),
                        },
                    ))
                    .await?;
            }
        }
        Ok(())
    }

    /// Process one VW log line — may advance in-flight jobs and record
    /// the line verbatim as an event.
    async fn handle_vw_line(&self, entry: hanshi_ports::VwLogEntry) -> CoreResult<()> {
        use hanshi_ports::VwLogKind;

        // Record verbatim for audit.
        if let Some(related) = self.infer_related_job(&entry).await? {
            self.log
                .append(JobEvent::new(
                    related,
                    entry.observed_at,
                    JobEventKind::VwLogLine {
                        text: entry.raw.clone(),
                    },
                ))
                .await?;
        }

        // Rough state transitions driven off classified lines.
        match entry.kind {
            VwLogKind::JobIngested { .. } => {
                self.advance_matching(JobState::Staged, JobState::AcceptedByVw, "vw ingested")
                    .await?;
            }
            VwLogKind::RipStarted { .. } => {
                self.advance_matching(
                    JobState::AcceptedByVw,
                    JobState::Ripping,
                    "vw rip started",
                )
                .await?;
            }
            VwLogKind::SentToPrinter { .. } => {
                self.advance_matching(JobState::Ripping, JobState::Sent, "vw sent to printer")
                    .await?;
            }
            VwLogKind::JobCompleted { .. } => {
                self.advance_matching(JobState::Printing, JobState::Completed, "vw completed")
                    .await?;
            }
            VwLogKind::Error { .. } => {
                self.advance_matching(JobState::Ripping, JobState::Failed, "vw error")
                    .await?;
                self.advance_matching(JobState::Sent, JobState::Failed, "vw error")
                    .await?;
                self.advance_matching(JobState::Printing, JobState::Failed, "vw error")
                    .await?;
            }
            VwLogKind::RipFinished { .. } | VwLogKind::Unknown => {}
        }
        Ok(())
    }

    /// Advance every in-flight job in `from` to `to`.
    async fn advance_matching(
        &self,
        from: JobState,
        to: JobState,
        reason: &str,
    ) -> CoreResult<()> {
        let in_flight = self.jobs.list_in_flight().await?;
        for mut job in in_flight {
            if job.state != from {
                continue;
            }
            if job.state.can_transition_to(to) {
                job.advance(to, now())?;
                self.jobs.update(job.clone()).await?;
                self.log
                    .append(JobEvent::new(
                        job.id,
                        now(),
                        JobEventKind::StateTransition {
                            from,
                            to,
                            reason: Some(reason.to_string()),
                        },
                    ))
                    .await?;
            }
        }
        Ok(())
    }

    /// Best-effort: match a VW log line to an in-flight job by filename.
    /// Since our hot-folder names embed the JobId, we can match by
    /// substring when the VW line mentions a filename.
    async fn infer_related_job(
        &self,
        entry: &hanshi_ports::VwLogEntry,
    ) -> CoreResult<Option<JobId>> {
        use hanshi_ports::VwLogKind;
        let filename = match &entry.kind {
            VwLogKind::JobIngested { filename }
            | VwLogKind::RipStarted { filename }
            | VwLogKind::RipFinished { filename }
            | VwLogKind::SentToPrinter { filename }
            | VwLogKind::JobCompleted { filename } => Some(filename.as_str()),
            VwLogKind::Error { filename, .. } => filename.as_deref(),
            VwLogKind::Unknown => None,
        };
        let Some(filename) = filename else {
            return Ok(None);
        };
        let in_flight = self.jobs.list_in_flight().await?;
        for job in in_flight {
            if filename.contains(&job.id.to_string()) {
                return Ok(Some(job.id));
            }
        }
        Ok(None)
    }
}

fn printer_state_label(s: &PrinterState) -> &'static str {
    match s {
        PrinterState::Idle => "idle",
        PrinterState::Waiting => "waiting",
        PrinterState::Printing => "printing",
        PrinterState::Paused => "paused",
        PrinterState::Error { .. } => "error",
        PrinterState::Unknown => "unknown",
    }
}

fn oldest_age_in_flight(jobs: &[Job], reference: Timestamp) -> Option<Duration> {
    jobs.iter()
        .map(|j| reference - j.updated_at)
        .max()
        .and_then(|d| d.to_std().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hanshi_domain::{MaterialId, ProfileId};
    use hanshi_ports::{MockHotFolderWriter, MockSnmpClient, MockVwLogReader};

    fn build_coordinator() -> Coordinator<
        crate::store::MemoryJobStore,
        crate::store::MemoryEventLog,
        MockSnmpClient,
        MockHotFolderWriter,
        MockVwLogReader,
    > {
        Coordinator {
            jobs: Arc::new(crate::store::MemoryJobStore::new()),
            log: Arc::new(crate::store::MemoryEventLog::new()),
            snmp: Arc::new(MockSnmpClient::new()),
            hot: Arc::new(MockHotFolderWriter::new()),
            vw_log: Arc::new(MockVwLogReader::new()),
            rules: Arc::new(RuleEngine::new()),
            config: CoordinatorConfig::default(),
        }
    }

    fn fresh_job() -> Job {
        Job::new("t", MaterialId::new(), ProfileId::new(), 1, now()).unwrap()
    }

    #[tokio::test]
    async fn submit_writes_initial_event() {
        let c = build_coordinator();
        let j = c.submit(fresh_job()).await.unwrap();
        let events = c.log.for_job(j.id).await.unwrap();
        assert_eq!(events.len(), 1);
    }

    #[tokio::test]
    async fn mark_staged_moves_to_staged() {
        let c = build_coordinator();
        let j = c.submit(fresh_job()).await.unwrap();
        let f = c.hot.stage_pdf(j.id, b"bytes", None).await.unwrap();
        c.mark_staged(j.id, &f).await.unwrap();
        let updated = c.jobs.get(j.id).await.unwrap();
        assert_eq!(updated.state, JobState::Staged);
    }

    #[tokio::test]
    async fn step_reports_observation_and_fires_ink_low() {
        let c = build_coordinator();
        c.snmp.set_ink(hanshi_domain::InkChannel::Cyan, 5).await;
        let r = c.step().await.unwrap();
        assert!(r.observation.is_some());
        assert!(r.advisories_fired >= 1);
    }

    #[tokio::test]
    async fn step_correlates_sent_to_printing() {
        let c = build_coordinator();
        let mut j = fresh_job();
        j.state = JobState::Sent;
        c.jobs.insert(j.clone()).await.unwrap();

        c.snmp.set_state(PrinterState::Printing).await;
        c.step().await.unwrap();

        let updated = c.jobs.get(j.id).await.unwrap();
        assert_eq!(updated.state, JobState::Printing);
    }

    #[tokio::test]
    async fn step_completes_on_return_to_idle_without_errors() {
        let c = build_coordinator();
        let mut j = fresh_job();
        j.state = JobState::Printing;
        c.jobs.insert(j.clone()).await.unwrap();

        c.snmp.set_state(PrinterState::Idle).await;
        c.step().await.unwrap();

        let updated = c.jobs.get(j.id).await.unwrap();
        assert_eq!(updated.state, JobState::Completed);
    }

    #[tokio::test]
    async fn vw_log_ingested_advances_staged_to_accepted() {
        let c = build_coordinator();
        let mut j = fresh_job();
        j.state = JobState::Staged;
        c.jobs.insert(j.clone()).await.unwrap();

        c.vw_log
            .push(
                hanshi_ports::VwLogKind::JobIngested {
                    filename: format!("{}.pdf", j.id),
                },
                "VW: ingested",
            )
            .await;
        c.step().await.unwrap();

        let updated = c.jobs.get(j.id).await.unwrap();
        assert_eq!(updated.state, JobState::AcceptedByVw);
    }

    #[tokio::test]
    async fn vw_error_marks_in_flight_job_failed() {
        let c = build_coordinator();
        let mut j = fresh_job();
        j.state = JobState::Ripping;
        c.jobs.insert(j.clone()).await.unwrap();

        c.vw_log
            .push(
                hanshi_ports::VwLogKind::Error {
                    filename: Some(format!("{}.pdf", j.id)),
                    message: "boom".into(),
                },
                "VW ERROR: boom",
            )
            .await;
        c.step().await.unwrap();

        let updated = c.jobs.get(j.id).await.unwrap();
        assert_eq!(updated.state, JobState::Failed);
    }
}
