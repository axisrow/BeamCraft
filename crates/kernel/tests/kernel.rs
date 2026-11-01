// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Every integration suite of the kernel, linked into one test binary.
//!
//! Each suite keeps its own file and becomes a module of the same name, so a suite runs with
//! `cargo test -p beamcraft-kernel --test kernel -- <suite>::` and a single case with
//! `cargo test -p beamcraft-kernel --test kernel -- --exact <suite>::<case>`. A new suite file is
//! listed here. The shared helpers are modules of this binary too; a suite imports them with
//! `use crate::common;` (the G06 suites name `common_mesh` as `common`).

mod common;
mod common_mesh;
mod common_ops;

mod g03_fixtures;
mod g05_negative_and_fuzz;
mod g05_profile_extrusion;
mod g06_fixtures;
mod g06_fuzz;
mod g06_measure;
mod g06_shells;
mod g09_budget_fuzz;
mod g09_cut;
mod g09_hole;
mod rectangular_plate;

/// With `autotests = false` a suite file missing from the list above would never build or run,
/// so every `tests/*.rs` file must be one of its modules.
#[test]
fn every_suite_file_is_a_module_of_this_binary() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let listing = std::fs::read_to_string(dir.join("kernel.rs")).unwrap();
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
        if name == "kernel" {
            continue;
        }
        assert!(
            listing.contains(&format!("\nmod {name};\n")),
            "tests/{name}.rs is not a module of tests/kernel.rs"
        );
        suites += 1;
    }
    assert!(suites > 0, "no suite file found in {}", dir.display());
}
