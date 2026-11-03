// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic plates, members and assemblies, MPL-2.0.

//! P09 acceptance: the one validation layer (dangling links, duplicates, module rules) is
//! enforced end to end through the real CLI binary on both doors that share it.
//!
//! The *client door* validates before anything is written: every authoring command opens the
//! live model through the structural validator, admits its batch through the change rules and
//! checks relationship preconditions (such as a delete that would leave a dangling link). The
//! *post-merge door* verifies merged bytes: every commit and every `publish-workspace` runs the
//! same structural validator over the complete layer set plus the identity index and the
//! derived re-evaluation. A violation reaches the post-merge door here through a head-based
//! layer edit committed without content verification — the way bytes edited outside BeamCraft
//! arrive — and must be refused with the same rule's code before the published head moves.
//!
//! Per-rule evidence that stays at the engine level (every structural and index rule with its
//! positive and negative case, budget bounds, fixed-phase-order determinism) is catalogued in
//! `docs/design/p09-validation.md`; this suite proves the doors share the verdicts.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support;

use support::{at, text};

use beamcraft_host::{
    ids::IdSource,
    revision::{ExpectedBase, LayerEntry, NoFault, RevisionDraft, RevisionStore, Uuid7},
    storage::BlobStore,
};
use beamcraft_json::Json;
use std::fs;
use std::path::Path;

const PLATE_A: &str = "019ad0c0-0000-7000-8000-000000009101";
const PLATE_B: &str = "019ad0c0-0000-7000-8000-000000009102";
const MEMBER: &str = "019ad0c0-0000-7000-8000-000000009103";
const LEVEL: &str = "019ad0c0-0000-7000-8000-000000009104";
const ASSEMBLY: &str = "019ad0c0-0000-7000-8000-000000009105";
const NESTED: &str = "019ad0c0-0000-7000-8000-000000009106";
const WELD: &str = "019ad0c0-0000-7000-8000-000000009107";
const GRID: &str = "019ad0c0-0000-7000-8000-000000009108";
const MISSING: &str = "019ad0c0-0000-7000-8000-0000000091ff";
const USER: &str = "019ad0c0-0000-7000-8000-0000000091f0";
const GRID_AXES: &str = "019ad0c0-0000-7000-8000-000000009109,019ad0c0-0000-7000-8000-00000000910a";

