// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! V12 model views: the view-book session file, headlessly. A view is display state, so these
//! commands never write a model store; the store is only read, to resolve a view plane from the
//! model's grids, levels, parts, assemblies or faces.

use beamcraft_commands::assembly::{AssemblyView, assemblies};
use beamcraft_commands::plate::{Inspection, inspect};
use beamcraft_commands::source_file::read_file;
use beamcraft_engine::views::{
    self, AxisPick, LevelChoice, ModelView, PartSide, Projection, ViewBook, WorkArea,
};
use beamcraft_engine::work_plane::WorkPlane;
use beamcraft_engine::{KindFilter, RenderMode};
use beamcraft_host::ids::IdSource;
use beamcraft_host::revision::RevisionStore;
use beamcraft_host::storage::BlobStore;
use beamcraft_host::views as views_json;
use beamcraft_json::{Json, Limits};
use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, Uuid7};
use session::Refusal;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
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

fn refusal(code: ErrorCode, message: &'static str) -> Refusal {
    Refusal {
        rule_id: None,
        code,
        message,
    }
}

fn invalid(message: &'static str) -> Refusal {
    refusal(ErrorCode::InvalidArgument, message)
}

fn unknown() -> Refusal {
    refusal(ErrorCode::NotFound, "VIEW_UNKNOWN")
}

fn numbers(text: &str) -> Run<Vec<f64>> {
    text.split(',')
        .take(10)
        .map(|v| {
            v.trim()
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite())
                .ok_or_else(|| invalid("expected finite numbers separated by commas"))
        })
        .collect()
}

fn fixed<const N: usize>(text: &str) -> Run<[f64; N]> {
    <[f64; N]>::try_from(numbers(text)?).map_err(|_| invalid("wrong number of values"))
}

fn points<const N: usize>(text: &str) -> Run<[[f64; 3]; N]> {
    let parsed = text
        .split(';')
        .take(N + 1)
        .map(fixed::<3>)
        .collect::<Run<Vec<_>>>()?;
    <[[f64; 3]; N]>::try_from(parsed).map_err(|_| invalid("wrong number of points"))
}

fn number(text: &str) -> Run<f64> {
    text.trim()
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| invalid("enter a finite number"))
}

fn flag(text: &str) -> Run<bool> {
    match text {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(invalid("booleans are true or false")),
    }
}

fn element_id(text: &str) -> Run<ElementId> {
    ElementId::parse(text).map_err(|_| refusal(ErrorCode::InvalidId, "expected a canonical UUIDv7"))
}

