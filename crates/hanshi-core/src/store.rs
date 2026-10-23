//! Storage traits + in-memory impls.
//!
//! The real deployment persists to SQLite via a future `hanshi-storage`
//! crate; this module carries traits so consumers of hanshi-core can
//! swap persistence later without touching the coordinator.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;

use hanshi_domain::{Advisory, Job, JobEvent, JobId, PrinterObservation};

use crate::error::{CoreError, CoreResult};

#[async_trait]
pub trait JobStore: Send + Sync {
    async fn insert(&self, job: Job) -> CoreResult<()>;
    async fn update(&self, job: Job) -> CoreResult<()>;
    async fn get(&self, id: JobId) -> CoreResult<Job>;
    async fn list(&self) -> CoreResult<Vec<Job>>;
    async fn list_in_flight(&self) -> CoreResult<Vec<Job>>;
}

#[async_trait]
pub trait EventLog: Send + Sync {
    async fn append(&self, event: JobEvent) -> CoreResult<()>;
    async fn for_job(&self, id: JobId) -> CoreResult<Vec<JobEvent>>;
    async fn recent(&self, limit: usize) -> CoreResult<Vec<JobEvent>>;
    async fn record_observation(&self, obs: PrinterObservation) -> CoreResult<()>;
    async fn recent_observations(&self, limit: usize) -> CoreResult<Vec<PrinterObservation>>;
    async fn record_advisory(&self, advisory: Advisory) -> CoreResult<()>;
    async fn advisories(&self) -> CoreResult<Vec<Advisory>>;
}

// ── In-memory implementations ─────────────────────────────────────────

#[derive(Clone, Default)]
pub struct MemoryJobStore {
    inner: Arc<Mutex<Vec<Job>>>,
}

impl MemoryJobStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl JobStore for MemoryJobStore {
    async fn insert(&self, job: Job) -> CoreResult<()> {
        self.inner.lock().await.push(job);
        Ok(())
    }

    async fn update(&self, job: Job) -> CoreResult<()> {
        let mut v = self.inner.lock().await;
        let Some(slot) = v.iter_mut().find(|j| j.id == job.id) else {
            return Err(CoreError::not_found("job", job.id.to_string()));
        };
        *slot = job;
        Ok(())
    }

    async fn get(&self, id: JobId) -> CoreResult<Job> {
        self.inner
            .lock()
            .await
            .iter()
            .find(|j| j.id == id)
            .cloned()
            .ok_or_else(|| CoreError::not_found("job", id.to_string()))
    }

    async fn list(&self) -> CoreResult<Vec<Job>> {
        Ok(self.inner.lock().await.clone())
    }

    async fn list_in_flight(&self) -> CoreResult<Vec<Job>> {
        Ok(self
            .inner
            .lock()
            .await
            .iter()
            .filter(|j| j.state.is_in_flight())
            .cloned()
            .collect())
    }
}

#[derive(Clone, Default)]
pub struct MemoryEventLog {
    events: Arc<Mutex<Vec<JobEvent>>>,
    observations: Arc<Mutex<Vec<PrinterObservation>>>,
    advisories: Arc<Mutex<Vec<Advisory>>>,
}

impl MemoryEventLog {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl EventLog for MemoryEventLog {
    async fn append(&self, event: JobEvent) -> CoreResult<()> {
        self.events.lock().await.push(event);
        Ok(())
    }

    async fn for_job(&self, id: JobId) -> CoreResult<Vec<JobEvent>> {
        Ok(self
            .events
            .lock()
            .await
            .iter()
            .filter(|e| e.job_id == id)
            .cloned()
            .collect())
    }

    async fn recent(&self, limit: usize) -> CoreResult<Vec<JobEvent>> {
        let guard = self.events.lock().await;
        let len = guard.len();
        let start = len.saturating_sub(limit);
        Ok(guard[start..].to_vec())
    }

    async fn record_observation(&self, obs: PrinterObservation) -> CoreResult<()> {
        self.observations.lock().await.push(obs);
        Ok(())
    }

    async fn recent_observations(&self, limit: usize) -> CoreResult<Vec<PrinterObservation>> {
        let guard = self.observations.lock().await;
        let len = guard.len();
        let start = len.saturating_sub(limit);
        Ok(guard[start..].to_vec())
    }

    async fn record_advisory(&self, advisory: Advisory) -> CoreResult<()> {
        self.advisories.lock().await.push(advisory);
        Ok(())
    }

    async fn advisories(&self) -> CoreResult<Vec<Advisory>> {
        Ok(self.advisories.lock().await.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hanshi_domain::{now, MaterialId, ProfileId};

    fn j() -> Job {
        Job::new("x", MaterialId::new(), ProfileId::new(), 1, now()).unwrap()
    }

    #[tokio::test]
    async fn insert_list_get_round_trip() {
        let s = MemoryJobStore::new();
        let job = j();
        let id = job.id;
        s.insert(job.clone()).await.unwrap();
        assert_eq!(s.list().await.unwrap().len(), 1);
        assert_eq!(s.get(id).await.unwrap().id, id);
    }

    #[tokio::test]
    async fn update_in_place() {
        let s = MemoryJobStore::new();
        let mut job = j();
        let id = job.id;
        s.insert(job.clone()).await.unwrap();
        job.advance(hanshi_domain::JobState::Staged, now()).unwrap();
        s.update(job.clone()).await.unwrap();
        assert_eq!(s.get(id).await.unwrap().state, hanshi_domain::JobState::Staged);
    }

    #[tokio::test]
    async fn list_in_flight_excludes_terminal() {
        let s = MemoryJobStore::new();
        let mut terminal = j();
        terminal.state = hanshi_domain::JobState::Failed;
        let mut live = j();
        live.state = hanshi_domain::JobState::Printing;
        s.insert(terminal).await.unwrap();
        s.insert(live.clone()).await.unwrap();
        let in_flight = s.list_in_flight().await.unwrap();
        assert_eq!(in_flight.len(), 1);
        assert_eq!(in_flight[0].id, live.id);
    }

    #[tokio::test]
    async fn event_log_partitions_by_job() {
        let log = MemoryEventLog::new();
        let ja = hanshi_domain::JobId::new();
        let jb = hanshi_domain::JobId::new();
        log.append(JobEvent::new(
            ja,
            now(),
            hanshi_domain::JobEventKind::OperatorNote {
                text: "a".into(),
            },
        ))
        .await
        .unwrap();
        log.append(JobEvent::new(
            jb,
            now(),
            hanshi_domain::JobEventKind::OperatorNote {
                text: "b".into(),
            },
        ))
        .await
        .unwrap();
        assert_eq!(log.for_job(ja).await.unwrap().len(), 1);
        assert_eq!(log.for_job(jb).await.unwrap().len(), 1);
        assert_eq!(log.recent(10).await.unwrap().len(), 2);
    }
}
