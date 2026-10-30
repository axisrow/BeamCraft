// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A14.2 fastener geometry: the components of one fastener as separate closed meshes (bolt
//! amendment section 3).
//!
//! Every component is a straight G05 extrusion of an admitted profile, so no new kernel
//! operation is introduced: a hex head and nut are extrusions of a regular hexagon, the shank
//! is an extrusion of a regular polygon, a washer is an extrusion of a polygon with a circular
//! cavity. Round parts follow the round-hole chord rule of F06 A-2 D-2: the chord deviation is
//! at most 0.05 mm and the vertex count a multiple of four. Threads are not modelled and edges
//! are not chamfered. The result is one prototype of [`FastenerSpec::lengths_mm`]-admitted
//! length: a group of `n` fasteners (A14.3) shares this one prototype placed `n` times.

use crate::profile::diagnostic::ProfileFailure;
use crate::profile::{
    Cavity, CurveApproximation, CurveKind, CurveSpec, Extrusion, SEGMENT_CAP, SEGMENT_FLOOR,
    choose_segments, extrude, unit_direction,
};
use beamcraft_geom::tolerance::{ABSOLUTE_FLOOR_MM, DIMENSION_CAP_MM};
use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::fastener::{FastenerKind, FastenerSpec};

/// The chord rule of the round fastener parts (F06 A-2 D-2): at most 0.05 mm deviation and a
/// vertex count that is a multiple of four.
pub const CHORD_RULE: CurveSpec = CurveSpec {
    max_sagitta_mm: 0.05,
    min_segments: SEGMENT_FLOOR,
    max_segments: SEGMENT_CAP,
};

/// Which component of the stack a solid is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComponentKind {
    /// The washer under the head.
    WasherHead,
    /// The head.
    Head,
    /// The shank.
    Shank,
    /// The nut (a bolt only).
    Nut,
    /// The washer under the nut (a bolt only).
    WasherNut,
}

impl ComponentKind {
    /// Every component, in stack order from the bearing face.
    pub const ALL: [Self; 5] = [
        Self::WasherHead,
        Self::Head,
        Self::Shank,
        Self::Nut,
        Self::WasherNut,
    ];

    /// Stable token.
    pub const fn token(self) -> &'static str {
        match self {
            Self::WasherHead => "washer_head",
            Self::Head => "head",
            Self::Shank => "shank",
            Self::Nut => "nut",
            Self::WasherNut => "washer_nut",
        }
    }
}

/// Why a prototype could not be built.
#[derive(Debug, Clone, PartialEq)]
pub enum FastenerFailure {
    /// An input was refused before any mesh was built, with the F04 code and its field.
    Input(CommandError),
    /// G05 refused a component outline or its extrusion.
    Profile(ProfileFailure),
}

impl FastenerFailure {
    /// The F04 code of the failure.
    pub fn f04_code(&self) -> ErrorCode {
        match self {
            Self::Input(error) => error.code,
            Self::Profile(failure) => failure
                .diagnostics()
                .first()
                .map_or(ErrorCode::UnsupportedGeometry, |d| d.code.f04_code()),
        }
    }
}

/// One component of the prototype: a sealed closed mesh, its place in the stack and its
/// round-part provenance.
#[derive(Debug, Clone, PartialEq)]
pub struct FastenerSolid {
    /// Which component of the stack.
    pub component: ComponentKind,
    /// Position of the component's extrusion base along the shank axis in the prototype frame
    /// (the bearing face is at zero, the shank runs towards +Z), in millimetres.
    pub z_offset_mm: f64,
    /// The extrusion height along +Z, in millimetres.
    pub height_mm: f64,
    /// The certified closed mesh with its exact volume and mesh digest.
    pub solid: Extrusion,
    /// Chord-rule provenance of a round part; `None` for an exact polygon (the hexagon).
    pub approximation: Option<CurveApproximation>,
}

