//! Verify hanshi-domain registers JobEventKind into the
//! gen-platform fleet catalog. hanshi is the ELEVENTH consumer
//! class adopting the typed-dispatcher catamorphism.
//!
//! JobEventKind enumerates every kind of event in a hanshi
//! print job's audit trail — state transitions, files written,
//! VW consumption, log lines, printer observations, advisory
//! firings, operator notes, QC outcomes.

use gen_platform::{catalog, TypedDispatcherTrait};
use hanshi_domain::JobEventKind;

#[test]
fn job_event_kind_registers_into_fleet_catalog() {
    let entry = catalog::by_label("hanshi.job-event-kind")
        .expect("hanshi-domain must register JobEventKind");
    assert_eq!((entry.variant_count)(), 8);
}

#[test]
fn job_event_kind_variant_kinds_kebab() {
    let kinds = JobEventKind::variant_kinds();
    assert_eq!(
        kinds,
        vec![
            "state-transition",
            "file-written",
            "vw-consumed",
            "vw-log-line",
            "printer-observed",
            "advisory-fired",
            "operator-note",
            "qc-recorded"
        ]
    );
}

#[test]
fn discriminant_distinct_from_short_label() {
    // discriminant() emits the canonical kebab variant name; the
    // pre-existing short_label() is human-curated shorthand and
    // intentionally diverges for some variants (advisory vs
    // advisory-fired, note vs operator-note, qc vs qc-recorded).
    let advisory = JobEventKind::OperatorNote {
        text: "hello".into(),
    };
    assert_eq!(advisory.discriminant(), "operator-note"); // stable kind
    assert_eq!(advisory.short_label(), "note"); // UI shorthand
}

#[test]
fn predicates() {
    let consumed = JobEventKind::VwConsumed;
    assert!(consumed.is_vw_consumed());
    assert!(!consumed.is_qc_recorded());
}

#[test]
fn variant_count_via_trait() {
    assert_eq!(JobEventKind::variant_count(), 8);
}