struct Fixture {
    shared: support::StoreFixture,
    store: String,
    published: String,
    ws: String,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            shared: support::StoreFixture::new("p09-validation-tests"),
            store: String::new(),
            published: String::new(),
            ws: String::new(),
        };
        for dir in ["published", "ws"] {
            fs::create_dir(fixture.shared.root.join(dir)).unwrap();
        }
        let root = fixture.shared.root.clone();
        Fixture {
            store: root.join("store").to_str().unwrap().to_owned(),
            published: root.join("published").to_str().unwrap().to_owned(),
            ws: root.join("ws").to_str().unwrap().to_owned(),
            ..fixture
        }
    }

    fn store(&self) -> &str {
        &self.store
    }
    fn published(&self) -> &str {
        &self.published
    }
    fn ws(&self) -> &str {
        &self.ws
    }

    fn run(&self, command: &str, extra: &[&str]) -> (i32, Json) {
        self.shared.run(command, extra)
    }
    fn run_in(&self, store: &str, command: &str, extra: &[&str]) -> (i32, Json) {
        support::StoreFixture::run_in(store, command, extra)
    }

    /// Runs the binary with the exact arguments, for the workspace commands that name both the
    /// published store and the workspace directory.
    fn raw(&self, args: &[&str]) -> (i32, Json) {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(args)
            .output()
            .unwrap();
        (
            output.status.code().unwrap(),
            beamcraft_json::decode(&output.stdout, &beamcraft_json::Limits::DEFAULT).unwrap(),
        )
    }

    fn ok_raw(&self, args: &[&str]) -> Json {
        let (code, body) = self.raw(args);
        assert_eq!(code, 0, "{args:?}: {body:?}");
        body
    }

    fn ok(&self, command: &str, extra: &[&str]) -> Json {
        let (code, body) = self.run(command, extra);
        assert_eq!(code, 0, "{command} {extra:?}: {body:?}");
        body
    }

    /// Asserts the command is refused with `expected`, writing no revision.
    fn rejected(&self, command: &str, extra: &[&str]) -> String {
        let before = self.head(self.store());
        let (code, body) = self.run(command, extra);
        assert_eq!(code, 2, "{command} {extra:?}: {body:?}");
        assert_eq!(text(&body, "outcome"), "rejected", "{body:?}");
        assert_eq!(self.head(self.store()), before, "{command} {extra:?}");
        text(&body, "code")
    }

    fn head(&self, store: &str) -> String {
        let (code, body) = self.run_in(store, "open-revision", &[]);
        assert_eq!(code, 0, "{body:?}");
        text(&body, "revision_id")
    }

    fn plate(&self, id: &str, name: &str) -> String {
        self.plate_in(self.store(), id, name)
    }

    fn plate_in(&self, store: &str, id: &str, name: &str) -> String {
        let (code, body) = self.run_in(
            store,
            "create-plate",
            &[
                "--width",
                "400",
                "--length",
                "300",
                "--thickness",
                "10",
                "--name",
                name,
                "--new-id",
                id,
            ],
        );
        assert_eq!(code, 0, "{body:?}");
        text(at(&body, &["plate"]), "element_id").to_owned()
    }

    /// Forks the published store into the personal workspace.
    fn fork_workspace(&self) {
        let (code, body) = self.raw(&[
            "create-workspace",
            "--store",
            self.published(),
            "--workspace-dir",
            self.ws(),
            "--user-id",
            USER,
        ]);
        assert_eq!(code, 0, "{body:?}");
    }

    /// Seeds the published store with one plate, forks a workspace and creates a second plate
    /// in the workspace draft: the minimal merged set for the post-merge door cases.
    fn workspace_with_two_plates(&self) {
        self.plate_in(self.published(), PLATE_A, "Seed");
        self.fork_workspace();
        self.plate_in(self.ws(), PLATE_B, "Work");
    }

    /// Creates a workspace assembly and attaches the workspace's second plate to it.
    fn attach_work_plate(&self) {
        let (code, body) = self.run_in(
            self.ws(),
            "create-assembly",
            &["--name", "Assembly 1", "--new-id", ASSEMBLY],
        );
        assert_eq!(code, 0, "{body:?}");
        let (code, body) = self.run_in(
            self.ws(),
            "attach-to-assembly",
            &["--id", PLATE_B, "--assembly-id", ASSEMBLY],
        );
        assert_eq!(code, 0, "{body:?}");
    }

    /// Publishes the workspace and asserts the refusal code, with the published head unmoved.
    fn publish_refused(&self, expected: &str) -> String {
        let before = self.head(self.published());
        let (code, body) = self.raw(&[
            "publish-workspace",
            "--store",
            self.published(),
            "--workspace-dir",
            self.ws(),
        ]);
        assert_eq!(code, 2, "{body:?}");
        assert_eq!(self.head(self.published()), before);
        assert_eq!(text(&body, "code"), expected, "{body:?}");
        text(&body, "message").to_owned()
    }

    /// Commits a head-based revision with edited layer text and no content verification into
    /// `store`, the way bytes edited outside BeamCraft arrive; the post-merge door must then
    /// refuse the violation when the bytes are merged.
    fn tamper(&self, store: &str, edit: impl Fn(&str, String) -> String) {
        let root = RevisionStore::new(BlobStore::new(Path::new(store)).unwrap());
        let head = root.open().unwrap();
        let content = head.manifest().content();
        let mut ids = IdSource::system();
        let mut entries = Vec::new();
        for entry in content.entries() {
            let original = String::from_utf8(head.layer_bytes(entry.name()).unwrap()).unwrap();
            let edited = edit(entry.name(), original);
            root.blobs().publish(edited.as_bytes()).unwrap();
            entries.push(
                LayerEntry::from_bytes(entry.name(), entry.role(), edited.as_bytes()).unwrap(),
            );
        }
        let draft = RevisionDraft {
            project: content.project().clone(),
            workspace: content.workspace().clone(),
            revision: Uuid7::parse(&ids.next_id().unwrap().to_string()).unwrap(),
            op: Uuid7::parse(&ids.next_id().unwrap().to_string()).unwrap(),
            workspace_generation: content.workspace_generation() + 1,
            schema: content.schema(),
            root_layer: content.root_layer().to_owned(),
            entries,
        };
        root.commit_with(
            &draft,
            &ExpectedBase::Revision(head.info().revision().clone()),
            None,
            &mut NoFault,
        )
        .unwrap();
    }

    /// Duplicates one element's prim under a second path while keeping its identity: a
    /// duplicate-identity shape that only the merged-byte door can see.
    fn duplicate_prim(layer: &str, text: String, leaf: &str, second_leaf: &str) -> String {
        if !layer.contains("authored") || !text.contains(leaf) {
            return text;
        }
        let marker = format!("def Xform \"PRT_{leaf}\"");
        let start = text.find(&marker).unwrap();
        // The block ends at the prim's own closing brace, which sits at the def line's
        // indentation; the next def of a parent scope would steal the scope's braces too.
        let line_start = text[..start].rfind('\n').map_or(0, |pos| pos + 1);
        let indent = &text[line_start..start];
        let close = format!("\n{indent}}}");
        let end = text[start..]
            .find(&close)
            .map_or(text.len(), |offset| start + offset + close.len() + 1);
        let copy = text[start..end].replace(&format!("PRT_{leaf}"), &format!("PRT_{second_leaf}"));
        // The copy keeps the original `bim:id`, so two paths claim one identity.
        format!("{}{}\n{}", &text[..end], copy, &text[end..])
    }
}