/// Reads the session file; a missing or empty file is an empty book when `allow_missing`.
fn load(path: &str, allow_missing: bool) -> Run<ViewBook> {
    let bytes = match read_file(path, FILE_CAP) {
        Ok(bytes) => bytes,
        Err(ErrorCode::NotFound) if allow_missing => return Ok(ViewBook::new()),
        Err(code) => return Err(refusal(code, "the view file could not be read")),
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
    if bytes.len() > FILE_CAP {
        return Err(refusal(
            ErrorCode::LimitExceeded,
            "the view file would exceed 1 MiB",
        ));
    }
    let backend = |m| refusal(ErrorCode::BackendUnavailable, m);
    let temp = format!("{path}.tmp");
    std::fs::write(&temp, &bytes).map_err(|_| backend("the view file could not be written"))?;
    std::fs::rename(&temp, path).map_err(|_| backend("the view file could not be replaced"))
}

/// The options that build or change a view; `save-view` and `update-view` share them.
const PLANE_OPTIONS: &[&str] = &[
    "--store",
    "--target",
    "--yaw",
    "--pitch",
    "--parallel",
    "--offset",
    "--plane",
    "--two-points",
    "--work-plane",
    "--three-points",
    "--grid",
    "--axis",
    "--u-axes",
    "--v-axes",
    "--u-prefix",
    "--v-prefix",
    "--level",
    "--levels",
    "--element",
    "--assembly",
    "--side",
    "--ray-origin",
    "--ray-direction",
    "--edge",
];

const PROPERTY_OPTIONS: &[&str] = &[
    "--projection",
    "--span",
    "--depth",
    "--rotation",
    "--render-mode",
    "--grids",
    "--hidden-kinds",
    "--work-area",
    "--color",
    "--opacity",
    "--folder",
    "--favorite",
    "--copy-from",
];

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
                .ok_or_else(|| invalid("argument is not bounded UTF-8"))
        })
        .collect::<Run<_>>()?;
    let (command, tail) = args
        .split_first()
        .ok_or_else(|| invalid("a command is required"))?;
    let own: &[&str] = match *command {
        "list-views" => &["--file", "--store"],
        "save-view" => &["--file", "--name", "--new-id"],
        "update-view" => &["--file", "--view", "--name"],
        "remove-view" | "apply-view" => &["--file", "--store", "--view"],
        _ => return Err(invalid("unknown named-view command")),
    };
    let changes = matches!(*command, "save-view" | "update-view");
    if !tail.len().is_multiple_of(2) || tail.len() > 96 {
        return Err(invalid("options come in name-value pairs"));
    }
    let mut options = BTreeMap::new();
    for pair in tail.as_chunks::<2>().0 {
        let [key, value] = pair;
        let known = own.contains(key)
            || (changes && (PLANE_OPTIONS.contains(key) || PROPERTY_OPTIONS.contains(key)));
        if !known || options.insert(*key, *value).is_some() {
            return Err(invalid("unknown or repeated option"));
        }
    }
    let file = *options
        .get("--file")
        .ok_or_else(|| invalid("--file is required"))?;
    let selected = |book: &ViewBook| -> Run<Uuid7> {
        let selector = options
            .get("--view")
            .ok_or_else(|| invalid("--view is required"))?;
        book.find(selector).map(|v| v.id).ok_or_else(unknown)
    };
    match *command {
        "list-views" => {
            let book = load(file, true)?;
            Ok(Json::object([
                ("outcome", Json::text("views")),
                ("view_count", Json::uint(book.len() as u64)),
                (
                    "folders",
                    Json::Array(book.folders().map(Json::text).collect()),
                ),
                ("views", views_array(book.views().iter())?),
            ]))
        }
        "save-view" => {
            let mut book = load(file, true)?;
            let created = save_view(&options, &book)?;
            let ids: Vec<Uuid7> = created.iter().map(|v| v.id).collect();
            book.insert_all(created).map_err(from_engine)?;
            store(file, &book)?;
            saved("view_saved", &book, &ids)
        }
        "update-view" => {
            let mut book = load(file, false)?;
            let id = selected(&book)?;
            let current = book.get(&id).cloned().ok_or_else(unknown)?;
            let next = update_view(&options, &book, current)?;
            book.update(next).map_err(from_engine)?;
            store(file, &book)?;
            saved("view_updated", &book, &[id])
        }
        "remove-view" => {
            let mut book = load(file, false)?;
            let id = selected(&book)?;
            let view = book.remove(&id).ok_or_else(unknown)?;
            store(file, &book)?;
            Ok(Json::object([
                ("outcome", Json::text("view_removed")),
                ("id", Json::text(view.id.to_string())),
                ("name", Json::text(view.name)),
                ("view_count", Json::uint(book.len() as u64)),
            ]))
        }
        _ => {
            let book = load(file, false)?;
            let id = selected(&book)?;
            let view = book.get(&id).ok_or_else(unknown)?;
            let camera = view.camera_plane().map_err(from_engine)?;
            let clip = view
                .clipping()
                .and_then(|c| c.effective())
                .map_err(from_engine)?;
            let plane_json = |p: &beamcraft_engine::view_clipping::ClipPlane| -> Run<Json> {
                let n = |v: f64| Json::number(v).map_err(|_| invalid("a clip plane is not finite"));
                Ok(Json::object([
                    (
                        "point_mm",
                        Json::Array(p.point_mm.iter().map(|v| n(*v)).collect::<Run<_>>()?),
                    ),
                    (
                        "normal",
                        Json::Array(p.normal.iter().map(|v| n(*v)).collect::<Run<_>>()?),
                    ),
                ]))
            };
            Ok(Json::object([
                ("outcome", Json::text("view_applied")),
                ("view", views_json::view_to_json(view).map_err(from_engine)?),
                (
                    "camera_plane",
                    views_json::plane_to_json(&camera).map_err(from_engine)?,
                ),
                (
                    "clip_planes",
                    Json::Array(clip.iter().map(plane_json).collect::<Run<_>>()?),
                ),
            ]))
        }
    }
}

