// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The `edit-drafting` replay contract: an exact retry replays the accepted revision, while
//! changed or dropped drafting under an accepted operation id is `OP_ID_REUSE` — even after
//! later edits. The drawn objects stay reachable from the document alone: no command in this
//! suite passes a model store except the explicit `--model-store` of the source pin.

#![allow(clippy::unwrap_used)]

use crate::support::{StoreFixture, at, text};
use beamcraft_json::Json;
use std::process::Command;

const MEMBER: &str = "01900000-0000-7000-8000-000000000220";
const DRAWING: &str = "01900000-0000-7000-8000-000000000201";
const VIEW: &str = "01900000-0000-7000-8000-000000000202";
const DIMENSION: &str = "01900000-0000-7000-8000-000000000203";
const ACTOR: &str = "01900000-0000-7000-8000-000000000204";
const OP_CREATE: &str = "01900000-0000-7000-8000-000000000205";
const REV_CREATE: &str = "01900000-0000-7000-8000-000000000206";
const OP_EDIT: &str = "01900000-0000-7000-8000-000000000207";
const REV_EDIT: &str = "01900000-0000-7000-8000-000000000208";
const OP_AFTER: &str = "01900000-0000-7000-8000-000000000211";
const REV_AFTER: &str = "01900000-0000-7000-8000-000000000212";
const LINE: &str = "01900000-0000-7000-8000-000000000214";
const LAYER: &str = "01900000-0000-7000-8000-00000000021c";

fn intent() -> Vec<String> {
    [
        "--drawing-id",
        DRAWING,
        "--view-id",
        VIEW,
        "--dimension-id",
        DIMENSION,
        "--id",
        MEMBER,
        "--anchor-a",
        &format!("fid1:{MEMBER}:member:v.x0.y0.z0"),
        "--anchor-b",
        &format!("fid1:{MEMBER}:member:v.x0.y0.z1"),
    ]
    .iter()
    .map(|value| (*value).to_owned())
    .collect()
}

fn argv(options: &[String]) -> Vec<&str> {
    options.iter().map(String::as_str).collect()
}

/// Runs a drawing command with no `--store` argument: the drawn objects must be reachable
/// from the document alone.
fn run(command: &str, extra: &[&str]) -> (i32, Json) {
    let mut args = vec![command];
    args.extend_from_slice(extra);
    let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
        .args(&args)
        .output()
        .unwrap();
    let body = beamcraft_json::decode(&output.stdout, &beamcraft_json::Limits::DEFAULT).unwrap();
    (output.status.code().unwrap_or(3), body)
}

fn ok(command: &str, extra: &[&str]) -> Json {
    let (code, body) = run(command, extra);
    assert_eq!(code, 0, "{command} {extra:?}: {body:?}");
    body
}

/// Runs `edit-drafting` with explicit drafting entities.
fn edit(
    document_dir: &str,
    expected_head: &str,
    op: &str,
    revision: &str,
    entities: &[String],
) -> (i32, Json) {
    let mut args = vec![
        "--document-dir".to_owned(),
        document_dir.to_owned(),
        "--expected-head".to_owned(),
        expected_head.to_owned(),
        "--op-id".to_owned(),
        op.to_owned(),
        "--revision-id".to_owned(),
        revision.to_owned(),
        "--actor-id".to_owned(),
        ACTOR.to_owned(),
    ];
    args.extend(entities.iter().cloned());
    run("edit-drafting", &argv(&args))
}

/// A store with one straight member, the source pin's live model.
fn model(f: &StoreFixture) {
    let (code, reply) = f.run("define-profile", &["--width", "100", "--height", "50"]);
    assert_eq!(code, 0, "{reply:?}");
    let profile = text(at(&reply, &["definition"]), "digest");
    let (code, reply) = f.run(
        "create-member",
        &[
            "--start",
            "0,0,0",
            "--end",
            "3000,0,0",
            "--profile-digest",
            &profile,
            "--new-id",
            MEMBER,
        ],
    );
    assert_eq!(code, 0, "{reply:?}");
}

#[test]
fn drafting_replay_binds_the_incoming_geometry_even_after_later_edits() {
    let f = StoreFixture::new("drawing-drafting");
    model(&f);
    let document_dir = f.root.join("replay-doc").to_str().unwrap().to_owned();
    let model_store = f.root.join("store").to_str().unwrap().to_owned();
    let mut create = vec![
        "--document-dir".to_owned(),
        document_dir.clone(),
        "--store".to_owned(),
        model_store,
        "--op-id".to_owned(),
        OP_CREATE.to_owned(),
        "--revision-id".to_owned(),
        REV_CREATE.to_owned(),
        "--actor-id".to_owned(),
        ACTOR.to_owned(),
    ];
    create.extend(intent());
    let head = text(&ok("create-drawing", &argv(&create)), "revision_id");
    let original = vec![
        "--layer".to_owned(),
        format!("{LAYER}:Outlines:on"),
        "--drafting".to_owned(),
        format!("{LINE}:line:0,0,10,0:{LAYER}:continuous"),
    ];
    let changed = vec![
        "--layer".to_owned(),
        format!("{LAYER}:Outlines:on"),
        "--drafting".to_owned(),
        format!("{LINE}:line:0,0,20,0:{LAYER}:continuous"),
    ];
    let (code, accepted) = edit(&document_dir, &head, OP_EDIT, REV_EDIT, &original);
    assert_eq!(code, 0, "{accepted:?}");
    let edited_head = text(&accepted, "revision_id");
    let (code, replayed) = edit(&document_dir, &head, OP_EDIT, REV_EDIT, &original);
    assert_eq!(code, 0, "{replayed:?}");
    assert_eq!(replayed, accepted);
    let (code, refused) = edit(&document_dir, &head, OP_EDIT, REV_EDIT, &changed);
    assert_eq!(code, 2, "{refused:?}");
    assert_eq!(text(&refused, "code"), "OP_ID_REUSE");
    assert_eq!(
        text(
            &ok("inspect-drawing", &["--document-dir", &document_dir]),
            "revision_id"
        ),
        edited_head
    );

    let (code, later) = edit(&document_dir, &edited_head, OP_AFTER, REV_AFTER, &changed);
    assert_eq!(code, 0, "{later:?}");
    let later_head = text(&later, "revision_id");
    let (code, replayed) = edit(&document_dir, &head, OP_EDIT, REV_EDIT, &original);
    assert_eq!(code, 0, "{replayed:?}");
    assert_eq!(replayed, accepted);
    for entities in [&changed[..], &[][..]] {
        let (code, refused) = edit(&document_dir, &head, OP_EDIT, REV_EDIT, entities);
        assert_eq!(code, 2, "{refused:?}");
        assert_eq!(text(&refused, "code"), "OP_ID_REUSE");
    }
    assert_eq!(
        text(
            &ok("inspect-drawing", &["--document-dir", &document_dir]),
            "revision_id"
        ),
        later_head
    );
}
