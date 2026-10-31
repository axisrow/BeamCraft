// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Headless snapshot-view commands (V02). They call the engine API and add no computation.
//!
//! P01 does not exist, so the scene is a synthetic kernel plate scene: every fifth element is
//! labelled `support` and, with `--stand-in-simplified true`, every third element carries a
//! distinct stand-in simplified mesh. Both are marked in the output (`stand_ins`).

use beamcraft_engine::synthetic::{SyntheticEdit, apply_edit, plate_scene};
use beamcraft_engine::view_clipping::{self, ClipPlane, Clipping, WorkArea};
use beamcraft_engine::{
    Aabb, ChangeSet, ElementEntry, ElementId, KindFilter, Limits, Representation, Selection,
    SnapshotHistory, SnapshotToken, ViewCode, ViewError, ViewPin, ViewProjection, ViewSettings,
    ViewSnapshot,
};
use beamcraft_json::Json;
use beamcraft_model::digest::{ContentDigest, Sha256};
use beamcraft_model::error::ErrorCode;
use std::collections::BTreeMap;
use std::ffi::OsString;

/// Output bound: keeps the JSON inside the read profile's control limits (65,536 tokens; about
/// 700 elements were measured to fit, so 500 leaves margin for a long `--select` list).
const MAX_PLATES: u32 = 500;

type View<T> = Result<T, ViewError>;

fn invalid() -> ViewError {
    ViewError::new(ViewCode::ArgumentInvalid)
}

/// Maps a rejected cut record onto the closed view vocabulary.
fn view_of(e: beamcraft_model::command::CommandError) -> ViewError {
    ViewError::new(match e.code {
        ErrorCode::NonFiniteNumber => ViewCode::NonFinite,
        ErrorCode::LimitExceeded => ViewCode::ElementLimit,
        _ => ViewCode::ArgumentInvalid,
    })
}

/// Parses `px,py,pz,nx,ny,nz;...`, at most [`view_clipping::MAX_PLANES`] planes.
fn cut_planes(text: &str) -> View<Vec<ClipPlane>> {
    if text.len() > 1024 {
        return Err(invalid());
    }
    text.split(';')
        .map(|plane| {
            let numbers = plane
                .split(',')
                .map(|v| v.trim().parse::<f64>().map_err(|_| invalid()))
                .collect::<View<Vec<_>>>()?;
            let [px, py, pz, nx, ny, nz]: [f64; 6] = numbers
                .try_into()
                .map_err(|_| ViewError::new(ViewCode::ArgumentInvalid))?;
            ClipPlane::new([px, py, pz], [nx, ny, nz]).map_err(view_of)
        })
        .collect()
}

/// Parses `minx,miny,minz,maxx,maxy,maxz`.
fn work_area(text: &str) -> View<WorkArea> {
    let numbers = text
        .split(',')
        .map(|v| v.trim().parse::<f64>().map_err(|_| invalid()))
        .collect::<View<Vec<_>>>()?;
    let [a, b, c, d, e, f]: [f64; 6] = numbers
        .try_into()
        .map_err(|_| ViewError::new(ViewCode::ArgumentInvalid))?;
    WorkArea::new([a, b, c], [d, e, f]).map_err(view_of)
}

/// Runs one view command and returns the JSON result and the process exit code.
pub fn execute(args: &[OsString]) -> (Json, u8) {
    match run(args) {
        Ok(json) => (json, 0),
        Err(e) => (
            Json::object([
                ("outcome", Json::text("rejected")),
                ("code", Json::text(e.f04_code().as_str())),
                ("message", Json::text("view query rejected")),
                ("observed_context", Json::Null),
                (
                    "diagnostics",
                    Json::Array(vec![Json::object([(
                        "code",
                        Json::text(e.code().as_str()),
                    )])]),
                ),
            ]),
            2,
        ),
    }
}