fn views_array<'a>(views: impl Iterator<Item = &'a ModelView>) -> Run<Json> {
    Ok(Json::Array(
        views
            .map(|v| views_json::view_to_json(v).map_err(from_engine))
            .collect::<Run<_>>()?,
    ))
}

fn saved(outcome: &str, book: &ViewBook, ids: &[Uuid7]) -> Run<Json> {
    Ok(Json::object([
        ("outcome", Json::text(outcome)),
        ("view_count", Json::uint(book.len() as u64)),
        (
            "views",
            views_array(ids.iter().filter_map(|id| book.get(id)))?,
        ),
    ]))
}

/// The read-only model facts a store-backed plane needs, read once.
struct Model {
    inspection: Inspection,
    assemblies: Vec<AssemblyView>,
}

fn open_model(options: &BTreeMap<&str, &str>) -> Run<Model> {
    let root = options
        .get("--store")
        .ok_or_else(|| invalid("--store is required to resolve this view from the model"))?;
    let blobs = BlobStore::new(root).map_err(|e| refusal(e.code(), "store root is not usable"))?;
    let store = RevisionStore::new(blobs);
    Ok(Model {
        inspection: inspect(&store)?,
        assemblies: assemblies(&store)?,
    })
}

impl Model {
    fn levels(&self) -> Vec<LevelChoice> {
        self.inspection
            .levels
            .levels()
            .map(|(id, record)| LevelChoice {
                id: *id,
                name: record.name.clone(),
                elevation_mm: record.elevation_mm,
            })
            .collect()
    }

    fn grid(&self, selector: &str) -> Run<&beamcraft_model::grid::Grid> {
        beamcraft_engine::grid_query::resolve_grid(&self.inspection.grids, selector)
            .map_err(from_engine)
    }

    /// Every part of an assembly and of its nested assemblies, the main part first. A part id
    /// stands for the assembly it belongs to.
    fn assembly_parts(&self, id: &ElementId) -> Run<Vec<ElementId>> {
        let root = if self.assemblies.iter().any(|a| a.id == *id) {
            *id
        } else {
            self.assemblies
                .iter()
                .find(|a| a.parts.contains(id))
                .map(|a| a.id)
                .ok_or_else(|| refusal(ErrorCode::NotFound, "the element is in no assembly"))?
        };
        let main = self
            .assemblies
            .iter()
            .find(|a| a.id == root)
            .and_then(|a| a.main);
        let mut seen = BTreeSet::new();
        let mut parts = BTreeSet::new();
        let mut queue = VecDeque::from([root]);
        while let Some(next) = queue.pop_front() {
            if !seen.insert(next) || seen.len() > views::MAX_GROUP_PARTS {
                if seen.len() > views::MAX_GROUP_PARTS {
                    return Err(refusal(
                        ErrorCode::LimitExceeded,
                        "the assembly has too many members",
                    ));
                }
                continue;
            }
            if let Some(assembly) = self.assemblies.iter().find(|a| a.id == next) {
                parts.extend(assembly.parts.iter().copied());
                queue.extend(
                    self.assemblies
                        .iter()
                        .filter(|a| a.parent == Some(next))
                        .map(|a| a.id),
                );
            }
        }
        let mut ordered: Vec<ElementId> = main.into_iter().filter(|m| parts.contains(m)).collect();
        ordered.extend(parts.into_iter().filter(|p| Some(*p) != main));
        Ok(ordered)
    }
}

