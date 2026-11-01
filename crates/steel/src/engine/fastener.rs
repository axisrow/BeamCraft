// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A14.2 fastener quantities: the prototype of a definition and the per-component quantities of
//! a group of `n` fasteners (bolt amendment section 3).
//!
//! The geometry itself is the kernel's G05 composition ([`crate::kernel::fastener`]); the
//! engine owns the decisions around it: it maps kernel refusals to command fields, it reports
//! the group totals as the union of the components — the shank runs through the head, so their
//! overlap is counted once — and it reports the mass with its source: `mass_each` of the
//! definition when it declares one, the computed volume and density otherwise, unknown when no
//! density is known. A declared mass is never split into components and a component mass is
//! never invented, so the source is shown exactly as the P-Mass decision does for sections.

use crate::kernel::fastener::FastenerFailure;
use crate::model::fastener::FastenerSpec;
use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::limits::{MAX_BOLT_GROUP_SIZE, MAX_DENSITY_KG_M3};

pub use crate::kernel::fastener::{ComponentKind, FastenerBuild, FastenerSolid};

/// Where a reported mass comes from (bolt amendment section 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MassSource {
    /// The definition declares `mass_each_kg`.
    Declared,
    /// Computed from the mesh volumes and a known density.
    Computed,
}

impl MassSource {
    /// Stable token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Declared => "declared",
            Self::Computed => "computed",
        }
    }
}

/// Quantities of one component across a group of `n` fasteners.
#[derive(Debug, Clone, PartialEq)]
pub struct ComponentQuantity {
    /// Which component of the stack.
    pub component: crate::kernel::fastener::ComponentKind,
    /// How many pieces the group has (`n` for every component).
    pub count: u32,
    /// Volume of one piece in mm3 (the exact polygon volume).
    pub volume_each_mm3: f64,
    /// Volume of all pieces in mm3.
    pub volume_mm3: f64,
    /// Mass of all pieces in kg; `None` when it is unknown or not computable.
    pub mass_kg: Option<f64>,
}

/// Quantities of one group of `n` fasteners of one prototype.
#[derive(Debug, Clone, PartialEq)]
pub struct FastenerQuantities {
    /// The group size the quantities were computed for.
    pub fastener_count: u32,
    /// One row per component, in stack order.
    pub components: Vec<ComponentQuantity>,
    /// Volume of one fastener in mm3: the union of the components (the shank-through-head
    /// overlap counted once), below the sum of the component rows.
    pub volume_each_mm3: f64,
    /// Volume of the whole group in mm3.
    pub volume_mm3: f64,
    /// Mass of the whole group in kg, with [`Self::mass_source`]; `None` when unknown.
    pub mass_kg: Option<f64>,
    /// Source of the mass; `None` when no mass is known.
    pub mass_source: Option<MassSource>,
}

/// Builds one fastener prototype, mapping kernel refusals to command fields.
///
/// The definition is re-admitted first; then the length follows the dimension codes and a
/// washer switch the definition cannot honour is `INVALID_ARGUMENT`, as for the definition
/// itself. A G05 refusal names the geometry, as a sweep refusal does for a member.
pub fn prototype(
    spec: &FastenerSpec,
    length_mm: f64,
    washer_under_head: bool,
    washer_under_nut: bool,
) -> Result<FastenerBuild, CommandError> {
    crate::kernel::fastener::build(spec, length_mm, washer_under_head, washer_under_nut).map_err(
        |failure| match failure {
            FastenerFailure::Input(error) => error,
            profile => CommandError::new(
                profile.f04_code(),
                "geometry",
                "fastener component certification failed",
            ),
        },
    )
}

