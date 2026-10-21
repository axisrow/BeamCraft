// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Loading the accepted snapshot: the headless CLI exports it in a separate process
//! (`beamcraft view-export --store ROOT --out -`) and the engine rebuilds and re-validates it.
//! The viewport never opens USD itself and never trusts the bytes it receives.
//!
//! A window refresh asks for the snapshot, the store's definitions and the selected element's
//! report in one process (`--with-definitions true --with-element ID`): one verified inspection
//! of one revision instead of one per query (issue 26, finding 2).

use crate::perf::RefreshTiming;
use crate::props::{Properties, WorkspacePanel, parse_element};
use crate::scene::Scene;
use crate::tools::{Definitions, ToolError, parse_definitions};
use crate::{Error, command};
use beamcraft_engine::query::bundle::max_bundle_bytes;
use beamcraft_engine::{Limits, Representation, SnapshotToken, ViewSnapshot};
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::id::ElementId;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

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
        return Err(rejected(&output.stdout));
    }
    Ok(ViewSnapshot::from_bundle(&output.stdout, &limits)?)
}

/// A rejection is one JSON object; report its stable code only.
fn rejected(stdout: &[u8]) -> Error {
    let text = String::from_utf8_lossy(stdout);
    let code = text
        .split("\"code\":\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .filter(|c| c.len() <= 32 && c.bytes().all(|b| b.is_ascii_uppercase() || b == b'_'))
        .unwrap_or("UNKNOWN");
    if code == "NOT_FOUND" {
        // A store with no published revision yet: typed, so the empty state cannot drift apart
        // from the matcher that treats it as normal.
        return Error::NoRevision;
    }
    Error::Cli(format!("view-export rejected: {code}"))
}

/// The first bytes of a framed `view-export` reply (the CLI defines the frame).
pub const FRAME_MAGIC: &[u8; 8] = b"BCVXFRM1";

/// The parts of a framed `view-export` reply, split but not yet read.
#[derive(Debug, PartialEq, Eq)]
pub struct Frame<'a> {
    /// The `definitions` reply; empty when it was not asked for.
    pub definitions: &'a [u8],
    /// The `plate-inspect --element ID` reply; empty when it was not asked for.
    pub element: &'a [u8],
    /// The engine bundle.
    pub bundle: &'a [u8],
}

/// Splits a framed reply. Nothing in it is trusted: each length is bounded by
/// [`command::MAX_OUTPUT`] and checked against the bytes received, the replies go through the
/// bounded JSON decoder and the bundle is re-validated by the engine.
pub fn split_frame(bytes: &[u8]) -> Result<Frame<'_>, Error> {
    fn section(bytes: &[u8]) -> Option<(&[u8], &[u8])> {
        let (length, rest) = bytes.split_first_chunk::<4>()?;
        let length = usize::try_from(u32::from_le_bytes(*length)).ok()?;
        if length > command::MAX_OUTPUT {
            return None;
        }
        rest.split_at_checked(length)
    }
    let bad = || Error::Cli("view-export returned an unreadable frame".into());
    let rest = bytes.strip_prefix(FRAME_MAGIC.as_slice()).ok_or_else(bad)?;
    let (definitions, rest) = section(rest).ok_or_else(bad)?;
    let (element, bundle) = section(rest).ok_or_else(bad)?;
    Ok(Frame {
        definitions,
        element,
        bundle,
    })
}

/// Runs one framed `view-export`: the bundle, the definitions and, for `selected`, that
/// element's report.
fn export_framed(
    cli: &Path,
    store: &Path,
    selected: Option<&str>,
    deadline: Duration,
) -> Result<Vec<u8>, Error> {
    let cap = FRAME_MAGIC
        .len()
        .saturating_add(2 * (4 + command::MAX_OUTPUT))
        .saturating_add(max_bundle_bytes(&Limits::default()));
    let mut args = vec![
        "view-export".to_owned(),
        "--store".into(),
        store.to_string_lossy().into_owned(),
        "--out".into(),
        "-".into(),
        "--with-definitions".into(),
        "true".into(),
    ];
    // An id the CLI would refuse is not asked for, so it cannot fail the snapshot with it.
    if let Some(id) = selected.filter(|id| ElementId::parse(id).is_ok()) {
        args.extend(["--with-element".into(), id.to_owned()]);
    }
    let output = command::run_capped(cli, &args, deadline, cap).map_err(export_error)?;
    if output.code != Some(0) {
        return Err(rejected(&output.stdout));
    }
    Ok(output.stdout)
}

/// What the window already shows, so a refresh skips the work it would only repeat: the token
/// of the loaded snapshot and the meshes the device already holds (issue 26, finding 3).
#[derive(Debug, Clone, Default)]
pub struct Known {
    /// The token of the loaded snapshot.
    pub token: Option<SnapshotToken>,
    /// Digests of the meshes on the device (see [`crate::render::Uploaded::resident`]).
    pub resident: BTreeSet<ContentDigest>,
}

