// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The command registry is the inventory of what the real binary dispatches.

#![allow(clippy::expect_used)]

use beamcraft_cli::registry::{Access, COMMANDS, Route};
use beamcraft_json::{Json, Limits};
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Runs `beamcraft ARGS`, killing it after a deadline so a command that waits for input or
/// serves cannot hang the suite. Returns the exit code and standard output.
fn run(args: &[&str]) -> (Option<i32>, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn beamcraft");
    let mut stdout = child.stdout.take().expect("stdout");
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        stdout.read_to_string(&mut text).ok();
        text
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    let code = loop {
        if let Some(status) = child.try_wait().expect("wait") {
            break status.code();
        }
        assert!(Instant::now() < deadline, "{args:?} did not finish");
        std::thread::sleep(Duration::from_millis(10));
    };
    (code, reader.join().expect("reader"))
}

/// A scratch store directory: with `--store` the generic blob and revision path reaches its command
/// check, so an undispatched word answers `UNSUPPORTED_COMMAND` instead of an argument refusal.
fn scratch_store() -> String {
    let dir = std::env::temp_dir().join(format!("beamcraft-registry-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch store");
    dir.to_str().expect("utf-8 path").to_owned()
}

#[test]
fn an_unregistered_word_is_unsupported() {
    let store = scratch_store();
    let (code, out) = run(&["no-such-command", "--store", &store]);
    assert_eq!(code, Some(2));
    assert!(out.contains("UNSUPPORTED_COMMAND"), "{out}");
}

#[test]
fn every_registered_command_is_routed_by_the_binary() {
    let store = scratch_store();
    for command in COMMANDS {
        let (code, out) = run(&[command.name, "--store", &store]);
        assert!(
            !out.contains("UNSUPPORTED_COMMAND"),
            "{} is registered but not dispatched: {out}",
            command.name
        );
        assert!(code.is_some(), "{}: no exit code", command.name);
    }
}

fn names(out: &str) -> Vec<String> {
    commands_of(out)
        .iter()
        .map(|c| {
            c.get("name")
                .and_then(Json::string)
                .expect("name")
                .to_owned()
        })
        .collect()
}

#[test]
fn commands_lists_the_registry() {
    let (code, out) = run(&["commands"]);
    assert_eq!(code, Some(0));
    assert!(out.contains("\"outcome\":\"commands_listed\""), "{out}");
    let all = names(&out);
    assert_eq!(all.len(), COMMANDS.len());
    assert!(all.iter().any(|n| n == "create-member"));

    for removed in ["show-settings", "set-settings", "list-environment"] {
        assert!(!all.iter().any(|n| n == removed), "{removed} was removed");
    }
}

#[test]
fn commands_takes_no_arguments() {
    for args in [
        &["commands", "--tier", "core"][..],
        &["commands", "--tier"],
        &["commands", "extra"],
    ] {
        let (code, out) = run(args);
        assert_eq!(code, Some(2), "{args:?}: {out}");
        assert!(out.contains("INVALID_ARGUMENT"), "{args:?}: {out}");
    }
}

#[test]
fn the_help_inventory_is_the_registry_in_order() {
    let (_, help) = run(&["--help"]);
    let line = help.lines().next().expect("inventory");
    let listed: Vec<_> = line
        .strip_prefix("BeamCraft: ")
        .expect("title")
        .split(" | ")
        .collect();
    let registered: Vec<_> = COMMANDS.iter().map(|c| c.name).collect();
    assert_eq!(listed, registered);
}

/// The `commands` array of a listing.
fn commands_of(out: &str) -> Vec<Json> {
    let json = beamcraft_json::decode(out.as_bytes(), &Limits::DEFAULT).expect("listing is json");
    json.get("commands")
        .and_then(Json::array)
        .expect("commands array")
        .to_vec()
}

fn field<'a>(json: &'a Json, key: &str) -> &'a Json {
    json.get(key).expect("field")
}

