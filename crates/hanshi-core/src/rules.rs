//! Rule engine / advisor.
//!
//! Turns observations (printer state, VW log entries, job timeouts)
//! into `Advisory` records with diagnosis + suggested action.
//!
//! Two paths:
//!   * Built-in hardcoded rules — ship by default, always available
//!     (out-of-ink, printer unreachable, VW stalled, etc.).
//!   * Blue-authored rules — a `.b` bundle loaded by `BlueRules`
//!     through `blue-lang-host`. The bundle's `rules` lists its rule
//!     definitions; each reads `inputs_json` and calls `emit_advisory`.
//!
//! This module wires both paths into a single `RuleEngine::evaluate`
//! call that callers drop observations into.

use std::time::Duration;

use hanshi_domain::{now, Advisory, InkLevel, Job, JobState, PrinterObservation, PrinterState, Severity};

/// Inputs the rule engine evaluates against.
#[derive(Clone, Debug)]
pub struct RuleInputs<'a> {
    pub observation: Option<&'a PrinterObservation>,
    pub in_flight_jobs: &'a [Job],
    /// Age of the oldest job that's been in its current state.
    pub oldest_stuck_age: Option<Duration>,
    /// Last-known VW queue depth if recently probed.
    pub vw_queue_depth: Option<usize>,
}

/// Result of evaluating one rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuleOutcome {
    Fired(Advisory),
    Silent,
}

pub struct RuleEngine {
    ink_low_threshold_percent: u8,
    stuck_threshold_secs: u64,
}

impl RuleEngine {
    pub fn new() -> Self {
        Self {
            ink_low_threshold_percent: 10,
            stuck_threshold_secs: 120,
        }
    }

    pub fn with_ink_threshold(mut self, percent: u8) -> Self {
        self.ink_low_threshold_percent = percent.min(100);
        self
    }

    pub fn with_stuck_threshold(mut self, secs: u64) -> Self {
        self.stuck_threshold_secs = secs;
        self
    }

    /// Evaluate every built-in rule against the inputs. Returns the
    /// advisories that fired (possibly empty).
    pub fn evaluate(&self, inputs: &RuleInputs<'_>) -> Vec<Advisory> {
        let mut out = Vec::new();

        if let Some(obs) = inputs.observation {
            if let Some(a) = self.rule_printer_error(obs) {
                out.push(a);
            }
            for a in self.rule_ink_low(obs) {
                out.push(a);
            }
        }

        if let Some(age) = inputs.oldest_stuck_age {
            if age.as_secs() >= self.stuck_threshold_secs {
                if let Some(job) = inputs.in_flight_jobs.first() {
                    out.push(self.advisory_stuck_job(job, age));
                }
            }
        }

        // Rule: VW queue should be draining — if we have jobs in
        // AcceptedByVw / Ripping but queue_depth stays > 0 after
        // threshold, that's stall candidate. Minimal signal here;
        // coordinator can attach richer context.
        if let (Some(depth), true) = (
            inputs.vw_queue_depth,
            inputs
                .in_flight_jobs
                .iter()
                .any(|j| matches!(j.state, JobState::Staged | JobState::AcceptedByVw)),
        ) {
            if depth > 0 {
                if inputs.oldest_stuck_age.map(|a| a.as_secs()).unwrap_or(0)
                    >= self.stuck_threshold_secs
                {
                    out.push(self.advisory_vw_queue_backed_up(depth));
                }
            }
        }

        out
    }

    fn rule_printer_error(&self, obs: &PrinterObservation) -> Option<Advisory> {
        match &obs.state {
            PrinterState::Error { reason } => Some(
                Advisory::new(
                    Severity::Error,
                    "Printer reports error",
                    format!("The printer is in an Error state: {reason}"),
                    "Check the LEF2-300 control panel for the displayed error code. \
                     If the issue is resolved, press Resume; otherwise follow the \
                     service manual for the specific code.",
                    "printer-error",
                    now(),
                ),
            ),
            PrinterState::Unknown => Some(Advisory::new(
                Severity::Warn,
                "Printer status unknown",
                "SNMP did not respond with a recognized printer state.",
                "Verify the printer is powered on and reachable on the network. \
                 Confirm SNMP is enabled in the printer's network configuration.",
                "printer-unknown",
                now(),
            )),
            _ => None,
        }
    }

    fn rule_ink_low(&self, obs: &PrinterObservation) -> Vec<Advisory> {
        obs.inks
            .iter()
            .filter(|i: &&InkLevel| i.percent < self.ink_low_threshold_percent)
            .map(|i| {
                let severity = if i.percent == 0 {
                    Severity::Critical
                } else if i.percent < 3 {
                    Severity::Error
                } else {
                    Severity::Warn
                };
                Advisory::new(
                    severity,
                    format!("Ink low: {}", i.channel.label()),
                    format!(
                        "The {} channel is at {}%, below the {}% warning threshold.",
                        i.channel.label(),
                        i.percent,
                        self.ink_low_threshold_percent
                    ),
                    format!(
                        "Have a {} cartridge ready. Prints may fail mid-run if it depletes.",
                        i.channel.label()
                    ),
                    "ink-low",
                    now(),
                )
            })
            .collect()
    }

