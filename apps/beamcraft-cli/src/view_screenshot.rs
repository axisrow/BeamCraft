// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! V03 headless command `screenshot-view`: one viewport render of the store's published head
//! as a PNG file. The pixels are drawn by the native viewport renderer, which lives in the
//! `beamcraft-ui` binary, so this adapter is a thin driver: it finds `beamcraft-ui` next to
//! its own executable (the same neighbourhood rule by which the window finds this CLI), runs
//! it with `--screenshot`, and relays its one JSON reply. This process never renders and
//! never reads the store.

use beamcraft_json::{Json, Limits};
use beamcraft_model::error::ErrorCode;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;

/// Longest `beamcraft-ui` reply this relay reads back.
const REPLY_CAP: usize = 8192;

/// A failure of this adapter: a static message of its own, or a dynamic one relayed from
/// `beamcraft-ui`.
struct Fail {
    code: ErrorCode,
    message: String,
}

fn invalid(message: &'static str) -> Fail {
    Fail {
        code: ErrorCode::InvalidArgument,
        message: message.to_owned(),
    }
}

fn backend(message: String) -> Fail {
    Fail {
        code: ErrorCode::BackendUnavailable,
        message,
    }
}

/// The `beamcraft-ui` binary next to this executable, by the neighbourhood rule the window
/// applies in reverse when it looks for this CLI.
fn ui_binary() -> Result<PathBuf, Fail> {
    let exe = std::env::current_exe().map_err(|_| backend("this executable has no path".into()))?;
    let dir = exe
        .parent()
        .ok_or_else(|| backend("this executable has no directory".into()))?;
    let ui = dir.join(format!("beamcraft-ui{}", std::env::consts::EXE_SUFFIX));
    if ui.is_file() {
        return Ok(ui);
    }
    Err(backend(
        "beamcraft-ui was not found next to beamcraft; the viewport render needs it".into(),
    ))
}

/// Parses a bounded decimal `u32`.
fn bounded(map: &BTreeMap<&str, &str>, key: &str) -> Result<Option<u32>, Fail> {
    match map.get(key) {
        None => Ok(None),
        Some(text) => text
            .parse::<u32>()
            .map(Some)
            .map_err(|_| invalid("width, height and samples are numbers")),
    }
}

/// Runs `screenshot-view`: drives `beamcraft-ui --screenshot` and relays its reply.
pub fn execute(args: &[OsString]) -> (Json, u8) {
    match run(args) {
        Ok((json, exit)) => (json, exit),
        Err(e) => (
            Json::object([
                ("outcome", Json::text("rejected")),
                ("code", Json::text(e.code.as_str())),
                ("message", Json::text(e.message)),
                ("observed_context", Json::Null),
                ("diagnostics", Json::Array(vec![])),
            ]),
            match e.code {
                ErrorCode::BackendUnavailable => 3,
                _ => 2,
            },
        ),
    }
}

fn run(args: &[OsString]) -> Result<(Json, u8), Fail> {
    let args: Vec<&str> = args
        .iter()
        .skip(1)
        .map(|a| {
            a.to_str()
                .filter(|s| s.len() <= 4096)
                .ok_or_else(|| invalid("argument is not bounded UTF-8"))
        })
        .collect::<Result<_, _>>()?;
    if !args.len().is_multiple_of(2) || args.len() > 26 {
        return Err(invalid("options are KEY VALUE pairs"));
    }
    let mut options = BTreeMap::new();
    for pair in args.as_chunks::<2>().0 {
        let [key, value] = pair;
        if !matches!(
            *key,
            "--store"
                | "--out"
                | "--width"
                | "--height"
                | "--samples"
                | "--yaw"
                | "--pitch"
                | "--projection"
                | "--zoom"
                | "--pan"
                | "--colour-set"
                | "--background"
                | "--grids"
        ) || options.insert(*key, *value).is_some()
        {
            return Err(invalid("unknown or repeated option"));
        }
    }
    let store = options
        .get("--store")
        .ok_or_else(|| invalid("--store is required"))?;
    let out = options
        .get("--out")
        .ok_or_else(|| invalid("--out is required"))?;
    let width = bounded(&options, "--width")?;
    let height = bounded(&options, "--height")?;
    let samples = match options.get("--samples") {
        None => None,
        Some(&value @ ("1" | "4")) => Some(value),
        Some(_) => return Err(invalid("--samples is 1 or 4")),
    };
    let grids = match options.get("--grids") {
        None => None,
        Some(&value @ ("true" | "false")) => Some(value),
        Some(_) => return Err(invalid("--grids is true or false")),
    };
    // The camera angles are relayed verbatim; the ui validates them again, so a bad value is
    // refused there with its own message instead of being re-implemented here.
    // Checked before the render runs, so a refused request never reaches the GPU.
    if std::fs::exists(out).map_err(|e| backend(e.to_string()))? {
        return Err(invalid("the output file exists"));
    }
    let ui = ui_binary()?;
    let mut spawn = Command::new(&ui);
    // The child inherits this process's directory, so a relative --out resolves as the caller
    // passed it.
    spawn.arg("--store").arg(store);
    spawn.arg("--screenshot").arg(out);
    if let Some(width) = width {
        spawn.arg("--width").arg(width.to_string());
    }
    if let Some(height) = height {
        spawn.arg("--height").arg(height.to_string());
    }
    if let Some(samples) = samples {
        spawn.arg("--samples").arg(samples);
    }
    if let Some(yaw) = options.get("--yaw") {
        spawn.arg("--yaw").arg(yaw);
    }
    if let Some(pitch) = options.get("--pitch") {
        spawn.arg("--pitch").arg(pitch);
    }
    if let Some(projection) = options.get("--projection") {
        spawn.arg("--projection").arg(projection);
    }
    if let Some(zoom) = options.get("--zoom") {
        spawn.arg("--zoom").arg(zoom);
    }
    if let Some(pan) = options.get("--pan") {
        spawn.arg("--pan").arg(pan);
    }
    // The colour set and the background are relayed verbatim too; the ui validates the set
    // name or file and the light|dark value, so a bad value is refused there with its own
    // message instead of being re-implemented here.
    if let Some(colour_set) = options.get("--colour-set") {
        spawn.arg("--colour-set").arg(colour_set);
    }
    if let Some(background) = options.get("--background") {
        spawn.arg("--background").arg(background);
    }
    if let Some(grids) = grids {
        spawn.arg("--grids").arg(grids);
    }
    let output = spawn.output().map_err(|e| backend(e.to_string()))?;
    let stdout = bounded_stdout(&output.stdout)?;
    match output.status.code() {
        Some(0) => {
            let reply = decode_reply(&stdout)?;
            let outcome = reply
                .get("outcome")
                .ok()
                .and_then(|v| v.string().ok())
                .map(str::to_owned);
            if outcome.as_deref() == Some("screenshot_saved") {
                Ok((reply, 0))
            } else {
                Err(backend(
                    "beamcraft-ui answered without a saved screenshot".into(),
                ))
            }
        }
        // The ui exits 3 for a missing adapter or a graphics failure, 2 for input it refuses.
        Some(3) => Err(backend(reply_message(
            &stdout,
            "the viewport render failed",
        ))),
        // A rejection the ui relayed from the CLI carries the CLI's own code, message and
        // diagnostics: re-issue them unchanged (issue 555 finding 6). Anything else the ui
        // refused is its own input refusal.
        Some(_) => match relayed_rejection(&stdout) {
            Some(rejection) => Ok(rejection),
            None => Err(Fail {
                code: ErrorCode::InvalidArgument,
                message: reply_message(&stdout, "beamcraft-ui refused the screenshot"),
            }),
        },
        None => Err(backend("beamcraft-ui was terminated by a signal".into())),
    }
}

