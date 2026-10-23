//! Print profiles — how the printer should process a job for a given
//! material class.
//!
//! A profile describes color management, white ink strategy, gloss/varnish
//! layer, UV cure plan, and pass counts. Multiple profiles can reference
//! the same physical material — e.g., "oak 5mm — vivid color" and
//! "oak 5mm — durable outdoor" for the same substrate.

use serde::{Deserialize, Serialize};

use crate::error::{DomainError, Result};
use crate::ids::ProfileId;

/// How white ink layers relate to the color layers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WhiteInkMode {
    /// No white ink.
    None,
    /// White under the color — for dark or translucent substrates.
    UnderPrint,
    /// White over the color — rare, for backlit effects.
    OverPrint,
    /// Color sandwiched between two white layers — premium look on glass
    /// and acrylic.
    Sandwich,
}

/// Varnish / gloss strategy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GlossMode {
    /// No varnish.
    None,
    /// Flat matte coating across the whole print area.
    Matte,
    /// Gloss applied only to a designated spot channel in the artwork.
    Spot,
    /// Gloss across the whole print area.
    Full,
}

/// Pass counts for a single job.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PassPlan {
    pub color_passes: u8,
    pub white_passes: u8,
    pub gloss_passes: u8,
    /// If true, run a UV cure pass between each ink layer (slower, more
    /// robust finish). If false, cure only at the end.
    #[serde(default)]
    pub cure_after_each: bool,
}

impl PassPlan {
    pub const fn simple(color: u8, white: u8, gloss: u8) -> Self {
        Self {
            color_passes: color,
            white_passes: white,
            gloss_passes: gloss,
            cure_after_each: false,
        }
    }

    /// Total pass count.
    pub fn total(&self) -> u32 {
        u32::from(self.color_passes) + u32::from(self.white_passes) + u32::from(self.gloss_passes)
    }

    /// Invariant: at least one pass of some kind must be configured.
    pub fn validate(&self) -> Result<()> {
        if self.total() == 0 {
            return Err(DomainError::Invariant(
                "pass plan must have at least one color/white/gloss pass",
            ));
        }
        Ok(())
    }
}

impl Default for PassPlan {
    fn default() -> Self {
        Self::simple(1, 0, 0)
    }
}

/// A print profile — immutable reference record. Edits produce a new
/// profile with a new `ProfileId`; history is preserved by never mutating.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrintProfile {
    pub id: ProfileId,
    pub name: String,
    pub passes: PassPlan,
    pub white_ink: WhiteInkMode,
    pub gloss: GlossMode,
    /// Number of explicit UV cure passes run *after* all ink layers.
    /// `cure_after_each` inside `passes` handles per-layer curing.
    pub final_cure_passes: u8,
    /// Optional ICC color profile name, as VW recognizes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icc_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

impl PrintProfile {
    /// Builder — validates internally consistent pass plan + channel
    /// selection.
    pub fn new(
        name: impl Into<String>,
        passes: PassPlan,
        white_ink: WhiteInkMode,
        gloss: GlossMode,
    ) -> Result<Self> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(DomainError::validation("name", "must not be empty"));
        }
        passes.validate()?;

        // Cross-invariants:
        if white_ink != WhiteInkMode::None && passes.white_passes == 0 {
            return Err(DomainError::Invariant(
                "white_ink is configured but pass plan has 0 white passes",
            ));
        }
        if white_ink == WhiteInkMode::None && passes.white_passes > 0 {
            return Err(DomainError::Invariant(
                "pass plan has white passes but white_ink is None",
            ));
        }
        if gloss != GlossMode::None && passes.gloss_passes == 0 {
            return Err(DomainError::Invariant(
                "gloss mode is configured but pass plan has 0 gloss passes",
            ));
        }
        if gloss == GlossMode::None && passes.gloss_passes > 0 {
            return Err(DomainError::Invariant(
                "pass plan has gloss passes but gloss mode is None",
            ));
        }

        Ok(Self {
            id: ProfileId::new(),
            name,
            passes,
            white_ink,
            gloss,
            final_cure_passes: 0,
            icc_profile: None,
            notes: None,
        })
    }

    pub fn with_icc(mut self, profile: impl Into<String>) -> Self {
        self.icc_profile = Some(profile.into());
        self
    }

    pub fn with_final_cure_passes(mut self, n: u8) -> Self {
        self.final_cure_passes = n;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_color_only_profile_is_valid() {
        let p = PrintProfile::new(
            "Color only",
            PassPlan::simple(2, 0, 0),
            WhiteInkMode::None,
            GlossMode::None,
        )
        .unwrap();
        assert_eq!(p.passes.color_passes, 2);
    }

    #[test]
    fn white_ink_with_zero_white_passes_rejected() {
        let err = PrintProfile::new(
            "Bad",
            PassPlan::simple(1, 0, 0),
            WhiteInkMode::UnderPrint,
            GlossMode::None,
        )
        .unwrap_err();
        assert!(matches!(err, DomainError::Invariant(_)));
    }

    #[test]
    fn white_passes_without_white_ink_rejected() {
        let err = PrintProfile::new(
            "Bad",
            PassPlan::simple(1, 1, 0),
            WhiteInkMode::None,
            GlossMode::None,
        )
        .unwrap_err();
        assert!(matches!(err, DomainError::Invariant(_)));
    }

    #[test]
    fn gloss_config_and_gloss_passes_must_agree() {
        let err = PrintProfile::new(
            "Bad",
            PassPlan::simple(1, 0, 0),
            WhiteInkMode::None,
            GlossMode::Full,
        )
        .unwrap_err();
        assert!(matches!(err, DomainError::Invariant(_)));

        let err = PrintProfile::new(
            "Bad",
            PassPlan::simple(1, 0, 2),
            WhiteInkMode::None,
            GlossMode::None,
        )
        .unwrap_err();
        assert!(matches!(err, DomainError::Invariant(_)));
    }

    #[test]
    fn full_spec_profile_valid() {
        let p = PrintProfile::new(
            "Premium sandwich on acrylic",
            PassPlan {
                color_passes: 3,
                white_passes: 2,
                gloss_passes: 1,
                cure_after_each: true,
            },
            WhiteInkMode::Sandwich,
            GlossMode::Full,
        )
        .unwrap()
        .with_icc("Roland_LEF2_Acrylic_v3")
        .with_final_cure_passes(1);
        assert_eq!(p.final_cure_passes, 1);
        assert_eq!(p.icc_profile.as_deref(), Some("Roland_LEF2_Acrylic_v3"));
    }

    #[test]
    fn empty_pass_plan_rejected() {
        let err = PrintProfile::new(
            "Nothing",
            PassPlan::simple(0, 0, 0),
            WhiteInkMode::None,
            GlossMode::None,
        )
        .unwrap_err();
        assert!(matches!(err, DomainError::Invariant(_)));
    }

    #[test]
    fn pass_plan_total_counts_all_channels() {
        let p = PassPlan {
            color_passes: 2,
            white_passes: 3,
            gloss_passes: 1,
            cure_after_each: false,
        };
        assert_eq!(p.total(), 6);
    }

    #[test]
    fn profile_serde_round_trip() {
        let p = PrintProfile::new(
            "X",
            PassPlan::simple(2, 0, 0),
            WhiteInkMode::None,
            GlossMode::None,
        )
        .unwrap();
        let s = serde_json::to_string(&p).unwrap();
        let back: PrintProfile = serde_json::from_str(&s).unwrap();
        assert_eq!(p, back);
    }
}
