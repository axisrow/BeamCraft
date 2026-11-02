// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic inputs (docs/fixtures/g11, set version 1). The references are
// exact rational evaluations of the binary64 inputs made by `beamcraft-fixtures g11`, not values
// read back from this code.

//! G11 accuracy fixtures: the real plate, member and fastener builders against exact references.
//!
//! Four classes: rounded-mesh cross-checks, subnormal mass, very large coordinates and
//! cancellation-heavy inputs. Every quantity is held to the F04 section 10 test rule (relative
//! 1e-6; lengths 0.001 mm plus relative 1e-6). A certified mesh's exact-sum volume enclosure is
//! held to the same relative bound plus the effect of rounding its vertices to binary64, which is
//! the only thing that separates a mesh from its exact solid. The oracle uses no production math:
//! it parses the committed references and compares. The largest relative error of each class is
//! pinned well inside the rule, so a loss of accuracy fails before the rule does.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use beamcraft_engine::solid::SolidMesh;
use beamcraft_geom::{Frame3, Point3, RigidTransform};
use beamcraft_kernel::measure::{MeshMeasurement, measure};
use beamcraft_kernel::topology::analyze;
use beamcraft_kernel::trimesh::TriMesh;
use beamcraft_model::command::{CommandError, Placement};
use beamcraft_model::digest::{ContentDigest, Sha256};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::ElementId;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use steel::engine::command::{MaterialInfo, create_plate};
use steel::engine::member::{ProfileInfo, create_straight_member};
use steel::kernel::fastener::{ComponentKind, build};
use steel::model::command::{CreatePlate, CreateStraightMember, MemberKind};
use steel::model::fastener::{FastenerKind, FastenerSpec, HeadKind, HoleMode, WasherSpec};
use steel::model::section::{ParametricSection, SectionFamily, SectionShape};

/// The largest relative error each class may show, for engineering quantities (`quantity`,
/// including lengths) and for certified mesh volumes (`mesh`); the F04 rule allows 1e-6.
const PINNED: [(&str, &str, f64); 8] = [
    // The thin-wall RHS area cancels about four digits (measured 1.0e-10 and 1.2e-10).
    ("cancel", "mesh", 1e-9),
    ("cancel", "quantity", 1e-9),
    // A 0.5 x 0.25 mm section with its vertices rounded near 1e9 mm (measured 1.4e-7); every
    // engineering quantity stays exact to the last bits (measured 2.4e-16).
    ("far", "mesh", 5e-7),
    ("far", "quantity", 1e-14),
    // Measured 9.5e-15 and 3.3e-14.
    ("mesh", "mesh", 1e-13),
    ("mesh", "quantity", 1e-12),
    // Every admitted mass and volume of the class is exact (measured 0).
    ("subnormal", "mesh", 1e-15),
    ("subnormal", "quantity", 1e-15),
];

const ID: &str = "019a7bf0-0000-7000-8000-0000000000b1";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/fixtures/g11")
}

fn rows(name: &str) -> Vec<Vec<String>> {
    std::fs::read_to_string(root().join(name))
        .unwrap()
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .map(|l| l.split('\t').map(str::to_owned).collect())
        .collect()
}

fn digest(bytes: &[u8]) -> ContentDigest {
    Sha256::digest(bytes).unwrap()
}

/// The decimal half of a `fraction decimal` reference cell.
fn value(cell: &str) -> Option<f64> {
    (cell != "-").then(|| cell.split_once(' ').unwrap().1.parse().unwrap())
}

fn vector(cell: &str) -> Option<[f64; 3]> {
    (cell != "-").then(|| {
        let v: Vec<f64> = cell.split(',').map(|c| value(c).unwrap()).collect();
        v.try_into().unwrap()
    })
}

fn numbers(text: &str) -> Vec<f64> {
    text.split(',').map(|t| t.parse().unwrap()).collect()
}

struct Case {
    id: String,
    family: String,
    body: String,
    params: BTreeMap<String, String>,
    density: Option<f64>,
    translation: [f64; 3],
    half_turn: bool,
}

