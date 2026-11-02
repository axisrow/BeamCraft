// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Every integration suite of `beamcraft-host`, linked into one test binary.
//!
//! Each suite keeps its own file and becomes a module of the same name, so a suite runs with
//! `cargo test -p beamcraft-host --test host -- <suite>::` and a single case with
//! `cargo test -p beamcraft-host --test host -- --exact <suite>::<case>`. A new suite file is listed
//! here.

mod changeset_claims;
mod compaction;
mod operation_evidence;
mod recovery;
mod revision;
mod storage;

/// With `autotests = false` a suite file missing from the list above would never build or run,
/// so every `tests/*.rs` file must be one of its modules.
#[test]
fn every_suite_file_is_a_module_of_this_binary() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let listing = std::fs::read_to_string(dir.join("host.rs")).unwrap();
    let mut suites = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let Some(name) = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_suffix(".rs"))
        else {
            continue;
        };
        if name == "host" {
            continue;
        }
        assert!(
            listing.contains(&format!("\nmod {name};\n")),
            "tests/{name}.rs is not a module of tests/host.rs"
        );
        suites += 1;
    }
    assert!(suites > 0, "no suite file found in {}", dir.display());
}
