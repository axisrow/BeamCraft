// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A14.2 fastener geometry: one closed mesh per component of one pinned fastener definition.
//!
//! A fastener of a definition is stacked along the shank axis `+z` from its bearing face
//! (bolt amendment section 3): the optional washer under the head, the head, the shank, the
//! optional nut and the optional washer under the nut. Every component is a separate sealed
//! kernel extrusion (G05) so colour, quantities and the BOM read them one by one; threads are
//! not modelled and edges are not chamfered. A group of `n` fasteners places this one prototype
//! `n` times — the builder takes no count, so identical definitions share identical meshes.
//!
//! Round parts (the shank, the washer faces) are regular polygons with a vertex count that is a
//! multiple of four and a chord deviation of at most 0.05 mm, the rule of the round hole (F06
//! A-2 D-2); the head and nut are exact hexagons. Volumes are the kernel's exact polygon
//! volumes. The mass is the definition's declared `mass_each_kg`; without a declared mass it
//! stays unknown — the definition carries no material density, and none is inferred.

use crate::solid::SolidMesh;
use beamcraft_kernel::profile::{Cavity, CurveSpec, Point2, extrude};
use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::fastener::{FastenerKind, FastenerSpec};

/// The chord deviation bound of every round part, in millimetres (F06 A-2 D-2).
const ROUND_CHORD_MM: f64 = 0.05;
/// The curve specification of a round fastener part: the 0.05 mm chord rule over the kernel's
/// segment range (a multiple of four, at least 8, at most 1020).
const ROUND_CURVE: CurveSpec = CurveSpec {
    max_sagitta_mm: ROUND_CHORD_MM,
    min_segments: 8,
    max_segments: 1020,
};

/// What a component of a fastener is (bolt amendment section 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ComponentKind {
    /// The head the shank grows from.
    Head,
    /// The plain cylindrical shank of the nominal diameter.
    Shank,
    /// The nut at the far end of a bolt.
    Nut,
    /// A round flat washer under the head or under the nut.
    Washer,
}

impl ComponentKind {
    /// Stable schema token.
    pub const fn token(self) -> &'static str {
        match self {
            Self::Head => "head",
            Self::Shank => "shank",
            Self::Nut => "nut",
            Self::Washer => "washer",
        }
    }
}

/// Where a fastener's mass comes from. `Computed` needs a density source the definition does not
/// carry yet, so a fastener without a declared mass keeps an unknown mass, never a guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MassSource {
    /// The definition's declared `mass_each_kg`.
    Declared,
    /// No declared mass and no density to compute one from.
    Unknown,
}

impl MassSource {
    /// Stable schema token.
    pub const fn token(self) -> &'static str {
        match self {
            Self::Declared => "declared",
            Self::Unknown => "unknown",
        }
    }
}

/// One component of a fastener prototype: a separate closed mesh, its exact volume and its
/// place in the stack.
#[derive(Debug, Clone, PartialEq)]
pub struct FastenerComponent {
    /// What the component is.
    pub kind: ComponentKind,
    /// The sealed closed mesh, prototype-local millimetres, extruded along `+z` over
    /// `z_end_mm - z_start_mm`.
    pub mesh: SolidMesh,
    /// Exact volume of the component in mm3 (the polygon area times the extrusion length).
    pub volume_mm3: f64,
    /// Offset of the component's start face along the shank axis, from the bearing face.
    pub z_start_mm: f64,
    /// Offset of the component's end face along the shank axis.
    pub z_end_mm: f64,
}

/// One fastener of a definition: the component stack and the fastener's mass. A group places
/// this prototype once per fastener; the meshes do not depend on the group.
#[derive(Debug, Clone, PartialEq)]
pub struct FastenerPrototype {
    /// The components in stack order (ascending `z_start_mm`).
    pub components: Vec<FastenerComponent>,
    /// The shank length the prototype was built with, in millimetres.
    pub length_mm: f64,
    /// The fastener's mass in kg, present exactly for a [`MassSource::Declared`].
    pub mass_kg: Option<f64>,
    /// Where the mass comes from.
    pub mass_source: MassSource,
}

/// The count of one component kind of a fastener (1 for every present kind; a group of `n`
/// multiplies by `n`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComponentCount {
    /// The component kind.
    pub kind: ComponentKind,
    /// How many the fastener carries.
    pub count: u64,
}

impl FastenerPrototype {
    /// The count of every component kind, in stack order. Each present kind appears once.
    pub fn counts(&self) -> Vec<ComponentCount> {
        let mut counts: Vec<ComponentCount> = Vec::new();
        for component in &self.components {
            match counts.iter_mut().find(|c| c.kind == component.kind) {
                Some(count) => count.count += 1,
                None => counts.push(ComponentCount {
                    kind: component.kind,
                    count: 1,
                }),
            }
        }
        counts
    }