/// The one valid model every rule family accepts: plates, a member, a level with an assigned
/// plate, an assembly with a part, a weld connecting two distinct parts, a grid and a user
/// attribute. Every commit passes the client door and the post-merge door.
fn seeded() -> Fixture {
    let f = Fixture::new();
    f.plate(PLATE_A, "Gusset A");
    f.plate(PLATE_B, "Gusset B");
    let profile = text(
        at(
            &f.ok("define-profile", &["--width", "100", "--height", "50"]),
            &["definition"],
        ),
        "digest",
    );
    f.ok(
        "create-member",
        &[
            "--start",
            "0,0,0",
            "--end",
            "3000,0,0",
            "--profile-digest",
            &profile,
            "--name",
            "Beam",
            "--new-id",
            MEMBER,
        ],
    );
    f.ok(
        "create-level",
        &["--name", "Storey 1", "--elevation", "0", "--new-id", LEVEL],
    );
    f.ok("update-element", &["--id", PLATE_A, "--level", LEVEL]);
    f.ok(
        "create-assembly",
        &["--name", "Assembly 1", "--new-id", ASSEMBLY],
    );
    f.ok(
        "attach-to-assembly",
        &["--id", PLATE_B, "--assembly-id", ASSEMBLY],
    );
    f.ok(
        "create-weld",
        &[
            "--name",
            "W1",
            "--new-id",
            WELD,
            "--main",
            PLATE_A,
            "--secondary",
            PLATE_B,
            "--size",
            "6",
            "--location",
            "workshop",
            "--side",
            "both",
        ],
    );
    f.ok(
        "create-grid",
        &[
            "--name",
            "Main",
            "--new-id",
            GRID,
            "--u-expression",
            "0 6000",
            "--u-axis-ids",
            GRID_AXES,
        ],
    );
    f.ok("attribute-define", &["--name", "Note", "--type", "text"]);
    f.ok(
        "attribute-set",
        &["--element", PLATE_A, "--name", "Note", "--value", "checked"],
    );
    f
}

