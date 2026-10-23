//! Advisories — structured outputs from the rule engine that tell the
//! operator what's wrong and what to do about it.
//!
//! An `Advisory` is a diagnosis paired with a suggested action, scoped
//! to a specific job or to the system as a whole. The rule engine in
//! hanshi-core produces these from observations; the UI renders them.

use serde::{Deserialize, Serialize};

use crate::ids::{AdvisoryId, JobId};
use crate::Timestamp;

/// Severity level for an advisory.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Severity {
    Info,
    Warn,
    Error,
    Critical,
}

impl Severity {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
            Self::Critical => "critical",
        }
    }
}

/// A fired advisory.
///
/// The fields mirror what the UI needs to render: a title, the diagnosis
/// (what we observed and why it's a problem), and the suggested action
/// (what the operator should do).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Advisory {
    pub id: AdvisoryId,
    pub severity: Severity,
    pub title: String,
    pub diagnosis: String,
    pub suggested_action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub related_job: Option<JobId>,
    pub fired_at: Timestamp,
    /// Name of the rule that fired this advisory — for traceability
    /// back to the blue rule bundle source.
    pub rule: String,
}

impl Advisory {
    /// Construct an advisory. No validation — rules author what they want.
    pub fn new(
        severity: Severity,
        title: impl Into<String>,
        diagnosis: impl Into<String>,
        action: impl Into<String>,
        rule: impl Into<String>,
        fired_at: Timestamp,
    ) -> Self {
        Self {
            id: AdvisoryId::new(),
            severity,
            title: title.into(),
            diagnosis: diagnosis.into(),
            suggested_action: action.into(),
            related_job: None,
            fired_at,
            rule: rule.into(),
        }
    }

    pub fn for_job(mut self, job: JobId) -> Self {
        self.related_job = Some(job);
        self
    }
}

/// Compact reference embedded inside `JobEventKind::AdvisoryFired` —
/// the event carries just enough to render the timeline entry without
/// duplicating the full advisory payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdvisoryRef {
    pub id: AdvisoryId,
    pub severity: Severity,
    pub title: String,
}

impl From<&Advisory> for AdvisoryRef {
    fn from(a: &Advisory) -> Self {
        Self {
            id: a.id,
            severity: a.severity,
            title: a.title.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_orders_info_below_critical() {
        assert!(Severity::Info < Severity::Warn);
        assert!(Severity::Warn < Severity::Error);
        assert!(Severity::Error < Severity::Critical);
    }

    #[test]
    fn advisory_for_job_attaches_job_id() {
        let j = JobId::new();
        let a = Advisory::new(
            Severity::Warn,
            "stalled",
            "vw did not consume after 120s",
            "open VW and click Resume on the queue header",
            "vw-stall",
            crate::now(),
        )
        .for_job(j);
        assert_eq!(a.related_job, Some(j));
    }

    #[test]
    fn advisory_ref_from_full() {
        let a = Advisory::new(
            Severity::Error,
            "ink out",
            "white ink channel at 0%",
            "swap in a fresh white cartridge",
            "ink-out",
            crate::now(),
        );
        let r = AdvisoryRef::from(&a);
        assert_eq!(r.id, a.id);
        assert_eq!(r.severity, a.severity);
        assert_eq!(r.title, a.title);
    }

    #[test]
    fn advisory_serde_round_trip() {
        let a = Advisory::new(
            Severity::Warn,
            "x",
            "y",
            "z",
            "rule-foo",
            crate::now(),
        );
        let s = serde_json::to_string(&a).unwrap();
        let back: Advisory = serde_json::from_str(&s).unwrap();
        assert_eq!(a, back);
    }
}