    /// The exact volume of the whole fastener in mm3, the sum of its components.
    pub fn volume_mm3(&self) -> f64 {
        self.components.iter().map(|c| c.volume_mm3).sum()
    }
}

/// A regular polygon on a circle: counter-clockwise vertices starting at angle 0, with a chord
/// deviation of at most [`ROUND_CHORD_MM`] and a vertex count that is a multiple of four.
fn ring(radius_mm: f64) -> Vec<Point2> {
    // r (1 - cos(pi/n)) <= chord bound  =>  n >= pi / acos(1 - sag/r); the argument is clamped,
    // and the kernel's own floor and cap (8 and 1020, rounded to a multiple of four) apply.
    let ratio = (1.0 - ROUND_CHORD_MM / radius_mm).clamp(-1.0, 1.0);
    let needed = core::f64::consts::PI / ratio.acos();
    let count = (needed.ceil() as u32).clamp(8, 1020).next_multiple_of(4);
    (0..count)
        .map(|i| {
            let angle = core::f64::consts::TAU * i as f64 / count as f64;
            Point2 {
                x: radius_mm * angle.cos(),
                y: radius_mm * angle.sin(),
            }
        })
        .collect()
}

/// A regular hexagon across the flats `across_flats_mm`, counter-clockwise from angle 0.
fn hexagon(across_flats_mm: f64) -> Vec<Point2> {
    let circumradius = across_flats_mm / 3.0_f64.sqrt();
    (0..6)
        .map(|i| {
            let angle = core::f64::consts::TAU * i as f64 / 6.0;
            Point2 {
                x: circumradius * angle.cos(),
                y: circumradius * angle.sin(),
            }
        })
        .collect()
}

/// The refusal for a component the kernel cannot build.
fn kernel_refused() -> CommandError {
    CommandError::new(
        ErrorCode::UnsupportedGeometry,
        "fastener",
        "the fastener component could not be built",
    )
}

/// Extrudes one component and seals it into an engine mesh with its exact volume.
fn solid(
    outer: &[Point2],
    cavities: &[Cavity],
    length_mm: f64,
) -> Result<(SolidMesh, f64), CommandError> {
    let extrusion = extrude(outer, cavities, length_mm).map_err(|_| kernel_refused())?;
    let volume = extrusion.volume_mm3();
    let mesh = SolidMesh::from_extrusion(&extrusion).ok_or_else(kernel_refused)?;
    Ok((mesh, volume))
}

/// One placed component: the mesh over `z_start_mm..z_end_mm`, pushed onto the stack.
fn place(
    components: &mut Vec<FastenerComponent>,
    z_start_mm: f64,
    kind: ComponentKind,
    outer: &[Point2],
    cavities: &[Cavity],
    length_mm: f64,
) -> Result<(), CommandError> {
    let (mesh, volume) = solid(outer, cavities, length_mm)?;
    components.push(FastenerComponent {
        kind,
        mesh,
        volume_mm3: volume,
        z_start_mm,
        z_end_mm: z_start_mm + length_mm,
    });
    Ok(())
}

/// The round washer of a definition, placed at `z_start_mm`, when the switch wants one.
fn washer(
    components: &mut Vec<FastenerComponent>,
    z_start_mm: f64,
    spec: &FastenerSpec,
    under_head: bool,
) -> Result<bool, CommandError> {
    let Some(washer) = &spec.washer else {
        return Ok(false);
    };
    let wanted = if under_head {
        spec.washer_under_head
    } else {
        spec.washer_under_nut
    };
    if !wanted {
        return Ok(false);
    }
    let cavity = Cavity::circle(
        Point2 { x: 0.0, y: 0.0 },
        washer.inner_diameter_mm / 2.0,
        ROUND_CURVE,
    )
    .map_err(|_| kernel_refused())?;
    place(
        components,
        z_start_mm,
        ComponentKind::Washer,
        &ring(washer.outer_diameter_mm / 2.0),
        &[cavity],
        washer.thickness_mm,
    )?;
    Ok(true)
}