/// One planned view: its name and plane, and a framing height when the model gives one.
struct Planned {
    name: String,
    plane: WorkPlane,
    span: Option<f64>,
    projection: Option<Projection>,
}

fn single(name: &str, plane: WorkPlane) -> Vec<Planned> {
    vec![Planned {
        name: name.to_owned(),
        plane,
        span: None,
        projection: None,
    }]
}

/// The views a construction option describes; several for a batch. `None` when no construction
/// option was given.
fn plan(options: &BTreeMap<&str, &str>, name: &str, batch: bool) -> Run<Option<Vec<Planned>>> {
    let get = |key: &str| options.get(key).copied();
    let modes = [
        "--yaw",
        "--parallel",
        "--plane",
        "--two-points",
        "--three-points",
        "--grid",
        "--level",
        "--levels",
        "--element",
        "--assembly",
        "--ray-origin",
    ];
    let used: Vec<&str> = modes.into_iter().filter(|m| get(m).is_some()).collect();
    if used.len() > 1 {
        return Err(invalid("give exactly one way to build the view plane"));
    }
    let needs = |option: &str, mode: &[&str]| {
        get(option).is_some() && !used.iter().any(|u| mode.contains(u))
    };
    if needs("--target", &["--yaw"])
        || needs("--pitch", &["--yaw"])
        || needs("--offset", &["--parallel"])
        || needs("--work-plane", &["--two-points"])
        || needs("--axis", &["--grid"])
        || needs("--u-axes", &["--grid"])
        || needs("--v-axes", &["--grid"])
        || needs("--u-prefix", &["--grid"])
        || needs("--v-prefix", &["--grid"])
        || needs("--side", &["--element", "--assembly"])
        || needs("--ray-direction", &["--ray-origin"])
        || needs("--edge", &["--ray-origin"])
    {
        return Err(invalid(
            "an option belongs to another way of building the plane",
        ));
    }
    let Some(mode) = used.first() else {
        return Ok(None);
    };
    let batch_only = |many: bool| -> Run<()> {
        if many && !batch {
            Err(invalid("a batch of views can only be created, not updated"))
        } else {
            Ok(())
        }
    };
    let planned = match *mode {
        "--yaw" => {
            let target = get("--target").map_or(Ok([0.0; 3]), fixed::<3>)?;
            let pitch = get("--pitch").ok_or_else(|| invalid("--pitch is required"))?;
            let plane = views::orbit_plane(
                target,
                number(get("--yaw").unwrap_or_default())?,
                number(pitch)?,
            )
            .map_err(from_engine)?;
            let mut out = single(name, plane);
            for p in &mut out {
                p.projection = Some(Projection::Perspective);
            }
            out
        }
        "--parallel" => {
            let offset = get("--offset").map_or(Ok(0.0), number)?;
            single(
                name,
                views::coordinate_plane(get("--parallel").unwrap_or_default(), offset)
                    .map_err(from_engine)?,
            )
        }
        "--plane" => single(
            name,
            WorkPlane::parse(get("--plane").unwrap_or_default()).map_err(from_engine)?,
        ),
        "--two-points" => {
            let [a, b] = points::<2>(get("--two-points").unwrap_or_default())?;
            let plane = match get("--work-plane") {
                Some(input) => {
                    views::two_points_in_plane(a, b, WorkPlane::parse(input).map_err(from_engine)?)
                }
                None => views::two_points(a, b),
            }
            .map_err(from_engine)?;
            single(name, plane)
        }
        "--three-points" => {
            let [a, b, c] = points::<3>(get("--three-points").unwrap_or_default())?;
            single(name, views::three_points(a, b, c).map_err(from_engine)?)
        }
        "--grid" => {
            let model = open_model(options)?;
            let grid = model.grid(get("--grid").unwrap_or_default())?;
            match get("--axis") {
                Some(axis) => {
                    if get("--u-axes").is_some() || get("--v-axes").is_some() {
                        return Err(invalid("--axis takes one axis, not a batch"));
                    }
                    single(name, views::grid_axis(grid, axis).map_err(from_engine)?)
                }
                None => {
                    batch_only(true)?;
                    let pick = |key| {
                        get(key).map_or(Ok(AxisPick::All), |t| {
                            AxisPick::parse(t).map_err(from_engine)
                        })
                    };
                    let prefixes = [
                        get("--u-prefix").unwrap_or(name),
                        get("--v-prefix").unwrap_or(name),
                    ];
                    views::grid_views(grid, pick("--u-axes")?, pick("--v-axes")?)
                        .map_err(from_engine)?
                        .into_iter()
                        .map(|axis| {
                            let prefix = prefixes
                                .get(usize::from(axis.family))
                                .copied()
                                .unwrap_or(name);
                            Planned {
                                name: join(prefix, &axis.label),
                                plane: axis.plane,
                                span: None,
                                projection: None,
                            }
                        })
                        .collect()
                }
            }
        }
        "--level" => {
            let levels = open_model(options)?.levels();
            let level = views::find_level(&levels, get("--level").unwrap_or_default())
                .map_err(from_engine)?;
            single(name, views::level_plane(level).map_err(from_engine)?)
        }
        "--levels" => {
            batch_only(true)?;
            let levels = open_model(options)?.levels();
            let pick = AxisPick::parse(get("--levels").unwrap_or_default()).map_err(from_engine)?;
            views::level_batch(&levels, pick)
                .into_iter()
                .map(|level| {
                    Ok(Planned {
                        name: join(name, &level.name),
                        plane: views::level_plane(level).map_err(from_engine)?,
                        span: None,
                        projection: None,
                    })
                })
                .collect::<Run<_>>()?
        }
        "--element" | "--assembly" => {
            let model = open_model(options)?;
            let id = element_id(get(mode).unwrap_or_default())?;
            let parts = if *mode == "--assembly" {
                model.assembly_parts(&id)?
            } else {
                vec![id]
            };
            let side = get("--side").unwrap_or("3d");
            let four = side == "four";
            let sides = if four {
                batch_only(true)?;
                PartSide::FOUR.to_vec()
            } else {
                vec![PartSide::parse(side).map_err(from_engine)?]
            };
            let snapshot = &model.inspection.snapshot;
            sides
                .into_iter()
                .map(|side| {
                    let (plane, span) = if *mode == "--assembly" {
                        views::group_plane(snapshot, &parts, side)
                    } else {
                        views::part_plane(snapshot, &id, side)
                    }
                    .map_err(from_engine)?;
                    Ok(Planned {
                        name: if four {
                            join(name, side.as_str())
                        } else {
                            name.to_owned()
                        },
                        plane,
                        span: Some(span),
                        projection: Some(if side == PartSide::Spatial {
                            Projection::Perspective
                        } else {
                            Projection::Orthographic
                        }),
                    })
                })
                .collect::<Run<_>>()?
        }
        _ => {
            let model = open_model(options)?;
            let origin = fixed::<3>(get("--ray-origin").unwrap_or_default())?;
            let direction = fixed::<3>(
                get("--ray-direction").ok_or_else(|| invalid("--ray-direction is required"))?,
            )?;
            let edge = get("--edge").map(points::<2>).transpose()?;
            single(
                name,
                views::surface_plane(&model.inspection.snapshot, None, origin, direction, edge)
                    .map_err(from_engine)?,
            )
        }
    };
    Ok(Some(planned))
}

