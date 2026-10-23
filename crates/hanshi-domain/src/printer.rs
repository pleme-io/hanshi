//! Printer state observation — what the LEF2-300 tells us.
//!
//! `PrinterObservation` is a single-point-in-time snapshot (typically
//! from an SNMP walk). `PrinterState` is the high-level status we derive
//! from that snapshot. Multiple observations over time + the correlation
//! logic in hanshi-core produce job state transitions.

use serde::{Deserialize, Serialize};

use crate::ids::MaterialId;
use crate::material::InkChannel;
use crate::Timestamp;

/// High-level printer status. Maps from SNMP printer-MIB state values
/// (RFC 3805, `hrPrinterStatus`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum PrinterState {
    /// Idle and ready — no job active.
    Idle,
    /// Accepting data — job has been sent but hasn't started printing.
    Waiting,
    /// Mid-print.
    Printing,
    /// Paused by operator or by a recoverable condition.
    Paused,
    /// Unrecoverable error. Carries the error text where available.
    Error { reason: String },
    /// SNMP did not respond or returned an unexpected value.
    Unknown,
}

impl PrinterState {
    /// Whether a job could meaningfully be submitted right now.
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Idle)
    }

    /// Whether the printer is doing something that will eventually
    /// generate more state transitions on its own.
    pub fn is_busy(&self) -> bool {
        matches!(self, Self::Waiting | Self::Printing)
    }
}

/// Ink level for a single channel. `percent` is 0..=100.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct InkLevel {
    pub channel: InkChannel,
    pub percent: u8,
}

impl InkLevel {
    pub fn new(channel: InkChannel, percent: u8) -> Self {
        Self {
            channel,
            percent: percent.min(100),
        }
    }

    pub fn is_low(&self) -> bool {
        self.percent < 10
    }
}

/// A single snapshot of the printer's observable state, typically from
/// one SNMP walk. Correlation against job state transitions happens at
/// higher layers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PrinterObservation {
    pub observed_at: Timestamp,
    pub state: PrinterState,
    #[serde(default)]
    pub inks: Vec<InkLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lamp_hours: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub error_codes: Vec<String>,
    /// Material currently on the bed, if the operator has declared one.
    /// The printer itself does not report this — the hanshi UI sets it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_material: Option<MaterialId>,
}

impl PrinterObservation {
    /// Convenience constructor — state + timestamp, everything else empty.
    pub fn at(observed_at: Timestamp, state: PrinterState) -> Self {
        Self {
            observed_at,
            state,
            inks: Vec::new(),
            lamp_hours: None,
            error_codes: Vec::new(),
            current_material: None,
        }
    }

    /// True iff this observation reveals any channel below `threshold`%.
    pub fn has_ink_below(&self, threshold: u8) -> bool {
        self.inks.iter().any(|i| i.percent < threshold)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_is_ready_printing_is_busy() {
        assert!(PrinterState::Idle.is_ready());
        assert!(!PrinterState::Printing.is_ready());
        assert!(PrinterState::Printing.is_busy());
        assert!(PrinterState::Waiting.is_busy());
        assert!(!PrinterState::Idle.is_busy());
        assert!(!PrinterState::Paused.is_busy());
    }

    #[test]
    fn ink_level_percent_clamped() {
        let i = InkLevel::new(InkChannel::Cyan, 250);
        assert_eq!(i.percent, 100);
    }

    #[test]
    fn ink_level_is_low_below_ten() {
        assert!(InkLevel::new(InkChannel::Cyan, 5).is_low());
        assert!(!InkLevel::new(InkChannel::Cyan, 15).is_low());
        assert!(!InkLevel::new(InkChannel::Cyan, 0).is_low() == false); // 0 is low
    }

    #[test]
    fn observation_at_defaults_empty() {
        let t = crate::now();
        let o = PrinterObservation::at(t, PrinterState::Idle);
        assert!(o.inks.is_empty());
        assert!(o.lamp_hours.is_none());
        assert!(o.error_codes.is_empty());
    }

    #[test]
    fn has_ink_below_threshold() {
        let t = crate::now();
        let mut o = PrinterObservation::at(t, PrinterState::Idle);
        o.inks.push(InkLevel::new(InkChannel::Cyan, 50));
        o.inks.push(InkLevel::new(InkChannel::Magenta, 5));
        assert!(o.has_ink_below(10));
        assert!(!o.has_ink_below(3));
    }

    #[test]
    fn error_variant_carries_reason() {
        let s = PrinterState::Error {
            reason: "out of white ink".into(),
        };
        match s {
            PrinterState::Error { reason } => assert_eq!(reason, "out of white ink"),
            _ => panic!(),
        }
    }

    #[test]
    fn observation_serde_round_trip() {
        let t = crate::now();
        let mut o = PrinterObservation::at(t, PrinterState::Printing);
        o.inks.push(InkLevel::new(InkChannel::Black, 80));
        o.lamp_hours = Some(12.5);
        o.error_codes.push("E02".into());
        let s = serde_json::to_string(&o).unwrap();
        let back: PrinterObservation = serde_json::from_str(&s).unwrap();
        assert_eq!(o, back);
    }
}
