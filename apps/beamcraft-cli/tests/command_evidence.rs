// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic commands and numeric inputs, MPL-2.0.

//! Persist actual command reads/writes, including invisible dependencies, through the real CLI.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use beamcraft_cli::command_evidence::{CapturedFootprint, read};
use beamcraft_engine::footprint::{Entry, Footprint, Group, Target};
use beamcraft_host::revision::{
    EvidencePin, ExpectedBase, NoFault, RevisionDraft, RevisionStore, Uuid7,
};
use beamcraft_host::storage::BlobStore;
use beamcraft_json::{Json, Limits};
use beamcraft_model::command::Placement;
use beamcraft_model::digest::Sha256;
use beamcraft_model::id::{ElementId, OpId};
use beamcraft_model::limits::MAX_FOOTPRINT_ENTRIES;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use steel::model::command::CreatePlate;
use steel::model::command::{CopyElement, ElementPatch, UpdateElement};

static NEXT: AtomicU64 = AtomicU64::new(0);
/// The shipped synthetic fastener library (contributor-original values).
const SYNTHETIC: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/libraries/synthetic-fasteners.usda"
);
fn element(n: usize) -> ElementId {
    ElementId::parse(&format!("019a7bf0-0000-7000-8000-{n:012x}")).unwrap()
}
fn capture(footprint: Footprint) -> CapturedFootprint {
    CapturedFootprint {
        op_id: OpId::parse(&element(100).to_string()).unwrap(),
        fingerprint: Sha256::digest(b"synthetic request").unwrap(),
        footprint,
    }
}
fn text<'a>(json: &'a Json, key: &str) -> &'a str {
    json.get(key).unwrap().string().unwrap()
}

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "command-evidence-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("store")).unwrap();
        Self(root)
    }
    fn store(&self) -> RevisionStore {
        RevisionStore::new(BlobStore::new(self.0.join("store")).unwrap())
    }
    fn ok(&self, command: &str, args: &[&str]) -> Json {
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args([command, "--store", self.0.join("store").to_str().unwrap()])
            .args(args)
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(0),
            "{command} {args:?}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap()
    }
    fn recorded(&self, command: &str, args: &[&str]) -> (Json, CapturedFootprint) {
        let before = self.store().head().unwrap().map(|h| h.manifest_digest());
        let result = self.ok(command, args);
        let revision = self.store().open().unwrap();
        assert_ne!(
            Some(revision.info().manifest_digest()),
            before,
            "{command} did not commit"
        );
        let captured = read(&revision)
            .unwrap()
            .unwrap_or_else(|| panic_no_evidence(command));
        assert_eq!(
            captured.op_id.to_string(),
            revision.manifest().content().op().as_str()
        );
        assert!(!captured.footprint.reads().is_empty() || !captured.footprint.writes().is_empty());
        (result, captured)
    }
    fn plate(&self, id: ElementId) -> CapturedFootprint {
        self.recorded(
            "create-plate",
            &[
                "--width",
                "1000",
                "--length",
                "500",
                "--thickness",
                "10",
                "--new-id",
                &id.to_string(),
                "--name",
                "Plate",
            ],
        )
        .1
    }
}
#[allow(clippy::panic)]
fn panic_no_evidence(command: &str) -> CapturedFootprint {
    panic!("{command} committed without evidence");
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn canonical_round_trip_preserves_both_target_types_and_every_atomic_group() {
    let digest = Sha256::digest(b"synthetic definition").unwrap();
    let entries: Vec<_> = [
        Group::Label,
        Group::ShapeInputs,
        Group::Placement,
        Group::MaterialRef,
        Group::Relationship,
        Group::Mark,
        Group::Lifecycle,
        Group::Derived,
    ]
    .into_iter()
    .flat_map(|group| {
        [Target::Element(element(1)), Target::Definition(digest)]
            .map(|target| Entry { target, group })
    })
    .collect();
    let captured = capture(Footprint::from_entries(&entries, &entries).unwrap());
    let bytes = captured.encode().unwrap();
    assert_eq!(CapturedFootprint::decode(&bytes).unwrap(), captured);
    assert_eq!(
        CapturedFootprint::decode(&bytes).unwrap().encode().unwrap(),
        bytes
    );
}

#[test]
fn hostile_or_noncanonical_proof_is_refused_without_an_empty_success() {
    let bytes =
        capture(Footprint::for_update(element(1), true, false, false, false, None).unwrap())
            .encode()
            .unwrap();
    let good = String::from_utf8(bytes.clone()).unwrap();
    for length in 0..bytes.len() {
        assert!(CapturedFootprint::decode(&bytes[..length]).is_err());
    }
    for bad in [
        good.replacen("footprint 1", "footprint 2", 1),
        good.replace(" label", " unknown_group"),
        good.replace("element ", "foreign "),
        good.replacen("reads 2", "reads 02", 1),
        good.replacen("reads 2", "reads 4097", 1),
        good.replacen(
            &format!("element {} lifecycle", element(1)),
            &format!("element {} label", element(1)),
            1,
        ),
        good.replace(
            &format!(
                "reads 2\nelement {} label\nelement {} lifecycle\n",
                element(1),
                element(1)
            ),
            &format!(
                "reads 2\nelement {} lifecycle\nelement {} label\n",
                element(1),
                element(1)
            ),
        ),
        good.replacen("reads 2", "reads 9999999999999999999999999", 1),
        format!("{good}extra\n"),
        good.replace(&element(1).to_string(), "short-id"),
    ] {
        assert!(CapturedFootprint::decode(bad.as_bytes()).is_err(), "{bad}");
    }
    assert!(CapturedFootprint::decode(&[0xff; 64]).is_err());
    assert!(capture(Footprint::default()).encode().is_err());
}

#[test]
fn aggregate_entry_caps_are_checked_before_reconstruction_and_allocation() {
    let mut entries: Vec<_> = (0..MAX_FOOTPRINT_ENTRIES)
        .map(|n| Entry {
            target: Target::Element(element(n)),
            group: Group::Lifecycle,
        })
        .collect();
    let captured = capture(Footprint::from_entries(&entries, &[]).unwrap());
    assert_eq!(
        CapturedFootprint::decode(&captured.encode().unwrap()).unwrap(),
        captured
    );
    entries.push(Entry {
        target: Target::Element(element(MAX_FOOTPRINT_ENTRIES)),
        group: Group::Lifecycle,
    });
    assert!(Footprint::from_entries(&entries, &[]).is_err());
}

#[test]
fn the_real_cli_persists_exact_reads_writes_and_request_fingerprints() {
    let f = Fixture::new();
    let created = f.plate(element(1));
    let command = CreatePlate {
        new_id: element(1),
        name: "Plate".into(),
        class: 0,
        contour_points_mm: vec![
            [0., 0., 0.],
            [1000., 0., 0.],
            [1000., 500., 0.],
            [0., 500., 0.],
        ],
        thickness_mm: 10.,
        thickness_side: None,
        corners: Vec::new(),
        placement: Placement::IDENTITY,
        material_digest: None,
        holes_mm: vec![],
    };
    assert_eq!(created.fingerprint, command.content_fingerprint().unwrap());
    assert_eq!(
        created.footprint,
        Footprint::for_create(element(1), None).unwrap()
    );
    let copied = f
        .recorded(
            "copy-element",
            &[
                "--id",
                &element(1).to_string(),
                "--new-id",
                &element(2).to_string(),
                "--name",
                "Copy",
                "--offset",
                "100,0,0",
            ],
        )
        .1;
    assert_eq!(
        copied.footprint,
        Footprint::for_copy(element(1), element(2), None, None).unwrap()
    );
    assert_eq!(
        copied.fingerprint,
        CopyElement {
            source_id: element(1),
            new_id: element(2),
            name: Some("Copy".into()),
            offset_mm: [100., 0., 0.]
        }
        .content_fingerprint()
        .unwrap()
    );
    let (_, renamed) = f.recorded(
        "update-element",
        &["--id", &element(1).to_string(), "--name", "Renamed"],
    );
    assert_eq!(
        renamed.footprint,
        Footprint::for_update(element(1), true, false, false, false, None).unwrap()
    );
    assert_eq!(
        renamed.fingerprint,
        UpdateElement {
            target_id: element(1),
            patch: ElementPatch {
                name: Some("Renamed".into()),
                ..ElementPatch::default()
            }
        }
        .content_fingerprint()
        .unwrap()
    );
    assert_eq!(
        renamed.footprint.writes().len(),
        1,
        "a label command must not acquire whole-model reads or writes"
    );
    let deleted = f
        .recorded("delete-element", &["--id", &element(2).to_string()])
        .1;
    assert_eq!(
        deleted.footprint,
        Footprint::for_delete(element(2)).unwrap()
    );
    let old = f.store().open_at(2).unwrap();
    assert_eq!(
        read(&old).unwrap().unwrap(),
        copied,
        "later edits cannot change earlier dependency evidence"
    );
}

#[test]
fn every_modeling_publication_path_records_its_own_operation() {
    let f = Fixture::new();
    let (profile, _) = f.recorded("define-profile", &["--width", "100", "--height", "50"]);
    let digest = text(profile.get("definition").unwrap(), "digest");
    f.recorded(
        "define-material",
        &["--grade", "Synthetic", "--density", "7850"],
    );
    f.plate(element(1));
    f.plate(element(2));
    let (_, member) = f.recorded(
        "create-member",
        &[
            "--start",
            "0,0,0",
            "--end",
            "1000,0,0",
            "--profile-digest",
            digest,
            "--new-id",
            &element(3).to_string(),
        ],
    );
    assert_eq!(
        member.footprint,
        Footprint::for_create_member(element(3), digest.parse().unwrap(), None).unwrap()
    );
    f.recorded(
        "create-level",
        &[
            "--name",
            "Level",
            "--elevation",
            "1000",
            "--new-id",
            &element(4).to_string(),
        ],
    );
    f.recorded(
        "create-assembly",
        &["--name", "Assembly", "--new-id", &element(5).to_string()],
    );
    f.recorded(
        "attach-to-assembly",
        &[
            "--id",
            &element(1).to_string(),
            "--assembly-id",
            &element(5).to_string(),
        ],
    );
    f.recorded("detach-from-assembly", &["--id", &element(1).to_string()]);
    f.recorded(
        "create-weld",
        &[
            "--name",
            "Weld",
            "--main",
            &element(1).to_string(),
            "--secondary",
            &element(2).to_string(),
            "--size",
            "6",
            "--location",
            "workshop",
            "--side",
            "both",
        ],
    );
    f.recorded(
        "create-bolt-group",
        &[
            "--name",
            "Bolts",
            "--parts",
            &format!("{},{}", element(1), element(2)),
            "--fastener-library",
            SYNTHETIC,
            "--designation",
            "SYN-BOLT-12x50",
            "--point1",
            "100,100,5",
            "--point2",
            "180,100,5",
            "--plane-normal",
            "0,0,1",
            "--offset-start",
            "0",
            "--spacings",
            "80",
            "--hole-mode",
            "none",
        ],
    );
    let preview = f.ok("preview-numbering", &[]);
    f.recorded(
        "approve-numbering",
        &["--preview-token", text(&preview, "preview_token")],
    );
    let source = f.0.join("foreign.usda");
    fs::write(&source, "#usda 1.0\n(\n defaultPrim = \"Site\"\n metersPerUnit = 1.0\n upAxis = \"Y\"\n)\ndef Xform \"Site\" {}\n").unwrap();
    f.recorded(
        "attach-reference",
        &[
            "--source",
            source.to_str().unwrap(),
            "--new-id",
            &element(6).to_string(),
        ],
    );
    f.recorded(
        "move-reference",
        &["--id", &element(6).to_string(), "--translation", "100,0,0"],
    );
    f.recorded(
        "correct-reference-units",
        &[
            "--id",
            &element(6).to_string(),
            "--meters-per-unit",
            "1",
            "--up-axis",
            "Y",
        ],
    );
}

#[test]
fn legacy_absence_and_wrong_operation_binding_are_distinct() {
    let f = Fixture::new();
    let captured = f.plate(element(1));
    let head = f.store().open().unwrap();
    let content = head.manifest().content();
    let draft = RevisionDraft {
        project: content.project().clone(),
        workspace: content.workspace().clone(),
        revision: Uuid7::parse(&element(90).to_string()).unwrap(),
        op: Uuid7::parse(&element(91).to_string()).unwrap(),
        workspace_generation: content.workspace_generation() + 1,
        schema: content.schema(),
        root_layer: content.root_layer().into(),
        entries: content.entries().to_vec(),
    };
    let bytes = captured.encode().unwrap();
    let (digest, _) = f.store().blobs().publish(&bytes).unwrap();
    f.store()
        .commit_with_evidence(
            &draft,
            &ExpectedBase::Revision(head.info().revision().clone()),
            Some(EvidencePin::new(digest, bytes.len() as u64).unwrap()),
            None,
            &mut NoFault,
        )
        .unwrap();
    assert_eq!(
        read(&f.store().open().unwrap()).unwrap_err().code,
        beamcraft_model::ErrorCode::IntegrityError
    );
    let legacy = RevisionDraft {
        revision: Uuid7::parse(&element(92).to_string()).unwrap(),
        op: Uuid7::parse(&element(93).to_string()).unwrap(),
        workspace_generation: draft.workspace_generation + 1,
        ..draft
    };
    f.store()
        .commit(
            &legacy,
            &ExpectedBase::Revision(Uuid7::parse(&element(90).to_string()).unwrap()),
        )
        .unwrap();
    assert_eq!(read(&f.store().open().unwrap()).unwrap(), None);
}
