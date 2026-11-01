// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! V12 named views: the saved-view session file, headlessly. A view is display state, so these
//! commands never write a model store; the only store access is the read-only grid lookup that
//! resolves a grid-line plane view at save time.

use beamcraft_cli::grid::grids;
use beamcraft_cli::source_file::read_file;
use beamcraft_engine::views::{self, NamedView, PlaneSpec, ViewBook, ViewPose, WorkArea};
use beamcraft_host::revision::RevisionStore;
use beamcraft_host::storage::BlobStore;
use beamcraft_host::views as views_json;
use beamcraft_json::{Json, Limits};
use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;
use session::Refusal;
use std::collections::BTreeMap;
use std::ffi::OsString;

type Run<T> = Result<T, Refusal>;

/// The session file is a small JSON document.
const FILE_CAP: usize = 1024 * 1024;

fn from_engine(e: CommandError) -> Refusal {
    Refusal {
        rule_id: e.rule_id,
        code: e.code,
        message: e.message,
    }
}

fn invalid(message: &'static str) -> Refusal {
    Refusal {
        rule_id: None,
        code: ErrorCode::InvalidArgument,
        message,
    }
}

fn backend(message: &'static str) -> Refusal {
    Refusal {
        rule_id: None,
        code: ErrorCode::BackendUnavailable,
        message,
    }
}

fn unknown() -> Refusal {
    Refusal {
        rule_id: None,
        code: ErrorCode::NotFound,
        message: "VIEW_UNKNOWN",
    }
}

fn point(text: &str) -> Run<[f64; 3]> {
    let numbers = text
        .split(|c: char| c == ',' || c.is_ascii_whitespace())
        .filter(|v| !v.is_empty())
        .map(|v| v.parse::<f64>().map_err(|_| invalid("enter three numbers")))
        .collect::<Run<Vec<_>>>()?;
    match numbers.as_slice() {
        [a, b, c] if [a, b, c].iter().all(|v| v.is_finite()) => Ok([*a, *b, *c]),
        _ => Err(invalid("expected three finite coordinates")),
    }
}

fn number(text: Option<&str>) -> Run<f64> {
    text.and_then(|t| t.parse::<f64>().ok())
        .filter(|v| v.is_finite())
        .ok_or_else(|| invalid("enter a finite number"))
}

fn flag(text: Option<&str>) -> Run<bool> {
    match text {
        None => Ok(false),
        Some("true") => Ok(true),
        Some("false") => Ok(false),
        Some(_) => Err(invalid("booleans are true or false")),
    }
}

/// Reads the session file; a missing or empty file is an empty book when `allow_missing`.
fn load(path: &str, allow_missing: bool) -> Run<ViewBook> {
    let bytes = match read_file(path, FILE_CAP) {
        Ok(bytes) => bytes,
        Err(ErrorCode::NotFound) if allow_missing => return Ok(ViewBook::new()),
        Err(code) => {
            return Err(Refusal {
                rule_id: None,
                code,
                message: "the view file could not be read",
            });
        }
    };
    if bytes.is_empty() {
        return Ok(ViewBook::new());
    }
    let json = beamcraft_json::decode(&bytes, &Limits::DEFAULT)
        .map_err(|_| invalid("the view file is not valid JSON"))?;
    views_json::book_from_json(&json).map_err(from_engine)
}

/// Writes the session file through a sibling temporary file, so a failed write keeps the old
/// content readable.
fn store(path: &str, book: &ViewBook) -> Run<()> {
    let bytes = views_json::book_to_json(book)
        .map_err(from_engine)?
        .encode(&Limits::DEFAULT)
        .map_err(|_| invalid("the view file content exceeds the JSON limits"))?;
    let temp = format!("{path}.tmp");
    std::fs::write(&temp, &bytes).map_err(|_| backend("the view file could not be written"))?;
    std::fs::rename(&temp, path).map_err(|_| backend("the view file could not be replaced"))
}

/// Runs one named-view command through the common JSON result protocol.
pub fn execute(args: &[OsString]) -> (Json, u8) {
    crate::report::report(run(args))
}