#[test]
fn positive_model_passes_both_doors() {
    let f = seeded();
    // The client door accepted every family; the head moved on every commit.
    let head = f.head(f.store());
    let welds = f.ok("welds", &[]);
    assert_eq!(support::list(&welds, "welds").len(), 1);
    let levels = f.ok("levels", &[]);
    assert_eq!(support::list(&levels, "levels").len(), 1);
    let assemblies = f.ok("assemblies", &[]);
    assert_eq!(support::list(&assemblies, "assemblies").len(), 1);
    assert_eq!(head, f.head(f.store()));

    // The post-merge door accepts the same model on the publication path: work in a workspace
    // fork and publish it, which verifies the complete merged layer set before the head moves.
    let seed = Fixture::new();
    seed.workspace_with_two_plates();
    seed.ok_raw(&[
        "publish-workspace",
        "--store",
        seed.published(),
        "--workspace-dir",
        seed.ws(),
    ]);
    assert_eq!(seed.head(seed.published()), seed.head(seed.ws()));
}

#[test]
fn duplicates_are_refused_by_both_doors() {
    let f = seeded();
    // Client door: creating an identity that already exists is DUPLICATE_ID (DV-BASE-002),
    // and the refusal writes nothing.
    assert_eq!(
        f.rejected(
            "create-plate",
            &[
                "--width",
                "400",
                "--length",
                "300",
                "--thickness",
                "10",
                "--name",
                "Again",
                "--new-id",
                PLATE_A
            ]
        ),
        "DUPLICATE_ID"
    );

    // Post-merge door: the same identity carried by a second prim reaches the merged bytes and
    // is refused there before the published head moves.
    let f = Fixture::new();
    f.workspace_with_two_plates();
    let leaf = PLATE_B.replace('-', "");
    let second = format!("{leaf}02");
    f.tamper(f.ws(), |layer, text| {
        Fixture::duplicate_prim(layer, text, &leaf, &second)
    });
    let before = f.head(f.published());
    let (code, body) = f.raw(&[
        "publish-workspace",
        "--store",
        f.published(),
        "--workspace-dir",
        f.ws(),
    ]);
    assert_eq!(code, 2, "{body:?}");
    assert_eq!(text(&body, "code"), "INVALID_ID", "{body:?}");
    assert_eq!(f.head(f.published()), before);
}

#[test]
fn dangling_links_are_refused_by_both_doors() {
    let f = seeded();
    // Client door: deleting a linked part would leave a dangling weld or level record.
    assert_eq!(
        f.rejected("delete-element", &["--id", PLATE_A]),
        "DEPENDENCY_CONFLICT"
    );
    assert_eq!(
        f.rejected("delete-element", &["--id", LEVEL]),
        "DEPENDENCY_CONFLICT"
    );
    // A part-of record towards an absent assembly is refused before anything is written.
    let before = f.head(f.store());
    let (code, body) = f.run(
        "attach-to-assembly",
        &["--id", PLATE_B, "--assembly-id", MISSING],
    );
    assert_eq!(code, 2, "{body:?}");
    assert_eq!(f.head(f.store()), before);

    // Post-merge door: a part-of record whose target is absent reaches the merged bytes and is
    // refused there with the dangling rule's code before the published head moves.
    let f = Fixture::new();
    f.workspace_with_two_plates();
    f.attach_work_plate();
    let absent = format!("/World/Steel/Assemblies/ASM_{}", MISSING.replace('-', ""));
    let present = format!("/World/Steel/Assemblies/ASM_{}", ASSEMBLY.replace('-', ""));
    f.tamper(f.ws(), |_layer, text| text.replace(&present, &absent));
    let message = f.publish_refused("DANGLING_RELATIONSHIP");
    assert_eq!(message, "part-of target unavailable");
}

