// SPDX-License-Identifier: MPL-2.0
// Contributor-original synthetic members, MPL-2.0.

//! Epic #556 slice 5 end to end: picking and snapping on the cut-generated edges and
//! vertices of member bodies through the real CLI binary. Every expected position is
//! derived here from the command inputs and the frame rules (the member-hole frame, the
//! cutters' canonical ring order and the tool mapping of the part cut), never read back
//! from the engine. The plane cut keeps its cap corners and names no cut edges, so a
//! well-formed cut edge id over it resolves as explicitly not found.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use beamcraft_json::Json;

const MEMBER: &str = "019a7bf0-0000-7000-8000-000000710001";
const TOOL: &str = "019a7bf0-0000-7000-8000-000000710002";
const OP_A: &str = "019a7bf0-0000-7000-8000-00000071000a";
const OP_B: &str = "019a7bf0-0000-7000-8000-00000071000b";

use crate::support::{StoreFixture, at, text};

struct Fixture {
    shared: StoreFixture,
}

impl Fixture {
    fn new() -> Self {
        Self {
            shared: StoreFixture::new("cut-snaps"),
        }
    }
    fn ok(&self, command: &str, extra: &[&str]) -> Json {
        let (code, body) = self.shared.run(command, extra);
        assert_eq!(code, 0, "{command} {extra:?}: {body:?}");
        body
    }
    fn refused(&self, command: &str, extra: &[&str]) -> (String, Option<String>) {
        let (code, body) = self.shared.run(command, extra);
        assert_eq!(code, 2, "{command} {extra:?}: {body:?}");
        let reason = body
            .get("reason")
            .ok()
            .and_then(|r| r.string().ok())
            .map(String::from);
        (text(&body, "code"), reason)
    }

    /// A solid 200 x 300 rectangle member along element X from the origin, 4000 long. Its
    /// F06 frame puts X_r on element +Y, Y_r on element +Z and Z_m on element +X, so the
    /// `f.x` faces sit at element y = -100/+100 and the `f.y` faces at z = -150/+150.
    fn member(&self) {
        let profile = text(
            at(
                &self.ok("define-profile", &["--width", "200", "--height", "300"]),
                &["definition"],
            ),
            "digest",
        );
        self.ok(
            "create-member",
            &[
                "--start",
                "0,0,0",
                "--end",
                "4000,0,0",
                "--profile-digest",
                &profile,
                "--new-id",
                MEMBER,
            ],
        );
    }

    fn list_locals(&self, element: &str) -> Vec<String> {
        list(&self.ok("list-feature", &["--id", element]), "features")
            .iter()
            .map(|f| {
                let id = text(f, "feature_id");
                id.splitn(3, ':').nth(2).unwrap().to_owned()
            })
            .collect()
    }
}

fn list<'a>(json: &'a Json, key: &str) -> &'a [Json] {
    at(json, &[key]).array().unwrap()
}
fn number(json: &Json) -> f64 {
    match json {
        Json::Number(n) => Some(n.parse().unwrap()),
        _ => None,
    }
    .expect("a JSON number")
}
fn triple(json: &Json) -> [f64; 3] {
    let v: Vec<f64> = json.array().unwrap().iter().map(number).collect();
    v.try_into().unwrap()
}
fn fid(element: &str, local: &str) -> String {
    format!("fid1:{element}:{local}")
}

/// The world position of a looked-up vertex feature.
fn vertex(f: &Fixture, element: &str, local: &str) -> [f64; 3] {
    triple(at(
        &f.ok("lookup-feature", &["--feature-id", &fid(element, local)]),
        &["world", "position"],
    ))
}

/// The world ends of a looked-up edge feature, with its endpoint vertex ids.
fn edges(f: &Fixture, element: &str, local: &str) -> ([[f64; 3]; 2], [String; 2]) {
    let found = f.ok("lookup-feature", &["--feature-id", &fid(element, local)]);
    let ends: Vec<[f64; 3]> = list(at(&found, &["world"]), "ends")
        .iter()
        .map(triple)
        .collect();
    let vertex_ids: Vec<String> = list(at(&found, &["world"]), "vertex_ids")
        .iter()
        .map(|v| v.string().unwrap().to_owned())
        .collect();
    (ends.try_into().unwrap(), vertex_ids.try_into().unwrap())
}

