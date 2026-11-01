// SPDX-License-Identifier: MPL-2.0
//! Required real-binary prerequisites; absence is a failed acceptance check.

// A module of the `ui` test binary: the suites' own allows do not reach it.
#![allow(clippy::expect_used)]
use std::path::{Path, PathBuf};

/// One random value per process boot, for fixture directory names. A pid alone is not unique
/// across runs — the OS recycles pids, and a directory left behind by a killed process then
/// collides with a later run whose counter restarted at zero (the #157 class, seen again in
/// #408 and #405). A process-wide random nonce cannot collide with any other boot.
pub fn boot_nonce() -> u64 {
    use std::hash::{BuildHasher, RandomState};
    static NONCE: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *NONCE.get_or_init(|| RandomState::new().hash_one(0u64))
}

/// Resolves the real CLI adjacent to the UI executable or fails the test explicitly.
pub fn cli(ui: &Path) -> Option<PathBuf> {
    let parent = ui.parent().expect("required CLI location has no parent");
    let cli = parent.join(format!("beamcraft{}", std::env::consts::EXE_SUFFIX));
    assert!(
        cli.is_file(),
        "required CLI binary is missing; build beamcraft-cli before UI acceptance"
    );
    Some(cli)
}

#[test]
#[should_panic(expected = "required CLI binary is missing")]
fn a_missing_cli_is_a_failed_check_not_a_successful_skip() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let ui = std::env::temp_dir()
        .join(format!("beamcraft-absent-cli-{}-{nonce}", boot_nonce()))
        .join("beamcraft-ui");
    let _ = cli(&ui);
}
