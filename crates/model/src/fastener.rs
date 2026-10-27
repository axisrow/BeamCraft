// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Logical content of an immutable fastener definition (A14.1, bolt amendment section 2).
//!
//! A fastener definition is the data of one size of one standard: the head, the optional nut and
//! washer, the standard hole clearance and the standard lengths, all as authored numbers in
//! millimetres. Like a profile or a material, the definition is admitted here by the F04 section 3
//! codes and caps and pinned elsewhere by the full content digest of its stored USD layer; this
//! module never reads a library and never infers a value from a designation. Threads are not
//! modelled, so there is no pitch or thread length here (bolt amendment section 3).

use crate::command::{CommandError, label_is_valid};
use crate::error::ErrorCode;
use crate::limits::{DIMENSION_CAP_MM, MAX_FASTENER_LENGTHS, MAX_FASTENER_MASS_KG};
use beamcraft_geom::tolerance::ABSOLUTE_FLOOR_MM;

/// What the fastener does in the joint (bolt amendment section 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FastenerKind {
    /// Runs through the parts; the nut tightens at the far end.
    Bolt,
    /// Threaded fastener without a nut (a screw, a stud, an anchor).
    Screw,
}

impl FastenerKind {
    /// Every kind, in token order.
    pub const ALL: [Self; 2] = [Self::Bolt, Self::Screw];

    /// Stable schema token.
    pub const fn token(self) -> &'static str {
        match self {
            Self::Bolt => "bolt",
            Self::Screw => "screw",
        }
    }

    /// The kind with this token, or `None`.
    pub fn from_token(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.token() == token)
    }
}

/// The head form. The first slice has only the hex head (bolt amendment section 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HeadKind {
    /// Hexagonal head, an extrusion of a regular hexagon.
    Hex,
}

impl HeadKind {
    /// Every head kind, in token order.
    pub const ALL: [Self; 1] = [Self::Hex];

    /// Stable schema token.
    pub const fn token(self) -> &'static str {
        match self {
            Self::Hex => "hex",
        }
    }

    /// The head kind with this token, or `None`.
    pub fn from_token(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.token() == token)
    }
}

/// Whether a group of this fastener cuts holes in the parts it passes through (bolt amendment
/// section 5). A definition carries the value the standard prefers; a group command may override
/// it (A14.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HoleMode {
    /// A round through hole is cut in every part the shank crosses.
    Generate,
    /// No hole is cut and no part is changed.
    None,
}

impl HoleMode {
    /// Every mode, in token order.
    pub const ALL: [Self; 2] = [Self::Generate, Self::None];

    /// Stable schema token.
    pub const fn token(self) -> &'static str {
        match self {
            Self::Generate => "generate",
            Self::None => "none",
        }
    }

    /// The mode with this token, or `None`.
    pub fn from_token(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.token() == token)
    }
}

/// The round flat washer of a definition (one form in the first slice).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WasherSpec {
    /// Outer diameter in millimetres.
    pub outer_diameter_mm: f64,
    /// Inner (hole) diameter in millimetres; the shank passes through it.
    pub inner_diameter_mm: f64,
    /// Thickness in millimetres.
    pub thickness_mm: f64,
}