    fn advisory_stuck_job(&self, job: &Job, age: Duration) -> Advisory {
        Advisory::new(
            Severity::Warn,
            format!("Job stalled in {:?}", job.state),
            format!(
                "Job '{}' has been in state {:?} for {}s — exceeds the {}s threshold.",
                job.name,
                job.state,
                age.as_secs(),
                self.stuck_threshold_secs
            ),
            match job.state {
                JobState::Staged => "Open VersaWorks on the Windows box. Check whether the input \
                                     folder is being watched and whether the queue is paused."
                    .into(),
                JobState::AcceptedByVw | JobState::Ripping => {
                    "VersaWorks may be stuck ripping. Check VW's queue UI for error dialogs or \
                     a paused queue header."
                        .into()
                }
                JobState::Sent | JobState::Printing => {
                    "Printer may be waiting for operator input (lid, material position). Check \
                     the LEF2-300 front panel."
                        .into()
                }
                _ => format!(
                    "Inspect the job timeline for the last few observations and the VW log for \
                     messages between now and the last state change ({}s ago).",
                    age.as_secs()
                ),
            },
            "stuck-job",
            now(),
        )
        .for_job(job.id)
    }

    fn advisory_vw_queue_backed_up(&self, depth: usize) -> Advisory {
        Advisory::new(
            Severity::Warn,
            "VW input queue not draining",
            format!(
                "{depth} file(s) remain in the VersaWorks hot folder beyond the stall threshold."
            ),
            "Check the Windows box: is VersaWorks running? Is the input-folder watch enabled? \
             Is the queue paused? If VW crashed, relaunching it usually drains the backlog.",
            "vw-queue-stall",
            now(),
        )
    }
}

impl Default for RuleEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hanshi_domain::{InkChannel, MaterialId, ProfileId};

    fn obs_with(state: PrinterState) -> PrinterObservation {
        PrinterObservation::at(now(), state)
    }

    fn job(state: JobState) -> Job {
        let mut j = Job::new("t", MaterialId::new(), ProfileId::new(), 1, now()).unwrap();
        j.state = state;
        j
    }

    #[test]
    fn printer_error_fires_error_advisory() {
        let e = RuleEngine::new();
        let obs = obs_with(PrinterState::Error {
            reason: "E04 cover open".into(),
        });
        let a = e.evaluate(&RuleInputs {
            observation: Some(&obs),
            in_flight_jobs: &[],
            oldest_stuck_age: None,
            vw_queue_depth: None,
        });
        assert!(a.iter().any(|x| x.rule == "printer-error"));
    }

    #[test]
    fn unknown_state_fires_warning() {
        let e = RuleEngine::new();
        let obs = obs_with(PrinterState::Unknown);
        let a = e.evaluate(&RuleInputs {
            observation: Some(&obs),
            in_flight_jobs: &[],
            oldest_stuck_age: None,
            vw_queue_depth: None,
        });
        assert!(a.iter().any(|x| x.rule == "printer-unknown"));
    }

    #[test]
    fn ink_low_fires_per_channel() {
        let e = RuleEngine::new();
        let mut obs = obs_with(PrinterState::Idle);
        obs.inks.push(InkLevel::new(InkChannel::Cyan, 5));
        obs.inks.push(InkLevel::new(InkChannel::White, 50));
        let a = e.evaluate(&RuleInputs {
            observation: Some(&obs),
            in_flight_jobs: &[],
            oldest_stuck_age: None,
            vw_queue_depth: None,
        });
        assert_eq!(a.iter().filter(|x| x.rule == "ink-low").count(), 1);
    }

    #[test]
    fn ink_critical_when_zero() {
        let e = RuleEngine::new();
        let mut obs = obs_with(PrinterState::Idle);
        obs.inks.push(InkLevel::new(InkChannel::Black, 0));
        let a = e.evaluate(&RuleInputs {
            observation: Some(&obs),
            in_flight_jobs: &[],
            oldest_stuck_age: None,
            vw_queue_depth: None,
        });
        let ink = a.iter().find(|x| x.rule == "ink-low").unwrap();
        assert_eq!(ink.severity, Severity::Critical);
    }

    #[test]
    fn stuck_job_fires_above_threshold() {
        let e = RuleEngine::new().with_stuck_threshold(60);
        let j = job(JobState::Staged);
        let a = e.evaluate(&RuleInputs {
            observation: None,
            in_flight_jobs: std::slice::from_ref(&j),
            oldest_stuck_age: Some(Duration::from_secs(120)),
            vw_queue_depth: None,
        });
        assert!(a.iter().any(|x| x.rule == "stuck-job"));
    }

    #[test]
    fn stuck_job_silent_below_threshold() {
        let e = RuleEngine::new().with_stuck_threshold(60);
        let j = job(JobState::Staged);
        let a = e.evaluate(&RuleInputs {
            observation: None,
            in_flight_jobs: std::slice::from_ref(&j),
            oldest_stuck_age: Some(Duration::from_secs(30)),
            vw_queue_depth: None,
        });
        assert!(!a.iter().any(|x| x.rule == "stuck-job"));
    }

    #[test]
    fn vw_queue_backed_up_fires_with_staged_jobs_over_threshold() {
        let e = RuleEngine::new().with_stuck_threshold(60);
        let j = job(JobState::Staged);
        let a = e.evaluate(&RuleInputs {
            observation: None,
            in_flight_jobs: std::slice::from_ref(&j),
            oldest_stuck_age: Some(Duration::from_secs(120)),
            vw_queue_depth: Some(3),
        });
        assert!(a.iter().any(|x| x.rule == "vw-queue-stall"));
    }

    #[test]
    fn idle_printer_silent() {
        let e = RuleEngine::new();
        let obs = obs_with(PrinterState::Idle);
        let a = e.evaluate(&RuleInputs {
            observation: Some(&obs),
            in_flight_jobs: &[],
            oldest_stuck_age: None,
            vw_queue_depth: None,
        });
        assert!(a.is_empty());
    }
}
