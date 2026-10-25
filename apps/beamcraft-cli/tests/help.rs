// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help aliases and the command inventory are a real-binary CLI contract.

#![allow(clippy::expect_used)]

use std::collections::BTreeSet;
use std::process::Command;

fn help(args: &[&str]) -> Vec<u8> {
    let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
        .args(args)
        .output()
        .expect("run beamcraft");
    assert!(output.status.success(), "{args:?}: {output:?}");
    assert!(output.stderr.is_empty(), "{args:?}: {output:?}");
    output.stdout
}

#[test]
fn help_aliases_share_a_unique_documented_command_inventory() {
    let output = help(&[]);
    assert_eq!(help(&["--help"]), output);
    assert_eq!(help(&["-h"]), output);
    let output = String::from_utf8(output).expect("UTF-8 help");
    let (inventory, details) = output.split_once('\n').expect("inventory and details");
    let commands = inventory.strip_prefix("BeamCraft: ").expect("help title");
    let mut seen = BTreeSet::new();
    for command in commands.split(" | ") {
        assert!(!command.is_empty(), "empty command in inventory");
        assert!(seen.insert(command), "duplicate command: {command}");
        assert!(details.contains(command), "missing help for {command}");
    }
    assert!(seen.contains("create-plate"));
    assert!(seen.contains("create-member"));
    assert!(seen.contains("publish-workspace"));
    assert!(details.ends_with('\n'));
}