impl WasherSpec {
    /// Admits the three dimensions: finite (`NON_FINITE_NUMBER`), at most the dimension cap
    /// (`LIMIT_EXCEEDED`), at least the geometric floor (`INVALID_ARGUMENT`); the inner diameter
    /// lies strictly between the nominal diameter and the outer diameter (`INVALID_ARGUMENT`).
    fn validate(&self, nominal_diameter_mm: f64) -> Result<(), CommandError> {
        let (outer, inner, thickness) = (
            self.outer_diameter_mm,
            self.inner_diameter_mm,
            self.thickness_mm,
        );
        let finite = [outer, inner, thickness];
        if finite.iter().any(|v| !v.is_finite()) {
            return Err(CommandError::new(
                ErrorCode::NonFiniteNumber,
                "washer_outer_diameter_mm",
                "value is not finite",
            ));
        }
        if finite.iter().any(|v| *v > DIMENSION_CAP_MM) {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "washer_outer_diameter_mm",
                "washer dimension exceeds the dimension cap",
            ));
        }
        let floor = |value: f64, field: &'static str, message: &'static str| {
            if value < ABSOLUTE_FLOOR_MM {
                Err(CommandError::new(
                    ErrorCode::InvalidArgument,
                    field,
                    message,
                ))
            } else {
                Ok(())
            }
        };
        floor(
            outer,
            "washer_outer_diameter_mm",
            "washer dimension is below the geometric floor",
        )?;
        floor(
            thickness,
            "washer_thickness_mm",
            "washer thickness is below the geometric floor",
        )?;
        if inner <= nominal_diameter_mm {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "washer_inner_diameter_mm",
                "the washer hole does not pass the shank",
            ));
        }
        if inner >= outer {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "washer_inner_diameter_mm",
                "the washer hole is not inside the washer",
            ));
        }
        Ok(())
    }
}

/// The logical content of one fastener definition (bolt amendment section 2). Every dimension is
/// an authored number in millimetres; no value is inferred from the designation. There is no
/// default definition: a `Default` would be an unvalidated fastener.
#[derive(Debug, Clone, PartialEq)]
pub struct FastenerSpec {
    /// Designation as printed in the cited standard, a label only.
    pub designation: Option<String>,
    /// Standard that defines the definition, a label only.
    pub standard: Option<String>,
    /// Declared grade (property class), a label only.
    pub grade: Option<String>,
    /// Readable name shown in listings; part of the definition content, so another name is
    /// another digest. A label only.
    pub display_name: Option<String>,
    /// What the fastener does in the joint.
    pub kind: FastenerKind,
    /// The head form.
    pub head_kind: HeadKind,
    /// Shank diameter in millimetres.
    pub nominal_diameter_mm: f64,
    /// Width across the head flats in millimetres.
    pub head_across_flats_mm: f64,
    /// Head height in millimetres.
    pub head_height_mm: f64,
    /// Width across the nut flats in millimetres; absent for a [`FastenerKind::Screw`].
    pub nut_across_flats_mm: Option<f64>,
    /// Nut height in millimetres; absent for a [`FastenerKind::Screw`].
    pub nut_height_mm: Option<f64>,
    /// The round washer the definition carries, when it does.
    pub washer: Option<WasherSpec>,
    /// Diameter added to the nominal diameter for the standard hole
    /// (`hole = nominal_diameter + clearance`); zero is a fitted hole.
    pub hole_clearance_mm: f64,
    /// The standard lengths, strictly ascending, at least one.
    pub lengths_mm: Vec<f64>,
    /// Minimum shank length beyond the nut (or beyond the last part for a screw), in millimetres.
    pub thread_protrusion_mm: f64,
    /// Optional declared mass of the bolt, nut and washers of one fastener in kg; otherwise mass
    /// is computed from volume and the density of the definition's material (A14.2).
    pub mass_each_kg: Option<f64>,
    /// Whether the standard prefers a washer under the head.
    pub washer_under_head: bool,
    /// Whether the standard prefers a washer under the nut.
    pub washer_under_nut: bool,
    /// The hole setting the standard prefers.
    pub hole_mode: HoleMode,
}

fn invalid(field: &'static str, message: &'static str) -> CommandError {
    CommandError::new(ErrorCode::InvalidArgument, field, message)
}

fn dimension(value: f64, field: &'static str, message: &'static str) -> Result<(), CommandError> {
    if !value.is_finite() {
        return Err(CommandError::new(
            ErrorCode::NonFiniteNumber,
            field,
            "value is not finite",
        ));
    }
    if value > DIMENSION_CAP_MM {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            field,
            "dimension exceeds the dimension cap",
        ));
    }
    if value < ABSOLUTE_FLOOR_MM {
        return Err(invalid(field, message));
    }
    Ok(())
}