struct Expected {
    outcome: String,
    curved: bool,
    volume: f64,
    mass: Option<f64>,
    area: Option<f64>,
    axis: Option<f64>,
    bounds: Option<([f64; 3], [f64; 3])>,
    centroid: Option<[f64; 3]>,
}

fn cases() -> Vec<(Case, Expected)> {
    let expected: BTreeMap<String, Vec<String>> = rows("expectations.tsv")
        .into_iter()
        .map(|r| (r[0].clone(), r))
        .collect();
    let all: Vec<(Case, Expected)> = rows("cases.tsv")
        .into_iter()
        .map(|r| {
            assert_eq!(r.len(), 7, "{r:?}");
            let case = Case {
                id: r[0].clone(),
                family: r[1].clone(),
                body: r[2].clone(),
                params: r[3]
                    .split(' ')
                    .map(|kv| {
                        let (k, v) = kv.split_once('=').unwrap();
                        (k.to_owned(), v.to_owned())
                    })
                    .collect(),
                density: (r[4] != "-").then(|| r[4].parse().unwrap()),
                translation: numbers(&r[5]).try_into().unwrap(),
                half_turn: match r[6].as_str() {
                    "identity" => false,
                    "half_turn_z" => true,
                    other => panic!("rotation {other}"),
                },
            };
            let e = expected
                .get(&case.id)
                .unwrap_or_else(|| panic!("{}", case.id));
            assert_eq!(e.len(), 11, "{}", case.id);
            let expected = Expected {
                outcome: e[1].clone(),
                curved: e[3] == "curved",
                volume: value(&e[4]).unwrap(),
                mass: value(&e[5]),
                area: value(&e[6]),
                axis: value(&e[7]),
                bounds: vector(&e[8]).zip(vector(&e[9])),
                centroid: vector(&e[10]),
            };
            (case, expected)
        })
        .collect();
    assert_eq!(all.len(), expected.len(), "one expectation per case");
    all
}

