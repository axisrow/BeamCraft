// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! Reflection is an independent authored copy, including operation records.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod support;
use beamcraft_json::Json;
use support::StoreFixture;
fn ok(f: &StoreFixture, c: &str, a: &[&str]) -> Json {
    let (code, j) = f.run(c, a);
    assert_eq!(code, 0, "{c} {a:?}: {j:?}");
    j
}
fn text(j: &Json, k: &str) -> String {
    j.get(k).unwrap().string().unwrap().to_owned()
}
fn reflected(f: &StoreFixture, id: &str) -> String {
    let j = ok(
        f,
        "mirror-elements",
        &["--ids", id, "--point", "0,0,0", "--normal", "1,0,0"],
    );
    text(
        &j.get("elements").unwrap().array().unwrap()[0],
        "element_id",
    )
}
fn inspect(f: &StoreFixture, id: &str) -> Json {
    ok(f, "inspect-elements", &["--element", id])
}
fn bounds(j: &Json, k: &str) -> Vec<f64> {
    j.get("snapshot")
        .unwrap()
        .get("elements")
        .unwrap()
        .array()
        .unwrap()[0]
        .get(k)
        .unwrap()
        .array()
        .unwrap()
        .iter()
        .map(|v| v.float().unwrap())
        .collect()
}
#[test]
fn plate_hole_and_oblique_cut_reflect_then_return_to_original_solid() {
    let f = StoreFixture::new("mirror-plate");
    let p = ok(
        &f,
        "create-plate",
        &[
            "--width",
            "100",
            "--length",
            "60",
            "--thickness",
            "10",
            "--translation",
            "20,30,40",
        ],
    );
    let id = text(p.get("plate").unwrap(), "element_id");
    ok(&f, "add-hole", &["--id", &id, "--hole-rect", "10,10,10,10"]);
    ok(
        &f,
        "cut-by-plane",
        &[
            "--id",
            &id,
            "--point",
            "70,0,3",
            "--normal",
            "1,0,1",
            "--keep",
            "negative",
            "--operation-id",
            "019a7bf0-0000-7000-8000-000000900001",
        ],
    );
    let before = inspect(&f, &id);
    let m = reflected(&f, &id);
    let mirrored = inspect(&f, &m);
    let min = bounds(&before, "world_min_mm");
    let max = bounds(&before, "world_max_mm");
    let reflected_min = bounds(&mirrored, "world_min_mm");
    let reflected_max = bounds(&mirrored, "world_max_mm");
    assert!((reflected_min[0] + max[0]).abs() < 1e-8);
    assert!((reflected_max[0] + min[0]).abs() < 1e-8);
    let twice = reflected(&f, &m);
    let after = inspect(&f, &twice);
    for k in ["world_min_mm", "world_max_mm"] {
        for (a, b) in bounds(&before, k).into_iter().zip(bounds(&after, k)) {
            assert!((a - b).abs() < 1e-8);
        }
    }
    let a = &before.get("plates").unwrap().array().unwrap()[0];
    let b = &after.get("plates").unwrap().array().unwrap()[0];
    assert_eq!(a.get("mesh_digest"), b.get("mesh_digest"));
    assert_eq!(a.get("hole_count"), b.get("hole_count"));
    assert_eq!(inspect(&f, &id).get("plates"), before.get("plates"));
}
#[test]
fn rotated_member_mirror_and_refusals_keep_the_source_unchanged() {
    let f = StoreFixture::new("mirror-member");
    let profile = ok(
        &f,
        "define-profile",
        &[
            "--family", "channel", "--h", "40", "--b", "20", "--tw", "3", "--tf", "4", "--r", "0",
        ],
    );
    let digest = text(profile.get("definition").unwrap(), "digest");
    let j = ok(
        &f,
        "create-member",
        &[
            "--start",
            "10,20,30",
            "--end",
            "110,120,230",
            "--profile-digest",
            &digest,
            "--orientation-xyzw",
            "0,0,0.7071067811865475,0.7071067811865476",
        ],
    );
    let id = text(j.get("member").unwrap(), "element_id");
    let before = inspect(&f, &id);
    let m = reflected(&f, &id);
    // Bounds intentionally enclose the rotated local AABB. Validate the reflected solid's
    // vertices directly; rotating a conservative box need not yield a tight world box.
    let store = beamcraft_host::revision::RevisionStore::new(
        beamcraft_host::storage::BlobStore::new(f.root.join("store")).unwrap(),
    );
    let inspection = beamcraft_cli::plate::inspect(&store).unwrap();
    let source = inspection
        .members
        .iter()
        .find(|r| r.authored.id.to_string() == id)
        .unwrap();
    let copy = inspection
        .members
        .iter()
        .find(|r| r.authored.id.to_string() == m)
        .unwrap();
    let world = |pose: beamcraft_model::command::Placement, v: [f64; 3]| {
        let [x, y, z] = beamcraft_engine::transform::rotate(pose.orientation_xyzw, v);
        let [a, b, c] = pose.translation_mm;
        [x + a, y + b, z + c]
    };
    let points = copy
        .body
        .solid
        .vertices()
        .iter()
        .map(|v| world(copy.authored.placement, *v))
        .collect::<Vec<_>>();
    for v in source.body.solid.vertices() {
        let [x, y, z] = world(source.authored.placement, *v);
        let expected = [-x, y, z];
        assert!(
            points
                .iter()
                .any(|p| p.iter().zip(expected).all(|(a, b)| (*a - b).abs() < 1e-8))
        );
    }
    assert_eq!(source.quantities.volume_mm3, copy.quantities.volume_mm3);
    let head = ok(&f, "open-revision", &[]);
    for normal in ["0,0,0", "NaN,0,0"] {
        assert_eq!(
            f.run(
                "mirror-elements",
                &["--ids", &id, "--point", "0,0,0", "--normal", normal]
            )
            .0,
            2
        );
        assert_eq!(ok(&f, "open-revision", &[]), head);
    }
    assert_eq!(inspect(&f, &id).get("members"), before.get("members"));
}
