//! Material catalog types — substrate, ink channels, material record.
//!
//! Materials describe *what you're printing on*. Profiles (see
//! `profile.rs`) describe *how you print on it*. A job references both.

use serde::{Deserialize, Serialize};

use crate::error::{DomainError, Result};
use crate::ids::MaterialId;

/// Physical substrate category. `Other(name)` carries an arbitrary label
/// for rare / niche substrates the operator wants to track without
/// needing a new enum variant.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Substrate {
    Acrylic,
    Wood,
    Metal,
    Glass,
    Leather,
    Fabric,
    Paper,
    Foam,
    Plastic,
    Other { name: String },
}

impl Substrate {
    /// Human-friendly display name.
    pub fn label(&self) -> &str {
        match self {
            Self::Acrylic => "acrylic",
            Self::Wood => "wood",
            Self::Metal => "metal",
            Self::Glass => "glass",
            Self::Leather => "leather",
            Self::Fabric => "fabric",
            Self::Paper => "paper",
            Self::Foam => "foam",
            Self::Plastic => "plastic",
            Self::Other { name } => name,
        }
    }
}

/// Ink channels the LEF2-300 supports. `Other` is an escape hatch for
/// experimental / unsupported channels the operator wants to track.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "channel", rename_all = "kebab-case")]
pub enum InkChannel {
    Cyan,
    Magenta,
    Yellow,
    Black,
    White,
    Gloss,
    Primer,
    Other { name: String },
}

impl InkChannel {
    pub fn label(&self) -> &str {
        match self {
            Self::Cyan => "cyan",
            Self::Magenta => "magenta",
            Self::Yellow => "yellow",
            Self::Black => "black",
            Self::White => "white",
            Self::Gloss => "gloss",
            Self::Primer => "primer",
            Self::Other { name } => name,
        }
    }
}

/// A material entry in the operator's catalog.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Material {
    pub id: MaterialId,
    pub name: String,
    pub substrate: Substrate,
    pub thickness_mm: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sku: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

impl Material {
    /// Create a new material. Validates that:
    ///   * name is non-empty
    ///   * thickness is finite and in (0, 200] mm (LEF2-300 bed capacity)
    pub fn new(
        name: impl Into<String>,
        substrate: Substrate,
        thickness_mm: f64,
    ) -> Result<Self> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(DomainError::validation("name", "must not be empty"));
        }
        if !thickness_mm.is_finite() {
            return Err(DomainError::validation("thickness_mm", "must be finite"));
        }
        if thickness_mm <= 0.0 {
            return Err(DomainError::validation("thickness_mm", "must be > 0"));
        }
        if thickness_mm > 200.0 {
            return Err(DomainError::validation(
                "thickness_mm",
                "exceeds LEF2-300 bed capacity (200mm)",
            ));
        }
        Ok(Self {
            id: MaterialId::new(),
            name,
            substrate,
            thickness_mm,
            vendor: None,
            sku: None,
            notes: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn construct_valid_material() {
        let m = Material::new("Acrylic 3mm clear", Substrate::Acrylic, 3.0).unwrap();
        assert_eq!(m.substrate, Substrate::Acrylic);
        assert_eq!(m.thickness_mm, 3.0);
    }

    #[test]
    fn reject_empty_name() {
        assert!(Material::new("", Substrate::Acrylic, 3.0).is_err());
        assert!(Material::new("   ", Substrate::Acrylic, 3.0).is_err());
    }

    #[test]
    fn reject_zero_or_negative_thickness() {
        assert!(Material::new("x", Substrate::Acrylic, 0.0).is_err());
        assert!(Material::new("x", Substrate::Acrylic, -1.0).is_err());
    }

    #[test]
    fn reject_thickness_beyond_bed() {
        assert!(Material::new("x", Substrate::Acrylic, 200.1).is_err());
        assert!(Material::new("x", Substrate::Acrylic, 500.0).is_err());
    }

    #[test]
    fn accept_thickness_at_bed_limit() {
        assert!(Material::new("x", Substrate::Acrylic, 200.0).is_ok());
    }

    #[test]
    fn reject_nan_thickness() {
        assert!(Material::new("x", Substrate::Acrylic, f64::NAN).is_err());
        assert!(Material::new("x", Substrate::Acrylic, f64::INFINITY).is_err());
    }

    #[test]
    fn substrate_other_carries_label() {
        let s = Substrate::Other {
            name: "corian".into(),
        };
        assert_eq!(s.label(), "corian");
    }

    #[test]
    fn ink_channel_labels_match() {
        assert_eq!(InkChannel::Cyan.label(), "cyan");
        assert_eq!(InkChannel::White.label(), "white");
        assert_eq!(
            InkChannel::Other {
                name: "orange".into()
            }
            .label(),
            "orange"
        );
    }

    #[test]
    fn material_serde_round_trip() {
        let m = Material::new("Oak 5mm", Substrate::Wood, 5.0).unwrap();
        let s = serde_json::to_string(&m).unwrap();
        let back: Material = serde_json::from_str(&s).unwrap();
        assert_eq!(m, back);
    }
}
