// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Loading the accepted snapshot: the headless CLI exports it in a separate process
//! (`beamcraft view-export --store ROOT --out -`) and the engine rebuilds and re-validates it.
//! The viewport never opens USD itself and never trusts the bytes it receives.

use crate::scene::Scene;
use crate::tools::{Definitions, ToolError, parse_definitions};
use crate::{Error, command};
use beamcraft_engine::query::bundle::max_bundle_bytes;
use beamcraft_engine::{Limits, Representation, ViewSnapshot};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The `beamcraft` CLI next to the running executable.
pub fn default_cli() -> Result<PathBuf, Error> {
    let exe = std::env::current_exe().map_err(Error::Io)?;
    let dir = exe
        .parent()
        .ok_or(Error::Input("executable has no directory"))?;
    Ok(dir.join(format!("beamcraft{}", std::env::consts::EXE_SUFFIX)))
}

/// The pinned view snapshot of the published head of `store`. The CLI runs under the same
/// deadline ([`command::DEADLINE`]) as the create commands, so a hung CLI ends in a typed error.
pub fn load(cli: &Path, store: &Path) -> Result<ViewSnapshot, Error> {
    load_within(cli, store, command::DEADLINE)
}

/// [`load`] with an explicit deadline.
pub fn load_within(cli: &Path, store: &Path, deadline: Duration) -> Result<ViewSnapshot, Error> {
    let limits = Limits::default();
    let cap = max_bundle_bytes(&limits);
    let args = [
        "view-export".to_owned(),
        "--store".into(),
        store.to_string_lossy().into_owned(),
        "--out".into(),
        "-".into(),
    ];
    let output = command::run_capped(cli, &args, deadline, cap).map_err(export_error)?;
    if output.code != Some(0) {
        // A rejection is one JSON object; report its stable code only.
        return Err(rejected(&output.stdout));
    }
    Ok(ViewSnapshot::from_bundle(&output.stdout, &limits)?)
}

fn rejected(bytes: &[u8]) -> Error {
    let text = String::from_utf8_lossy(bytes);
    let code = text
        .split("\"code\":\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .filter(|c| c.len() <= 32 && c.bytes().all(|b| b.is_ascii_uppercase() || b == b'_'))
        .unwrap_or("UNKNOWN");
    Error::Cli(format!("view-export rejected: {code}"))
}

/// A runner failure (no CLI, deadline, oversize output) as the viewport's typed error.
fn export_error(e: ToolError) -> Error {
    match e.code.as_str() {
        "OUTCOME_UNKNOWN" => {
            Error::Cli("view-export did not finish in time and was stopped".into())
        }
        "CLI_FAILED" if e.message.contains("size limit") => {
            Error::Cli("view bundle exceeds the snapshot limits".into())
        }
        code => Error::Cli(format!("view-export failed: {code}")),
    }
}
/// Everything one window refresh reads, gathered without a window or a GPU so it can run on a
/// worker thread: the accepted snapshot with its scene, and the store's definitions. The two
/// results are independent; one failing never hides the other.
pub struct Reloaded {
    /// The accepted snapshot and the scene built from it.
    pub scene: Result<(ViewSnapshot, Scene), Error>,
    /// The profile and material definitions of the same store.
    pub definitions: Result<Definitions, ToolError>,
}

/// Runs one pinned CLI inspection for geometry and definitions (blocking; call off the UI thread).
pub fn reload(cli: &Path, store: &Path) -> Reloaded {
    let limits = Limits::default();
    let snapshot_cap = max_bundle_bytes(&limits);
    let cap = beamcraft_protocol::refresh::max_bytes(snapshot_cap).unwrap_or(0);
    let args = [
        "view-export".into(),
        "--store".into(),
        store.to_string_lossy().into_owned(),
        "--out".into(),
        "-".into(),
        "--include-definitions".into(),
        "true".into(),
    ];
    let output = match command::run_capped(cli, &args, command::DEADLINE, cap) {
        Ok(output) => output,
        Err(error) => {
            return Reloaded {
                scene: Err(Error::Cli(format!("view-export failed: {}", error.code))),
                definitions: Err(error),
            };
        }
    };
    if output.code != Some(0) {
        return Reloaded {
            scene: Err(rejected(&output.stdout)),
            definitions: parse_definitions(output.code, &output.stdout),
        };
    }
    let (bundle, metadata) = match beamcraft_protocol::refresh::decode(&output.stdout, snapshot_cap)
    {
        Ok(parts) => parts,
        Err(message) => {
            return Reloaded {
                scene: Err(Error::Input(message)),
                definitions: Err(ToolError {
                    code: "UNREADABLE_REPLY".into(),
                    message: message.into(),
                }),
            };
        }
    };
    let scene = ViewSnapshot::from_bundle(bundle, &limits)
        .map_err(Error::View)
        .and_then(|snapshot| {
            let scene = Scene::from_snapshot(&snapshot, Representation::Full)?;
            Ok((snapshot, scene))
        });
    Reloaded {
        scene,
        definitions: parse_definitions(Some(0), metadata),
    }
}