/// The record of the largest relative error per class and kind.
#[derive(Default)]
struct Errors(BTreeMap<(String, &'static str), (f64, String)>);

impl Errors {
    /// F04 section 10 quantity rule; records the relative error.
    fn quantity(&mut self, case: &Case, what: &str, actual: f64, expected: f64) {
        assert!(actual.is_finite(), "{} {what}: {actual}", case.id);
        let relative = if expected == 0.0 {
            actual.abs()
        } else {
            (actual - expected).abs() / expected.abs()
        };
        assert!(
            relative <= 1e-6,
            "{} {what}: {actual:e} vs {expected:e} (relative {relative:e})",
            case.id
        );
        self.record(case, "quantity", what, relative);
    }

    /// F04 section 10 length rule: 0.001 mm plus relative 1e-6.
    fn length(&mut self, case: &Case, what: &str, actual: f64, expected: f64) {
        let error = (actual - expected).abs();
        assert!(
            error <= 0.001 + 1e-6 * expected.abs(),
            "{} {what}: {actual:e} vs {expected:e}",
            case.id
        );
        self.record(case, "quantity", what, error / expected.abs().max(1.0));
    }

    fn record(&mut self, case: &Case, kind: &'static str, what: &str, relative: f64) {
        let slot = self
            .0
            .entry((case.family.clone(), kind))
            .or_insert((0.0, String::new()));
        if relative > slot.0 {
            *slot = (relative, format!("{} {what}", case.id));
        }
    }
}

fn identity_frame() -> Frame3 {
    Frame3::try_new(
        beamcraft_geom::Vec3::X,
        beamcraft_geom::Vec3::Y,
        beamcraft_geom::Vec3::Z,
    )
    .unwrap()
}

/// The production world map of an element: the placement rotation of the element-local mesh,
/// then the translation by the kernel's rigid transform.
fn world(solid: &SolidMesh, case: &Case) -> TriMesh {
    let solid = if case.half_turn {
        solid.clone().rotated_by([0.0, 0.0, 1.0, 0.0]).unwrap()
    } else {
        solid.clone()
    };
    let [x, y, z] = case.translation;
    let shift = RigidTransform::from_body_frame(identity_frame(), Point3::new(x, y, z)).unwrap();
    TriMesh::from_buffers(solid.vertices(), solid.triangles())
        .unwrap()
        .transformed(&shift)
        .unwrap()
}

fn measured(mesh: &TriMesh) -> MeshMeasurement {
    measure(mesh, &analyze(mesh).unwrap()).unwrap()
}

/// The volume a mesh may differ from its exact solid by rounding its vertices to binary64: each
/// vertex moves by at most half an ulp of its largest coordinate, so the volume moves by at most
/// the surface area times that distance (doubled for safety).
fn rounding_bound(mesh: &TriMesh, q: &MeshMeasurement) -> f64 {
    let largest = mesh
        .vertices()
        .iter()
        .flatten()
        .fold(0.0f64, |m, v| m.max(v.abs()));
    let ulp = largest * f64::EPSILON;
    q.area_mm2().unwrap().hi * ulp
}

/// The mesh volume enclosure agrees with `expected` within the quantity rule and the rounding.
fn mesh_volume(errors: &mut Errors, case: &Case, mesh: &TriMesh, expected: f64) {
    let q = measured(mesh);
    let v = q
        .volume_mm3()
        .unwrap_or_else(|| panic!("{}: not certified", case.id));
    let allowed = 1e-6 * expected.abs() + rounding_bound(mesh, &q);
    for end in [v.lo, v.hi] {
        assert!(
            (end - expected).abs() <= allowed,
            "{} mesh volume [{:e}, {:e}] vs {expected:e} (allowed {allowed:e})",
            case.id,
            v.lo,
            v.hi
        );
    }
    errors.record(
        case,
        "mesh",
        "volume",
        (v.midpoint() - expected).abs() / expected,
    );
}

fn mesh_placement(errors: &mut Errors, case: &Case, mesh: &TriMesh, expected: &Expected) {
    let q = measured(mesh);
    if let Some((lo, hi)) = expected.bounds {
        let b = q.bounds();
        for i in 0..3 {
            errors.length(case, "bounds min", b.min[i], lo[i]);
            errors.length(case, "bounds max", b.max[i], hi[i]);
        }
    }
    if let Some(c) = expected.centroid {
        let m = q.centroid_mm().unwrap();
        for i in 0..3 {
            errors.length(case, "centroid", m[i].midpoint(), c[i]);
        }
    }
}

fn material(density: Option<f64>) -> impl Fn(&ContentDigest) -> Result<MaterialInfo, CommandError> {
    move |_| {
        Ok(MaterialInfo {
            density_kg_m3: density,
        })
    }
}

fn placement(case: &Case) -> Placement {
    Placement {
        translation_mm: case.translation,
        orientation_xyzw: if case.half_turn {
            [0.0, 0.0, 1.0, 0.0]
        } else {
            [0.0, 0.0, 0.0, 1.0]
        },
    }
}

fn refused(case: &Case, expected: &Expected, error: &CommandError) {
    let code = match expected.outcome.as_str() {
        "UNSUPPORTED_GEOMETRY" => ErrorCode::UnsupportedGeometry,
        other => panic!("{}: refused with {error:?}, expected {other}", case.id),
    };
    assert_eq!(error.code, code, "{}: {error:?}", case.id);
    assert_eq!(error.field, "density_kg_m3", "{}", case.id);
}

fn run_plate(errors: &mut Errors, case: &Case, expected: &Expected) {
    let points: Vec<[f64; 3]> = case.params["points"]
        .split(';')
        .map(|p| {
            let [x, y]: [f64; 2] = numbers(p).try_into().unwrap();
            [x, y, 0.0]
        })
        .collect();
    let command = CreatePlate {
        new_id: ElementId::parse(ID).unwrap(),
        name: case.id.clone(),
        class: 0,
        contour_points_mm: points,
        thickness_mm: case.params["thickness"].parse().unwrap(),
        thickness_side: None,
        corners: vec![],
        placement: placement(case),
        material_digest: case.density.map(|_| digest(b"g11 material")),
        holes_mm: vec![],
    };
    let outcome = match create_plate(&command, &|_| false, &material(case.density)) {
        Ok(outcome) => outcome,
        Err(e) => return refused(case, expected, &e),
    };
    assert_eq!(expected.outcome, "ok", "{}: accepted", case.id);
    errors.quantity(
        case,
        "volume",
        outcome.body.gross_volume_mm3,
        expected.volume,
    );
    match (outcome.quantities.mass_kg, expected.mass) {
        (Some(actual), Some(want)) => errors.quantity(case, "mass", actual, want),
        (None, None) => {}
        other => panic!("{}: mass {other:?}", case.id),
    }
    let mesh = world(&outcome.body.solid, case);
    mesh_volume(errors, case, &mesh, expected.volume);
    mesh_placement(errors, case, &mesh, expected);
}

fn profile(case: &Case) -> ProfileInfo {
    if let Some(wh) = case.params.get("rectangle") {
        let [width_mm, height_mm]: [f64; 2] = numbers(wh).try_into().unwrap();
        return ProfileInfo::Rectangle {
            width_mm,
            height_mm,
            declared_area_mm2: None,
        };
    }
    let (family, values) = SectionFamily::ALL
        .into_iter()
        .find_map(|f| case.params.get(f.token()).map(|v| (f, numbers(v))))
        .unwrap_or_else(|| panic!("{}: no profile", case.id));
    ProfileInfo::Section {
        shape: SectionShape::Parametric(ParametricSection::new(family, values).unwrap()),
        declared_area_mm2: None,
    }
}

fn run_member(errors: &mut Errors, case: &Case, expected: &Expected) {
    let profile = profile(case);
    let profiles = move |_: &ContentDigest| Ok(profile.clone());
    let command = CreateStraightMember {
        new_id: ElementId::parse(ID).unwrap(),
        kind: MemberKind::Beam,
        name: case.id.clone(),
        class: 0,
        axis_points_mm: [
            numbers(&case.params["start"]).try_into().unwrap(),
            numbers(&case.params["end"]).try_into().unwrap(),
        ],
        profile_digest: digest(b"g11 profile"),
        placement: placement(case),
        roll_rad: None,
        section_placement: None,
        material_digest: case.density.map(|_| digest(b"g11 material")),
    };
    let outcome =
        match create_straight_member(&command, &|_| false, &profiles, &material(case.density)) {
            Ok(outcome) => outcome,
            Err(e) => return refused(case, expected, &e),
        };
    assert_eq!(expected.outcome, "ok", "{}: accepted", case.id);
    let body = &outcome.body;
    errors.quantity(
        case,
        "section area",
        body.section_area_mm2,
        expected.area.unwrap(),
    );
    errors.length(
        case,
        "axis length",
        body.axis_length_mm,
        expected.axis.unwrap(),
    );
    errors.quantity(
        case,
        "axis length",
        body.axis_length_mm,
        expected.axis.unwrap(),
    );
    errors.quantity(case, "volume", body.volume_mm3, expected.volume);
    match (outcome.quantities.mass_kg, expected.mass) {
        (Some(actual), Some(want)) => errors.quantity(case, "mass", actual, want),
        (None, None) => {}
        other => panic!("{}: mass {other:?}", case.id),
    }
    let mesh = world(&body.solid, case);
    // A curved section's mesh is its polygon: it agrees with the builder's polygon volume, and
    // the engineering volume comes from the exact section area instead.
    let mesh_reference = if expected.curved {
        body.solid.volume_mm3()
    } else {
        expected.volume
    };
    mesh_volume(errors, case, &mesh, mesh_reference);
    mesh_placement(errors, case, &mesh, expected);
}

fn run_bolt(errors: &mut Errors, case: &Case, expected: &Expected) {
    let p = |k: &str| numbers(&case.params[k]);
    let [head_af, head_h]: [f64; 2] = p("head").try_into().unwrap();
    let [nut_af, nut_h]: [f64; 2] = p("nut").try_into().unwrap();
    let [od, id, t]: [f64; 3] = p("washer").try_into().unwrap();
    let length = p("length")[0];
    let spec = FastenerSpec {
        designation: None,
        standard: None,
        grade: None,
        display_name: None,
        kind: FastenerKind::Bolt,
        head_kind: HeadKind::Hex,
        nominal_diameter_mm: p("diameter")[0],
        head_across_flats_mm: head_af,
        head_height_mm: head_h,
        nut_across_flats_mm: Some(nut_af),
        nut_height_mm: Some(nut_h),
        washer: Some(WasherSpec {
            outer_diameter_mm: od,
            inner_diameter_mm: id,
            thickness_mm: t,
        }),
        hole_clearance_mm: 2.0,
        lengths_mm: vec![length],
        thread_protrusion_mm: 5.0,
        mass_each_kg: None,
        washer_under_head: true,
        washer_under_nut: false,
        hole_mode: HoleMode::Generate,
    };
    let kind = match case.params["part"].as_str() {
        "washer" => ComponentKind::WasherHead,
        "head" => ComponentKind::Head,
        "shank" => ComponentKind::Shank,
        "nut" => ComponentKind::Nut,
        other => panic!("part {other}"),
    };
    let built = build(&spec, length, true, false).unwrap();
    let part = built
        .components
        .iter()
        .find(|c| c.component == kind)
        .unwrap();
    let volume = part.solid.volume_m3() * 1e9;
    if expected.curved {
        // A round part is a polygon on the circle: inside the circle, short of it by at most the
        // chord region along every circle (perimeter times the sagitta bound), times the height.
        let approximation = part.approximation.unwrap();
        let circles: f64 = match kind {
            ComponentKind::WasherHead => od + id,
            _ => p("diameter")[0],
        };
        let allowed =
            core::f64::consts::PI * circles * approximation.max_sagitta_mm * part.height_mm;
        assert!(
            (volume - expected.volume).abs() <= allowed,
            "{}: polygon volume {volume:e} vs circle {:e} (allowed {allowed:e})",
            case.id,
            expected.volume
        );
    } else {
        errors.quantity(case, "volume", volume, expected.volume);
    }
    let solid = SolidMesh::from_extrusion(&part.solid).unwrap();
    mesh_volume(errors, case, &world(&solid, case), volume);
}

#[test]
fn the_committed_set_matches_its_manifest() {
    let manifest = rows("manifest.tsv");
    assert_eq!(manifest.len(), 2);
    for row in manifest {
        let bytes = std::fs::read(root().join(&row[0])).unwrap();
        assert_eq!(bytes.len().to_string(), row[1], "{}", row[0]);
        assert_eq!(digest(&bytes).to_string(), row[2], "{}", row[0]);
    }
}

#[test]
fn every_fixture_stays_inside_the_f06_comparison_rule() {
    let mut errors = Errors::default();
    let mut counts = BTreeMap::<String, usize>::new();
    for (case, expected) in cases() {
        *counts.entry(case.family.clone()).or_default() += 1;
        match case.body.as_str() {
            "plate" => run_plate(&mut errors, &case, &expected),
            "member" => run_member(&mut errors, &case, &expected),
            "bolt" => run_bolt(&mut errors, &case, &expected),
            other => panic!("{}: body {other}", case.id),
        }
    }
    for ((family, kind), (error, at)) in &errors.0 {
        println!("{family} {kind}: largest relative error {error:e} at {at}");
    }
    for (family, kind, bound) in PINNED {
        let (error, at) = errors
            .0
            .get(&(family.to_owned(), kind))
            .cloned()
            .unwrap_or_default();
        assert!(
            error <= bound,
            "{family} {kind}: {error:e} at {at} exceeds the pinned {bound:e}"
        );
    }
    assert_eq!(
        counts,
        BTreeMap::from([
            ("cancel".into(), 9),
            ("far".into(), 8),
            ("mesh".into(), 25),
            ("subnormal".into(), 8)
        ])
    );
}