/// One fastener prototype: every component as a separate closed mesh, in stack order from the
/// bearing face. A group of `n` fasteners shares one prototype (A14.2; the placements are
/// A14.3's).
#[derive(Debug, Clone, PartialEq)]
pub struct FastenerBuild {
    /// The components in stack order.
    pub components: Vec<FastenerSolid>,
    /// The admitted shank length, in millimetres.
    pub length_mm: f64,
}

/// Counter-clockwise ring of a circle of `radius_mm`, first vertex on +X, with its chord-rule
/// provenance. The vertices lie on the circle; the kernel's deterministic trigonometry keeps
/// them bit-identical on every target.
fn ring(
    radius_mm: f64,
) -> Result<(Vec<crate::profile::Point2>, CurveApproximation), FastenerFailure> {
    let (segments, bound) =
        choose_segments(radius_mm, CHORD_RULE).map_err(FastenerFailure::Profile)?;
    let n = u64::from(segments);
    let points = (0..n)
        .map(|k| {
            let (c, s) = unit_direction(k, n);
            crate::profile::Point2::new(radius_mm * c, radius_mm * s)
        })
        .collect();
    Ok((
        points,
        CurveApproximation {
            kind: CurveKind::Circle,
            segments,
            max_sagitta_mm: CHORD_RULE.max_sagitta_mm,
            sagitta_bound_mm: bound,
        },
    ))
}

/// Regular hexagon across flats, one vertex on +X, counter-clockwise. The coordinates are
/// constructed from halves and one square root, so the shape is exactly symmetric about both
/// axes on every target.
fn hexagon(across_flats_mm: f64) -> Vec<crate::profile::Point2> {
    let circumradius = across_flats_mm / 3.0f64.sqrt();
    let half_across = across_flats_mm / 2.0;
    let half_circum = circumradius / 2.0;
    let p = |x: f64, y: f64| crate::profile::Point2::new(x, y);
    vec![
        p(circumradius, 0.0),
        p(half_circum, half_across),
        p(-half_circum, half_across),
        p(-circumradius, 0.0),
        p(-half_circum, -half_across),
        p(half_circum, -half_across),
    ]
}

fn dimension(value: f64) -> Result<(), FastenerFailure> {
    let refused = |code: ErrorCode| {
        FastenerFailure::Input(CommandError::new(
            code,
            "length_mm",
            "the length is not admissible",
        ))
    };
    if !value.is_finite() {
        return Err(refused(ErrorCode::NonFiniteNumber));
    }
    if value > DIMENSION_CAP_MM {
        return Err(refused(ErrorCode::LimitExceeded));
    }
    if value < ABSOLUTE_FLOOR_MM {
        return Err(refused(ErrorCode::InvalidArgument));
    }
    Ok(())
}

fn refused(field: &'static str, message: &'static str) -> FastenerFailure {
    FastenerFailure::Input(CommandError::new(
        ErrorCode::InvalidArgument,
        field,
        message,
    ))
}

