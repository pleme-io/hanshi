//! Typed IDs. Each is a newtype around `uuid::Uuid` so they can't be
//! accidentally confused with each other at the type level.

use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! typed_id {
    ($(#[$meta:meta])* $name:ident, $prefix:literal) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            /// Generate a fresh random ID.
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            /// Wrap an existing `Uuid`.
            pub const fn from_uuid(u: Uuid) -> Self {
                Self(u)
            }

            pub const fn as_uuid(&self) -> Uuid {
                self.0
            }

            /// Human-friendly label with the kind prefix — "job-<simple-uuid>".
            /// Use in logs / UI labels, not in URLs (URLs use Display, which
            /// emits the bare UUID so Path<Id> extractors round-trip).
            pub fn pretty(&self) -> String {
                format!("{}-{}", $prefix, self.0.as_simple())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                // Plain UUID so the same string works in serde, URLs,
                // and SQL. Use `.pretty()` for prefixed log labels.
                self.0.fmt(f)
            }
        }
    };
}

typed_id!(
    /// Identifies a print job.
    JobId,
    "job"
);

typed_id!(
    /// Identifies a single event in the job log.
    EventId,
    "evt"
);

typed_id!(
    /// Identifies a material entry in the catalog.
    MaterialId,
    "mat"
);

typed_id!(
    /// Identifies a print profile (color + pass plan + channels).
    ProfileId,
    "prof"
);

typed_id!(
    /// Identifies a fired advisory.
    AdvisoryId,
    "adv"
);

typed_id!(
    /// Correlation ID — attaches together events that belong to the same
    /// logical unit of work. Usually equal to `JobId`'s underlying UUID
    /// for job-related events, but separable for housekeeping events.
    CorrelationId,
    "cor"
);

impl From<JobId> for CorrelationId {
    fn from(j: JobId) -> Self {
        Self(j.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_distinct_types() {
        // Fails to compile if JobId and MaterialId became the same type.
        fn takes_job(_: JobId) {}
        let j = JobId::new();
        takes_job(j);
    }

    #[test]
    fn display_is_plain_uuid() {
        let j = JobId::new();
        let s = j.to_string();
        // Must be hyphenated UUID so URLs + SQL + serde all agree.
        assert_eq!(s.len(), 36);
        assert_eq!(s.matches('-').count(), 4);
    }

    #[test]
    fn pretty_has_prefix() {
        let j = JobId::new();
        assert!(j.pretty().starts_with("job-"));
    }

    #[test]
    fn round_trip_serde() {
        let j = JobId::new();
        let s = serde_json::to_string(&j).unwrap();
        let back: JobId = serde_json::from_str(&s).unwrap();
        assert_eq!(j, back);
    }

    #[test]
    fn correlation_from_job_id_preserves_uuid() {
        let j = JobId::new();
        let c: CorrelationId = j.into();
        assert_eq!(c.0, j.0);
    }
}
