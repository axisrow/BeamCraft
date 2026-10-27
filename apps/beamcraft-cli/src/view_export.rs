// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! V03 headless command `export-view`: the pinned view snapshot of the verified head revision as
//! one engine bundle (`beamcraft_engine::query::bundle`), the input of the native viewport.
//!
//! `--out -` writes the raw bundle to standard output and nothing else (the viewport reads it
//! through a pipe); `--out FILE` creates a new file (never overwrites) and prints a JSON summary.
//!
//! `--with-definitions true` and `--with-element ID` (only with `--out -`) add replies taken from
//! the same verified inspection, so one process and one inspection serve a whole viewport
//! refresh and everything it shows belongs to the exported revision (issue 26, finding 2).
//! Standard output is then a frame: [`FRAME_MAGIC`]; the length of the definitions reply as a
//! little-endian `u32` and that reply byte for byte as `beamcraft definitions --store ROOT`
//! prints it; the length and the reply of `beamcraft inspect-elements --store ROOT --element ID`
//! the same way; then the bundle. A length of zero is a reply that was not asked for.
//! `--with-grids true` selects [`GRID_FRAME_MAGIC`] and adds a third length-prefixed reply,
//! containing `list-grids` display data from that same inspection, before the bundle.

use crate::plate_cmd::{definitions_json, id_error, inspection_json};
use crate::report::report;
use beamcraft_cli::plate::{PlateError, inspect};
use beamcraft_host::{revision::RevisionStore, storage::BlobStore};
use beamcraft_json::Json;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::ElementId;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::Write;

/// The first bytes of a framed reply (`--with-definitions true` or `--with-element ID`).
pub const FRAME_MAGIC: &[u8; 8] = b"BCVXFRM1";
/// Version 2 adds pinned grid display data as a third length-prefixed reply.
pub const GRID_FRAME_MAGIC: &[u8; 8] = b"BCVXFRM2";
/// Version 3 adds a fourth JSON presentation section; the engineering bundle stays unchanged.
pub const PRESENTATION_FRAME_MAGIC: &[u8; 8] = b"BCVXFRM3";

/// What `execute` produced.
pub enum Output {
    /// A JSON object for standard output, and the exit code.
    Json(Json, u8),
    /// Raw bundle bytes for standard output.
    Raw(Vec<u8>),
    /// A frame: the magic and bounded length-prefixed replies, then the bundle.
    Framed {
        /// Magic, lengths and replies.
        head: Vec<u8>,
        /// The bundle.
        bundle: Vec<u8>,
    },
}

/// A reply exactly as the command that owns it prints it (the CLI's `print_json`).
fn printed(result: Result<Json, PlateError>) -> Vec<u8> {
    let (json, _) = report(result);
    json.encode(&beamcraft_json::Limits::DEFAULT)
        .unwrap_or_else(|_| {
            // What `print_json` prints when a reply exceeds the encoder budget.
            b"{\"outcome\":\"rejected\",\"code\":\"LIMIT_EXCEEDED\"}\n".to_vec()
        })
}

/// Appends `reply` with its length; `None` is a reply that was not asked for.
fn section(head: &mut Vec<u8>, reply: Option<&[u8]>) -> Result<(), PlateError> {
    let reply = reply.unwrap_or_default();
    let length = u32::try_from(reply.len()).map_err(|_| PlateError {
        rule_id: None,
        code: ErrorCode::LimitExceeded,
        message: "a framed reply is too large",
    })?;
    head.extend_from_slice(&length.to_le_bytes());
    head.extend_from_slice(reply);
    Ok(())
}

fn invalid(message: &'static str) -> PlateError {
    PlateError {
        rule_id: None,
        code: ErrorCode::InvalidArgument,
        message,
    }
}

/// Runs `export-view`.
pub fn execute(args: &[OsString]) -> Output {
    match run(args) {
        Ok(output) => output,
        Err(e) => Output::Json(
            Json::object([
                ("outcome", Json::text("rejected")),
                ("code", Json::text(e.code.as_str())),
                ("message", Json::text(e.message)),
                ("observed_context", Json::Null),
                (
                    "diagnostics",
                    Json::Array(crate::plate_cmd::domain_diagnostics(&e)),
                ),
            ]),
            match e.code {
                ErrorCode::BackendUnavailable => 3,
                _ => 2,
            },
        ),
    }
}

