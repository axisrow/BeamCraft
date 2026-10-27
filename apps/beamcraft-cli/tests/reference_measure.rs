// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic sources, MPL-2.0. The "foreign" models below are invented text
// written by this test; no third-party model, sample or asset is used.

//! R02 end to end: `measure-reference` reports the kernel measurement of a wrapper's composed
//! foreign surface through the real `beamcraft` binary. Units and the full placement are
//! respected; open, self-intersecting and unsupported surfaces report their status without
//! values; mass is never invented; the command is read-only and deterministic.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

mod support;

use support::{at, text};

use beamcraft_json::{Json, Limits};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

/// An invented architect model: metres, Y up, one box body 2 x 1 x 3 (volume 6 m3).
const SITE: &str = r#"#usda 1.0
(
    defaultPrim = "Site"
    metersPerUnit = 1.0
    upAxis = "Y"
)
def Xform "Site"
{
    def Xform "Block"
    {
        def Mesh "Body"
        {
            uniform token subdivisionScheme = "none"
            point3f[] points = [(0, 0, 0), (2, 0, 0), (2, 1, 0), (0, 1, 0), (0, 0, 3), (2, 0, 3), (2, 1, 3), (0, 1, 3)]
            int[] faceVertexCounts = [4, 4, 4, 4, 4, 4]
            int[] faceVertexIndices = [0, 3, 2, 1, 4, 5, 6, 7, 0, 1, 5, 4, 1, 2, 6, 5, 2, 3, 7, 6, 3, 0, 4, 7]
        }
    }
}
"#;

/// An open surface: two triangles sharing one edge, no enclosed volume.
const FIN: &str = r#"#usda 1.0
(
    defaultPrim = "Site"
    metersPerUnit = 0.001
    upAxis = "Z"
)
def Xform "Site"
{
    def Mesh "Fin"
    {
        uniform token subdivisionScheme = "none"
        point3f[] points = [(0, 0, 0), (2000, 0, 0), (0, 2000, 0), (0, 0, 2000)]
        int[] faceVertexCounts = [3, 3]
        int[] faceVertexIndices = [0, 1, 2, 0, 1, 3]
    }
}
"#;

/// Two closed shells that interpenetrate: never an exact measurement.
const OVERLAP: &str = r#"#usda 1.0
(
    defaultPrim = "Site"
    metersPerUnit = 0.001
    upAxis = "Z"
)
def Xform "Site"
{
    def Mesh "Tangle"
    {
        uniform token subdivisionScheme = "none"
        point3f[] points = [(0, 0, 0), (2, 0, 0), (2, 2, 0), (0, 2, 0), (0, 0, 2), (2, 0, 2), (2, 2, 2), (0, 2, 2), (1, 1, 1), (3, 1, 1), (3, 3, 1), (1, 3, 1), (1, 1, 3), (3, 1, 3), (3, 3, 3), (1, 3, 3)]
        int[] faceVertexCounts = [4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4]
        int[] faceVertexIndices = [0, 3, 2, 1, 4, 5, 6, 7, 0, 1, 5, 4, 1, 2, 6, 5, 2, 3, 7, 6, 3, 0, 4, 7, 8, 11, 10, 9, 12, 13, 14, 15, 8, 9, 13, 12, 9, 10, 14, 13, 10, 11, 15, 14, 11, 8, 12, 15]
    }
}
"#;

#[test]
fn the_composed_surface_measures_in_world_millimetres_and_answers_twice_the_same() {
    let f = Fixture::new();
    let id = "019a7bf0-0000-7000-8000-000000000201";
    f.attach(&f.source("site.usda", SITE), id);
    f.ok(
        "correct-reference-units",
        &["--id", id, "--meters-per-unit", "1", "--up-axis", "Y"],
    );
    // Deterministic: the same head answers with byte-identical output.
    let first = f.raw("measure-reference", &[]);
    let second = f.raw("measure-reference", &[]);
    assert_eq!(
        first, second,
        "the same head must answer with the same bytes"
    );
    let body = decode(&first);
    assert_eq!(text(&body, "outcome"), "reference_measurements");
    assert_eq!(at(&body, &["measurements"]).array().unwrap().len(), 1);
    let mesh = item(&body, "measurements", 0, &["measurement"]);
    assert_eq!(at(mesh, &["source_version"]).unsigned().unwrap(), 1);
    let reading = at(mesh, &["mesh"]);
    assert_eq!(text(reading, "status"), "certified");
    // 6 m3 of millimetres, inside the exact-sum enclosure.
    let volume = at(reading, &["volume_mm3"]);
    assert!(number(volume, &["lo"]) <= 6_000_000_000.0);
    assert!(number(volume, &["hi"]) >= 6_000_000_000.0);
    let area = at(reading, &["area_mm2"]);
    assert!(number(area, &["lo"]) <= 22_000_000.0);
    assert!(number(area, &["hi"]) >= 22_000_000.0);
    assert_eq!(
        (bounds(reading, "min"), bounds(reading, "max")),
        ([0.0, -3000.0, 0.0], [2000.0, 0.0, 1000.0]),
    );
    // Mass is never invented.
    assert_eq!(text(at(reading, &["mass"]), "status"), "unsupported");
    assert_eq!(*at(reading, &["mass", "kg"]), Json::Null);
    // The input signature names the pinned source.
    let inspect = f.ok("inspect-reference", &[]);
    assert_eq!(
        text(mesh, "source_digest"),
        text(item(&inspect, "references", 0, &["source"]), "digest"),
    );
    // A move changes the world geometry, its signature and the bounds, not the volume.
    let before = text(reading, "mesh_digest");
    f.ok("move-reference", &["--id", id, "--translation", "100,0,0"]);
    let moved = f.ok("measure-reference", &[]);
    let reading = item(&moved, "measurements", 0, &["measurement", "mesh"]);
    assert_eq!(text(reading, "status"), "certified");
    assert_ne!(text(reading, "mesh_digest"), before);
    assert_eq!(bounds(reading, "min"), [100.0, -3000.0, 0.0]);
    assert_eq!(bounds(reading, "max"), [2100.0, 0.0, 1000.0]);
}