fn join(prefix: &str, label: &str) -> String {
    let prefix = prefix.trim();
    if prefix.is_empty() {
        label.to_owned()
    } else {
        format!("{prefix} {label}")
    }
}

/// Applies the display-property options to `view`.
fn apply_properties(
    options: &BTreeMap<&str, &str>,
    book: &ViewBook,
    view: &mut ModelView,
) -> Run<()> {
    let get = |key: &str| options.get(key).copied();
    if let Some(source) = get("--copy-from") {
        let source = book.find(source).ok_or_else(unknown)?;
        view.apply_display_properties(source);
    }
    if let Some(text) = get("--projection") {
        view.projection = Projection::parse(text).map_err(from_engine)?;
    }
    if let Some(text) = get("--span") {
        view.span_mm = number(text)?;
    }
    if let Some(text) = get("--depth") {
        view.depth = match text {
            "none" => None,
            _ => Some(fixed::<2>(text)?),
        };
    }
    if let Some(text) = get("--rotation") {
        view.rotation_rad = fixed::<2>(text)?.map(f64::to_radians);
    }
    if let Some(text) = get("--render-mode") {
        view.render_mode = RenderMode::parse(text).ok_or_else(|| invalid("unknown render mode"))?;
    }
    if let Some(text) = get("--grids") {
        view.grids_visible = flag(text)?;
    }
    if let Some(text) = get("--hidden-kinds") {
        view.hidden_kinds = match text {
            "none" => KindFilter::SHOW_ALL,
            _ => KindFilter::parse_list(text).map_err(|_| invalid("unknown element kind"))?,
        };
    }
    if let Some(text) = get("--work-area") {
        view.work_area = match text {
            "none" => None,
            _ => {
                let [a, b, c, d, e, f] = fixed::<6>(text)?;
                Some(WorkArea::new([a, b, c], [d, e, f]).map_err(from_engine)?)
            }
        };
    }
    if let Some(text) = get("--color") {
        view.color = match text {
            "default" => None,
            _ => Some(fixed::<3>(text)?),
        };
    }
    if let Some(text) = get("--opacity") {
        view.opacity = number(text)?;
    }
    if let Some(text) = get("--folder") {
        text.trim().clone_into(&mut view.folder);
    }
    if let Some(text) = get("--favorite") {
        view.favorite = flag(text)?;
    }
    view.validate().map_err(from_engine)
}