fn run(args: &[OsString]) -> Result<Output, PlateError> {
    let args: Vec<&str> = args
        .iter()
        .skip(1)
        .map(|a| {
            a.to_str()
                .filter(|s| s.len() <= 4096)
                .ok_or_else(|| invalid("argument is not bounded UTF-8"))
        })
        .collect::<Result<_, _>>()?;
    if !args.len().is_multiple_of(2) || args.len() > 28 {
        return Err(invalid("options are KEY VALUE pairs"));
    }
    let mut options = BTreeMap::new();
    for pair in args.as_chunks::<2>().0 {
        let [key, value] = pair;
        if !matches!(
            *key,
            "--store"
                | "--out"
                | "--with-definitions"
                | "--with-element"
                | "--with-grids"
                | "--view"
                | "--render-mode"
                | "--select"
                | "--select-file"
                | "--representation"
                | "--overlay"
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
    let with_definitions = match options.get("--with-definitions") {
        None | Some(&"false") => false,
        Some(&"true") => true,
        Some(_) => return Err(invalid("--with-definitions is true or false")),
    };
    let with_element = match options.get("--with-element") {
        Some(text) => Some(ElementId::parse(text).map_err(id_error)?),
        None => None,
    };
    let with_grids = match options.get("--with-grids") {
        None | Some(&"false") => false,
        Some(&"true") => true,
        Some(_) => return Err(invalid("--with-grids is true or false")),
    };
    let selected_view = options
        .get("--view")
        .map(|s| ElementId::parse(s).map_err(id_error))
        .transpose()?;
    let mode = options
        .get("--render-mode")
        .map(|s| {
            beamcraft_engine::render::Mode::parse(s).ok_or_else(|| invalid("unknown render mode"))
        })
        .transpose()?
        .unwrap_or_default();
    let selected = match (options.get("--select"), options.get("--select-file")) {
        (Some(_), Some(_)) => return Err(invalid("select and select-file are mutually exclusive")),
        (Some(s), None) if !s.is_empty() => s
            .split(',')
            .map(|s| ElementId::parse(s).map_err(id_error))
            .collect::<Result<Vec<_>, _>>()?,
        (None, Some(path)) => beamcraft_cli::display::selection_file(path)?,
        _ => Vec::new(),
    };
    let representation = options
        .get("--representation")
        .map(|s| ElementId::parse(s).map_err(id_error))
        .transpose()?;
    let overlay = options
        .get("--overlay")
        .map(|path| {
            let bytes = beamcraft_cli::source_file::read_file(path, 1024 * 1024)
                .map_err(|c| display_error(c, "overlay file is not usable"))?;
            let j = beamcraft_json::decode(&bytes, &beamcraft_cli::display::LIMITS)
                .map_err(|_| invalid("invalid overlay JSON"))?;
            beamcraft_cli::representations::overlay(&j)
        })
        .transpose()?
        .unwrap_or_default();
    let display_requested = representation.is_some()
        || options.contains_key("--overlay")
        || selected_view.is_some()
        || options.contains_key("--render-mode")
        || options.contains_key("--select")
        || options.contains_key("--select-file");
    let replies_framed = with_definitions || with_element.is_some() || with_grids;
    let framed = replies_framed || display_requested;
    // Checked before the store is opened, so a refused request never creates the file.
    if replies_framed && *out != "-" {
        return Err(invalid("a framed reply needs --out -"));
    }
    let blobs = BlobStore::new(store).map_err(|e| PlateError {
        rule_id: None,
        code: e.code(),
        message: "store root is not usable",
    })?;
    let inspection = if selected_view.is_some() {
        beamcraft_cli::plane_views::current(std::path::Path::new(store))?
    } else {
        inspect(&RevisionStore::new(blobs))?
    };
    let presentation = display_requested
        .then(|| {
            presentation_json(
                &inspection,
                std::path::Path::new(store),
                selected_view,
                mode,
                &selected,
                representation,
                &overlay,
            )
        })
        .transpose()?;
    inspection.require_evaluated_geometry()?;
    if let Some(error) = inspection.reference_view_error {
        return Err(error);
    }
    let snapshot = &inspection.snapshot;
    let bundle = snapshot.to_bundle().map_err(|e| PlateError {
        rule_id: None,
        code: e.f04_code(),
        message: "the view snapshot could not be bundled",
    })?;
    let mut frame_head = Vec::new();
    if framed {
        let definitions =
            with_definitions.then(|| printed(definitions_json(&inspection, &BTreeMap::new())));
        let element = with_element.map(|id| printed(inspection_json(&inspection, Some(id))));
        frame_head.extend_from_slice(if presentation.is_some() {
            PRESENTATION_FRAME_MAGIC
        } else if with_grids {
            GRID_FRAME_MAGIC
        } else {
            FRAME_MAGIC
        });
        section(&mut frame_head, definitions.as_deref())?;
        section(&mut frame_head, element.as_deref())?;
        if with_grids || presentation.is_some() {
            let grids = with_grids.then(|| {
                printed(crate::grid_cmd::views_json(&inspection.grids).map(|grids| {
                    Json::object([("outcome", Json::text("grids")), ("grids", grids)])
                }))
            });
            section(&mut frame_head, grids.as_deref())?;
        }
        if let Some(p) = &presentation {
            let bytes = p
                .encode(&beamcraft_cli::display::LIMITS)
                .map_err(|_| invalid("presentation reply exceeds its limit"))?;
            section(&mut frame_head, Some(&bytes))?;
        }
    }
    if *out == "-" {
        return Ok(if framed {
            Output::Framed {
                head: frame_head,
                bundle,
            }
        } else {
            Output::Raw(bundle)
        });
    }
    let total_bytes = frame_head.len().saturating_add(bundle.len());
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(out)
        .map_err(|_| PlateError {
            rule_id: None,
            code: ErrorCode::InvalidArgument,
            message: "the output file exists or cannot be created",
        })?;
    file.write_all(&frame_head)
        .and_then(|()| file.write_all(&bundle))
        .and_then(|()| file.sync_all())
        .map_err(|_| PlateError {
            rule_id: None,
            code: ErrorCode::BackendUnavailable,
            message: "the bundle could not be written",
        })?;
    let mut reply = Json::object([
        ("outcome", Json::text("view_exported")),
        ("revision_id", Json::text(&inspection.revision.revision_id)),
        ("token", Json::text(snapshot.token().as_str())),
        ("elements", Json::text(snapshot.len().to_string())),
        ("meshes", Json::text(snapshot.meshes().len().to_string())),
        ("bytes", Json::text(total_bytes.to_string())),
    ]);
    if let (Some(p), Json::Object(fields)) = (presentation, &mut reply) {
        let mut p = p;
        if let Json::Object(f) = &mut p {
            for key in ["visible_elements", "selected_elements", "appearances"] {
                if let Some(Json::Array(a)) = f.remove(key) {
                    f.insert(format!("{key}_count"), Json::uint(a.len() as u64));
                }
            }
        }
        fields.insert("presentation".into(), p);
    }
    Ok(Output::Json(reply, 0))
}

fn presentation_json(
    inspection: &beamcraft_cli::plate::Inspection,
    dir: &std::path::Path,
    id: Option<ElementId>,
    mode: beamcraft_engine::render::Mode,
    selected: &[ElementId],
    representation: Option<ElementId>,
    overlay: &BTreeMap<ElementId, beamcraft_engine::representation::Appearance>,
) -> Result<Json, PlateError> {
    let mut ids = beamcraft_engine::view_clipping::visible_elements(&inspection.snapshot, &[])?;
    let mut view_json = Json::Null;
    let mut version = Json::Null;
    let mut filter_id = None;
    let mut planes = Vec::new();
    let mut personal = None;
    let mut representation_id = representation;
    if let Some(id) = id {
        let views = beamcraft_cli::saved_views::read(dir)?;
        let view = views
            .document
            .views
            .iter()
            .find(|v| v.id == id)
            .ok_or_else(|| display_error(ErrorCode::NotFound, "saved view not found"))?;
        let restored = beamcraft_cli::plane_views::restore_against(&views, view, inspection)?;
        if restored.unresolved {
            return Err(display_error(
                ErrorCode::DependencyUnavailable,
                "saved view source is unresolved",
            ));
        }
        planes = beamcraft_cli::view_clipping::effective(&views, &restored.view)?;
        ids = beamcraft_engine::view_clipping::visible_elements(&inspection.snapshot, &planes)?;
        filter_id = beamcraft_cli::filters::binding(&views, id)?;
        if let Some(f) = filter_id {
            let matching =
                beamcraft_cli::filters::matching(&views, f, &inspection.user_attributes)?;
            ids.retain(|id| matching.binary_search(id).is_ok());
        }
        if representation_id.is_none() {
            representation_id = beamcraft_cli::representations::binding(&views, id)?;
        }
        view_json = crate::saved_views_cmd::view_json(&restored.view)?;
        version = views
            .version
            .map_or(Json::Null, |v| Json::text(v.to_string()));
        personal = Some(views);
    }
    let render = beamcraft_engine::render::query(&inspection.snapshot, mode, selected, &ids)
        .map_err(|e| display_error(e.f04_code(), "render selection is invalid"))?;
    for id in overlay.keys() {
        if inspection.snapshot.element(id).is_none() {
            return Err(display_error(
                ErrorCode::NotFound,
                "overlay element is not in the snapshot",
            ));
        }
    }
    let (by_class, rules) = if let Some(id) = representation_id {
        if personal.is_none() {
            personal = Some(beamcraft_cli::saved_views::read(dir)?);
        }
        let views = personal
            .as_ref()
            .ok_or_else(|| invalid("personal views unavailable"))?;
        version = views
            .version
            .map_or(Json::Null, |v| Json::text(v.to_string()));
        let set = beamcraft_cli::representations::saved(views, id)?;
        (
            set.by_class,
            beamcraft_cli::representations::compile(views, &set, &inspection.user_attributes)?,
        )
    } else {
        (false, Vec::new())
    };
    let classes = inspection
        .user_attributes
        .elements
        .iter()
        .filter_map(|(id, r)| r.class.map(|c| (*id, c)))
        .collect();
    let appearances = beamcraft_engine::representation::query(
        &beamcraft_cli::filters::dataset(&inspection.user_attributes)?,
        &render.visible,
        &classes,
        beamcraft_engine::representation::Appearance {
            color: render.color,
            transparency: render.transparency,
        },
        by_class,
        &rules,
        overlay,
    )?;
    Ok(Json::object([
        ("version", Json::uint(3)),
        (
            "representation_id",
            representation_id.map_or(Json::Null, |id| Json::text(id.to_string())),
        ),
        ("overlay_count", Json::uint(overlay.len() as u64)),
        (
            "appearances",
            Json::Array(
                appearances
                    .into_iter()
                    .map(|(id, a)| {
                        let mut j = beamcraft_cli::representations::appearance_json(a);
                        if let Json::Object(f) = &mut j {
                            f.insert("id".into(), Json::text(id.to_string()));
                        }
                        j
                    })
                    .collect(),
            ),
        ),
        ("render", beamcraft_cli::display::settings(&render)),
        (
            "selected_elements",
            Json::Array(
                render
                    .selected
                    .iter()
                    .map(|id| Json::text(id.to_string()))
                    .collect(),
            ),
        ),
        (
            "filter_id",
            filter_id.map_or(Json::Null, |id| Json::text(id.to_string())),
        ),
        (
            "snapshot_token",
            Json::text(inspection.snapshot.token().as_str()),
        ),
        (
            "model_manifest",
            Json::text(inspection.revision.manifest_digest.to_string()),
        ),
        ("personal_version", version),
        ("view", view_json),
        (
            "clip_planes",
            beamcraft_cli::view_clipping::planes_json(&planes)?,
        ),
        (
            "visible_elements",
            Json::Array(
                render
                    .visible
                    .into_iter()
                    .map(|id| Json::text(id.to_string()))
                    .collect(),
            ),
        ),
    ]))
}

fn display_error(code: ErrorCode, message: &'static str) -> PlateError {
    PlateError {
        code,
        message,
        rule_id: None,
    }
}