/// The one snap candidate at `point`, one millimetre from it, for exactly one kind.
fn snap_one(f: &Fixture, point: [f64; 3], kinds: &str) -> Json {
    let snapped = f.ok(
        "snap",
        &[
            "--point",
            &format!("{},{},{}", point[0], point[1], point[2]),
            "--radius",
            "1",
            "--kinds",
            kinds,
        ],
    );
    let candidates = list(&snapped, "candidates");
    assert_eq!(candidates.len(), 1, "{snapped:?}");
    candidates[0].clone()
}

#[test]
fn a_polygon_cut_seam_picks_and_snaps_at_known_coordinates() {
    let f = Fixture::new();
    f.member();
    // The square through cut along Y_r (element +Z) at the member's mid-length: the solid
    // section gives one crossing, the entry ring on `f.y0` (z = -150) and the exit ring on
    // `f.y1` (+150). The hole frame puts x along Z_m (element X) and y along
    // `direction x Z_m` (element +Y), so corner (px, py) sits at (2000 + px, py, ring z).
    f.ok(
        "cut-by-polygon",
        &[
            "--id",
            MEMBER,
            "--operation-id",
            OP_A,
            "--op-id",
            OP_A,
            "--axis-point",
            "0,0,2000",
            "--axis-direction",
            "0,1,0",
            "--polygon",
            "-10,-10;-10,10;10,10;10,-10",
        ],
    );
    let locals = f.list_locals(MEMBER);
    for local in [
        "cut:v.h0.w0.y0.0",
        "cut:v.h0.w0.y0.1",
        "cut:v.h0.w0.y1.0",
        "cut:e.h0.w0.s0.y0",
        "cut:e.h0.w0.s0.y1",
        "cut:e.h0.w0.v0",
    ] {
        assert!(locals.iter().any(|n| n == local), "{local} missing");
    }
    assert_eq!(
        locals.iter().filter(|n| n.starts_with("cut:v.")).count(),
        8,
        "four rim vertices per ring"
    );
    assert_eq!(
        locals.iter().filter(|n| n.starts_with("cut:e.")).count(),
        12,
        "eight rim edges and four corner edges"
    );
    assert_eq!(
        vertex(&f, MEMBER, "cut:v.h0.w0.y0.0"),
        [1990.0, -10.0, -150.0]
    );
    assert_eq!(
        vertex(&f, MEMBER, "cut:v.h0.w0.y1.0"),
        [1990.0, -10.0, 150.0]
    );
    let (ends, vertex_ids) = edges(&f, MEMBER, "cut:e.h0.w0.s0.y0");
    let rim = |end: [f64; 3], y: f64| {
        (end[0] - 1990.0).abs() < 1e-6 && (end[1] - y).abs() < 1e-6 && (end[2] + 150.0).abs() < 1e-6
    };
    assert!(
        (rim(ends[0], -10.0) && rim(ends[1], 10.0)) || (rim(ends[0], 10.0) && rim(ends[1], -10.0)),
        "{ends:?}"
    );
    // Both ends sit on the entry ring: rim vertices y0.0 and y0.1, in either order.
    assert!(
        (vertex_ids[0].ends_with("cut:v.h0.w0.y0.0")
            && vertex_ids[1].ends_with("cut:v.h0.w0.y0.1"))
            || (vertex_ids[0].ends_with("cut:v.h0.w0.y0.1")
                && vertex_ids[1].ends_with("cut:v.h0.w0.y0.0")),
        "{vertex_ids:?}"
    );
    // The corner edge v0 runs between the two rings of cavity point 0.
    let (ends, vertex_ids) = edges(&f, MEMBER, "cut:e.h0.w0.v0");
    let on_ring = |end: [f64; 3], z: f64| {
        (end[0] - 1990.0).abs() < 1e-6 && (end[1] + 10.0).abs() < 1e-6 && (end[2] - z).abs() < 1e-6
    };
    assert!(
        (on_ring(ends[0], -150.0) && on_ring(ends[1], 150.0))
            || (on_ring(ends[0], 150.0) && on_ring(ends[1], -150.0)),
        "{ends:?}"
    );
    assert!(
        vertex_ids.contains(&fid(MEMBER, "cut:v.h0.w0.y0.0"))
            && vertex_ids.contains(&fid(MEMBER, "cut:v.h0.w0.y1.0")),
        "{vertex_ids:?}"
    );

    // The rim vertex snaps like any box vertex; the rim edge resolves its nearest point
    // and its midpoint.
    let corner = fid(MEMBER, "cut:v.h0.w0.y0.0");
    let snapped = snap_one(&f, [1990.0, -10.0, -150.0], "vertex");
    assert_eq!(text(&snapped, "feature_id"), corner);
    assert_eq!(
        triple(at(&snapped, &["point_world_mm"])),
        [1990.0, -10.0, -150.0]
    );
    let snapped = snap_one(&f, [1990.0, 0.0, -149.0], "edge");
    assert_eq!(
        text(&snapped, "feature_id"),
        fid(MEMBER, "cut:e.h0.w0.s0.y0")
    );
    assert_eq!(
        triple(at(&snapped, &["point_world_mm"])),
        [1990.0, 0.0, -150.0]
    );
    let snapped = snap_one(&f, [1990.0, 0.0, -149.0], "midpoint");
    assert_eq!(
        text(&snapped, "feature_id"),
        fid(MEMBER, "cut:e.h0.w0.s0.y0")
    );
    assert_eq!(
        triple(at(&snapped, &["point_world_mm"])),
        [1990.0, 0.0, -150.0]
    );
    assert_eq!(number(at(&snapped, &["axis_parameter"])), 0.5);

    // A point pick on the seam edge names it.
    let picked = f.ok(
        "pick",
        &[
            "--point",
            "1990,0,-150",
            "--radius",
            "1",
            "--targets",
            "edge",
        ],
    );
    let hits = list(&picked, "hits");
    assert_eq!(hits.len(), 1);
    assert_eq!(
        text(&hits[0], "feature_id"),
        fid(MEMBER, "cut:e.h0.w0.s0.y0")
    );
    assert_eq!(
        triple(at(&hits[0], &["point_world_mm"])),
        [1990.0, 0.0, -150.0]
    );

    // A move keeps every name and moves the feature (F06 section 5 rule 4).
    f.ok(
        "update-element",
        &["--id", MEMBER, "--translation", "100,0,0"],
    );
    assert_eq!(f.list_locals(MEMBER), locals);
    assert_eq!(
        vertex(&f, MEMBER, "cut:v.h0.w0.y0.0"),
        [2090.0, -10.0, -150.0]
    );
}

