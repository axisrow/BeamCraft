// SPDX-License-Identifier: MPL-2.0
//! Required real-binary prerequisites; absence is a failed acceptance check.
use std::path::{Path, PathBuf};

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
        .join(format!(
            "beamcraft-absent-cli-{}-{nonce}",
            std::process::id()
        ))
        .join("beamcraft-ui");
    let _ = cli(&ui);
}
