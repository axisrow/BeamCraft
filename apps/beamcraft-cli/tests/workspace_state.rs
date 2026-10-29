// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic metadata, stores and hostile entries, MPL-2.0.

//! C03.6: real filesystem CAS, interleaved writers, killed holders and existing CLI entry points.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use beamcraft_cli::workspace::{WorkspaceMeta, WorkspaceState, read_meta};
use beamcraft_host::lease::{LeaseError, StoreLease};
use beamcraft_host::revision::{RevisionStore, Uuid7};
use beamcraft_host::storage::BlobStore;
use beamcraft_json::{Json, Limits};
use beamcraft_model::digest::Sha256;
use beamcraft_model::error::ErrorCode;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    Arc, Barrier,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

static NEXT: AtomicU64 = AtomicU64::new(0);
fn id(n: usize) -> Uuid7 {
    Uuid7::parse(&format!("019a7bf0-0000-7000-8000-{n:012x}")).unwrap()
}
fn initial() -> WorkspaceMeta {
    WorkspaceMeta {
        workspace_id: id(1),
        user_id: id(2),
        base_revision: id(3),
        base_manifest: Sha256::digest(b"synthetic accepted pin").unwrap(),
        synced_sequence: 1,
        rebase_sequence: 0,
    }
}
fn next(old: &WorkspaceMeta, n: usize) -> WorkspaceMeta {
    WorkspaceMeta {
        base_revision: id(100 + n),
        base_manifest: Sha256::digest(&n.to_le_bytes()).unwrap(),
        ..old.clone()
    }
}
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "workspace-state-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        Self(dir)
    }
    fn initialize(&self) -> WorkspaceMeta {
        let meta = initial();
        assert!(
            WorkspaceState::open(&self.0)
                .unwrap()
                .compare_and_swap(None, &meta)
                .unwrap()
        );
        meta
    }
    fn bytes(&self) -> Vec<u8> {
        fs::read(self.0.join("workspace.json")).unwrap()
    }
    fn cli(&self, args: &[&str]) -> (i32, Json) {
        let out = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(args)
            .output()
            .unwrap();
        (
            out.status.code().unwrap(),
            beamcraft_json::decode(&out.stdout, &Limits::DEFAULT).unwrap(),
        )
    }
    fn ok(&self, args: &[&str]) -> Json {
        let (code, json) = self.cli(args);
        assert_eq!(code, 0, "{args:?}: {json:?}");
        json
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn metadata_cas_retains_v1_bytes_and_refuses_stale_identity_or_prefix() {
    let f = Fixture::new();
    let old = f.initialize();
    let expected = format!(
        "beamcraft-workspace 1\nworkspace_id={}\nuser_id={}\nbase_revision={}\nbase_manifest={}\nsynced_sequence=1\n",
        old.workspace_id, old.user_id, old.base_revision, old.base_manifest
    );
    assert_eq!(f.bytes(), expected.as_bytes());
    let mut state = WorkspaceState::open(&f.0).unwrap();
    let mut accepted = next(&old, 1);
    accepted.synced_sequence = 2;
    assert!(state.compare_and_swap(Some(&old), &accepted).unwrap());
    let bytes = f.bytes();
    assert!(!state.compare_and_swap(Some(&accepted), &accepted).unwrap());
    assert_eq!(
        state
            .compare_and_swap(Some(&old), &next(&old, 2))
            .unwrap_err()
            .code,
        ErrorCode::BaseRevisionMismatch
    );
    assert_eq!(
        state.compare_and_swap(None, &initial()).unwrap_err().code,
        ErrorCode::BaseRevisionMismatch
    );
    for variant in 0..4 {
        let mut invalid = accepted.clone();
        match variant {
            0 => invalid.workspace_id = id(90),
            1 => invalid.user_id = id(90),
            2 => invalid.synced_sequence = 0,
            _ => invalid.synced_sequence = 1,
        }
        assert_eq!(
            state
                .compare_and_swap(Some(&accepted), &invalid)
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(f.bytes(), bytes);
        assert!(!f.0.join("workspace.json.tmp").exists());
    }
    drop(state);
    assert_eq!(read_meta(&f.0).unwrap(), accepted);
}

#[test]
fn rebased_metadata_uses_strict_v2_without_publishing_unsent_work_or_rolling_back_its_boundary() {
    let f = Fixture::new();
    let old = f.initialize();
    let mut rebased = next(&old, 1);
    rebased.rebase_sequence = 3;
    {
        let mut state = WorkspaceState::open(&f.0).unwrap();
        state.compare_and_swap(Some(&old), &rebased).unwrap();
        assert_eq!(state.meta().unwrap().synced_sequence, 1);
        let mut rollback = rebased.clone();
        rollback.rebase_sequence = 2;
        assert_eq!(
            state
                .compare_and_swap(Some(&rebased), &rollback)
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
    }
    assert_eq!(read_meta(&f.0).unwrap(), rebased);
    let bytes = String::from_utf8(f.bytes()).unwrap();
    assert!(bytes.starts_with("beamcraft-workspace 2\n"));
    assert!(bytes.ends_with("rebase_sequence=3\n"));
    for malformed in [
        bytes.replace("rebase_sequence=3", "rebase_sequence=0"),
        bytes.replace("rebase_sequence=3", "rebase_sequence=03"),
        bytes.replace("rebase_sequence=3", "rebase_sequence=100001"),
        bytes.replace('\n', "\r\n"),
        bytes.replacen("workspace 2", "workspace 999", 1),
    ] {
        fs::write(f.0.join("workspace.json"), malformed.as_bytes()).unwrap();
        assert!(read_meta(&f.0).is_err());
        assert_eq!(f.bytes(), malformed.as_bytes());
    }
}

#[test]
fn eight_racing_writers_install_exactly_one_complete_metadata_snapshot() {
    let f = Fixture::new();
    let old = f.initialize();
    let barrier = Arc::new(Barrier::new(8));
    let handles: Vec<_> = (0..8)
        .map(|n| {
            let (dir, expected, barrier) = (f.0.clone(), old.clone(), barrier.clone());
            std::thread::spawn(move || {
                let desired = next(&expected, n);
                barrier.wait();
                let result = WorkspaceState::open(&dir)
                    .and_then(|mut state| state.compare_and_swap(Some(&expected), &desired));
                (desired, result)
            })
        })
        .collect();
    let outcomes: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    let winners: Vec<_> = outcomes
        .iter()
        .filter(|(_, result)| result == &Ok(true))
        .collect();
    assert_eq!(winners.len(), 1);
    for (_, result) in &outcomes {
        if let Err(error) = result {
            assert!(matches!(
                error.code,
                ErrorCode::BackendUnavailable | ErrorCode::BaseRevisionMismatch
            ));
        }
    }
    assert_eq!(&read_meta(&f.0).unwrap(), &winners[0].0);
    assert!(!f.0.join("workspace.json.tmp").exists());
}

#[test]
fn metadata_serialization_still_allows_draft_edits_and_protects_cli_publication() {
    let f = Fixture::new();
    let published = f.0.join("published");
    let draft = f.0.join("draft");
    fs::create_dir(&published).unwrap();
    let pub_path = published.to_str().unwrap();
    let draft_path = draft.to_str().unwrap();
    let plate = id(42).to_string();
    f.ok(&[
        "create-plate",
        "--store",
        pub_path,
        "--width",
        "1000",
        "--length",
        "500",
        "--thickness",
        "10",
        "--new-id",
        &plate,
    ]);
    f.ok(&[
        "create-workspace",
        "--store",
        pub_path,
        "--workspace-dir",
        draft_path,
        "--user-id",
        id(2).as_str(),
    ]);
    let revision_store = |path: &Path| RevisionStore::new(BlobStore::new(path).unwrap());
    let accepted = revision_store(&published).head().unwrap().unwrap();
    let old_draft = revision_store(&draft).head().unwrap().unwrap();
    let state = WorkspaceState::open(&draft).unwrap();
    assert!(matches!(
        StoreLease::exclusive(revision_store(&draft).blobs()),
        Err(LeaseError::Busy)
    ));
    // A local edit does not interpret base/unsent metadata and still uses the ordinary draft CAS.
    f.ok(&[
        "update-element",
        "--store",
        draft_path,
        "--id",
        &plate,
        "--name",
        "Unsent",
    ]);
    let edited = revision_store(&draft).head().unwrap().unwrap();
    assert_ne!(edited, old_draft);
    for command in ["inspect-workspace", "publish-workspace", "create-changeset"] {
        let (code, json) = f.cli(&[command, "--store", pub_path, "--workspace-dir", draft_path]);
        assert_eq!(code, 3, "{command}: {json:?}");
        assert_eq!(
            json.get("code").unwrap().string().unwrap(),
            "BACKEND_UNAVAILABLE"
        );
        assert_eq!(
            revision_store(&published).head().unwrap().unwrap(),
            accepted
        );
        assert_eq!(revision_store(&draft).head().unwrap().unwrap(), edited);
    }
    drop(state);
    let status = f.ok(&[
        "inspect-workspace",
        "--store",
        pub_path,
        "--workspace-dir",
        draft_path,
    ]);
    assert_eq!(
        status
            .get("workspace")
            .unwrap()
            .get("unsent_revisions")
            .unwrap()
            .string()
            .unwrap(),
        "1"
    );
    // A pending rebase must be resolved before any interpretation of base/unsent state.
    fs::write(draft.join("rebase.pending"), b"torn rebase intent").unwrap();
    for command in ["inspect-workspace", "publish-workspace", "create-changeset"] {
        let (code, json) = f.cli(&[command, "--store", pub_path, "--workspace-dir", draft_path]);
        assert_eq!(code, 2, "{command}: {json:?}");
        assert_eq!(
            json.get("code").unwrap().string().unwrap(),
            "INTEGRITY_ERROR"
        );
        assert_eq!(
            revision_store(&published).head().unwrap().unwrap(),
            accepted
        );
        assert_eq!(revision_store(&draft).head().unwrap().unwrap(), edited);
    }
    fs::remove_file(draft.join("rebase.pending")).unwrap();
    f.ok(&[
        "publish-workspace",
        "--store",
        pub_path,
        "--workspace-dir",
        draft_path,
    ]);
    assert_eq!(
        revision_store(&published)
            .open()
            .unwrap()
            .manifest()
            .content()
            .entries(),
        revision_store(&draft)
            .open()
            .unwrap()
            .manifest()
            .content()
            .entries()
    );
}

#[test]
fn malformed_or_oversized_metadata_never_gets_replaced() {
    for bytes in [
        Vec::new(),
        b"beamcraft-workspace 999\n".to_vec(),
        vec![b'x'; 4097],
        vec![0xff],
    ] {
        let f = Fixture::new();
        f.initialize();
        fs::write(f.0.join("workspace.json"), &bytes).unwrap();
        assert!(WorkspaceState::open(&f.0).is_err());
        assert_eq!(f.bytes(), bytes);
        assert!(!f.0.join("workspace.json.tmp").exists());
    }
}

#[cfg(unix)]
#[test]
fn planted_metadata_and_state_lock_links_never_touch_their_targets() {
    use std::os::unix::fs::symlink;
    for name in ["workspace.json", "workspace-state.lock"] {
        let f = Fixture::new();
        f.initialize();
        let target = f.0.join("do-not-change");
        fs::write(&target, b"original bytes").unwrap();
        let entry = f.0.join(name);
        fs::remove_file(&entry).unwrap();
        symlink(&target, &entry).unwrap();
        assert!(WorkspaceState::open(&f.0).is_err());
        assert_eq!(fs::read(target).unwrap(), b"original bytes");
        assert!(
            fs::symlink_metadata(entry)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }
}

#[test]
fn child_process_holds_state() {
    let Some(dir) = std::env::var_os("BEAMCRAFT_STATE_TEST_CHILD") else {
        return;
    };
    let dir = PathBuf::from(dir);
    let mut state = WorkspaceState::open(&dir).unwrap();
    let old = state.meta().unwrap().clone();
    if std::env::var_os("BEAMCRAFT_STATE_TEST_INSTALLED").is_some() {
        state.compare_and_swap(Some(&old), &next(&old, 99)).unwrap();
    } else {
        fs::write(dir.join("workspace.json.tmp"), b"incomplete staged bytes").unwrap();
    }
    fs::write(dir.join("child-ready"), b"locked").unwrap();
    loop {
        std::thread::park_timeout(Duration::from_millis(100));
    }
}

#[test]
fn a_killed_writer_releases_the_lease_and_leaves_old_or_installed_metadata_complete() {
    for installed in [false, true] {
        let f = Fixture::new();
        let old = f.initialize();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "workspace_state::child_process_holds_state",
                "--nocapture",
            ])
            .env("BEAMCRAFT_STATE_TEST_CHILD", &f.0)
            .env_remove("BEAMCRAFT_STATE_TEST_INSTALLED")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if installed {
            command.env("BEAMCRAFT_STATE_TEST_INSTALLED", "1");
        }
        let mut child = command.spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        while !f.0.join("child-ready").exists() && Instant::now() < deadline {
            assert!(
                child.try_wait().unwrap().is_none(),
                "holder exited before ready"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        if !f.0.join("child-ready").exists() {
            let _ = child.kill();
            let _ = child.wait();
            panic!("holder did not become ready");
        }
        assert_eq!(
            read_meta(&f.0).unwrap_err().code,
            ErrorCode::BackendUnavailable
        );
        child.kill().unwrap();
        child.wait().unwrap();
        let expected = if installed { next(&old, 99) } else { old };
        assert_eq!(read_meta(&f.0).unwrap(), expected);
        let mut state = WorkspaceState::open(&f.0).unwrap();
        assert!(
            state
                .compare_and_swap(Some(&expected), &next(&expected, 100))
                .unwrap()
        );
        assert!(!f.0.join("workspace.json.tmp").exists());
    }
}