/// Computes the per-component quantities of a group of `count` fasteners over one prototype.
///
/// The component rows carry the volumes of their separate closed meshes. The totals are the
/// **union** of the components: the shank starts at the under-head plane and runs through the
/// head, so the group totals subtract the shank-section area over
/// `min(head_height_mm, length_mm)` — the head clears the shank by the definition's own rule,
/// so the overlap is exactly that prism, counted once. The mass follows the definition: a
/// declared `mass_each_kg` is the mass of one fastener and is shown as `declared` (never split
/// into components); otherwise a known density gives every component the computed mass of its
/// volume, shown as `computed`; with no density the mass is unknown. `count` follows the A05
/// group cap.
pub fn quantities(
    spec: &FastenerSpec,
    build: &FastenerBuild,
    count: u32,
    density_kg_m3: Option<f64>,
) -> Result<FastenerQuantities, CommandError> {
    if count == 0 {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "count",
            "a group has at least one fastener",
        ));
    }
    if count > MAX_BOLT_GROUP_SIZE as u32 {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "count",
            "the group exceeds the bolt group cap",
        ));
    }
    if let Some(density) = density_kg_m3 {
        if !density.is_finite() {
            return Err(CommandError::new(
                ErrorCode::NonFiniteNumber,
                "density_kg_m3",
                "value is not finite",
            ));
        }
        if density > MAX_DENSITY_KG_M3 {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "density_kg_m3",
                "density exceeds the density cap",
            ));
        }
        if density <= 0.0 {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "density_kg_m3",
                "density must be greater than zero",
            ));
        }
    }
    let declared = spec.mass_each_kg;
    let component_mass = |volume_mm3: f64| {
        declared
            .is_none()
            .then_some(density_kg_m3)
            .flatten()
            .map(|density| volume_mm3 * density / 1_000_000_000.0)
    };
    let components = build
        .components
        .iter()
        .map(|solid| {
            let volume_each_mm3 = solid.solid.volume_mm3();
            let volume_mm3 = volume_each_mm3 * f64::from(count);
            ComponentQuantity {
                component: solid.component,
                count,
                volume_each_mm3,
                volume_mm3,
                mass_kg: component_mass(volume_mm3),
            }
        })
        .collect();
    let volume_each_mm3 = build
        .components
        .iter()
        .map(|solid| solid.solid.volume_mm3())
        .sum::<f64>()
        - build
            .components
            .iter()
            .find(|solid| solid.component == ComponentKind::Shank)
            .map(|shank| shank.solid.net_area_mm2() * spec.head_height_mm.min(build.length_mm))
            .unwrap_or(0.0);
    Ok(FastenerQuantities {
        fastener_count: count,
        volume_each_mm3,
        volume_mm3: volume_each_mm3 * f64::from(count),
        mass_kg: match declared {
            Some(mass_each) => Some(mass_each * f64::from(count)),
            None => component_mass(volume_each_mm3 * f64::from(count)),
        },
        mass_source: match (declared, density_kg_m3) {
            (Some(_), _) => Some(MassSource::Declared),
            (None, Some(_)) => Some(MassSource::Computed),
            (None, None) => None,
        },
        components,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::fastener::{HeadKind, HoleMode, WasherSpec};

    /// The washered M16 bolt of the kernel suite, mass-declared variant included.
    fn spec(mass_each_kg: Option<f64>) -> FastenerSpec {
        FastenerSpec {
            designation: Some("M16".into()),
            standard: None,
            grade: None,
            display_name: None,
            kind: crate::model::fastener::FastenerKind::Bolt,
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
            mass_each_kg,
        }
    }

    fn build() -> FastenerBuild {
        prototype(&spec(None), 50.0, true, true).unwrap()
    }

    #[test]
    fn counts_and_volumes_scale_with_the_group_size() {
        let build = build();
        let one = quantities(&spec(None), &build, 1, Some(7850.0)).unwrap();
        let group = quantities(&spec(None), &build, 8, Some(7850.0)).unwrap();
        assert_eq!(group.components.len(), one.components.len());
        for (piece, row) in one.components.iter().zip(&group.components) {
            assert_eq!(row.count, 8);
            assert_eq!(row.volume_each_mm3, piece.volume_each_mm3);
            assert_eq!(row.volume_mm3, piece.volume_each_mm3 * 8.0);
            assert_eq!(row.mass_kg, piece.mass_kg.map(|m| m * 8.0));
        }
        // Every component of a group has n pieces: one prototype, placed n times.
        assert!(group.components.iter().all(|row| row.count == 8));
        assert_eq!(group.volume_each_mm3 * 8.0, group.volume_mm3);
    }

    #[test]
    fn a_declared_mass_is_shown_as_declared_and_never_split() {
        let build = build();
        let declared = quantities(&spec(Some(0.4)), &build, 4, Some(7850.0)).unwrap();
        assert_eq!(declared.mass_kg, Some(1.6));
        assert_eq!(declared.mass_source, Some(MassSource::Declared));
        assert!(declared.components.iter().all(|row| row.mass_kg.is_none()));
        // The declared mass wins over a known density.
        assert_eq!(
            quantities(&spec(Some(0.4)), &build, 4, None)
                .unwrap()
                .mass_kg,
            Some(1.6)
        );
    }

    #[test]
    fn the_totals_are_the_union_and_the_component_masses_stay_per_component() {
        let build = build();
        let computed = quantities(&spec(None), &build, 2, Some(7850.0)).unwrap();
        assert_eq!(computed.mass_source, Some(MassSource::Computed));
        // Analytic union: the sum of the analytic component volumes minus the shank prism the
        // head covers (the shank clears the head, so the overlap is exactly that prism, and a
        // polygon area stays inside its circle within the chord rule).
        let shank = build
            .components
            .iter()
            .find(|solid| solid.component == ComponentKind::Shank)
            .unwrap();
        let n = shank.approximation.unwrap().segments;
        let shank_area = polygon_area(8.0, n);
        let analytic_sum = hexagon_area(24.0) * 10.0
            + hexagon_area(24.0) * 14.8
            + washer_volume()
            + washer_volume()
            + shank_area * 50.0;
        let overlap = shank_area * 10.0f64.min(50.0);
        let union = analytic_sum - overlap;
        assert!(computed.volume_each_mm3 <= union);
        assert!(union - computed.volume_each_mm3 < union * 0.01);
        // Every computed mass is its own row's volume times the density; the group mass is the
        // union's, below the sum of the rows by the overlap.
        for row in &computed.components {
            assert!((row.mass_kg.unwrap() - row.volume_mm3 * 7850.0 / 1e9).abs() < 1e-12);
        }
        let rows: f64 = computed
            .components
            .iter()
            .map(|row| row.mass_kg.unwrap())
            .sum();
        assert!((computed.mass_kg.unwrap() - computed.volume_mm3 * 7850.0 / 1e9).abs() < 1e-12);
        assert!(computed.mass_kg.unwrap() < rows);
    }

    fn polygon_area(radius: f64, n: u32) -> f64 {
        f64::from(n) / 2.0 * radius * radius * (core::f64::consts::TAU / f64::from(n)).sin()
    }

    fn hexagon_area(across_flats: f64) -> f64 {
        3.0f64.sqrt() / 2.0 * across_flats * across_flats
    }

    /// Analytic washer volume of the suite washer (30/17/3) within its chord deficit.
    fn washer_volume() -> f64 {
        // The exact polygon values are the kernel's; the union test bounds the sum from above
        // by the analytic circle annulus and from below with the chord tolerance.
        core::f64::consts::PI * (15.0 * 15.0 - 8.5 * 8.5) * 3.0
    }

    #[test]
    fn an_unknown_density_leaves_the_mass_unknown() {
        let unknown = quantities(&spec(None), &build(), 3, None).unwrap();
        assert_eq!(unknown.mass_kg, None);
        assert_eq!(unknown.mass_source, None);
        assert!(unknown.components.iter().all(|row| row.mass_kg.is_none()));
        assert!(unknown.volume_mm3 > 0.0);
    }

    #[test]
    fn the_group_size_and_the_density_follow_the_codes() {
        let code = |count: u32, density: Option<f64>| {
            quantities(&spec(None), &build(), count, density).map_err(|e| e.code)
        };
        assert_eq!(code(0, Some(7850.0)), Err(ErrorCode::InvalidArgument));
        assert_eq!(
            code(MAX_BOLT_GROUP_SIZE as u32 + 1, Some(7850.0)),
            Err(ErrorCode::LimitExceeded)
        );
        assert_eq!(code(1, Some(f64::NAN)), Err(ErrorCode::NonFiniteNumber));
        assert_eq!(
            code(1, Some(MAX_DENSITY_KG_M3 * 2.0)),
            Err(ErrorCode::LimitExceeded)
        );
        assert_eq!(code(1, Some(0.0)), Err(ErrorCode::InvalidArgument));
    }

    #[test]
    fn prototype_refusals_name_the_field() {
        let error = prototype(&spec(None), 0.0005, true, true).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidArgument);
        assert_eq!(error.field, "length_mm");
        // A switch the definition cannot honour refuses before any mesh is built.
        let bare = FastenerSpec {
            washer: None,
            washer_under_head: false,
            washer_under_nut: false,
            ..spec(None)
        };
        assert_eq!(
            prototype(&bare, 50.0, true, false).unwrap_err().field,
            "washer_under_head"
        );
    }
}