/// Builds the prototype of one fastener of `spec` with the shank length `length_mm`, which must
/// be one of the definition's standard lengths.
///
/// The standard length is the shank length under the head, and the parts hang on it: it covers
/// the grip, the washers the shank carries (the washer under the nut), the nut and the
/// `thread_protrusion` (bolt amendment section 4). The layout is the part-free exploded stack:
/// the grip space between the head and the washer under the nut is where the connected parts
/// sit, and the nut ends at `shank end - protrusion`, leaving the protrusion as bare shank
/// beyond it. A14.3 places the group from these offsets and the grip
/// (`length - washer - nut - protrusion`), without re-deriving the stack.
///
/// Errors: `INVALID_ARGUMENT` for a definition or a length that is not admitted;
/// `UNSUPPORTED_GEOMETRY` when the kernel refuses a component or the length does not cover the
/// definition's own nut, washer and protrusion. Nothing here depends on the group size: a group
/// of `n` builds this prototype once.
pub fn prototype(spec: &FastenerSpec, length_mm: f64) -> Result<FastenerPrototype, CommandError> {
    spec.validate()?;
    if !spec.lengths_mm.contains(&length_mm) {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "length_mm",
            "the length is not one of the definition's standard lengths",
        ));
    }
    let mut components: Vec<FastenerComponent> = Vec::new();
    let mut z = 0.0;
    let under_head = washer(&mut components, z, spec, true)?;
    if under_head {
        z += spec.washer.as_ref().map_or(0.0, |w| w.thickness_mm);
    }
    place(
        &mut components,
        z,
        ComponentKind::Head,
        &hexagon(spec.head_across_flats_mm),
        &[],
        spec.head_height_mm,
    )?;
    let shank_start = z + spec.head_height_mm;
    // The shank runs under the head over the whole standard length; the nut ends
    // `thread_protrusion` before its end.
    let shank_end = shank_start + length_mm;
    place(
        &mut components,
        shank_start,
        ComponentKind::Shank,
        &ring(spec.nominal_diameter_mm / 2.0),
        &[],
        length_mm,
    )?;
    if spec.kind == FastenerKind::Bolt {
        // A bolt always carries a nut; the unwrapped defaults are only for the compiler.
        let nut_height = spec.nut_height_mm.unwrap_or_default();
        let nut_across = spec.nut_across_flats_mm.unwrap_or_default();
        let nut_end = shank_end - spec.thread_protrusion_mm;
        let washer_thickness = if spec.washer_under_nut {
            spec.washer.as_ref().map_or(0.0, |w| w.thickness_mm)
        } else {
            0.0
        };
        let nut_start = nut_end - washer_thickness - nut_height;
        if nut_start < shank_start {
            return Err(CommandError::new(
                ErrorCode::UnsupportedGeometry,
                "length_mm",
                "the standard length does not cover the definition's own nut, washer and protrusion",
            ));
        }
        let under_nut = washer(&mut components, nut_start, spec, false)?;
        let nut_offset = if under_nut { washer_thickness } else { 0.0 };
        place(
            &mut components,
            nut_start + nut_offset,
            ComponentKind::Nut,
            &hexagon(nut_across),
            &[],
            nut_height,
        )?;
    }
    Ok(FastenerPrototype {
        mass_kg: spec.mass_each_kg,
        mass_source: if spec.mass_each_kg.is_some() {
            MassSource::Declared
        } else {
            MassSource::Unknown
        },
        components,
        length_mm,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use beamcraft_model::fastener::{HeadKind, HoleMode, WasherSpec};

    const PI: f64 = core::f64::consts::PI;

    /// A washered bolt definition, as the amendment's acceptance case 1 describes one.
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
            lengths_mm: vec![45.0, 50.0, 60.0],
            thread_protrusion_mm: 5.0,
            washer_under_head: true,
            washer_under_nut: true,
            hole_mode: HoleMode::Generate,
            mass_each_kg: None,
            designation: None,
            standard: None,
            grade: None,
            display_name: None,
        }
    }

    fn kinds(prototype: &FastenerPrototype) -> Vec<ComponentKind> {
        prototype.components.iter().map(|c| c.kind).collect()
    }

    #[test]
    fn component_volumes_match_the_analytic_values_within_the_chord_rule() {
        let prototype = prototype(&bolt(), 60.0).unwrap();
        // The hexagon is an exact polygon: its prism volume is exact.
        let hex_area = 3.0_f64.sqrt() / 2.0 * 24.0 * 24.0;
        // The round parts are polygons on the 0.05 mm chord: each cap area misses the exact
        // disc by at most its perimeter times the chord bound (every segment loses at most
        // chord * sagitta).
        let chord_gap = |radius_mm: f64| core::f64::consts::TAU * radius_mm * ROUND_CHORD_MM;
        for (kind, analytic) in [
            (
                ComponentKind::Washer,
                PI * (15.0_f64.powi(2) - 8.5_f64.powi(2)) * 3.0,
            ),
            (ComponentKind::Head, hex_area * 10.0),
            (ComponentKind::Shank, PI * 64.0 * 60.0),
            (ComponentKind::Nut, hex_area * 14.8),
        ] {
            let component = prototype
                .components
                .iter()
                .find(|c| c.kind == kind)
                .unwrap();
            let bound = match kind {
                ComponentKind::Washer => chord_gap(15.0) * 3.0 + chord_gap(8.5) * 3.0,
                ComponentKind::Shank => chord_gap(8.0) * 60.0,
                _ => 1e-9,
            };
            assert!(
                (component.volume_mm3 - analytic).abs() <= bound,
                "{kind:?}: {} vs {analytic}",
                component.volume_mm3
            );
            assert!(component.volume_mm3 > 0.0);
            assert!(!component.mesh.vertices().is_empty());
        }
        assert!((prototype.volume_mm3() - analytic_total(&prototype)).abs() < 1e-9);
    }

    fn analytic_total(prototype: &FastenerPrototype) -> f64 {
        prototype.components.iter().map(|c| c.volume_mm3).sum()
    }

    #[test]
    fn the_stack_runs_from_the_bearing_face_and_keeps_the_protrusion() {
        let prototype = prototype(&bolt(), 60.0).unwrap();
        assert_eq!(
            kinds(&prototype),
            [
                ComponentKind::Washer,
                ComponentKind::Head,
                ComponentKind::Shank,
                ComponentKind::Washer,
                ComponentKind::Nut,
            ]
        );
        let starts: Vec<f64> = prototype.components.iter().map(|c| c.z_start_mm).collect();
        // Washer 0..3, head 3..13, shank 13..73; the nut ends at 73 - 5 protrusion, and the
        // washer under the nut sits right before it.
        assert_eq!(starts, [0.0, 3.0, 13.0, 50.2, 53.2]);
        let last = prototype.components.last().unwrap();
        assert!((last.z_end_mm - 68.0).abs() < 1e-9);
        assert_eq!(
            prototype.counts(),
            vec![
                ComponentCount {
                    kind: ComponentKind::Washer,
                    count: 2
                },
                ComponentCount {
                    kind: ComponentKind::Head,
                    count: 1
                },
                ComponentCount {
                    kind: ComponentKind::Shank,
                    count: 1
                },
                ComponentCount {
                    kind: ComponentKind::Nut,
                    count: 1
                },
            ]
        );
    }

    #[test]
    fn a_screw_has_no_nut_and_a_bare_bolt_has_no_washers() {
        let screw = FastenerSpec {
            kind: FastenerKind::Screw,
            nut_across_flats_mm: None,
            nut_height_mm: None,
            washer_under_nut: false,
            ..bolt()
        };
        let screw_prototype = prototype(&screw, 45.0).unwrap();
        assert_eq!(
            kinds(&screw_prototype),
            [
                ComponentKind::Washer,
                ComponentKind::Head,
                ComponentKind::Shank
            ]
        );
        let bare = FastenerSpec {
            washer: None,
            washer_under_head: false,
            washer_under_nut: false,
            ..bolt()
        };
        let bare_prototype = prototype(&bare, 45.0).unwrap();
        assert_eq!(
            kinds(&bare_prototype),
            [
                ComponentKind::Head,
                ComponentKind::Shank,
                ComponentKind::Nut
            ]
        );
    }

    #[test]
    fn the_mass_source_is_the_declared_value_or_unknown() {
        let declared = FastenerSpec {
            mass_each_kg: Some(0.108),
            ..bolt()
        };
        let declared_prototype = prototype(&declared, 60.0).unwrap();
        assert_eq!(declared_prototype.mass_kg, Some(0.108));
        assert_eq!(declared_prototype.mass_source, MassSource::Declared);
        let unknown = prototype(&bolt(), 60.0).unwrap();
        assert_eq!(unknown.mass_kg, None);
        assert_eq!(unknown.mass_source, MassSource::Unknown);
    }

    #[test]
    fn only_a_standard_length_builds_and_the_prototype_is_deterministic() {
        assert_eq!(
            prototype(&bolt(), 55.0).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        let first = prototype(&bolt(), 60.0).unwrap();
        let second = prototype(&bolt(), 60.0).unwrap();
        // A group of n fasteners shares one prototype: the same definition builds the same
        // meshes, and nothing in the builder depends on the group size.
        for (a, b) in first.components.iter().zip(&second.components) {
            assert_eq!(a.mesh.digest(), b.mesh.digest(), "{:?}", a.kind);
        }
        assert_eq!(first, second);
    }
}
