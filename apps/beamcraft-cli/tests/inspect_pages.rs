// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! `inspect-elements --offset/--limit`: pages of a revision's elements that together list the
//! whole revision exactly once, framed by its counts.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use crate::support;
use beamcraft_json::Json;
use std::collections::BTreeSet;
use support::{StoreFixture, at, text};

fn ok(f: &StoreFixture, command: &str, options: &[&str]) -> Json {
    let (code, reply) = f.run(command, options);
    assert_eq!(code, 0, "{command} {options:?}: {reply:?}");
    reply
}

fn ids(reply: &Json, key: &str, id: &str) -> Vec<String> {
    at(reply, &[key])
        .array()
        .unwrap()
        .iter()
        .map(|e| text(e, id))
        .collect()
}

fn number(reply: &Json, key: &str) -> Option<u64> {
    reply.get(key).unwrap().unsigned().ok()
}

#[test]
fn pages_list_every_element_once_and_keep_the_whole_revision_counts() {
    let f = StoreFixture::new("inspect-pages");
    for x in ["0", "1000", "2000"] {
        ok(
            &f,
            "create-plate",
            &[
                "--width",
                "100",
                "--length",
                "50",
                "--thickness",
                "10",
                "--translation",
                &format!("{x},0,0"),
            ],
        );
    }
    let profile = text(
        at(
            &ok(&f, "define-profile", &["--width", "100", "--height", "200"]),
            &["definition"],
        ),
        "digest",
    );
    for y in ["0", "1000"] {
        ok(
            &f,
            "create-member",
            &[
                "--start",
                &format!("0,{y},500"),
                "--end",
                &format!("3000,{y},500"),
                "--profile-digest",
                &profile,
            ],
        );
    }
    let whole = ok(&f, "inspect-elements", &[]);
    let mut expected: Vec<String> = ids(&whole, "plates", "element_id");
    expected.extend(ids(&whole, "members", "element_id"));
    expected.sort();
    assert_eq!(expected.len(), 5);

    let mut listed = Vec::new();
    let mut offset = Some(0u64);
    let mut sizes = Vec::new();
    while let Some(at_offset) = offset {
        let page = ok(
            &f,
            "inspect-elements",
            &["--offset", &at_offset.to_string(), "--limit", "2"],
        );
        assert_eq!(number(&page, "element_count"), Some(5));
        assert_eq!(number(&page, "offset"), Some(at_offset));
        assert_eq!(number(&page, "plate_count"), Some(3));
        assert_eq!(number(&page, "member_count"), Some(2));
        let mut on_page = ids(&page, "plates", "element_id");
        on_page.extend(ids(&page, "members", "element_id"));
        on_page.sort();
        // The snapshot part of the page lists exactly the same elements.
        let mut snapshot = ids(at(&page, &["snapshot"]), "elements", "id");
        snapshot.sort();
        assert_eq!(snapshot, on_page);
        assert_eq!(number(&page, "page_size"), Some(on_page.len() as u64));
        sizes.push(on_page.len());
        listed.extend(on_page);
        offset = number(&page, "next_offset");
    }
    assert_eq!(sizes, vec![2, 2, 1]);
    listed.sort();
    assert_eq!(listed, expected);
    assert_eq!(listed.iter().collect::<BTreeSet<_>>().len(), 5);

    // Past the end: an empty last page.
    let past = ok(&f, "inspect-elements", &["--offset", "9", "--limit", "2"]);
    assert_eq!(number(&past, "page_size"), Some(0));
    assert_eq!(number(&past, "next_offset"), None);

    // A page and a single element do not combine; the page size is bounded.
    for options in [
        vec!["--element", expected[0].as_str(), "--limit", "2"],
        vec!["--limit", "0"],
        vec!["--limit", "257"],
        vec!["--offset", "-1"],
    ] {
        let (code, reply) = f.run("inspect-elements", &options);
        assert_eq!(code, 2, "{options:?}: {reply:?}");
        assert_eq!(text(&reply, "code"), "INVALID_ARGUMENT", "{options:?}");
    }
}