#[test]
fn a_member_hole_rim_picks_and_snaps_at_known_coordinates() {
    let f = Fixture::new();
    f.member();
    // The d=100 through hole along X_r (element +Y): the entry ring on `f.x0`
    // (element y = -100), the exit ring on `f.x1`. Ring point 0 sits at angle 0, the +x
    // side of the hole frame (element +X), 50 mm from the axis; the 0.05 mm chord bound
    // gives 72 segments.
    f.ok(
        "add-hole",
        &[
            "--id",
            MEMBER,
            "--operation-id",
            OP_A,
            "--axis-point",
            "0,0,2000",
            "--axis-direction",
            "1,0,0",
            "--diameter",
            "100",
        ],
    );
    let locals = f.list_locals(MEMBER);
    for local in [
        "cut:v.h0.w0.x0.0",
        "cut:v.h0.w0.x1.0",
        "cut:e.h0.w0.v0",
        "cut:e.h0.w0.s0.x0",
    ] {
        assert!(locals.iter().any(|n| n == local), "{local} missing");
    }
    assert_eq!(
        vertex(&f, MEMBER, "cut:v.h0.w0.x0.0"),
        [2050.0, -100.0, 0.0]
    );
    assert_eq!(vertex(&f, MEMBER, "cut:v.h0.w0.x1.0"), [2050.0, 100.0, 0.0]);
    let (ends, _) = edges(&f, MEMBER, "cut:e.h0.w0.v0");
    let on_ring = |end: [f64; 3], y: f64| {
        (end[0] - 2050.0).abs() < 1e-6 && (end[1] - y).abs() < 1e-6 && end[2].abs() < 1e-6
    };
    assert!(
        (on_ring(ends[0], -100.0) && on_ring(ends[1], 100.0))
            || (on_ring(ends[0], 100.0) && on_ring(ends[1], -100.0)),
        "{ends:?}"
    );
    // Ring point 1 walks clockwise: hole-frame (50cos, -50sin) is element +z.
    let angle = core::f64::consts::TAU / 72.0;
    assert_eq!(
        vertex(&f, MEMBER, "cut:v.h0.w0.x0.1"),
        [2000.0 + 50.0 * angle.cos(), -100.0, 50.0 * angle.sin()]
    );

    // The rim vertex snaps with its own id; the corner edge resolves its nearest point.
    let snapped = snap_one(&f, [2050.0, -100.0, 0.0], "vertex");
    assert_eq!(
        text(&snapped, "feature_id"),
        fid(MEMBER, "cut:v.h0.w0.x0.0")
    );
    let snapped = snap_one(&f, [2050.2, 0.0, 0.0], "edge");
    assert_eq!(text(&snapped, "feature_id"), fid(MEMBER, "cut:e.h0.w0.v0"));
    assert_eq!(
        triple(at(&snapped, &["point_world_mm"])),
        [2050.0, 0.0, 0.0]
    );

    // A point pick near the entry ring names the rim vertex, and the ray down the hole
    // axis crosses the corner edge together with the four rim edges that touch it at the
    // shared ring vertices.
    let picked = f.ok(
        "pick",
        &[
            "--point",
            "2050,-101,0",
            "--radius",
            "2",
            "--targets",
            "vertex",
        ],
    );
    let hits = list(&picked, "hits");
    assert_eq!(hits.len(), 1);
    assert_eq!(
        text(&hits[0], "feature_id"),
        fid(MEMBER, "cut:v.h0.w0.x0.0")
    );
    let picked = f.ok(
        "pick",
        &[
            "--origin",
            "2050,-200,0",
            "--direction",
            "0,1,0",
            "--t-max",
            "400",
            "--radius",
            "1",
            "--targets",
            "edge",
        ],
    );
    let hits = list(&picked, "hits");
    assert_eq!(hits.len(), 5, "{picked:?}");
    let ids: Vec<String> = hits.iter().map(|h| text(h, "feature_id")).collect();
    assert!(ids.contains(&fid(MEMBER, "cut:e.h0.w0.v0")), "{ids:?}");
    assert!(ids.contains(&fid(MEMBER, "cut:e.h0.w0.s0.x0")), "{ids:?}");
    assert!(ids.contains(&fid(MEMBER, "cut:e.h0.w0.s71.x1")), "{ids:?}");
    assert_eq!(number(at(&hits[0], &["t_mm"])), 100.0);
}