/// Builds one fastener prototype from an admitted definition (bolt amendment section 3).
///
/// The bearing face of the head (under the head washer when there is one) is the plane z = 0
/// and the shank runs along +Z: the head washer, head, shank, nut and nut washer stack upwards,
/// the shank always starting at the under-head plane. The definition is re-admitted and the
/// washer switches follow its rules, so a switch a definition cannot honour refuses the build
/// and nothing is produced.
pub fn build(
    spec: &FastenerSpec,
    length_mm: f64,
    washer_under_head: bool,
    washer_under_nut: bool,
) -> Result<FastenerBuild, FastenerFailure> {
    spec.validate().map_err(FastenerFailure::Input)?;
    dimension(length_mm)?;
    if washer_under_head && spec.washer.is_none() {
        return Err(refused(
            "washer_under_head",
            "the definition carries no washer to place under the head",
        ));
    }
    if washer_under_nut && (spec.washer.is_none() || spec.kind != FastenerKind::Bolt) {
        return Err(refused(
            "washer_under_nut",
            "a washer under the nut needs a washer and a bolt",
        ));
    }
    let nut = (spec.nut_across_flats_mm, spec.nut_height_mm);
    let washer = spec.washer;
    let under_head = if washer_under_head {
        washer.map_or(0.0, |w| w.thickness_mm)
    } else {
        0.0
    };
    let shank_end = under_head + length_mm;
    let nut_end = shank_end + nut.1.unwrap_or(0.0);
    let mut components = Vec::new();
    let mut push = |component: ComponentKind,
                    z_offset_mm: f64,
                    height_mm: f64,
                    solid: Result<Extrusion, ProfileFailure>,
                    approximation: Option<CurveApproximation>|
     -> Result<(), FastenerFailure> {
        components.push(FastenerSolid {
            component,
            z_offset_mm,
            height_mm,
            solid: solid.map_err(FastenerFailure::Profile)?,
            approximation,
        });
        Ok(())
    };
    if let (true, Some(washer)) = (washer_under_head, washer) {
        let (outer, approximation) = ring(washer.outer_diameter_mm / 2.0)?;
        let cavity = Cavity::circle(
            crate::profile::Point2::new(0.0, 0.0),
            washer.inner_diameter_mm / 2.0,
            CHORD_RULE,
        )
        .map_err(FastenerFailure::Profile)?;
        push(
            ComponentKind::WasherHead,
            0.0,
            washer.thickness_mm,
            extrude(&outer, &[cavity], washer.thickness_mm),
            Some(approximation),
        )?;
    }
    push(
        ComponentKind::Head,
        under_head,
        spec.head_height_mm,
        extrude(
            &hexagon(spec.head_across_flats_mm),
            &[],
            spec.head_height_mm,
        ),
        None,
    )?;
    let (outer, approximation) = ring(spec.nominal_diameter_mm / 2.0)?;
    push(
        ComponentKind::Shank,
        under_head,
        length_mm,
        extrude(&outer, &[], length_mm),
        Some(approximation),
    )?;
    if spec.kind == FastenerKind::Bolt {
        let (Some(across), Some(height)) = nut else {
            return Err(refused(
                "nut_across_flats_mm",
                "a bolt definition carries a nut",
            ));
        };
        push(
            ComponentKind::Nut,
            shank_end,
            height,
            extrude(&hexagon(across), &[], height),
            None,
        )?;
        if let (true, Some(washer)) = (washer_under_nut, washer) {
            let (outer, approximation) = ring(washer.outer_diameter_mm / 2.0)?;
            let cavity = Cavity::circle(
                crate::profile::Point2::new(0.0, 0.0),
                washer.inner_diameter_mm / 2.0,
                CHORD_RULE,
            )
            .map_err(FastenerFailure::Profile)?;
            push(
                ComponentKind::WasherNut,
                nut_end,
                washer.thickness_mm,
                extrude(&outer, &[cavity], washer.thickness_mm),
                Some(approximation),
            )?;
        }
    }
    Ok(FastenerBuild {
        components,
        length_mm,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use beamcraft_model::fastener::{HeadKind, HoleMode, WasherSpec};
    use core::f64::consts::TAU;

    /// The test bolt of the model crate's own suite: a washered M16 bolt.
    fn bolt() -> FastenerSpec {
        FastenerSpec {
            designation: Some("M16".into()),
            standard: None,
            grade: None,
            display_name: None,
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
            mass_each_kg: None,
        }
    }

    fn built() -> FastenerBuild {
        build(&bolt(), 50.0, true, true).unwrap()
    }

    /// The refusal of a build that changed nothing.
    fn refusal(result: Result<FastenerBuild, FastenerFailure>) -> CommandError {
        match result {
            Err(FastenerFailure::Input(error)) => error,
            other => panic!("expected an input refusal, got {other:?}"),
        }
    }

    fn of(build: &FastenerBuild, component: ComponentKind) -> &FastenerSolid {
        build
            .components
            .iter()
            .find(|s| s.component == component)
            .unwrap()
    }

    /// Area of the inscribed regular n-gon of radius r.
    fn polygon_area(radius: f64, n: u32) -> f64 {
        f64::from(n) / 2.0 * radius * radius * (TAU / f64::from(n)).sin()
    }

    /// Area of a regular hexagon across flats.
    fn hexagon_area(across_flats: f64) -> f64 {
        3.0f64.sqrt() / 2.0 * across_flats * across_flats
    }

    #[test]
    fn hex_components_match_the_analytic_hexagon_volume() {
        let build = built();
        let head = of(&build, ComponentKind::Head);
        assert!((head.solid.volume_mm3() - hexagon_area(24.0) * 10.0).abs() < 1e-6);
        let nut = of(&build, ComponentKind::Nut);
        assert!((nut.solid.volume_mm3() - hexagon_area(24.0) * 14.8).abs() < 1e-6);
        // The hexagon is exact: no approximation flag on either.
        assert_eq!(head.approximation, None);
        assert_eq!(nut.approximation, None);
    }

    #[test]
    fn shank_volume_is_the_polygon_cylinder_within_the_chord_rule() {
        let build = built();
        let shank = of(&build, ComponentKind::Shank);
        let approximation = shank.approximation.unwrap();
        let (radius, n) = (8.0, approximation.segments);
        // The exact polygon volume, and the chord-rule relation to the circle.
        let polygon = polygon_area(radius, n) * shank.height_mm;
        assert!((shank.solid.volume_mm3() - polygon).abs() < 1e-6);
        let circle = core::f64::consts::PI * radius * radius * shank.height_mm;
        assert!(shank.solid.volume_mm3() <= circle);
        assert!(circle - shank.solid.volume_mm3() <= circle * 0.01);
        // The rule itself: at most 0.05 mm deviation, a multiple of four vertices.
        assert!(approximation.sagitta_bound_mm <= 0.05);
        assert_eq!(n % 4, 0);
        assert!(
            approximation.segments >= 16,
            "an M16 shank needs more than 16"
        );
    }

    #[test]
    fn washer_volume_is_the_analytic_annulus_within_the_chord_rule() {
        let build = built();
        for component in [ComponentKind::WasherHead, ComponentKind::WasherNut] {
            let washer = of(&build, component);
            let approximation = washer.approximation.unwrap();
            let (outer, inner) = (15.0, 8.5);
            // Every round contour of the component follows the chord rule on its own radius,
            // so the inner ring has its own (smaller) vertex count.
            let (inner_segments, inner_bound) = choose_segments(inner, CHORD_RULE).unwrap();
            let polygon = (polygon_area(outer, approximation.segments)
                - polygon_area(inner, inner_segments))
                * washer.height_mm;
            assert!((washer.solid.volume_mm3() - polygon).abs() < 1e-6);
            let circle = core::f64::consts::PI * (outer * outer - inner * inner) * washer.height_mm;
            assert!(washer.solid.volume_mm3() <= circle);
            assert!(approximation.sagitta_bound_mm <= 0.05);
            assert!(inner_bound <= 0.05);
            assert_eq!(approximation.segments % 4, 0);
            assert_eq!(inner_segments % 4, 0);
        }
    }

    #[test]
    fn the_stack_places_every_component_from_the_bearing_face() {
        let build = built();
        let washer = of(&build, ComponentKind::WasherHead);
        let head = of(&build, ComponentKind::Head);
        let shank = of(&build, ComponentKind::Shank);
        let nut = of(&build, ComponentKind::Nut);
        let top = of(&build, ComponentKind::WasherNut);
        assert_eq!(washer.z_offset_mm, 0.0);
        assert_eq!(head.z_offset_mm, 3.0);
        // The shank starts at the under-head plane, not at the top of the head.
        assert_eq!(shank.z_offset_mm, 3.0);
        assert_eq!(shank.height_mm, 50.0);
        assert_eq!(nut.z_offset_mm, 53.0);
        assert_eq!(top.z_offset_mm, 67.8);
        assert_eq!(build.components.len(), 5);
    }

    #[test]
    fn without_washers_the_head_bears_at_zero() {
        let build = build(&bolt(), 50.0, false, false).unwrap();
        assert_eq!(of(&build, ComponentKind::Head).z_offset_mm, 0.0);
        assert_eq!(of(&build, ComponentKind::Shank).z_offset_mm, 0.0);
        assert_eq!(of(&build, ComponentKind::Nut).z_offset_mm, 50.0);
        assert_eq!(build.components.len(), 3);
    }

    #[test]
    fn a_screw_has_no_nut_and_no_washer_under_the_nut() {
        let screw = FastenerSpec {
            kind: FastenerKind::Screw,
            nut_across_flats_mm: None,
            nut_height_mm: None,
            washer_under_nut: false,
            ..bolt()
        };
        let error = refusal(build(&screw, 30.0, true, true));
        assert_eq!(error.field, "washer_under_nut");
        let build = build(&screw, 30.0, true, false).unwrap();
        assert_eq!(build.components.len(), 3);
        assert!(
            !build
                .components
                .iter()
                .any(|s| matches!(s.component, ComponentKind::Nut | ComponentKind::WasherNut))
        );
    }

    #[test]
    fn the_switch_rules_follow_the_definition() {
        let bare = FastenerSpec {
            washer: None,
            washer_under_head: false,
            washer_under_nut: false,
            ..bolt()
        };
        assert_eq!(
            refusal(build(&bare, 50.0, true, false)).field,
            "washer_under_head"
        );
        // The build re-admits the definition itself: a bolt without a nut is refused there.
        let no_nut = FastenerSpec {
            nut_across_flats_mm: None,
            nut_height_mm: None,
            ..bolt()
        };
        assert_eq!(
            refusal(build(&no_nut, 50.0, true, true)).field,
            "nut_across_flats_mm"
        );
    }

    #[test]
    fn the_length_follows_the_dimension_codes() {
        for (length, code) in [
            (f64::NAN, ErrorCode::NonFiniteNumber),
            (2e6, ErrorCode::LimitExceeded),
            (0.0005, ErrorCode::InvalidArgument),
        ] {
            let error = refusal(build(&bolt(), length, true, true));
            assert_eq!((error.code, error.field), (code, "length_mm"));
        }
    }

    #[test]
    fn the_same_definition_builds_the_same_prototype() {
        // One prototype per group: identical inputs give identical sealed meshes, digests
        // included, so a group of any size shares one mesh.
        assert_eq!(built(), built());
        let other_length = build(&bolt(), 45.0, true, true).unwrap();
        assert_ne!(built(), other_length);
    }

    #[test]
    fn every_component_is_centred_on_the_shank_axis() {
        // The first shank vertex lies on +X (y is exactly zero by the kernel's symmetric
        // trigonometry) and the hexagon's shoelace area matches the analytic one.
        let build = built();
        let shank = of(&build, ComponentKind::Shank);
        let first = shank.solid.mesh().vertices().first().unwrap();
        assert_eq!(first.y, 0.0);
        assert!(first.x > 0.0);
        let corners = hexagon(24.0);
        let area: f64 = corners
            .iter()
            .zip(corners.iter().skip(1).chain(corners.first()))
            .map(|(a, b)| a.x * b.y - b.x * a.y)
            .sum::<f64>()
            .abs()
            / 2.0;
        assert!((area - hexagon_area(24.0)).abs() < 1e-9);
    }
}