#[test]
fn the_listing_carries_access_and_parameters() {
    let (code, out) = run(&["commands"]);
    assert_eq!(code, Some(0));
    let listed = commands_of(&out);
    assert_eq!(listed.len(), COMMANDS.len());
    for (entry, command) in listed.iter().zip(COMMANDS) {
        assert_eq!(field(entry, "name").string(), Ok(command.name));
        let access = match command.access {
            Access::Read => "read",
            Access::Write => "write",
        };
        assert_eq!(
            field(entry, "access").string(),
            Ok(access),
            "{}",
            command.name
        );
        let params = field(entry, "parameters").array().expect("parameters");
        assert!(params.len() >= command.params.len(), "{}", command.name);
        for p in params {
            let name = field(p, "name").string().expect("param name");
            assert!(name.starts_with("--"), "{}: {name}", command.name);
            assert!(field(p, "required").boolean().is_ok());
        }
    }
    // A required option, an optional one, an alternative group and the common identity options.
    let weld = listed
        .iter()
        .find(|c| field(c, "name").string() == Ok("create-weld"))
        .expect("create-weld");
    let params = field(weld, "parameters").array().expect("parameters");
    let find = |name: &str| {
        params
            .iter()
            .find(|p| field(p, "name").string() == Ok(name))
            .unwrap_or_else(|| panic!("create-weld lists {name}"))
    };
    assert_eq!(field(find("--size"), "required").boolean(), Ok(true));
    assert_eq!(field(find("--path"), "required").boolean(), Ok(false));
    assert_eq!(field(find("--op-id"), "value").string(), Ok("UUID"));
    let add_hole = listed
        .iter()
        .find(|c| field(c, "name").string() == Ok("add-hole"))
        .expect("add-hole");
    let group = field(add_hole, "parameters")
        .array()
        .expect("parameters")
        .iter()
        .filter(|p| field(p, "one_of").unsigned() == Ok(1))
        .count();
    assert_eq!(group, 3);
}

/// Runs `beamcraft ARGS` in an empty scratch directory with the user configuration redirected
/// into it, so a probe can neither create files in the checkout nor touch the real settings.
fn probe(args: &[&str]) -> String {
    let dir = std::env::temp_dir().join(format!("beamcraft-registry-probe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("probe directory");
    let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
        .args(args)
        .current_dir(&dir)
        .env("BEAMCRAFT_CONFIG_DIR", &dir)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .expect("run beamcraft");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// These routes refuse an unrecognised option without saying that it is unknown: the blob and
/// revision path gives one generic refusal, the view queries a diagnostic code, and `commands`
/// takes a single option.
fn names_unknown_options(route: Route) -> bool {
    !matches!(route, Route::Generic | Route::View | Route::Commands)
}

#[test]
fn an_unlisted_option_is_refused_as_unknown() {
    for command in COMMANDS.iter().filter(|c| names_unknown_options(c.route)) {
        let out = probe(&[command.name, "--no-such-option", "x"]);
        assert!(
            out.contains("unknown"),
            "{}: the unknown-option probe is not recognised: {out}",
            command.name
        );
    }
}

#[test]
fn every_listed_option_is_accepted_by_the_command() {
    let (_, listing) = run(&["commands"]);
    for entry in commands_of(&listing) {
        let name = field(&entry, "name").string().expect("name");
        let route = COMMANDS
            .iter()
            .find(|c| c.name == name)
            .map(|c| c.route)
            .expect("registered");
        if !names_unknown_options(route) {
            continue;
        }
        let params = field(&entry, "parameters").array().expect("parameters");
        let (flags, valued): (Vec<_>, Vec<_>) = params
            .iter()
            .map(|p| {
                (
                    field(p, "name").string().expect("param name"),
                    field(p, "value").string().expect("param value"),
                )
            })
            .partition(|(_, value)| value.is_empty());
        // A few at a time: some adapters cap the number of options in one call.
        for chunk in valued.chunks(6) {
            let mut args = vec![name];
            for (option, _) in chunk {
                args.extend([*option, "x"]);
            }
            let out = probe(&args);
            assert!(
                !out.contains("unknown"),
                "{name}: an option of {chunk:?} is listed but refused as unknown: {out}"
            );
        }
        // A valueless option stands alone; one that also takes `true`/`false` gets that too.
        for (option, _) in flags {
            let bare = probe(&[name, option]);
            let with_value = probe(&[name, option, "true"]);
            assert!(
                !bare.contains("unknown") || !with_value.contains("unknown"),
                "{name}: {option} is listed but refused as unknown: {bare} {with_value}"
            );
        }
    }
}

#[test]
fn the_reference_is_current() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/COMMANDS.md");
    let expected = beamcraft_cli::registry::reference_markdown();
    if std::env::var_os("BEAMCRAFT_BLESS").is_some() {
        std::fs::write(path, &expected).expect("write the reference");
    }
    let committed = std::fs::read_to_string(path).expect("COMMANDS.md is committed");
    // A checkout may convert line endings.
    assert!(
        committed.replace("\r\n", "\n") == expected,
        "COMMANDS.md is out of date: regenerate it with BEAMCRAFT_BLESS=1 \
         cargo test -p beamcraft-cli --test cli registry::the_reference_is_current"
    );
}