fn run(args: &[OsString]) -> Run<Json> {
    let args: Vec<&str> = args
        .iter()
        .map(|a| {
            a.to_str()
                .filter(|s| s.len() <= 4096)
                .ok_or_else(|| invalid("argument is not UTF-8"))
        })
        .collect::<Run<_>>()?;
    let (command, tail) = args
        .split_first()
        .ok_or_else(|| invalid("a command is required"))?;
    let allowed: &[&str] = match *command {
        "list-views" => &["--file", "--store"],
        "save-view" => &[
            "--file",
            "--store",
            "--name",
            "--target",
            "--yaw",
            "--pitch",
            "--distance",
            "--ortho",
            "--work-area",
            "--plane-grid",
            "--plane-line",
            "--plane-reverse",
            "--plane-level",
            "--from",
            "--to",
            "--centre",
        ],
        "remove-view" | "apply-view" => &["--file", "--store", "--name"],
        _ => return Err(invalid("unknown named-view command")),
    };
    if !tail.len().is_multiple_of(2) || tail.len() > 32 {
        return Err(invalid("options come in name-value pairs"));
    }
    let mut options = BTreeMap::new();
    for pair in tail.as_chunks::<2>().0 {
        let [key, value] = pair;
        if !allowed.contains(key) || options.insert(*key, *value).is_some() {
            return Err(invalid("unknown or repeated option"));
        }
    }
    let file = options
        .get("--file")
        .ok_or_else(|| invalid("--file is required"))?;
    let name = options.get("--name").copied();
    match *command {
        "list-views" => {
            let book = load(file, true)?;
            list_json(&book)
        }
        "save-view" => {
            let name = name.ok_or_else(|| invalid("--name is required"))?;
            let (view, resolved) = save_view(&options, name)?;
            let mut book = load(file, true)?;
            book.save(view.clone()).map_err(from_engine)?;
            store(file, &book)?;
            Ok(Json::object([
                ("outcome", Json::text("view_saved")),
                ("name", Json::text(&view.name)),
                ("view_count", Json::uint(book.len() as u64)),
                ("pose", pose_json(&resolved)),
                ("work_area", work_area_json(view.work_area.as_ref())?),
            ]))
        }
        "remove-view" => {
            let name = name.ok_or_else(|| invalid("--name is required"))?;
            let mut book = load(file, false)?;
            let view = book.remove(name).ok_or_else(unknown)?;
            store(file, &book)?;
            Ok(Json::object([
                ("outcome", Json::text("view_removed")),
                ("name", Json::text(view.name)),
                ("view_count", Json::uint(book.len() as u64)),
            ]))
        }
        _ => {
            let name = name.ok_or_else(|| invalid("--name is required"))?;
            let book = load(file, false)?;
            let view = book.get(name).ok_or_else(unknown)?;
            Ok(Json::object([
                ("outcome", Json::text("view_applied")),
                ("name", Json::text(&view.name)),
                ("pose", pose_json(&view.pose)),
                ("work_area", work_area_json(view.work_area.as_ref())?),
                (
                    "plane",
                    view.plane
                        .as_ref()
                        .map_or(Json::Null, views_json::plane_to_json),
                ),
            ]))
        }
    }
}

fn pose_json(pose: &ViewPose) -> Json {
    views_json::pose_to_json(pose).unwrap_or(Json::Null)
}

/// The work area as JSON, `Null` when the view carries none. An admitted box always encodes.
fn work_area_json(area: Option<&WorkArea>) -> Run<Json> {
    area.map(views_json::work_area_to_json)
        .transpose()
        .map_err(from_engine)
        .map(|json| json.unwrap_or(Json::Null))
}

/// Every saved view with its resolved state, in name order.
fn list_json(book: &ViewBook) -> Run<Json> {
    let views = book
        .iter()
        .map(|v| {
            Ok(Json::object([
                ("name", Json::text(&v.name)),
                (
                    "plane",
                    v.plane
                        .as_ref()
                        .map_or(Json::Null, views_json::plane_to_json),
                ),
                ("pose", pose_json(&v.pose)),
                ("work_area", work_area_json(v.work_area.as_ref())?),
            ]))
        })
        .collect::<Run<Vec<_>>>()?;
    Ok(Json::object([
        ("outcome", Json::text("views")),
        ("view_count", Json::uint(book.len() as u64)),
        ("views", Json::Array(views)),
    ]))
}