#[test]
fn a_rotated_wrapper_measures_where_it_stands() {
    let f = Fixture::new();
    let id = "019a7bf0-0000-7000-8000-000000000202";
    f.attach(&f.source("site.usda", SITE), id);
    f.ok(
        "correct-reference-units",
        &["--id", id, "--meters-per-unit", "1", "--up-axis", "Y"],
    );
    f.ok(
        "move-reference",
        &[
            "--id",
            id,
            "--translation",
            "10,20,30",
            "--rotation-z-deg",
            "90",
        ],
    );
    let body = f.ok("measure-reference", &[]);
    let reading = item(&body, "measurements", 0, &["measurement", "mesh"]);
    assert_eq!(text(reading, "status"), "certified");
    // The corrected box spans x 0..2000, y -3000..0; a quarter turn about Z maps that onto
    // x 0..3000, y 0..2000, then the translation moves it over (10, 20, 30). The turn is
    // axis-preserving here, so the bounds follow to within the quaternion's float noise.
    let near = |got: [f64; 3], want: [f64; 3]| {
        for (a, b) in got.iter().zip(want) {
            assert!((a - b).abs() < 1e-6, "{got:?} != {want:?}");
        }
    };
    near(bounds(reading, "min"), [10.0, 20.0, 30.0]);
    near(bounds(reading, "max"), [3010.0, 2020.0, 1030.0]);
    let volume = at(reading, &["volume_mm3"]);
    assert!(number(volume, &["lo"]) <= 6_000_000_000.0);
    assert!(number(volume, &["hi"]) >= 6_000_000_000.0);
}

#[test]
fn an_open_or_self_intersecting_surface_is_never_measured_exactly() {
    let f = Fixture::new();
    let fin = "019a7bf0-0000-7000-8000-000000000203";
    f.attach(&f.source("fin.usda", FIN), fin);
    let body = f.ok("measure-reference", &["--id", fin]);
    let reading = item(&body, "measurements", 0, &["measurement", "mesh"]);
    assert_eq!(text(reading, "status"), "open");
    assert_eq!(*reading.get("volume_mm3").unwrap(), Json::Null);
    assert_eq!(*reading.get("area_mm2").unwrap(), Json::Null);
    assert_eq!(*reading.get("centroid_mm").unwrap(), Json::Null);
    assert_eq!(text(at(reading, &["mass"]), "status"), "unsupported");
    // The bounds exist even where the values do not.
    assert_eq!(bounds(reading, "max"), [2000.0, 2000.0, 2000.0]);
    let tangle = "019a7bf0-0000-7000-8000-000000000204";
    f.attach(&f.source("tangle.usda", OVERLAP), tangle);
    let body = f.ok("measure-reference", &["--id", tangle]);
    let reading = item(&body, "measurements", 0, &["measurement", "mesh"]);
    assert_eq!(text(reading, "status"), "overlapping");
    assert_eq!(*reading.get("volume_mm3").unwrap(), Json::Null);
    assert_eq!(text(at(reading, &["mass"]), "status"), "unsupported");
}

#[test]
fn an_unknown_wrapper_is_not_found_and_a_measurement_writes_nothing() {
    let f = Fixture::new();
    let id = "019a7bf0-0000-7000-8000-000000000205";
    f.attach(&f.source("site.usda", SITE), id);
    let before = f.raw("inspect-reference", &[]);
    let listing = f.listing();
    let (code, body) = f.run(
        "measure-reference",
        &["--id", "019a7bf0-0000-7000-8000-000000000299"],
    );
    assert_eq!(code, 2);
    assert_eq!(text(&body, "code"), "NOT_FOUND");
    assert_eq!(
        f.raw("inspect-reference", &[]),
        before,
        "the head is unchanged"
    );
    assert_eq!(f.listing(), listing, "no byte of the store was written");
}

