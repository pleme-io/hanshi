//! Job + state machine.
//!
//! The `JobState` machine models the observable lifecycle of a print
//! job from submission through completion. Transitions are checked at
//! the domain layer — illegal jumps return `DomainError::InvalidTransition`.

use serde::{Deserialize, Serialize};

use crate::error::{DomainError, Result};
use crate::ids::{JobId, MaterialId, ProfileId};
use crate::Timestamp;

/// Observable lifecycle states of a print job.
///
/// Transitions:
///
/// ```text
///  Submitted
///     │  (write PDF + sidecar into hot folder)
///     ▼
///  Staged
///     │  (VW removes the file from the hot folder)
///     ▼
///  AcceptedByVw
///     │  (VW log / duration heuristic says RIPping)
///     ▼
///  Ripping
///     │  (printer transitions to Waiting/Printing via SNMP)
///     ▼
///  Sent
///     │
///     ▼
///  Printing
///     │  (printer returns to Idle without any error codes firing)
///     ▼
///  Completed
///     │
///     ▼
///  QcPending   (optional — if QC workflow enabled)
///     │
///     ├── PassedQc
///     └── FailedQc
///
///  Any non-terminal state can also transition to Failed.
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum JobState {
    Submitted,
    Staged,
    AcceptedByVw,
    Ripping,
    Sent,
    Printing,
    Completed,
    QcPending,
    PassedQc,
    FailedQc,
    Failed,
}

impl JobState {
    /// The "happy path" forward transitions. Does not include Failed
    /// branches, which are allowed from any non-terminal state.
    pub fn happy_path_next(self) -> Option<JobState> {
        Some(match self {
            Self::Submitted => Self::Staged,
            Self::Staged => Self::AcceptedByVw,
            Self::AcceptedByVw => Self::Ripping,
            Self::Ripping => Self::Sent,
            Self::Sent => Self::Printing,
            Self::Printing => Self::Completed,
            Self::Completed => Self::QcPending,
            Self::QcPending | Self::PassedQc | Self::FailedQc | Self::Failed => return None,
        })
    }

    /// Terminal states never transition further. Useful for rule
    /// engines that want to skip resolved jobs.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::PassedQc | Self::FailedQc | Self::Failed)
    }

    /// Whether a job in this state is still actively in the printing
    /// pipeline (will generate more events).
    pub fn is_in_flight(self) -> bool {
        matches!(
            self,
            Self::Submitted
                | Self::Staged
                | Self::AcceptedByVw
                | Self::Ripping
                | Self::Sent
                | Self::Printing
        )
    }

    /// Check whether `self → next` is a legal transition. Any
    /// non-terminal state may transition to Failed; otherwise the
    /// happy path and QC branches are the only legal moves.
    pub fn can_transition_to(self, next: JobState) -> bool {
        if self == next {
            return false; // No-op is not a legal transition.
        }
        if self.is_terminal() {
            return false; // Terminal means terminal.
        }
        // Failed is always reachable from any non-terminal state.
        if next == JobState::Failed {
            return true;
        }
        // QC branches only from QcPending.
        if matches!(next, JobState::PassedQc | JobState::FailedQc) {
            return self == JobState::QcPending;
        }
        // Otherwise must be the happy path next step.
        self.happy_path_next() == Some(next)
    }

    /// Validated transition — produces a `DomainError::InvalidTransition`
    /// if illegal, else returns the new state.
    pub fn transition(self, next: JobState) -> Result<JobState> {
        if self.can_transition_to(next) {
            Ok(next)
        } else {
            Err(DomainError::invalid_transition(
                self,
                next,
                if self.is_terminal() {
                    "source state is terminal"
                } else if self == next {
                    "cannot transition to self"
                } else {
                    "not a legal next state"
                },
            ))
        }
    }
}

/// A print job.
///
/// Minimal invariants enforced at construction:
///   - `material` and `profile` IDs must be non-nil (enforced by UUID
///     newtype via `JobId::new`)
///   - `copies >= 1`
///   - `name` non-empty
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub id: JobId,
    pub name: String,
    pub material: MaterialId,
    pub profile: ProfileId,
    pub copies: u32,
    pub state: JobState,
    pub submitted_at: Timestamp,
    pub updated_at: Timestamp,
    /// Free-form notes from the operator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