/// Builds the named view the save options describe: an orbit pose directly, or a plane spec
/// resolved once through the store's grids.
fn save_view(options: &BTreeMap<&str, &str>, name: &str) -> Run<(NamedView, ViewPose)> {
    let get = |key| options.get(key).copied();
    let grid_line = get("--plane-grid").is_some() || get("--plane-line").is_some();
    let level = get("--plane-level").is_some();
    let two_points = get("--from").is_some() || get("--to").is_some();
    // An orbit pose: any pose field, projection included, so `--ortho` alone is a mode choice
    // rather than a silently ignored option on a plane view.
    let orbit = get("--target").is_some()
        || get("--yaw").is_some()
        || get("--pitch").is_some()
        || get("--ortho").is_some();
    if [grid_line, level, two_points, orbit]
        .iter()
        .filter(|used| **used)
        .count()
        > 1
    {
        return Err(invalid("save exactly one orbit pose or plane view"));
    }
    if orbit && get("--centre").is_some() {
        return Err(invalid("--centre belongs to a plane view"));
    }
    let centre = match get("--centre") {
        Some(text) => point(text)?,
        None => [0.0; 3],
    };
    let distance = number(get("--distance"))?;
    let work_area = match get("--work-area") {
        Some(text) => {
            let values = text
                .split(',')
                .map(|v| v.trim().parse::<f64>())
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| invalid("work area is six numbers"))?;
            if values.iter().any(|v| !v.is_finite()) {
                return Err(invalid("work area is six finite numbers"));
            }
            let values: [f64; 6] =
                <[f64; 6]>::try_from(values).map_err(|_| invalid("work area is six numbers"))?;
            let (low, high) = values.split_at(3);
            let low = <[f64; 3]>::try_from(low).map_err(|_| invalid("work area is six numbers"))?;
            let high =
                <[f64; 3]>::try_from(high).map_err(|_| invalid("work area is six numbers"))?;
            Some(WorkArea::new(low, high).map_err(from_engine)?)
        }
        None => None,
    };
    let (plane, pose) = if grid_line {
        let store_root =
            get("--store").ok_or_else(|| invalid("--store is required to resolve a grid line"))?;
        let store = RevisionStore::new(BlobStore::new(store_root).map_err(|e| Refusal {
            rule_id: None,
            code: e.code(),
            message: "store root is not usable",
        })?);
        let grids = grids(&store)?;
        let spec = PlaneSpec::GridLine {
            grid: get("--plane-grid")
                .ok_or_else(|| invalid("--plane-grid is required"))?
                .to_owned(),
            line: get("--plane-line")
                .ok_or_else(|| invalid("--plane-line is required"))?
                .to_owned(),
            reverse: flag(get("--plane-reverse"))?,
        };
        let pose = views::resolve_plane(&spec, &grids, centre, distance).map_err(from_engine)?;
        (Some(spec), pose)
    } else if level {
        let spec = PlaneSpec::Level;
        let pose = views::resolve_plane(&spec, &[], centre, distance).map_err(from_engine)?;
        (Some(spec), pose)
    } else if two_points {
        let spec = PlaneSpec::TwoPoints {
            from_mm: point(get("--from").ok_or_else(|| invalid("--from is required"))?)?,
            to_mm: point(get("--to").ok_or_else(|| invalid("--to is required"))?)?,
        };
        let pose = views::resolve_plane(&spec, &[], centre, distance).map_err(from_engine)?;
        (Some(spec), pose)
    } else {
        let pose = ViewPose::new(
            point(get("--target").unwrap_or("0,0,0"))?,
            number(get("--yaw"))?,
            number(get("--pitch"))?,
            number(get("--distance"))?,
            flag(get("--ortho"))?,
        )
        .map_err(from_engine)?;
        (None, pose)
    };
    let view = NamedView::new(name, plane, pose, work_area).map_err(from_engine)?;
    Ok((view, pose))
}