fn save_view(options: &BTreeMap<&str, &str>, book: &ViewBook) -> Run<Vec<ModelView>> {
    let name = options
        .get("--name")
        .ok_or_else(|| invalid("--name is required"))?
        .trim();
    let planned = plan(options, name, true)?
        .ok_or_else(|| invalid("give one way to build the view plane"))?;
    // A batch option suffixes repeated names even when it selects one view.
    let many = options.contains_key("--levels")
        || (options.contains_key("--grid") && !options.contains_key("--axis"))
        || options.get("--side") == Some(&"four");
    if planned.is_empty() {
        return Err(invalid("the batch selects no view"));
    }
    if many && options.contains_key("--new-id") {
        return Err(invalid("--new-id names one view, not a batch"));
    }
    let mut ids = IdSource::system();
    let mut names: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for p in planned {
        let id = match options.get("--new-id") {
            Some(text) => Uuid7::parse(text)
                .map_err(|_| refusal(ErrorCode::InvalidId, "--new-id is not a UUIDv7"))?,
            None => ids
                .next_id()
                .map_err(|_| refusal(ErrorCode::BackendUnavailable, "no identity could be made"))?,
        };
        // A batch suffixes repeated names; a single view keeps its name and a clash refuses.
        let unique = if many {
            book.unique_name(&p.name, &names)
        } else {
            p.name
        };
        names.push(unique.clone());
        let mut view = ModelView::new(
            id,
            &unique,
            p.plane,
            p.span.unwrap_or(views::DEFAULT_SPAN_MM),
        );
        if let Some(projection) = p.projection {
            view.projection = projection;
        }
        apply_properties(options, book, &mut view)?;
        out.push(view);
    }
    Ok(out)
}

fn update_view(
    options: &BTreeMap<&str, &str>,
    book: &ViewBook,
    mut view: ModelView,
) -> Run<ModelView> {
    if let Some(name) = options.get("--name") {
        name.trim().clone_into(&mut view.name);
    }
    if let Some(mut planned) = plan(options, &view.name, false)? {
        let p = planned
            .pop()
            .ok_or_else(|| invalid("the plane option selects no view"))?;
        view.plane = p.plane;
        if let Some(span) = p.span {
            view.span_mm = span;
        }
    }
    apply_properties(options, book, &mut view)?;
    Ok(view)
}
