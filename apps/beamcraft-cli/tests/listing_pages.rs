// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic scenarios; the fastener values are the shipped synthetic
// library's, which follows no national standard, MPL-2.0.

//! `--offset/--limit` pages of the whole-model listings (`bolts`, `welds`, `list-grids`,
//! `list-feature`): a walk pages every entry exactly once with no gaps or duplicates, the
//! reply without the options keeps its legacy shape, and a bad page refuses. The shared
//! `fit_page` shrink and the option bounds are unit-tested in `plate_cmd`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support;
use beamcraft_json::Json;
use support::{StoreFixture, at, list, text};

const SYNTHETIC: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/libraries/synthetic-fasteners.usda"
);

fn ok(f: &StoreFixture, command: &str, options: &[&str]) -> Json {
    let (code, reply) = f.run(command, options);
    assert_eq!(code, 0, "{command} {options:?}: {reply:?}");
    reply
}

fn rejected_code(f: &StoreFixture, command: &str, options: &[&str]) -> String {
    let (code, reply) = f.run(command, options);
    assert_eq!(code, 2, "{command} {options:?}: {reply:?}");
    text(&reply, "code")
}

fn field_ids(reply: &Json, key: &str, field: &str) -> Vec<String> {
    list(reply, key).iter().map(|e| text(e, field)).collect()
}

fn number(reply: &Json, key: &str) -> Option<u64> {
    reply.get(key).ok().and_then(|v| v.unsigned().ok())
}

/// Walks `command` one entry per page and returns the listed entry ids in page order.
fn walk(f: &StoreFixture, command: &str, keys: &[&str]) -> Vec<String> {
    let mut listed = Vec::new();
    let mut offset = 0u64;
    loop {
        let offset_text = offset.to_string();
        let page = ok(
            f,
            command,
            &["--offset", offset_text.as_str(), "--limit", "1"],
        );
        assert_eq!(number(&page, "offset"), Some(offset), "{command}");
        assert_eq!(number(&page, "page_size"), Some(1), "{command}");
        for key in keys {
            listed.extend(field_ids(&page, key, "id"));
        }
        match number(&page, "next_offset") {
            Some(next) => offset = next,
            None => return listed,
        }
    }
}

/// Two stacked plates and two washered groups of two fasteners at distinct points.
fn two_bolt_groups(f: &StoreFixture) {
    for (id, translation) in [
        ("019a7bf0-0000-7000-8000-0000007100f1", None),
        ("019a7bf0-0000-7000-8000-0000007100f2", Some("0,0,10")),
    ] {
        let mut args = vec![
            "--width",
            "1000",
            "--length",
            "500",
            "--thickness",
            "10",
            "--new-id",
            id,
        ];
        if let Some(at) = translation {
            args.extend(["--translation", at]);
        }
        ok(f, "create-plate", &args);
    }
    let parts = "019a7bf0-0000-7000-8000-0000007100f1,019a7bf0-0000-7000-8000-0000007100f2";
    for (name, id, point1, point2) in [
        (
            "G1",
            "019a7bf0-0000-7000-8000-000000710001",
            "0,250,0",
            "600,250,0",
        ),
        (
            "G2",
            "019a7bf0-0000-7000-8000-000000710002",
            "200,250,0",
            "800,250,0",
        ),
    ] {
        ok(
            f,
            "create-bolt-group",
            &[
                "--name",
                name,
                "--new-id",
                id,
                "--parts",
                parts,
                "--fastener-library",
                SYNTHETIC,
                "--designation",
                "SYN-BOLT-12x50",
                "--point1",
                point1,
                "--point2",
                point2,
                "--plane-normal",
                "0,0,1",
                "--offset-start",
                "100",
                "--spacings",
                "150",
            ],
        );
    }
}

#[test]
fn bolt_pages_walk_both_collections_once_and_the_legacy_reply_stays_whole() {
    let f = StoreFixture::new("listing-pages-bolts");
    two_bolt_groups(&f);
    let whole = ok(&f, "bolts", &[]);
    // The legacy reply carries no page framing.
    for key in ["offset", "page_size", "next_offset", "bolt_count"] {
        assert!(
            whole.get(key).is_err(),
            "{key} leaked into the legacy reply"
        );
    }
    let expected: Vec<String> = field_ids(&whole, "bolts", "id")
        .into_iter()
        .chain(field_ids(&whole, "bolt_groups", "id"))
        .collect();
    let total = expected.len();
    assert!(total >= 2, "{total}");

    let listed = walk(&f, "bolts", &["bolts", "bolt_groups"]);
    assert_eq!(listed, expected, "the pages tile the shared range exactly");

    // A page past the end is empty and last.
    let past = ok(
        &f,
        "bolts",
        &["--offset", &total.to_string(), "--limit", "2"],
    );
    assert!(field_ids(&past, "bolts", "id").is_empty());
    assert!(field_ids(&past, "bolt_groups", "id").is_empty());
    assert_eq!(number(&past, "page_size"), Some(0));
    assert_eq!(number(&past, "next_offset"), None);
    // The whole-revision counts sit on every page.
    let middle = ok(&f, "bolts", &["--offset", "1", "--limit", "1"]);
    assert_eq!(
        number(&middle, "bolt_count"),
        Some(list(&whole, "bolts").len() as u64)
    );
    assert_eq!(
        number(&middle, "bolt_group_count"),
        Some(list(&whole, "bolt_groups").len() as u64)
    );
    // Bad pages refuse.
    for bad in [
        ["--limit", "0"],
        ["--limit", "257"],
        ["--limit", "abc"],
        ["--offset", "-1"],
    ] {
        assert_eq!(
            rejected_code(&f, "bolts", &bad),
            "INVALID_ARGUMENT",
            "{bad:?}"
        );
    }
}