fn run(args: &[OsString]) -> View<Json> {
    let args: Vec<&str> = args
        .iter()
        .map(|a| a.to_str().filter(|s| s.len() <= 4096).ok_or_else(invalid))
        .collect::<View<_>>()?;
    let (command, tail) = args.split_first().ok_or_else(invalid)?;
    let allowed: &[&str] = match *command {
        "snapshot-view" => &[
            "--plates",
            "--hide",
            "--representation",
            "--stand-in-simplified",
            "--select",
            "--revision",
            "--generation",
        ],
        "inspect-view-changes" => &[
            "--plates",
            "--edit",
            "--since",
            "--stand-in-simplified",
            "--revision",
            "--generation",
        ],
        "clip-view" => &[
            "--plates",
            "--cut",
            "--area",
            "--stand-in-simplified",
            "--revision",
            "--generation",
        ],
        _ => return Err(invalid()),
    };
    if !tail.len().is_multiple_of(2) || tail.len() > 16 {
        return Err(invalid());
    }
    let mut options = BTreeMap::new();
    for pair in tail.as_chunks::<2>().0 {
        let [key, value] = pair;
        if !allowed.contains(key) || options.insert(*key, *value).is_some() {
            return Err(invalid());
        }
    }
    let plates = options
        .get("--plates")
        .ok_or_else(invalid)?
        .parse::<u32>()
        .map_err(|_| invalid())?;
    if plates > MAX_PLATES {
        return Err(ViewError::new(ViewCode::ElementLimit));
    }
    let stand_in = match options.get("--stand-in-simplified").copied() {
        None | Some("false") => false,
        Some("true") => true,
        Some(_) => return Err(invalid()),
    };
    let revision = match options.get("--revision") {
        Some(text) => text.parse::<ContentDigest>().map_err(|_| invalid())?,
        None => Sha256::digest(b"beamcraft-synthetic-scene").map_err(|_| invalid())?,
    };
    let generation = match options.get("--generation") {
        Some(text) => text.parse::<u64>().map_err(|_| invalid())?,
        None => 1,
    };
    let pin = ViewPin::new(revision, generation);
    let scene = plate_scene(plates, stand_in)?;
    if *command == "clip-view" {
        let mut clip = Clipping::default();
        if let Some(text) = options.get("--cut") {
            clip.planes = cut_planes(text)?;
        }
        if let Some(text) = options.get("--area") {
            clip.area = Some(work_area(text)?);
        }
        if clip.planes.is_empty() && clip.area.is_none() {
            return Err(invalid());
        }
        let snapshot = ViewSnapshot::build(pin, scene, &Limits::default())?;
        let planes = clip.effective().map_err(view_of)?;
        let visible = view_clipping::visible_elements(&snapshot, &planes).map_err(view_of)?;
        // Non-destruction evidence: the same scene rebuilt after masking hashes to the
        // identical token, and the cut itself is absent from the pin.
        let rebuilt = ViewSnapshot::build(pin, plate_scene(plates, stand_in)?, &Limits::default())?;
        let unchanged = rebuilt.token().as_str() == snapshot.token().as_str();
        return clip_json(
            &snapshot,
            &planes,
            &visible,
            unchanged,
            options.contains_key("--area"),
            stand_in,
        );
    }
    if *command == "snapshot-view" {
        let snapshot = ViewSnapshot::build(pin, scene, &Limits::default())?;
        let settings = ViewSettings {
            hidden_kinds: options
                .get("--hide")
                .map_or(Ok(KindFilter::SHOW_ALL), |t| KindFilter::parse_list(t))?,
            representation: options
                .get("--representation")
                .map_or(Ok(Representation::Full), |t| {
                    Representation::parse(t).ok_or_else(invalid)
                })?,
        };
        let selection = options
            .get("--select")
            .map(|text| {
                let ids = text
                    .split(',')
                    .map(|id| ElementId::parse(id).map_err(ViewError::from))
                    .collect::<View<Vec<_>>>()?;
                Selection::from_ids(&snapshot, &ids)
            })
            .transpose()?;
        snapshot_json(&snapshot, &settings, selection.as_ref(), stand_in)
    } else {
        let edits = options
            .get("--edit")
            .ok_or_else(invalid)?
            .split(',')
            .map(|t| SyntheticEdit::parse(t).ok_or_else(invalid))
            .collect::<View<Vec<_>>>()?;
        let mut history = SnapshotHistory::new(2);
        let before = ViewSnapshot::build(pin, scene.clone(), &Limits::default())?;
        history.push(before.clone());
        let mut edited = scene;
        for edit in edits {
            apply_edit(&mut edited, edit, stand_in)?;
        }
        let next_pin = ViewPin::new(revision, generation.checked_add(1).ok_or_else(invalid)?);
        let after = ViewSnapshot::build(next_pin, edited, &Limits::default())?;
        history.push(after.clone());
        let since = match options.get("--since") {
            Some(text) => SnapshotToken::parse(text)?,
            None => before.token().clone(),
        };
        let delta = history.changes_since(&since)?;
        changes_json(&delta, &after, stand_in)
    }
}