#[test]
fn an_oblique_part_cut_wall_picks_and_snaps_at_known_coordinates() {
    let f = Fixture::new();
    f.member();
    // The knife plate: a 40 x 40 contour whose normal leans along the member at 45
    // degrees (element (sqrt2/2, 0, sqrt2/2)), centred at (2000, -20, 0). The admission
    // maps the contour into the member hole frame (x = the member axis projected onto the
    // plane perpendicular to the cut direction, y = the direction cross it): with this
    // tool orientation the mapped polygon is the tool's local XY, reversed to the
    // clockwise [(0,40), (40,40), (40,0), (0,0)]. The prism crosses the two y-walls of the
    // solid section once. A prism point (a, b) = (2000 + px, -20 + py) leans along the
    // cut, so the wall piercing of corner (px, py) sits at element
    // (sqrt2 x (2000 + px) -/+ 150, -20 + py, -/+150) - the + sign on the entry wall
    // `f.y0` at z = -150, the - sign on the exit wall `f.y1` at z = +150 (the axial
    // travel of the lean: the two rings 300 mm apart along x).
    f.ok(
        "create-plate",
        &[
            "--width",
            "40",
            "--length",
            "40",
            "--thickness",
            "500",
            "--translation",
            "2000,-20,0",
            "--orientation-xyzw",
            "0,0.3826834323650898,0,0.9238795325112867",
            "--new-id",
            TOOL,
        ],
    );
    f.ok(
        "cut-by-part",
        &["--id", MEMBER, "--operation-id", OP_A, "--tool", TOOL],
    );
    let locals = f.list_locals(MEMBER);
    for local in [
        "cut:v.h0.w0.y0.0",
        "cut:v.h0.w0.y0.1",
        "cut:v.h0.w0.y1.0",
        "cut:e.h0.w0.s0.y0",
        "cut:e.h0.w0.s0.y1",
        "cut:e.h0.w0.v0",
    ] {
        assert!(locals.iter().any(|n| n == local), "{local} missing");
    }
    let sqrt2 = core::f64::consts::SQRT_2;
    let entry_x = |px: f64| sqrt2 * (2000.0 + px) - 150.0;
    let exit_x = |px: f64| sqrt2 * (2000.0 + px) + 150.0;
    assert_eq!(
        vertex(&f, MEMBER, "cut:v.h0.w0.y0.0"),
        [entry_x(0.0), 20.0, -150.0]
    );
    assert_eq!(
        vertex(&f, MEMBER, "cut:v.h0.w0.y1.0"),
        [exit_x(0.0), 20.0, 150.0]
    );
    assert_eq!(
        vertex(&f, MEMBER, "cut:v.h0.w0.y0.1"),
        [entry_x(40.0), 20.0, -150.0]
    );
    assert_eq!(
        vertex(&f, MEMBER, "cut:v.h0.w0.y1.3"),
        [exit_x(0.0), -20.0, 150.0]
    );
    let (ends, _) = edges(&f, MEMBER, "cut:e.h0.w0.v0");
    let on_point = |end: [f64; 3], x: f64, z: f64| {
        (end[0] - x).abs() < 1e-6 && (end[1] - 20.0).abs() < 1e-6 && (end[2] - z).abs() < 1e-6
    };
    assert!(
        (on_point(ends[0], entry_x(0.0), -150.0) && on_point(ends[1], exit_x(0.0), 150.0))
            || (on_point(ends[0], exit_x(0.0), 150.0) && on_point(ends[1], entry_x(0.0), -150.0)),
        "{ends:?}"
    );
    let (ends, _) = edges(&f, MEMBER, "cut:e.h0.w0.s0.y0");
    let on_rim = |end: [f64; 3], x: f64, y: f64| {
        (end[0] - x).abs() < 1e-6 && (end[1] - y).abs() < 1e-6 && (end[2] + 150.0).abs() < 1e-6
    };
    assert!(
        (on_rim(ends[0], entry_x(0.0), 20.0) && on_rim(ends[1], entry_x(40.0), 20.0))
            || (on_rim(ends[0], entry_x(40.0), 20.0) && on_rim(ends[1], entry_x(0.0), 20.0)),
        "{ends:?}"
    );

    // The entry rim vertex and the leaning corner edge snap at their exact positions.
    let snapped = snap_one(&f, [entry_x(0.0), 20.0, -150.0], "vertex");
    assert_eq!(
        text(&snapped, "feature_id"),
        fid(MEMBER, "cut:v.h0.w0.y0.0")
    );
    let snapped = snap_one(&f, [sqrt2 * 2000.0, 20.0, 0.0], "edge");
    assert_eq!(text(&snapped, "feature_id"), fid(MEMBER, "cut:e.h0.w0.v0"));
    assert_eq!(
        triple(at(&snapped, &["point_world_mm"])),
        [sqrt2 * 2000.0, 20.0, 0.0]
    );
}