impl FastenerSpec {
    /// Admits every value with the F04 codes and caps, then the definition's own consistency
    /// rules: a bolt carries a nut, a screw none; a washer switch is only true for a washer the
    /// definition carries (and under a nut only for a bolt); the head and nut clear the shank;
    /// the standard lengths ascend strictly.
    pub fn validate(&self) -> Result<(), CommandError> {
        for (value, field) in [
            (&self.designation, "designation"),
            (&self.standard, "standard"),
            (&self.grade, "grade"),
            (&self.display_name, "display_name"),
        ] {
            if let Some(text) = value
                && !label_is_valid(text)
            {
                return Err(invalid(
                    field,
                    "label is empty, too long or has control characters",
                ));
            }
        }
        dimension(
            self.nominal_diameter_mm,
            "nominal_diameter_mm",
            "nominal diameter is below the geometric floor",
        )?;
        dimension(
            self.head_across_flats_mm,
            "head_across_flats_mm",
            "head across flats is below the geometric floor",
        )?;
        dimension(
            self.head_height_mm,
            "head_height_mm",
            "head height is below the geometric floor",
        )?;
        if self.head_across_flats_mm <= self.nominal_diameter_mm {
            return Err(invalid(
                "head_across_flats_mm",
                "the head does not clear the shank",
            ));
        }
        if self.kind == FastenerKind::Bolt {
            let (across, height) = (self.nut_across_flats_mm, self.nut_height_mm);
            let (Some(across), Some(height)) = (across, height) else {
                return Err(invalid(
                    "nut_across_flats_mm",
                    "a bolt definition carries a nut",
                ));
            };
            dimension(
                across,
                "nut_across_flats_mm",
                "nut across flats is below the geometric floor",
            )?;
            dimension(
                height,
                "nut_height_mm",
                "nut height is below the geometric floor",
            )?;
            if across <= self.nominal_diameter_mm {
                return Err(invalid(
                    "nut_across_flats_mm",
                    "the nut does not clear the shank",
                ));
            }
        } else if self.nut_across_flats_mm.is_some() || self.nut_height_mm.is_some() {
            return Err(invalid(
                "nut_across_flats_mm",
                "a screw definition carries no nut",
            ));
        }
        if let Some(washer) = &self.washer {
            washer.validate(self.nominal_diameter_mm)?;
        }
        if self.washer_under_head && self.washer.is_none() {
            return Err(invalid(
                "washer_under_head",
                "the definition carries no washer to place under the head",
            ));
        }
        if self.washer_under_nut && (self.washer.is_none() || self.kind != FastenerKind::Bolt) {
            return Err(invalid(
                "washer_under_nut",
                "a washer under the nut needs a washer and a bolt",
            ));
        }
        if !self.hole_clearance_mm.is_finite() {
            return Err(CommandError::new(
                ErrorCode::NonFiniteNumber,
                "hole_clearance_mm",
                "value is not finite",
            ));
        }
        if self.hole_clearance_mm < 0.0 || self.hole_clearance_mm > DIMENSION_CAP_MM {
            return Err(invalid(
                "hole_clearance_mm",
                "hole clearance is negative or above the dimension cap",
            ));
        }
        if self.lengths_mm.is_empty() {
            return Err(invalid(
                "lengths_mm",
                "a definition lists at least one standard length",
            ));
        }
        if self.lengths_mm.len() > MAX_FASTENER_LENGTHS {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "lengths_mm",
                "too many standard lengths",
            ));
        }
        for pair in self.lengths_mm.windows(2) {
            if !pair.iter().all(|v| v.is_finite()) {
                return Err(CommandError::new(
                    ErrorCode::NonFiniteNumber,
                    "lengths_mm",
                    "value is not finite",
                ));
            }
            let [a, b] = pair else {
                continue;
            };
            if *a > DIMENSION_CAP_MM || *b > DIMENSION_CAP_MM {
                return Err(CommandError::new(
                    ErrorCode::LimitExceeded,
                    "lengths_mm",
                    "a standard length exceeds the dimension cap",
                ));
            }
            if *a < ABSOLUTE_FLOOR_MM {
                return Err(invalid(
                    "lengths_mm",
                    "a standard length is below the geometric floor",
                ));
            }
            if a >= b {
                return Err(invalid(
                    "lengths_mm",
                    "the standard lengths do not ascend strictly",
                ));
            }
        }
        dimension(
            self.thread_protrusion_mm,
            "thread_protrusion_mm",
            "thread protrusion is below the geometric floor",
        )?;
        if let Some(mass) = self.mass_each_kg {
            if !mass.is_finite() {
                return Err(CommandError::new(
                    ErrorCode::NonFiniteNumber,
                    "mass_each_kg",
                    "value is not finite",
                ));
            }
            if mass > MAX_FASTENER_MASS_KG {
                return Err(CommandError::new(
                    ErrorCode::LimitExceeded,
                    "mass_each_kg",
                    "declared mass exceeds the mass cap",
                ));
            }
            if mass <= 0.0 {
                return Err(invalid(
                    "mass_each_kg",
                    "declared mass must be greater than zero",
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal admitted bolt definition.
    fn bolt() -> FastenerSpec {
        FastenerSpec {
            kind: FastenerKind::Bolt,
            head_kind: HeadKind::Hex,
            nominal_diameter_mm: 16.0,
            head_across_flats_mm: 24.0,
            head_height_mm: 10.0,
            nut_across_flats_mm: Some(24.0),
            nut_height_mm: Some(14.8),
            washer: Some(WasherSpec {
                outer_diameter_mm: 30.0,
                inner_diameter_mm: 17.0,
                thickness_mm: 3.0,
            }),
            hole_clearance_mm: 2.0,
            lengths_mm: vec![45.0, 50.0, 60.0, 80.0],
            thread_protrusion_mm: 5.0,
            washer_under_head: true,
            washer_under_nut: true,
            hole_mode: HoleMode::Generate,
            designation: Some("M16".into()),
            standard: Some("DIN EN ISO 4014".into()),
            grade: Some("8.8".into()),
            display_name: None,
            mass_each_kg: None,
        }
    }

    fn code(spec: &FastenerSpec) -> Result<(), ErrorCode> {
        spec.validate().map_err(|e| e.code)
    }

    #[test]
    fn an_admitted_bolt_and_a_screw_validate() {
        assert_eq!(code(&bolt()), Ok(()));
        let screw = FastenerSpec {
            kind: FastenerKind::Screw,
            nut_across_flats_mm: None,
            nut_height_mm: None,
            washer_under_nut: false,
            ..bolt()
        };
        assert_eq!(code(&screw), Ok(()));
    }

    #[test]
    fn bolt_and_screw_rules_name_the_field() {
        let nutless = FastenerSpec {
            nut_across_flats_mm: None,
            ..bolt()
        };
        assert_eq!(
            nutless.validate().map_err(|e| e.field),
            Err("nut_across_flats_mm")
        );
        let screw_with_nut = FastenerSpec {
            kind: FastenerKind::Screw,
            nut_across_flats_mm: Some(24.0),
            nut_height_mm: Some(14.8),
            washer_under_nut: false,
            ..bolt()
        };
        assert_eq!(
            screw_with_nut.validate().map_err(|e| e.field),
            Err("nut_across_flats_mm")
        );
    }

    #[test]
    fn washer_switches_need_a_washer_and_a_bolt() {
        let bare = FastenerSpec {
            washer: None,
            ..bolt()
        };
        assert_eq!(
            bare.validate().map_err(|e| e.field),
            Err("washer_under_head")
        );
        let screw = FastenerSpec {
            kind: FastenerKind::Screw,
            nut_across_flats_mm: None,
            nut_height_mm: None,
            washer_under_nut: true,
            ..bolt()
        };
        assert_eq!(
            screw.validate().map_err(|e| e.field),
            Err("washer_under_nut")
        );
    }

    #[test]
    fn washer_dimensions_must_pass_the_shank_and_stay_round() {
        let small = FastenerSpec {
            washer: Some(WasherSpec {
                outer_diameter_mm: 30.0,
                inner_diameter_mm: 15.0,
                thickness_mm: 3.0,
            }),
            ..bolt()
        };
        assert_eq!(
            small.validate().map_err(|e| e.field),
            Err("washer_inner_diameter_mm")
        );
        let outer = FastenerSpec {
            washer: Some(WasherSpec {
                outer_diameter_mm: 16.0,
                inner_diameter_mm: 17.0,
                thickness_mm: 3.0,
            }),
            ..bolt()
        };
        assert_eq!(
            outer.validate().map_err(|e| e.field),
            Err("washer_inner_diameter_mm")
        );
    }

    #[test]
    fn stage_order_is_finite_then_caps_then_floor() {
        let nan = FastenerSpec {
            nominal_diameter_mm: f64::NAN,
            ..bolt()
        };
        assert_eq!(code(&nan), Err(ErrorCode::NonFiniteNumber));
        let huge = FastenerSpec {
            head_across_flats_mm: 2e6,
            ..bolt()
        };
        assert_eq!(code(&huge), Err(ErrorCode::LimitExceeded));
        let tiny = FastenerSpec {
            head_height_mm: 0.0005,
            ..bolt()
        };
        assert_eq!(code(&tiny), Err(ErrorCode::InvalidArgument));
        let negative_clearance = FastenerSpec {
            hole_clearance_mm: -1.0,
            ..bolt()
        };
        assert_eq!(code(&negative_clearance), Err(ErrorCode::InvalidArgument));
        let zero_clearance_is_a_fitted_hole = FastenerSpec {
            hole_clearance_mm: 0.0,
            ..bolt()
        };
        assert_eq!(code(&zero_clearance_is_a_fitted_hole), Ok(()));
    }

    #[test]
    fn lengths_ascend_strictly_and_are_capped() {
        let equal = FastenerSpec {
            lengths_mm: vec![50.0, 50.0],
            ..bolt()
        };
        assert_eq!(code(&equal), Err(ErrorCode::InvalidArgument));
        let descending = FastenerSpec {
            lengths_mm: vec![60.0, 50.0],
            ..bolt()
        };
        assert_eq!(code(&descending), Err(ErrorCode::InvalidArgument));
        let many = FastenerSpec {
            lengths_mm: (1..=MAX_FASTENER_LENGTHS as i64 + 1)
                .map(|n| n as f64 * 10.0)
                .collect(),
            ..bolt()
        };
        assert_eq!(code(&many), Err(ErrorCode::LimitExceeded));
        let empty = FastenerSpec {
            lengths_mm: vec![],
            ..bolt()
        };
        assert_eq!(code(&empty), Err(ErrorCode::InvalidArgument));
    }

    #[test]
    fn labels_and_declared_mass_follow_the_label_rules() {
        let bad_label = FastenerSpec {
            designation: Some("a\nb".into()),
            ..bolt()
        };
        assert_eq!(code(&bad_label), Err(ErrorCode::InvalidArgument));
        let bad_mass = FastenerSpec {
            mass_each_kg: Some(0.0),
            ..bolt()
        };
        assert_eq!(code(&bad_mass), Err(ErrorCode::InvalidArgument));
        let heavy = FastenerSpec {
            mass_each_kg: Some(MAX_FASTENER_MASS_KG * 2.0),
            ..bolt()
        };
        assert_eq!(code(&heavy), Err(ErrorCode::LimitExceeded));
        let unknown_mass_is_admitted = FastenerSpec {
            mass_each_kg: None,
            ..bolt()
        };
        assert_eq!(code(&unknown_mass_is_admitted), Ok(()));
    }

    #[test]
    fn tokens_round_trip() {
        for kind in FastenerKind::ALL {
            assert_eq!(FastenerKind::from_token(kind.token()), Some(kind));
        }
        for head in HeadKind::ALL {
            assert_eq!(HeadKind::from_token(head.token()), Some(head));
        }
        for mode in HoleMode::ALL {
            assert_eq!(HoleMode::from_token(mode.token()), Some(mode));
        }
        assert_eq!(FastenerKind::from_token("stud"), None);
        assert_eq!(HeadKind::from_token("round"), None);
        assert_eq!(HoleMode::from_token("blind"), None);
    }
}