fn num(value: f64) -> View<Json> {
    Json::number(value).map_err(|_| ViewError::new(ViewCode::NumericUncertain))
}

fn triple(values: [f64; 3]) -> View<Json> {
    Ok(Json::Array(
        values.into_iter().map(num).collect::<View<Vec<_>>>()?,
    ))
}

fn quaternion(values: [f64; 4]) -> View<Json> {
    Ok(Json::Array(
        values.into_iter().map(num).collect::<View<Vec<_>>>()?,
    ))
}

fn bounds_json(bounds: Option<Aabb>) -> View<Json> {
    bounds.map_or(Ok(Json::Null), |b| {
        Ok(Json::object([
            ("min", triple(b.min)?),
            ("max", triple(b.max)?),
        ]))
    })
}

fn pin_json(pin: ViewPin) -> Json {
    Json::object([
        ("revision", Json::text(pin.revision().to_string())),
        ("generation", Json::text(pin.generation().to_string())),
    ])
}

fn stand_ins(simplified: bool) -> Json {
    Json::object([
        ("support_kind", Json::Bool(true)),
        ("simplified", Json::Bool(simplified)),
    ])
}

fn mesh_json(snapshot: &ViewSnapshot, digest: &ContentDigest) -> Json {
    let entry = snapshot.mesh(digest);
    Json::object([
        ("digest", Json::text(digest.to_string())),
        (
            "bytes",
            Json::uint(entry.map_or(0, |m| m.bytes().len() as u64)),
        ),
        (
            "vertices",
            Json::uint(entry.map_or(0, |m| u64::from(m.vertex_count()))),
        ),
        (
            "triangles",
            Json::uint(entry.map_or(0, |m| u64::from(m.triangle_count()))),
        ),
    ])
}

/// The clip-view report: the effective half-spaces and the conservative visible mask, with
/// the token equality that evidences the cut changed no model or measurement data.
fn clip_json(
    snapshot: &ViewSnapshot,
    planes: &[ClipPlane],
    visible: &[ElementId],
    unchanged: bool,
    boxed: bool,
    simplified: bool,
) -> View<Json> {
    let plane = |p: &ClipPlane| -> View<Json> {
        Ok(Json::object([
            ("point_mm", triple(p.point_mm)?),
            ("normal", triple(p.normal)?),
        ]))
    };
    let planes = planes.iter().map(plane).collect::<View<Vec<_>>>()?;
    let visible = visible
        .iter()
        .map(|id| Json::text(id.to_string()))
        .collect::<Vec<_>>();
    Ok(Json::object([
        ("outcome", Json::text("view_clip")),
        ("synthetic", Json::Bool(true)),
        ("stand_ins", stand_ins(simplified)),
        ("pin", pin_json(snapshot.pin())),
        ("snapshot_token", Json::text(snapshot.token().as_str())),
        ("model_unchanged", Json::Bool(unchanged)),
        ("work_area_applied", Json::Bool(boxed)),
        ("plane_count", Json::uint(planes.len() as u64)),
        ("effective_planes", Json::Array(planes)),
        ("element_count", Json::uint(snapshot.len() as u64)),
        ("visible_count", Json::uint(visible.len() as u64)),
        (
            "clipped_count",
            Json::uint((snapshot.len() - visible.len()) as u64),
        ),
        ("visible", Json::Array(visible)),
    ]))
}