#[test]
fn record_kinds_without_cut_edges_refuse_a_cut_edge_id_explicitly() {
    let f = Fixture::new();
    f.member();
    // A member with no records: no cut features at all.
    assert!(!f.list_locals(MEMBER).iter().any(|n| n.starts_with("cut:")));
    assert_eq!(
        f.refused(
            "lookup-feature",
            &["--feature-id", &fid(MEMBER, "cut:e.h0.w0.v0")]
        ),
        (
            "NOT_FOUND".to_owned(),
            Some("GEO_FEATURE_NOT_FOUND".to_owned())
        )
    );
    // A plane cut keeps its cap corners (the cap-corner amendment) but names no cut
    // edges: the prism-cut rim and corner names are the only cut edges there are. The
    // cut plane is skewed across the member (the line along element Y, the axis (1,0,1):
    // the plane x - z = 500), so the kept body is not a box and the cap corners name.
    f.ok(
        "cut-by-line",
        &[
            "--id",
            MEMBER,
            "--operation-id",
            OP_B,
            "--op-id",
            OP_B,
            "--point-a",
            "500,-500,0",
            "--point-b",
            "500,500,0",
            "--axis",
            "1,0,1",
            "--keep",
            "positive",
        ],
    );
    let locals = f.list_locals(MEMBER);
    assert!(locals.iter().any(|n| n.starts_with("cut:v.p0.")));
    assert!(!locals.iter().any(|n| n.starts_with("cut:e.")));
    assert_eq!(
        f.refused(
            "lookup-feature",
            &["--feature-id", &fid(MEMBER, "cut:e.h0.w0.v0")]
        ),
        (
            "NOT_FOUND".to_owned(),
            Some("GEO_FEATURE_NOT_FOUND".to_owned())
        )
    );
    // A malformed cut edge id stays an id error, not a not-found.
    assert_eq!(
        f.refused(
            "lookup-feature",
            &["--feature-id", &fid(MEMBER, "cut:e.h0.s0")]
        )
        .1,
        Some("GEO_FEATURE_ID_INVALID".to_owned())
    );
}