/// The snapshot one refresh read.
#[derive(Debug)]
pub enum Refreshed {
    /// Another snapshot than the loaded one, and its scene; meshes the device already holds are
    /// not expanded again ([`Scene::from_snapshot_reusing`]).
    New(Box<(ViewSnapshot, Scene)>),
    /// The loaded snapshot again (the same token): there is nothing to build or upload.
    Unchanged,
}

/// What one refresh reads about the model: the snapshot as far as it differs from what the
/// window shows, the definitions and the selected element's properties.
type Parts = (
    Result<Refreshed, Error>,
    Result<Definitions, ToolError>,
    Option<Result<Option<Properties>, ToolError>>,
);

fn frame_parts(frame: &Frame<'_>, selected: Option<&str>, known: &Known) -> Parts {
    let definitions = parse_definitions(Some(0), frame.definitions);
    let properties = selected.map(|id| match ElementId::parse(id) {
        Ok(_) => parse_element(Some(0), frame.element, id),
        // An id no revision can hold is not in this one.
        Err(_) => Ok(None),
    });
    let scene = ViewSnapshot::from_bundle(frame.bundle, &Limits::default())
        .map_err(Error::from)
        .and_then(|snapshot| {
            // The token covers the pin and every element, kind, placement and mesh digest.
            if known.token.as_ref() == Some(snapshot.token()) {
                return Ok(Refreshed::Unchanged);
            }
            let scene =
                Scene::from_snapshot_reusing(&snapshot, Representation::Full, &known.resident)?;
            Ok(Refreshed::New(Box::new((snapshot, scene))))
        });
    (scene, definitions, properties)
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

/// True when the failure is only that the store has no published revision yet: a normal state
/// of a new store, not an error to show in red.
pub fn is_no_revision(error: &Error) -> bool {
    matches!(error, Error::NoRevision)
}
/// Everything one window refresh reads, gathered without a window or a GPU so it can run on a
/// worker thread: the accepted snapshot with its scene, the store's definitions, the selected
/// element's properties and the workspace panel. The results are independent; one failing never
/// hides another.
pub struct Reloaded {
    /// The accepted snapshot and the scene built from it, as far as it differs from what the
    /// window shows.
    pub scene: Result<Refreshed, Error>,
    /// The profile and material definitions of the same store.
    pub definitions: Result<Definitions, ToolError>,
    /// The selected element's properties as of this refresh; `Some(Ok(None))` when the selected
    /// element is no longer in the store. `None` when nothing is selected.
    pub properties: Option<Result<Option<Properties>, ToolError>>,
    /// The publication panel, when the window edits a personal workspace.
    pub workspace: Option<Result<WorkspacePanel, ToolError>>,
    /// The CLI processes this refresh started and its wall time on the worker.
    pub timing: RefreshTiming,
}

/// Runs the CLI queries of one refresh (blocking; call it off the UI thread). The model is read
/// by one framed `view-export`, so the snapshot, the definitions and the selected element's
/// properties come from one verified inspection of one revision; only when that export fails
/// are the definitions and the properties asked on their own, so a failing snapshot never hides
/// them (issue 26, finding 2). The workspace queries read the draft's history, not the model,
/// and run alongside. `selected` is the selected element's id, `workspace` the published store
/// and the workspace directory, `known` what the window already shows: the same snapshot again
/// is reported as [`Refreshed::Unchanged`].
pub fn reload(
    cli: &Path,
    store: &Path,
    selected: Option<&str>,
    workspace: Option<(&Path, &Path)>,
    known: &Known,
) -> Reloaded {
    let started = Instant::now();
    // The framed export, and the two workspace queries.
    let mut processes = 1 + 2 * u32::from(workspace.is_some());
    std::thread::scope(|scope| {
        let workspace = scope.spawn(|| {
            workspace.map(|(published, dir)| {
                let state = command::workspace_state(cli, published, dir)?;
                let (undo_units, redo_units) = command::workspace_history(cli, dir)?;
                Ok(WorkspacePanel {
                    state,
                    undo_units,
                    redo_units,
                })
            })
        });
        let framed = export_framed(cli, store, selected, command::DEADLINE)
            .and_then(|bytes| Ok(frame_parts(&split_frame(&bytes)?, selected, known)));
        let (scene, definitions, properties) = match framed {
            Ok(parts) => parts,
            Err(e) => {
                processes += 1 + u32::from(selected.is_some());
                let properties =
                    scope.spawn(|| selected.map(|id| command::properties(cli, store, id)));
                let definitions = command::definitions(cli, store);
                // A panicked worker is a bug, not model data: report it as its typed failure.
                let properties = properties.join().unwrap_or_else(|_| {
                    Some(Err(ToolError::local(
                        "CLI_FAILED",
                        "the properties thread failed",
                    )))
                });
                (Err(e), definitions, properties)
            }
        };
        let workspace = workspace.join().unwrap_or_else(|_| {
            Some(Err(ToolError::local(
                "CLI_FAILED",
                "the workspace thread failed",
            )))
        });
        Reloaded {
            scene,
            definitions,
            properties,
            workspace,
            timing: RefreshTiming {
                processes,
                total_ms: started.elapsed().as_secs_f64() * 1000.0,
            },
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(definitions: &[u8], element: &[u8], bundle: &[u8]) -> Vec<u8> {
        let mut out = FRAME_MAGIC.to_vec();
        for reply in [definitions, element] {
            out.extend_from_slice(&u32::try_from(reply.len()).unwrap().to_le_bytes());
            out.extend_from_slice(reply);
        }
        out.extend_from_slice(bundle);
        out
    }

    #[test]
    fn a_frame_splits_into_its_replies_and_bundle() {
        let bytes = frame(b"{}", b"{\"a\":1}", b"BCVIEW01rest");
        assert_eq!(
            split_frame(&bytes).unwrap(),
            Frame {
                definitions: b"{}",
                element: b"{\"a\":1}",
                bundle: b"BCVIEW01rest",
            }
        );
        let empty = frame(b"", b"", b"");
        assert_eq!(
            split_frame(&empty).unwrap(),
            Frame {
                definitions: b"",
                element: b"",
                bundle: b"",
            }
        );
    }

    #[test]
    fn a_damaged_frame_is_refused_without_reading_past_it() {
        let good = frame(b"{\"a\":1}", b"{}", b"bundle");
        // Magic 0..8, first length 8..12 and reply 12..19, second length 19..23 and reply
        // 23..25, bundle 25..31.
        for cut in [0, 7, 11, 15, 19, 22, 24] {
            assert!(split_frame(&good[..cut]).is_err(), "cut at {cut}");
        }
        assert!(
            split_frame(&good[..25]).is_ok(),
            "an empty bundle is the engine's to refuse"
        );
        let mut magic = good.clone();
        magic[0] = b'X';
        assert!(split_frame(&magic).is_err());
        for at in [8, 19] {
            let mut huge = good.clone();
            huge[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
            assert!(split_frame(&huge).is_err(), "length at {at}");
        }
        let mut over = frame(&vec![b' '; command::MAX_OUTPUT + 1], b"", b"");
        assert!(
            split_frame(&over).is_err(),
            "longer than a CLI reply may be"
        );
        over.truncate(12);
        assert!(split_frame(&over).is_err());
    }

    #[test]
    fn a_refresh_builds_nothing_for_the_snapshot_on_screen_and_reuses_resident_meshes() {
        use beamcraft_engine::ViewPin;
        use beamcraft_engine::synthetic::plate_scene;
        use beamcraft_model::digest::Sha256;
        let pin = ViewPin::new(Sha256::digest(b"revision").unwrap(), 1);
        let snapshot =
            ViewSnapshot::build(pin, plate_scene(6, false).unwrap(), &Limits::default()).unwrap();
        let bytes = frame(b"{}", b"", &snapshot.to_bundle().unwrap());
        let frame = split_frame(&bytes).unwrap();
        // Nothing on screen yet: a new scene with every mesh expanded.
        let Ok(Refreshed::New(first)) = frame_parts(&frame, None, &Known::default()).0 else {
            panic!("a first refresh builds the scene");
        };
        assert!(
            first
                .1
                .meshes
                .iter()
                .all(|m| !m.resident && !m.vertices.is_empty())
        );
        // The same snapshot again: nothing to build or upload.
        let known = Known {
            token: Some(snapshot.token().clone()),
            resident: first.1.meshes.iter().map(|m| m.digest).collect(),
        };
        assert!(matches!(
            frame_parts(&frame, None, &known).0,
            Ok(Refreshed::Unchanged)
        ));
        // Another snapshot whose meshes the device holds: built, but nothing expanded again.
        let known = Known {
            token: None,
            ..known
        };
        let Ok(Refreshed::New(again)) = frame_parts(&frame, None, &known).0 else {
            panic!("another token builds the scene");
        };
        assert!(
            again
                .1
                .meshes
                .iter()
                .all(|m| m.resident && m.vertices.is_empty())
        );
    }

    #[test]
    fn a_frame_gives_the_definitions_and_the_selected_element() {
        let definitions = br#"{"outcome":"definitions","definitions":[]}"#;
        let element = br#"{"outcome":"plate_inspection","plates":[],"members":[]}"#;
        let bytes = frame(definitions, element, b"not a bundle");
        let frame = split_frame(&bytes).unwrap();
        let id = "019a7bf0-0000-7000-8000-000000000010";
        let known = Known::default();
        let (scene, definitions, properties) = frame_parts(&frame, Some(id), &known);
        // A bundle the engine refuses fails the scene alone.
        assert!(scene.is_err());
        assert!(definitions.is_ok());
        // The element is not in the report: the selected element is gone.
        assert!(matches!(properties, Some(Ok(None))));
        assert!(frame_parts(&frame, None, &known).2.is_none());
        // An id no revision can hold is not looked for.
        assert!(matches!(
            frame_parts(&frame, Some("x"), &known).2,
            Some(Ok(None))
        ));
    }
}