#[test]
fn wrappers_measure_in_id_order_and_an_id_selects_one() {
    let f = Fixture::new();
    let b = "019a7bf0-0000-7000-8000-000000000206";
    let a = "019a7bf0-0000-7000-8000-000000000207";
    f.attach(&f.source("fin.usda", FIN), b);
    f.attach(&f.source("site.usda", SITE), a);
    let body = f.ok("measure-reference", &[]);
    let ids: Vec<String> = at(&body, &["measurements"])
        .array()
        .unwrap()
        .iter()
        .map(|m| text(m, "id"))
        .collect();
    assert_eq!(ids, [b.to_string(), a.to_string()]);
    let body = f.ok("measure-reference", &["--id", b]);
    let measured = at(&body, &["measurements"]).array().unwrap();
    assert_eq!(measured.len(), 1);
    assert_eq!(text(&measured[0], "id"), b);
}

#[test]
fn the_measurement_refuses_what_the_profile_cannot_compose() {
    let f = Fixture::new();
    let id = "019a7bf0-0000-7000-8000-000000000208";
    // A pentagon face is outside the triangle/convex-quad profile, exactly as for the view.
    let pentagon = r#"#usda 1.0
(
    defaultPrim = "Site"
    metersPerUnit = 0.001
    upAxis = "Z"
)
def Xform "Site"
{
    def Mesh "Plate"
    {
        uniform token subdivisionScheme = "none"
        point3f[] points = [(0, 0, 0), (2, 0, 0), (3, 1, 0), (1, 2, 0), (0, 1, 0), (1, 1, 1)]
        int[] faceVertexCounts = [5, 5]
        int[] faceVertexIndices = [0, 1, 2, 3, 4, 0, 1, 2, 3, 5]
    }
}
"#;
    f.attach(&f.source("pentagon.usda", pentagon), id);
    let listing = f.listing();
    let (code, body) = f.run("measure-reference", &["--id", id]);
    assert_eq!(code, 2);
    assert_eq!(text(&body, "code"), "UNSUPPORTED_GEOMETRY");
    assert_eq!(f.listing(), listing, "no byte of the store was written");
}

struct Fixture {
    shared: support::StoreFixture,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            shared: support::StoreFixture::new("r02-tests"),
        };
        fs::create_dir(fixture.shared.root.join("sources")).unwrap();
        fixture
    }
    fn store(&self) -> String {
        self.shared.root.join("store").to_str().unwrap().to_owned()
    }
    fn source(&self, name: &str, bytes: impl AsRef<[u8]>) -> String {
        let path = self.shared.root.join("sources").join(name);
        fs::write(&path, bytes).unwrap();
        path.to_str().unwrap().to_owned()
    }
    fn raw(&self, command: &str, extra: &[&str]) -> Vec<u8> {
        let mut args: Vec<String> = vec![command.to_owned(), "--store".to_owned(), self.store()];
        args.extend(extra.iter().map(|s| (*s).to_owned()));
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(&args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{command} {extra:?}: {output:?}");
        output.stdout
    }
    fn run(&self, command: &str, extra: &[&str]) -> (i32, Json) {
        let mut args: Vec<String> = vec![command.to_owned(), "--store".to_owned(), self.store()];
        args.extend(extra.iter().map(|s| (*s).to_owned()));
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(&args)
            .output()
            .unwrap();
        (output.status.code().unwrap(), decode(&output.stdout))
    }
    fn ok(&self, command: &str, extra: &[&str]) -> Json {
        let stdout = self.raw(command, extra);
        decode(&stdout)
    }
    fn attach(&self, source: &str, id: &str) -> Json {
        self.ok("attach-reference", &["--source", source, "--new-id", id])
    }
    /// Every file below the store root with its size, in path order.
    fn listing(&self) -> BTreeMap<String, u64> {
        fn walk(dir: &Path, root: &Path, out: &mut BTreeMap<String, u64>) {
            for entry in fs::read_dir(dir).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, root, out);
                } else {
                    let name = path
                        .strip_prefix(root)
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .replace('\\', "/");
                    out.insert(name, entry.metadata().unwrap().len());
                }
            }
        }
        let root = self.shared.root.join("store");
        let mut out = BTreeMap::new();
        walk(&root, &root, &mut out);
        out
    }
}

fn decode(bytes: &[u8]) -> Json {
    beamcraft_json::decode(bytes, &Limits::DEFAULT).unwrap()
}

/// The `index`-th element of the array at `key`.
fn item<'a>(json: &'a Json, key: &str, index: usize, path: &[&str]) -> &'a Json {
    path.iter().fold(
        json.get(key).unwrap().array().unwrap().get(index).unwrap(),
        |j, k| j.get(k).unwrap(),
    )
}

fn number(json: &Json, path: &[&str]) -> f64 {
    at(json, path).float().unwrap()
}

fn bounds(reading: &Json, corner: &str) -> [f64; 3] {
    let values = at(reading, &["bounds_mm", corner]).array().unwrap();
    [
        values[0].float().unwrap(),
        values[1].float().unwrap(),
        values[2].float().unwrap(),
    ]
}