/// Two stacked plates, one beam, and two welds on the plate.
fn weld_fixture(f: &StoreFixture) -> Vec<String> {
    let profile = text(
        at(
            &ok(f, "define-profile", &["--width", "100", "--height", "200"]),
            &["definition"],
        ),
        "digest",
    );
    ok(
        f,
        "create-plate",
        &["--width", "300", "--length", "200", "--thickness", "10"],
    );
    ok(
        f,
        "create-member",
        &[
            "--start",
            "0,1000,500",
            "--end",
            "3000,1000,500",
            "--profile-digest",
            &profile,
        ],
    );
    let plates = list(&ok(f, "inspect-elements", &[]), "plates").to_vec();
    let members = list(&ok(f, "inspect-elements", &[]), "members").to_vec();
    let plate = text(&plates[0], "element_id");
    let beam = text(&members[0], "element_id");
    for (n, name) in ["W1", "W2"].into_iter().enumerate() {
        ok(
            f,
            "create-weld",
            &[
                "--name",
                name,
                "--main",
                plate.as_str(),
                "--secondary",
                beam.as_str(),
                "--size",
                "6",
                "--location",
                "workshop",
                "--side",
                "both",
                "--new-id",
                &format!("019a7bf0-0000-7000-8000-00000073000{n}"),
            ],
        );
    }
    vec![plate, beam]
}

#[test]
fn weld_pages_walk_the_listing_once_and_refuse_bad_pages() {
    let f = StoreFixture::new("listing-pages-welds");
    weld_fixture(&f);
    let whole = ok(&f, "welds", &[]);
    assert!(whole.get("offset").is_err(), "legacy shape changed");
    let expected = field_ids(&whole, "welds", "id");
    assert_eq!(expected.len(), 2);
    let listed = walk(&f, "welds", &["welds"]);
    assert_eq!(listed, expected);
    let page = ok(&f, "welds", &["--offset", "0", "--limit", "1"]);
    assert_eq!(number(&page, "weld_count"), Some(2));
    assert_eq!(number(&page, "next_offset"), Some(1));
    for bad in [["--limit", "0"], ["--limit", "257"], ["--offset", "-1"]] {
        assert_eq!(
            rejected_code(&f, "welds", &bad),
            "INVALID_ARGUMENT",
            "{bad:?}"
        );
    }
}

#[test]
fn grid_pages_walk_the_listing_once_and_refuse_bad_pages() {
    let f = StoreFixture::new("listing-pages-grids");
    for (name, x) in [("A", "0"), ("B", "4000")] {
        ok(
            &f,
            "create-grid",
            &[
                "--name",
                name,
                "--origin",
                &format!("{x},0"),
                "--u-expression",
                "1000 2000 3000",
            ],
        );
    }
    let whole = ok(&f, "list-grids", &[]);
    assert!(whole.get("offset").is_err(), "legacy shape changed");
    let expected = field_ids(&whole, "grids", "id");
    assert_eq!(expected.len(), 2);
    let listed = walk(&f, "list-grids", &["grids"]);
    assert_eq!(listed, expected);
    let page = ok(&f, "list-grids", &["--offset", "1", "--limit", "1"]);
    assert_eq!(number(&page, "grid_count"), Some(2));
    assert_eq!(number(&page, "next_offset"), None);
    for bad in [["--limit", "0"], ["--limit", "257"], ["--offset", "-1"]] {
        assert_eq!(
            rejected_code(&f, "list-grids", &bad),
            "INVALID_ARGUMENT",
            "{bad:?}"
        );
    }
}

#[test]
fn feature_pages_list_one_element_exactly_once() {
    let f = StoreFixture::new("listing-pages-features");
    ok(
        &f,
        "create-plate",
        &["--width", "300", "--length", "200", "--thickness", "10"],
    );
    let plate = text(
        &list(&ok(&f, "inspect-elements", &[]), "plates")[0],
        "element_id",
    );
    let whole = ok(&f, "list-feature", &["--id", plate.as_str()]);
    assert!(whole.get("offset").is_err(), "legacy shape changed");
    let expected = field_ids(&whole, "features", "feature_id");
    let total = expected.len();
    assert!(total > 1, "{total} features of a box plate");

    let mut listed = Vec::new();
    let mut offset = 0u64;
    loop {
        let offset_text = offset.to_string();
        let page = ok(
            &f,
            "list-feature",
            &[
                "--id",
                plate.as_str(),
                "--offset",
                offset_text.as_str(),
                "--limit",
                "10",
            ],
        );
        assert_eq!(number(&page, "feature_count"), Some(total as u64));
        assert_eq!(number(&page, "offset"), Some(offset));
        let ids = field_ids(&page, "features", "feature_id");
        assert!(ids.len() <= 10, "page over the limit");
        let done = number(&page, "next_offset").is_none();
        listed.extend(ids);
        if done {
            break;
        }
        offset = number(&page, "next_offset").unwrap();
    }
    assert_eq!(listed, expected);
    for bad in [["--limit", "0"], ["--limit", "257"], ["--offset", "-1"]] {
        assert_eq!(
            rejected_code(
                &f,
                "list-feature",
                &["--id", plate.as_str(), bad[0], bad[1]]
            ),
            "INVALID_ARGUMENT",
            "{bad:?}"
        );
    }
}