#[test]
fn module_rules_are_refused_by_the_client_door() {
    let f = seeded();
    // A weld connects two distinct parts (DV-REL-007); the same part twice is refused.
    assert_eq!(
        f.rejected(
            "create-weld",
            &[
                "--name",
                "W2",
                "--new-id",
                NESTED,
                "--main",
                PLATE_A,
                "--secondary",
                PLATE_A,
                "--size",
                "6",
                "--location",
                "workshop",
                "--side",
                "both"
            ]
        ),
        "INVALID_ARGUMENT"
    );
    // Only a plate, beam or column is welded (DV-REL-003); an assembly is refused.
    assert_eq!(
        f.rejected(
            "create-weld",
            &[
                "--name",
                "W2",
                "--new-id",
                NESTED,
                "--main",
                ASSEMBLY,
                "--secondary",
                PLATE_A,
                "--size",
                "6",
                "--location",
                "workshop",
                "--side",
                "both"
            ]
        ),
        "UNSUPPORTED_GEOMETRY"
    );
    // Only a plate, beam or column is assigned to a level (DV-REL-003); an assembly is refused.
    assert_eq!(
        f.rejected("update-element", &["--id", ASSEMBLY, "--level", LEVEL]),
        "UNSUPPORTED_PROPERTY"
    );
    // Part-of records form no cycle of any length (DV-REL-005): nesting A1 under A3 would make
    // A1 -> A3 -> A1.
    f.ok(
        "create-assembly",
        &["--name", "Assembly 3", "--new-id", NESTED],
    );
    f.ok(
        "attach-to-assembly",
        &["--id", ASSEMBLY, "--assembly-id", NESTED],
    );
    assert_eq!(
        f.rejected(
            "attach-to-assembly",
            &["--id", NESTED, "--assembly-id", ASSEMBLY]
        ),
        "DEPENDENCY_CONFLICT"
    );
}

#[test]
fn violations_are_deterministic_and_bounded() {
    // Determinism: the same dangling record with its element prim moved to the end of the layer
    // (a different composition order) is refused with the same code and message.
    let refused = |permuted: bool| {
        let f = Fixture::new();
        f.workspace_with_two_plates();
        f.attach_work_plate();
        let absent = format!("/World/Steel/Assemblies/ASM_{}", MISSING.replace('-', ""));
        let present = format!("/World/Steel/Assemblies/ASM_{}", ASSEMBLY.replace('-', ""));
        f.tamper(f.ws(), |_layer, text| {
            let replaced = text.replace(&present, &absent);
            if !permuted {
                return replaced;
            }
            // Move every element prim block after the first one to the front, reversing the
            // composed order without changing any record.
            let mut parts: Vec<&str> = replaced.split("\ndef ").collect();
            if parts.len() > 2 {
                let rest: Vec<&str> = parts[1..].iter().rev().copied().collect();
                parts = vec![parts[0]];
                for part in rest {
                    parts.push(part);
                }
            }
            parts.join("\ndef ")
        });
        let message = f.publish_refused("DANGLING_RELATIONSHIP");
        (message,)
    };
    let (first_message,) = refused(false);
    let (second_message,) = refused(true);
    assert_eq!(first_message, second_message);

    // Bounded: a label beyond the 256-byte bound is refused by the admission bounds, and the
    // refusal writes nothing.
    let f = seeded();
    let long_name = "a".repeat(300);
    assert_eq!(
        f.rejected(
            "create-plate",
            &[
                "--width",
                "400",
                "--length",
                "300",
                "--thickness",
                "10",
                "--name",
                &long_name
            ]
        ),
        "INVALID_ARGUMENT"
    );
}