/// The first line of the ui's standard output, bounded; the ui prints exactly one JSON object.
fn bounded_stdout(raw: &[u8]) -> Result<String, Fail> {
    let text = String::from_utf8_lossy(raw);
    let line = text.lines().next().unwrap_or_default();
    if line.len() > REPLY_CAP {
        return Err(backend("beamcraft-ui reply is too large".into()));
    }
    Ok(line.to_owned())
}

/// Parses the ui's JSON reply, refusing anything that does not decode.
fn decode_reply(line: &str) -> Result<Json, Fail> {
    beamcraft_json::decode(line.as_bytes(), &Limits::DEFAULT)
        .map_err(|_| backend("beamcraft-ui reply is not valid JSON".into()))
}

/// The ui's reply as this command's own rejection when it names a stable F04 code: the code,
/// the message and the diagnostics are kept, and the exit code follows the code.
fn relayed_rejection(line: &str) -> Option<(Json, u8)> {
    let reply = decode_reply(line).ok()?;
    let code = reply
        .get("code")
        .ok()
        .and_then(|v| v.string().ok())
        .and_then(ErrorCode::parse)?;
    let message = reply
        .get("message")
        .ok()
        .and_then(|v| v.string().ok())
        .unwrap_or_default()
        .to_owned();
    let diagnostics = match reply.get("diagnostics").ok() {
        Some(Json::Array(items)) => Json::Array(items.clone()),
        _ => Json::Array(vec![]),
    };
    Some((
        Json::object([
            ("outcome", Json::text("rejected")),
            ("code", Json::text(code.as_str())),
            ("message", Json::text(message)),
            ("observed_context", Json::Null),
            ("diagnostics", diagnostics),
        ]),
        beamcraft_host::control::HostError::logical(code).exit_code(),
    ))
}

/// The reply's `message` field, or a fallback when the reply carries none.
fn reply_message(line: &str, fallback: &str) -> String {
    let owned = decode_reply(line).ok().and_then(|reply| {
        reply
            .get("message")
            .ok()
            .and_then(|v| v.string().ok())
            .map(str::to_owned)
    });
    owned.unwrap_or_else(|| fallback.to_owned())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn a_relayed_cli_rejection_keeps_its_code_message_and_diagnostics() {
        // Issue 555 finding 6: the ui relays the CLI's refusal; this command re-issues it with
        // the same code (and the code's exit), message and diagnostics, not INVALID_ARGUMENT.
        let line = r#"{"outcome":"rejected","code":"UNSUPPORTED_VERSION","message":"layer schema digest mismatch","diagnostics":[{"schema":null}]}"#;
        let (json, exit) = relayed_rejection(line).unwrap();
        assert_eq!(exit, 2);
        assert_eq!(
            json.get("code").unwrap().string().unwrap(),
            "UNSUPPORTED_VERSION"
        );
        assert_eq!(
            json.get("message").unwrap().string().unwrap(),
            "layer schema digest mismatch"
        );
        assert_eq!(json.get("diagnostics").unwrap().array().unwrap().len(), 1);
        // The ui's own refusal names no code: it stays this command's input refusal.
        assert!(relayed_rejection(r#"{"outcome":"rejected","message":"bad width"}"#).is_none());
        assert!(relayed_rejection(r#"{"code":"NOT_A_CODE","message":"m"}"#).is_none());
    }
}
