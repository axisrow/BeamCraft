// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic batch fixtures, MPL-2.0; existing attribution applies.

//! C05: bounded real storage/rebase/publication with 1000 new IDs per offline client.
//! Test-only batches admit each typed create and union actual footprints. This is not a bulk CLI.
//! The final independent reader runs as a separate binary process.

use super::*;
use crate::plate::{self, PublishRequest};
use beamcraft_engine::{
    command::create_plate, footprint::Footprint, query::committed::PlateRecord,
};
use beamcraft_host::revision::{CommitPoint, Injected, NoFault, RevisionInfo};
use beamcraft_layer_engine::authored::{
    Author, Change, EditTargets, Element, Placement as Pose, Prepare, Shape, schema_identity,
};
use beamcraft_model::{
    command::{CreatePlate, Placement},
    digest::Sha256,
    id::{ElementId, OpId, ProjectId, RevisionId, WorkspaceId},
};
use std::{collections::BTreeSet, fs, process::Command};

fn store(path: &Path) -> Result<RevisionStore, beamcraft_host::storage::StorageError> {
    Ok(RevisionStore::new(beamcraft_host::storage::BlobStore::new(
        path,
    )?))
}

const BATCH: usize = 125;
const ROUNDS: usize = 8;
fn id(n: usize) -> Uuid7 {
    Uuid7::parse(&format!("019a7bf0-0000-7000-8000-{n:012x}")).unwrap()
}
fn element(n: usize) -> ElementId {
    ElementId::parse(id(n).as_str()).unwrap()
}
fn command(n: usize) -> CreatePlate {
    CreatePlate {
        new_id: element(n),
        name: format!("original part {n}"),
        contour_points_mm: vec![
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [10.0, 20.0, 0.0],
            [0.0, 20.0, 0.0],
        ],
        thickness_mm: 3.0,
        placement: Placement {
            translation_mm: [n as f64 * 20.0, 0.0, 0.0],
            ..Placement::IDENTITY
        },
        material_digest: None,
        holes_mm: Vec::new(),
        cut: None,
    }
}