impl Job {
    /// Create a fresh job in `Submitted` state. Validates inputs.
    pub fn new(
        name: impl Into<String>,
        material: MaterialId,
        profile: ProfileId,
        copies: u32,
        now: Timestamp,
    ) -> Result<Self> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(DomainError::validation("name", "must not be empty"));
        }
        if copies == 0 {
            return Err(DomainError::validation("copies", "must be >= 1"));
        }
        Ok(Self {
            id: JobId::new(),
            name,
            material,
            profile,
            copies,
            state: JobState::Submitted,
            submitted_at: now,
            updated_at: now,
            notes: None,
        })
    }

    /// Apply a state transition. Mutates `state` + `updated_at`. Returns
    /// the prior state on success so the caller can log it.
    pub fn advance(&mut self, next: JobState, now: Timestamp) -> Result<JobState> {
        let prior = self.state;
        self.state = self.state.transition(next)?;
        self.updated_at = now;
        Ok(prior)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mat() -> MaterialId {
        MaterialId::new()
    }

    fn prof() -> ProfileId {
        ProfileId::new()
    }

    // ── State machine transitions ─────────────────────────────────

    #[test]
    fn happy_path_traversal() {
        let mut s = JobState::Submitted;
        for expected in [
            JobState::Staged,
            JobState::AcceptedByVw,
            JobState::Ripping,
            JobState::Sent,
            JobState::Printing,
            JobState::Completed,
            JobState::QcPending,
        ] {
            assert!(
                s.can_transition_to(expected),
                "expected {s:?} → {expected:?}"
            );
            s = s.transition(expected).unwrap();
        }
    }

    #[test]
    fn failed_reachable_from_every_non_terminal() {
        for s in [
            JobState::Submitted,
            JobState::Staged,
            JobState::AcceptedByVw,
            JobState::Ripping,
            JobState::Sent,
            JobState::Printing,
            JobState::Completed,
            JobState::QcPending,
        ] {
            assert!(
                s.can_transition_to(JobState::Failed),
                "{s:?} should reach Failed"
            );
        }
    }

    #[test]
    fn terminal_states_are_terminal() {
        for s in [JobState::PassedQc, JobState::FailedQc, JobState::Failed] {
            assert!(s.is_terminal());
            for next in [
                JobState::Submitted,
                JobState::Printing,
                JobState::Completed,
                JobState::Failed,
            ] {
                assert!(
                    !s.can_transition_to(next),
                    "{s:?} should not transition to {next:?}"
                );
            }
        }
    }

    #[test]
    fn cannot_transition_to_self() {
        for s in [JobState::Submitted, JobState::Printing, JobState::Completed] {
            assert!(!s.can_transition_to(s), "{s:?} → {s:?} should be illegal");
        }
    }

    #[test]
    fn qc_branches_only_from_qc_pending() {
        assert!(JobState::QcPending.can_transition_to(JobState::PassedQc));
        assert!(JobState::QcPending.can_transition_to(JobState::FailedQc));
        // Not from elsewhere
        for s in [
            JobState::Submitted,
            JobState::Printing,
            JobState::Completed,
        ] {
            assert!(!s.can_transition_to(JobState::PassedQc));
            assert!(!s.can_transition_to(JobState::FailedQc));
        }
    }

    #[test]
    fn skipping_happy_path_is_illegal() {
        // Submitted → Printing (skipping Staged/Accepted/Ripping/Sent)
        assert!(!JobState::Submitted.can_transition_to(JobState::Printing));
        // Ripping → Completed (skipping Sent/Printing)
        assert!(!JobState::Ripping.can_transition_to(JobState::Completed));
    }

    #[test]
    fn transition_returns_error_on_illegal() {
        let err = JobState::Completed
            .transition(JobState::Submitted)
            .unwrap_err();
        assert!(matches!(err, DomainError::InvalidTransition { .. }));
    }

    // ── Is-in-flight ──────────────────────────────────────────────

    #[test]
    fn in_flight_states_are_pipeline_not_terminal() {
        assert!(JobState::Submitted.is_in_flight());
        assert!(JobState::Printing.is_in_flight());
        assert!(!JobState::Completed.is_in_flight());
        assert!(!JobState::Failed.is_in_flight());
        assert!(!JobState::PassedQc.is_in_flight());
    }

    // ── Job construction validation ──────────────────────────────

    #[test]
    fn new_job_is_in_submitted_state() {
        let j = Job::new("test", mat(), prof(), 1, crate::now()).unwrap();
        assert_eq!(j.state, JobState::Submitted);
    }

    #[test]
    fn empty_name_rejected() {
        let err = Job::new("", mat(), prof(), 1, crate::now()).unwrap_err();
        assert!(matches!(err, DomainError::Validation { field: "name", .. }));
    }

    #[test]
    fn zero_copies_rejected() {
        let err = Job::new("x", mat(), prof(), 0, crate::now()).unwrap_err();
        assert!(matches!(
            err,
            DomainError::Validation { field: "copies", .. }
        ));
    }

    #[test]
    fn whitespace_only_name_rejected() {
        let err = Job::new("   ", mat(), prof(), 1, crate::now()).unwrap_err();
        assert!(matches!(err, DomainError::Validation { field: "name", .. }));
    }

    // ── advance() ─────────────────────────────────────────────────

    #[test]
    fn advance_updates_state_and_timestamp() {
        let t0 = crate::now();
        let mut j = Job::new("x", mat(), prof(), 1, t0).unwrap();
        // sleep a bit so updated_at > submitted_at
        std::thread::sleep(std::time::Duration::from_millis(1));
        let t1 = crate::now();
        let prior = j.advance(JobState::Staged, t1).unwrap();
        assert_eq!(prior, JobState::Submitted);
        assert_eq!(j.state, JobState::Staged);
        assert_eq!(j.submitted_at, t0);
        assert_eq!(j.updated_at, t1);
        assert!(j.updated_at > j.submitted_at);
    }

    #[test]
    fn advance_illegal_transition_does_not_mutate() {
        let t0 = crate::now();
        let mut j = Job::new("x", mat(), prof(), 1, t0).unwrap();
        let err = j.advance(JobState::Completed, t0).unwrap_err();
        assert!(matches!(err, DomainError::InvalidTransition { .. }));
        assert_eq!(j.state, JobState::Submitted);
    }

    #[test]
    fn serde_round_trip_for_job() {
        let t = crate::now();
        let j = Job::new("banner", mat(), prof(), 3, t).unwrap();
        let s = serde_json::to_string(&j).unwrap();
        let back: Job = serde_json::from_str(&s).unwrap();
        assert_eq!(j, back);
    }

    #[test]
    fn serde_state_is_kebab_case() {
        let s = serde_json::to_string(&JobState::AcceptedByVw).unwrap();
        assert_eq!(s, "\"accepted-by-vw\"");
    }
}