fn snapshot_json(
    snapshot: &ViewSnapshot,
    settings: &ViewSettings,
    selection: Option<&Selection>,
    simplified: bool,
) -> View<Json> {
    let projection: ViewProjection<'_> = snapshot.project(settings)?;
    let elements = projection
        .items()
        .iter()
        .map(|p| {
            let kind = snapshot
                .element(&p.id)
                .map_or("unknown", |e| e.kind.as_token());
            Ok(Json::object([
                ("id", Json::text(p.id.to_string())),
                ("kind", Json::text(kind)),
                ("instance", Json::uint(u64::from(p.instance_index))),
                ("first_triangle", Json::uint(u64::from(p.first_triangle))),
                ("triangle_count", Json::uint(u64::from(p.triangle_count))),
                ("representation", Json::text(p.representation.as_token())),
                ("exact", Json::Bool(p.exact)),
                ("mesh_digest", Json::text(p.mesh_digest.to_string())),
                ("placement_mm", triple(p.placement_mm)?),
                ("orientation_xyzw", quaternion(p.orientation_xyzw)?),
                ("world_bounds", bounds_json(Some(p.world_bounds))?),
            ]))
        })
        .collect::<View<Vec<_>>>()?;
    let required = projection
        .required_meshes()
        .iter()
        .map(|d| mesh_json(snapshot, d))
        .collect();
    let selection = match selection {
        Some(s) => Json::object([
            (
                "ids",
                Json::Array(s.ids().map(|i| Json::text(i.to_string())).collect()),
            ),
            (
                "instances",
                Json::Array(
                    s.instances(&projection)?
                        .into_iter()
                        .map(|i| Json::uint(u64::from(i)))
                        .collect(),
                ),
            ),
        ]),
        None => Json::Null,
    };
    Ok(Json::object([
        ("outcome", Json::text("view_snapshot")),
        ("synthetic", Json::Bool(true)),
        ("stand_ins", stand_ins(simplified)),
        ("pin", pin_json(snapshot.pin())),
        ("snapshot_token", Json::text(snapshot.token().as_str())),
        (
            "settings",
            Json::object([
                (
                    "hidden_kinds",
                    Json::Array(
                        settings
                            .hidden_kinds
                            .hidden_kinds()
                            .into_iter()
                            .map(|k| Json::text(k.as_token()))
                            .collect(),
                    ),
                ),
                (
                    "representation",
                    Json::text(settings.representation.as_token()),
                ),
            ]),
        ),
        ("element_count", Json::uint(snapshot.len() as u64)),
        ("visible_count", Json::uint(projection.len() as u64)),
        ("hidden_count", Json::uint(projection.hidden_count() as u64)),
        (
            "triangle_count",
            Json::uint(u64::from(projection.total_triangles())),
        ),
        ("all_exact", Json::Bool(projection.all_exact())),
        ("bounds", bounds_json(projection.bounds())?),
        (
            "engineering_bounds",
            bounds_json(snapshot.engineering_bounds())?,
        ),
        ("required_meshes", Json::Array(required)),
        ("elements", Json::Array(elements)),
        ("selection", selection),
    ]))
}

fn entry_json(entry: &ElementEntry) -> View<Json> {
    Ok(Json::object([
        ("kind", Json::text(entry.kind.as_token())),
        ("placement_mm", triple(entry.placement_mm)?),
        ("orientation_xyzw", quaternion(entry.orientation_xyzw)?),
        ("full_mesh", Json::text(entry.full.mesh_digest.to_string())),
        (
            "simplified_mesh",
            Json::text(entry.simplified.mesh_digest.to_string()),
        ),
        ("simplified_exact", Json::Bool(entry.simplified.exact)),
        ("world_bounds", bounds_json(Some(entry.full.world_bounds))?),
    ]))
}

fn changes_json(delta: &ChangeSet, after: &ViewSnapshot, simplified: bool) -> View<Json> {
    let changes = delta
        .changes
        .iter()
        .map(|c| {
            Ok(Json::object([
                ("id", Json::text(c.id.to_string())),
                ("change", Json::text(c.kind.as_token())),
                (
                    "fields",
                    Json::object([
                        ("kind", Json::Bool(c.fields.kind)),
                        ("placement", Json::Bool(c.fields.placement)),
                        ("shape", Json::Bool(c.fields.shape)),
                    ]),
                ),
                (
                    "entry",
                    c.entry.as_ref().map_or(Ok(Json::Null), entry_json)?,
                ),
            ]))
        })
        .collect::<View<Vec<_>>>()?;
    Ok(Json::object([
        ("outcome", Json::text("view_changes")),
        ("synthetic", Json::Bool(true)),
        ("stand_ins", stand_ins(simplified)),
        ("from_token", Json::text(delta.from.as_str())),
        ("to_token", Json::text(delta.to.as_str())),
        ("to_pin", pin_json(delta.to_pin)),
        ("change_count", Json::uint(delta.changes.len() as u64)),
        ("changes", Json::Array(changes)),
        (
            "new_meshes",
            Json::Array(
                delta
                    .new_meshes
                    .iter()
                    .map(|d| mesh_json(after, d))
                    .collect(),
            ),
        ),
    ]))
}