struct Fixture {
    root: PathBuf,
    published: RevisionStore,
    a: RevisionStore,
    b: RevisionStore,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::var_os("BEAMCRAFT_C05_OUTPUT")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::temp_dir().join(format!("beamcraft-c05-{}", std::process::id()))
            });
        // Never overwrite an existing fixture or user path.
        fs::create_dir(&root).unwrap();
        for name in ["published", "a", "b", "layers"] {
            fs::create_dir(root.join(name)).unwrap();
        }
        let f = Self {
            published: store(&root.join("published")).unwrap(),
            a: store(&root.join("a")).unwrap(),
            b: store(&root.join("b")).unwrap(),
            root,
        };
        plate::create_plate(
            &f.published,
            &plate::CreateRequest {
                op_id: OpId::parse(id(90).as_str()).unwrap(),
                revision_id: RevisionId::parse(id(91).as_str()).unwrap(),
                actor: id(3),
                project: ProjectId::parse(id(1).as_str()).unwrap(),
                workspace: WorkspaceId::parse(id(4).as_str()).unwrap(),
                command: command(7),
            },
            &mut NoFault,
        )
        .unwrap();
        for (name, draft, n) in [("a", &f.a, 10), ("b", &f.b, 11)] {
            create_workspace(
                &f.published,
                draft,
                &f.root.join(name),
                &CreateWorkspace {
                    workspace_id: id(n),
                    user_id: id(3),
                    revision_id: id(n + 100),
                    op_id: id(n + 200),
                },
                &mut NoFault,
            )
            .unwrap();
        }
        f
    }
    fn batch(&self, draft: &RevisionStore, start: usize, op: usize) {
        let (_, base) = plate::load_base(
            draft,
            &ProjectId::parse(id(1).as_str()).unwrap(),
            &WorkspaceId::parse(id(4).as_str()).unwrap(),
        )
        .unwrap();
        let control = plate::control();
        let schema = schema_identity().unwrap();
        let mut existing: BTreeSet<_> = plate::inspect(draft)
            .unwrap()
            .plates
            .iter()
            .map(|p| p.authored.id)
            .collect();
        let (mut author, opened) = Author::open(
            plate::open_base(&base).unwrap(),
            EditTargets {
                authored: plate::AUTHORED_LAYER.into(),
                derived: plate::DERIVED_LAYER.into(),
            },
            schema,
            &control,
        )
        .unwrap();
        let mut footprint = Footprint::default();
        let mut changes = Vec::new();
        let mut fingerprints = Vec::new();
        for n in start..start + BATCH {
            let typed = command(n);
            let result = create_plate(
                &typed,
                &|id| existing.contains(id),
                &beamcraft_engine::command::no_materials,
            )
            .unwrap();
            assert!(existing.insert(result.authored.id));
            footprint = footprint.union(result.footprint).unwrap();
            fingerprints.extend_from_slice(result.fingerprint.to_string().as_bytes());
            let record = PlateRecord {
                authored: result.authored,
                body: result.body,
                quantities: result.quantities,
            };
            changes.push(Change::Create(Element {
                id: record.authored.id.to_string(),
                name: record.authored.name.clone(),
                shape: Shape::Plate {
                    contour_mm: typed.contour_points_mm,
                    thickness_mm: typed.thickness_mm,
                    holes: Vec::new(),
                    cut: None,
                },
                placement: Pose {
                    translation_mm: typed.placement.translation_mm,
                    orientation_xyzw: typed.placement.orientation_xyzw,
                },
                material: None,
            }));
            changes.push(
                plate::mesh_change(
                    &record.authored.id,
                    &record.body.solid,
                    &plate::plate_record(&record),
                )
                .unwrap(),
            );
        }
        assert_eq!(
            footprint.reads().len() + footprint.writes().len(),
            BATCH * 8
        );
        let prepared = author
            .prepare(
                Prepare {
                    context: opened.context,
                    op_id: id(op).to_string(),
                    actor: id(3).to_string(),
                    schema,
                    resources: Vec::new(),
                    changes,
                },
                &control,
            )
            .unwrap();
        plate::check_footprint(&prepared, &footprint).unwrap();
        plate::publish_revision(
            draft,
            &PublishRequest {
                footprint: &footprint,
                fingerprint: Sha256::digest(&fingerprints).unwrap(),
                prepared: &prepared,
                schema,
                project: &base.project,
                workspace: &base.workspace,
                revision_id: &RevisionId::parse(id(op + 1).as_str()).unwrap(),
                op: id(op),
                base_generation: base.generation,
                expected: &base.expected,
            },
            &mut NoFault,
        )
        .unwrap();
    }
    fn rebase(&self, name: &str, draft: &RevisionStore, op: usize, interrupt: bool) {
        let original = draft.head().unwrap().unwrap();
        let accepted = self.published.head().unwrap().unwrap();
        let request = rebase::RebaseRequest {
            op_id: OpId::parse(id(op).as_str()).unwrap(),
            actor: id(3),
            expected_head: accepted.revision().clone(),
            expected_draft: original.revision().clone(),
        };
        let dir = self.root.join(name);
        let mut reached = false;
        if interrupt {
            let attempted =
                rebase::rebase_workspace(&self.published, draft, &dir, &request, &mut |point| {
                    if point == CommitPoint::Switched {
                        reached = true;
                        Err(Injected)
                    } else {
                        Ok(())
                    }
                });
            assert!(reached);
            assert_eq!(attempted.err().unwrap().code, ErrorCode::OutcomeUnknown);
        }
        let result =
            rebase::rebase_workspace(&self.published, draft, &dir, &request, &mut NoFault).unwrap();
        let rebase::RebaseResult::Rebased(result) = result else {
            panic!("independent IDs conflicted")
        };
        assert_eq!(result.replayed, interrupt);
        let candidate = draft.head().unwrap().unwrap();
        assert_eq!(candidate.sequence(), original.sequence() + 1);
        let replay =
            rebase::rebase_workspace(&self.published, draft, &dir, &request, &mut NoFault).unwrap();
        let rebase::RebaseResult::Rebased(replay) = replay else {
            panic!("retry conflicted")
        };
        assert!(replay.replayed);
        assert_eq!(replay.revision.revision_id, result.revision.revision_id);
        assert_eq!(
            draft.head().unwrap().unwrap().manifest_digest(),
            candidate.manifest_digest()
        );
        assert_eq!(
            self.published.head().unwrap().unwrap().manifest_digest(),
            accepted.manifest_digest()
        );
    }
    fn publish(&self, name: &str, draft: &RevisionStore) -> RevisionInfo {
        let dir = self.root.join(name);
        let first = publish_workspace(&self.published, draft, &dir, &mut NoFault).unwrap();
        assert!(!first.replayed);
        let accepted = self.published.head().unwrap().unwrap();
        // Discard the first response: retry must return the same result without a new revision.
        let again = publish_workspace(&self.published, draft, &dir, &mut NoFault).unwrap();
        assert!(again.replayed);
        assert_eq!(again.revision.revision_id, first.revision.revision_id);
        assert_eq!(
            self.published.head().unwrap().unwrap().manifest_digest(),
            accepted.manifest_digest()
        );
        accepted
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if std::env::var_os("BEAMCRAFT_C05_OUTPUT").is_none() {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

#[test]
#[ignore = "large C05 model: run explicitly with --release in CI"]
fn two_offline_clients_retain_one_thousand_new_ids_each_through_retry_and_recovery() {
    let f = Fixture::new();
    for round in 0..ROUNDS {
        let metadata_b = fs::read(f.root.join("b/workspace.json")).unwrap();
        f.batch(&f.a, 1000 + round * BATCH, 100_000 + round * 100);
        f.batch(&f.b, 2000 + round * BATCH, 200_000 + round * 100);
        // Both disconnected drafts exist before either publication in every round.
        if round != 0 {
            f.rebase("a", &f.a, 300_000 + round * 100, false);
        }
        let accepted_a = f.publish("a", &f.a);
        assert_eq!(
            fs::read(f.root.join("b/workspace.json")).unwrap(),
            metadata_b
        );
        let before_b = f.b.head().unwrap().unwrap();
        assert_eq!(
            publish_workspace(&f.published, &f.b, &f.root.join("b"), &mut NoFault)
                .unwrap_err()
                .code,
            ErrorCode::BaseRevisionMismatch
        );
        assert_eq!(
            f.b.head().unwrap().unwrap().manifest_digest(),
            before_b.manifest_digest()
        );
        f.rebase("b", &f.b, 400_000 + round * 100, round == 3);
        assert_eq!(
            f.published.head().unwrap().unwrap().manifest_digest(),
            accepted_a.manifest_digest()
        );
        f.publish("b", &f.b);
        let report = plate::inspect(&f.published).unwrap();
        assert_eq!(report.plates.len(), 1 + 2 * BATCH * (round + 1));
        println!(
            "C05 round {}/{}: {} accepted parts",
            round + 1,
            ROUNDS,
            report.plates.len()
        );
    }
    let expected_ids: BTreeSet<_> = std::iter::once(7).chain(1000..3000).map(element).collect();
    let report = plate::inspect(&f.published).unwrap();
    assert_eq!(
        report
            .plates
            .iter()
            .map(|p| p.authored.id)
            .collect::<BTreeSet<_>>(),
        expected_ids
    );
    for part in &report.plates {
        assert_eq!(part.quantities.volume_mm3, 600.0);
        assert_eq!(part.quantities.mass_kg, None);
        assert_eq!(
            part.authored.placement.orientation_xyzw,
            Placement::IDENTITY.orientation_xyzw
        );
    }
    let final_revision = f.published.open().unwrap();
    let layers = f.root.join("layers");
    for entry in final_revision.manifest().content().entries() {
        fs::write(
            layers.join(entry.name()),
            final_revision.layer_bytes(entry.name()).unwrap(),
        )
        .unwrap();
    }
    let expected = std::iter::once(7).chain(1000..3000).map(|n| format!(r#"{{"id":"{}","name":"original part {n}","width":10,"length":20,"thickness":3,"translation":[{},0,0],"cli_volume_mm3":600}}"#, id(n), n * 20)).collect::<Vec<_>>().join(",");
    let expectation = layers.join("expected.json");
    fs::write(&expectation, format!("[{expected}]")).unwrap();
    // Workspace tests build this binary for integration tests before running library tests.
    // A filtered --lib invocation must build --bin beamcraft-usd-oracle first.
    let executable = std::env::current_exe().unwrap();
    let oracle = executable.parent().unwrap().parent().unwrap().join(format!(
        "beamcraft-usd-oracle{}",
        std::env::consts::EXE_SUFFIX
    ));
    let output = Command::new(oracle)
        .arg("revision")
        .arg(layers.join("root.usda"))
        .arg(expectation)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "reader: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    println!(
        "C05: 1000+1000 new IDs, 8 offline rounds of 125/client, 2001 final parts including seed; independent reader passed"
    );
}
