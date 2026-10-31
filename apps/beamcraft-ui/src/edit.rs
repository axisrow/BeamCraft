// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! V05.2 edit tools, as pure logic: selecting by a mouse click, moving, copying and deleting the
//! selected element, and undo and redo. Every action is the argument list of the same CLI command
//! a user types (`pick`, `update-element --translation`, `copy-element --offset`,
//! `delete-element`, `undo-workspace`, `redo-workspace`); the reply is read back into a result or
//! the CLI's own refusal. Nothing here touches a window, a GPU or a process.
//!
//! The UI computes no engineering result. The one piece of arithmetic is the display ray through a
//! clicked pixel (camera math, like the projection in [`crate::camera`]); what the ray hits is the
//! engine's `pick`. The window runs that query itself on the snapshot it shows ([`pick_in`], the
//! same engine call the CLI's `pick` makes), so a click selects what is on screen and starts no
//! process for it. A move is the absolute translation the user types (the form is pre-filled with
//! the element's present translation as the engine reports it), so the UI adds nothing up itself.
//! Property editing is V06.

use crate::camera::View;
use crate::tools::{FormError, ToolError, decode, rejection, text};
use beamcraft_engine::ViewSnapshot;
use beamcraft_engine::query::feature::FeatureKind;
use beamcraft_engine::query::pick::{PickQuery, RayPick, SelectionFilter, pick};
use beamcraft_json::Json;
use beamcraft_model::limits::COORDINATE_CAP_MM;
use std::path::Path;

/// A world-space ray in millimetres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ray {
    /// Where it starts.
    pub origin: [f64; 3],
    /// Unit direction.
    pub direction: [f64; 3],
}

/// The normalized device coordinates (`-1..1`, y up) of a pixel inside a rectangle, or `None`
/// when the rectangle is empty, a value is not finite or the pixel lies outside it.
pub fn ndc_of(point: [f32; 2], rect_min: [f32; 2], size: [f32; 2]) -> Option<[f32; 2]> {
    let all = point.into_iter().chain(rect_min).chain(size);
    if !all.clone().all(f32::is_finite) || size[0] <= 0.0 || size[1] <= 0.0 {
        return None;
    }
    let fx = (point[0] - rect_min[0]) / size[0];
    let fy = (point[1] - rect_min[1]) / size[1];
    if !(0.0..=1.0).contains(&fx) || !(0.0..=1.0).contains(&fy) {
        return None;
    }
    Some([fx * 2.0 - 1.0, 1.0 - fy * 2.0])
}

/// The world ray through `ndc` for a camera view. Display coordinates are relative to
/// `origin_mm` (the scene origin), so the ray is shifted back to model millimetres in f64. This is
/// the inverse of [`View::project`]: under the perspective projection the ray leaves the eye and
/// aims through the frustum; under the orthographic one every pixel is a parallel ray from the
/// laterally offset eye point, so both draw and pick agree in either projection (V13).
pub fn ray_through(view: &View, ndc: [f32; 2], origin_mm: [f64; 3]) -> Option<Ray> {
    let [scale, aspect, _, _] = view.params;
    let [nx, ny] = ndc;
    if !ndc.into_iter().chain([scale, aspect]).all(f32::is_finite)
        || nx.abs() > 1.0
        || ny.abs() > 1.0
    {
        return None;
    }
    let orthographic = view.mode[0] > 0.5;
    let mut direction = [0.0f64; 3];
    let mut origin = [0.0f64; 3];
    for (axis, (slot, start)) in direction.iter_mut().zip(origin.iter_mut()).enumerate() {
        let right = f64::from(*view.right.get(axis)?);
        let up = f64::from(*view.up.get(axis)?);
        let back = f64::from(*view.back.get(axis)?);
        let eye = f64::from(*view.eye.get(axis)?);
        let lateral = right * f64::from(nx) * f64::from(scale) * f64::from(aspect)
            + up * f64::from(ny) * f64::from(scale);
        *slot = if orthographic { -back } else { lateral - back };
        *start = eye + origin_mm.get(axis).copied()?;
        if orthographic {
            *start += lateral;
        }
    }
    let length = direction.iter().map(|v| v * v).sum::<f64>().sqrt();
    if !length.is_finite() || length < 1e-9 {
        return None;
    }
    Some(Ray {
        origin,
        direction: direction.map(|v| v / length),
    })
}

fn triple(values: [f64; 3]) -> String {
    format!("{},{},{}", values[0], values[1], values[2])
}

/// The arguments of the CLI `pick` command for a ray: the nearest face hit only.
pub fn pick_args(store: &Path, ray: &Ray) -> Vec<String> {
    vec![
        "pick".into(),
        "--store".into(),
        store.to_string_lossy().into_owned(),
        "--origin".into(),
        triple(ray.origin),
        "--direction".into(),
        triple(ray.direction),
        "--targets".into(),
        "face".into(),
        "--max-hits".into(),
        "1".into(),
    ]
}

/// Reads the reply of `pick`: the element under the ray, or `None` for empty space.
pub fn parse_pick(exit_code: Option<i32>, stdout: &[u8]) -> Result<Option<String>, ToolError> {
    let reply = decode(stdout)?;
    if let Some(error) = rejection(&reply, exit_code) {
        return Err(error);
    }
    let unreadable = || ToolError {
        code: "UNREADABLE_REPLY".into(),
        message: "the CLI reply lacks a field".into(),
    };
    let hits = reply
        .get("hits")
        .and_then(Json::array)
        .map_err(|_| unreadable())?;
    match hits.first() {
        None => Ok(None),
        Some(hit) => Ok(Some(text(hit, "element_id").ok_or_else(unreadable)?)),
    }
}

/// What the CLI's `pick` with [`pick_args`] answers for `ray`, computed in this process on
/// `snapshot` by the same engine query: the element of the nearest front face along the ray, or
/// `None` for empty space. `selection` is the window's switches (V11): a class that is off is
/// never the answer. A refusal carries the engine's code, as the CLI's reply would.
pub fn pick_in(
    snapshot: &ViewSnapshot,
    ray: &Ray,
    selection: SelectionFilter,
) -> Result<Option<String>, ToolError> {
    Ok(face_pick(snapshot, ray, selection)?.map(|(_, id)| id))
}

/// The nearest front face's named feature along the ray, with the hit point: the qualified
/// FeatureId a D06 annotation placement stores. Named features only; unnamed display geometry
/// is no anchor, and the selection switches do not apply to a placement tool.
pub fn feature_pick(
    snapshot: &ViewSnapshot,
    ray: &Ray,
) -> Result<Option<(String, [f64; 3])>, ToolError> {
    let mut query = RayPick::new(ray.origin, ray.direction, vec![FeatureKind::Face]);
    query.max_hits = 1;
    let result = pick(snapshot, &PickQuery::Ray(query)).map_err(|e| ToolError {
        code: e.f04_code().as_str().to_owned(),
        message: format!("the pick was refused ({})", e.code().as_str()),
    })?;
    Ok(result.hits.first().and_then(|hit| {
        hit.feature_id
            .as_ref()
            .map(|id| (id.to_string(), hit.point_world_mm))
    }))
}

/// Where a click lands in 3D (issue #134 placement): the engine's front-face hit point on the
/// snapshot on screen together with the element it names. The selection switches do not apply:
/// placement is a tool's point resolution, not a selection. `None` for empty space; the caller
/// falls back to [`ground_point`] then.
pub fn pick_point_in(
    snapshot: &ViewSnapshot,
    ray: &Ray,
) -> Result<Option<([f64; 3], String)>, ToolError> {
    face_pick(snapshot, ray, SelectionFilter::ALL)
}

fn face_pick(
    snapshot: &ViewSnapshot,
    ray: &Ray,
    selection: SelectionFilter,
) -> Result<Option<([f64; 3], String)>, ToolError> {
    let mut query = RayPick::new(ray.origin, ray.direction, vec![FeatureKind::Face]);
    query.max_hits = 1;
    query.selection = selection;
    let result = pick(snapshot, &PickQuery::Ray(query)).map_err(|e| ToolError {
        code: e.f04_code().as_str().to_owned(),
        message: format!("the pick was refused ({})", e.code().as_str()),
    })?;
    Ok(result
        .hits
        .first()
        .map(|hit| (hit.point_world_mm, hit.element_id.to_string())))
}

/// Where `ray` meets the ground plane `z = 0` ahead of the camera: where a placement click
/// lands when it misses every face. `None` for a parallel ray, a crossing behind the camera or
/// a point beyond the member endpoint cap `COORDINATE_CAP_MM`: such a point would pass no
/// `create-member` at Commit, so it is no placement point to begin with.
pub fn ground_point(ray: &Ray) -> Option<[f64; 3]> {
    if ray.direction[2].abs() < 1e-9 {
        return None;
    }
    let t = -ray.origin[2] / ray.direction[2];
    if !(t.is_finite() && t > 0.0) {
        return None;
    }
    let point = [
        ray.origin[0] + t * ray.direction[0],
        ray.origin[1] + t * ray.direction[1],
        0.0,
    ];
    (point
        .iter()
        .all(|v| v.is_finite() && v.abs() <= COORDINATE_CAP_MM))
    .then_some(point)
}

/// What the panel shows about the selected element, as `inspect-elements` reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct ElementInfo {
    /// The element's id.
    pub id: String,
    /// `plate`, `beam` or `column`.
    pub kind: String,
    /// The element's name.
    pub name: String,
    /// Its placement translation in millimetres.
    pub translation_mm: [f64; 3],
}

/// The arguments of the CLI command that reports one element of the accepted revision with its
/// placement and properties (`inspect-elements --element ID`: a reply of one element, whatever the
/// size of the model).
pub fn inspect_element_args(store: &Path, id: &str) -> Vec<String> {
    vec![
        "inspect-elements".into(),
        "--store".into(),
        store.to_string_lossy().into_owned(),
        "--element".into(),
        id.to_owned(),
    ]
}

/// Reads the reply of `inspect-elements` for one element. An id the revision does not hold is
/// `NOT_FOUND`.
pub fn parse_element_info(
    id: &str,
    exit_code: Option<i32>,
    stdout: &[u8],
) -> Result<ElementInfo, ToolError> {
    let reply = decode(stdout)?;
    if let Some(error) = rejection(&reply, exit_code) {
        return Err(error);
    }
    let unreadable = || ToolError {
        code: "UNREADABLE_REPLY".into(),
        message: "the CLI reply lacks a field".into(),
    };
    for key in ["plates", "members"] {
        let Ok(list) = reply.get(key).and_then(Json::array) else {
            continue;
        };
        for element in list {
            if text(element, "element_id").as_deref() != Some(id) {
                continue;
            }
            let translation = element
                .get("placement")
                .and_then(|p| p.get("translation_mm"))
                .and_then(Json::array)
                .map_err(|_| unreadable())?;
            let [x, y, z] = translation else {
                return Err(unreadable());
            };
            let value = |v: &Json| v.float().map_err(|_| unreadable());
            return Ok(ElementInfo {
                id: id.to_owned(),
                kind: text(element, "kind").unwrap_or_else(|| "plate".into()),
                name: text(element, "name").unwrap_or_default(),
                translation_mm: [value(x)?, value(y)?, value(z)?],
            });
        }
    }
    Err(ToolError {
        code: "NOT_FOUND".into(),
        message: "the element is not in the accepted revision".into(),
    })
}

/// The edit actions of this slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditKind {
    /// `update-element --translation`.
    Move,
    /// `copy-element --offset`.
    Copy,
    /// `delete-element`.
    Delete,
    /// `undo-workspace`.
    Undo,
    /// `redo-workspace`.
    Redo,
}

impl EditKind {
    /// The CLI command.
    pub fn command(self) -> &'static str {
        match self {
            Self::Move => "update-element",
            Self::Copy => "copy-element",
            Self::Delete => "delete-element",
            Self::Undo => "undo-workspace",
            Self::Redo => "redo-workspace",
        }
    }

    /// The `outcome` token of a successful reply.
    fn outcome(self) -> &'static str {
        match self {
            Self::Move => "element_updated",
            Self::Copy => "element_copied",
            Self::Delete => "element_deleted",
            Self::Undo => "workspace_undone",
            Self::Redo => "workspace_redone",
        }
    }

    /// A past-tense label for the status line.
    pub fn done(self) -> &'static str {
        match self {
            Self::Move => "Moved",
            Self::Copy => "Copied",
            Self::Delete => "Deleted",
            Self::Undo => "Undid the last change of",
            Self::Redo => "Redid the last undone change of",
        }
    }
}

/// Where edits go: the store the window shows and, when it is a personal workspace, its
/// directory (the target of undo and redo).
#[derive(Debug, Clone, Copy)]
pub struct Target<'a> {
    /// The store that every command and the viewport use.
    pub store: &'a Path,
    /// The workspace directory, when the window was opened on one.
    pub workspace: Option<&'a Path>,
}

/// One edit command, ready to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    /// The action.
    pub kind: EditKind,
    /// The arguments exactly as passed to the CLI, starting with the command name.
    pub args: Vec<String>,
}

impl Edit {
    /// The argument list, one JSON string per line (not a shell command line; see
    /// [`crate::tools::Prepared::argument_list`]).
    pub fn argument_list(&self) -> String {
        self.args
            .iter()
            .map(|a| crate::json_string(a))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn number(field: &'static str, text: &str) -> Result<String, FormError> {
    let text = text.trim();
    if text.is_empty() || text.len() > 32 {
        return Err(FormError {
            field,
            message: "enter a number".into(),
        });
    }
    match text.parse::<f64>() {
        Ok(value) if value.is_finite() => Ok(text.to_owned()),
        _ => Err(FormError {
            field,
            message: format!("\"{text}\" is not a finite number"),
        }),
    }
}

fn point(field: &'static str, parts: &[String; 3]) -> Result<String, FormError> {
    let [x, y, z] = parts;
    Ok(format!(
        "{},{},{}",
        number(field, x)?,
        number(field, y)?,
        number(field, z)?
    ))
}

/// The edit panel's state: the selection, the two point forms, and the preview / commit state
/// machine of the destructive and placing actions. Undo and redo run directly.
#[derive(Debug, Clone)]
pub struct Editor {
    selected: Option<ElementInfo>,
    action: EditKind,
    /// `--translation` of a move: the new absolute translation.
    pub move_to: [String; 3],
    /// `--offset` of a copy.
    pub copy_offset: [String; 3],
    /// Copy as a linked instance of the shared definition.
    pub copy_instance: bool,
    /// The author recorded as `--actor-id` of a move, copy or delete, when the window has one.
    pub actor: Option<String>,
    staged: Option<Edit>,
    busy: bool,
}

impl Default for Editor {
    fn default() -> Self {
        Self {
            selected: None,
            action: EditKind::Move,
            move_to: Default::default(),
            copy_offset: Default::default(),
            copy_instance: false,
            actor: None,
            staged: None,
            busy: false,
        }
    }
}

impl Editor {
    /// The action the panel is set to (move, copy or delete).
    pub fn action(&self) -> EditKind {
        self.action
    }

    /// Switches the panel to another action. A staged edit was reviewed for the old action, so
    /// switching drops it: the new action needs its own preview.
    pub fn set_action(&mut self, action: EditKind) {
        if self.action != action {
            self.staged = None;
            self.action = action;
        }
    }

    /// The selected element.
    pub fn selected(&self) -> Option<&ElementInfo> {
        self.selected.as_ref()
    }

    /// Selects an element and pre-fills the move form with its present translation, as the
    /// engine reported it. Selecting the already selected element again keeps the typed
    /// coordinates, so a confirming click does not undo the typing. Any staged edit is
    /// dropped: it was for another selection, or the selection was made again.
    pub fn select(&mut self, info: ElementInfo) {
        if self.selected.as_ref() != Some(&info) {
            // Debug formatting: the shortest round-trip form, which stays within the
            // `number` length cap for every finite value (exponents included).
            self.move_to = info.translation_mm.map(|v| format!("{v:?}"));
        }
        if self.copy_offset.iter().all(String::is_empty) {
            self.copy_offset = ["0".into(), "0".into(), "0".into()];
        }
        self.staged = None;
        self.selected = Some(info);
    }

    /// Selects an element whose details could not be read: the move form is left empty rather
    /// than filled with a guess.
    pub fn select_id(&mut self, id: String) {
        self.move_to = Default::default();
        if self.copy_offset.iter().all(String::is_empty) {
            self.copy_offset = ["0".into(), "0".into(), "0".into()];
        }
        self.staged = None;
        self.selected = Some(ElementInfo {
            id,
            kind: "element".into(),
            name: "(details unavailable)".into(),
            translation_mm: [0.0; 3],
        });
    }

    /// Clears the selection and any staged edit.
    pub fn deselect(&mut self) {
        self.selected = None;
        self.staged = None;
    }

    /// The command of `kind` for the present selection and forms. Undo and redo need a workspace.
    pub fn prepare(&self, kind: EditKind, target: Target<'_>) -> Result<Edit, FormError> {
        let store = target.store.to_string_lossy().into_owned();
        let id = || {
            self.selected
                .as_ref()
                .map(|s| s.id.clone())
                .ok_or_else(|| FormError {
                    field: "Selection",
                    message: "click an element first".into(),
                })
        };
        let mut args = match kind {
            EditKind::Move => vec![
                "update-element".to_owned(),
                "--store".into(),
                store,
                "--id".into(),
                id()?,
                "--translation".into(),
                point("Position", &self.move_to)?,
            ],
            EditKind::Copy => vec![
                if self.copy_instance {
                    "copy-instance"
                } else {
                    "copy-element"
                }
                .to_owned(),
                "--store".into(),
                store,
                "--id".into(),
                id()?,
                "--offset".into(),
                point("Offset", &self.copy_offset)?,
            ],
            EditKind::Delete => vec![
                "delete-element".to_owned(),
                "--store".into(),
                store,
                "--id".into(),
                id()?,
            ],
            EditKind::Undo | EditKind::Redo => {
                let workspace = target.workspace.ok_or_else(|| FormError {
                    field: "Workspace",
                    message: "undo and redo need a personal workspace: open the window with --workspace DIR (beamcraft create-workspace makes one)".into(),
                })?;
                vec![
                    kind.command().to_owned(),
                    "--workspace-dir".into(),
                    workspace.to_string_lossy().into_owned(),
                ]
            }
        };
        if matches!(kind, EditKind::Move | EditKind::Copy | EditKind::Delete) {
            crate::tools::push_actor(&mut args, self.actor.as_deref());
        }
        Ok(Edit { kind, args })
    }

    /// Stages a move, copy or delete for review; nothing is run or written.
    pub fn preview(&mut self, kind: EditKind, target: Target<'_>) -> Result<&Edit, FormError> {
        let edit = self.prepare(kind, target)?;
        Ok(self.staged.insert(edit))
    }

    /// The staged edit, if any.
    pub fn staged(&self) -> Option<&Edit> {
        self.staged.as_ref()
    }

    /// True while an edit runs.
    pub fn busy(&self) -> bool {
        self.busy
    }

    /// True when an edit is staged and the forms still produce it.
    pub fn can_commit(&self, target: Target<'_>) -> bool {
        !self.busy
            && self
                .staged
                .as_ref()
                .is_some_and(|s| self.prepare(s.kind, target).is_ok_and(|now| now == *s))
    }

    /// Drops the staged edit. Nothing was run, so nothing is published.
    pub fn cancel(&mut self) {
        self.staged = None;
    }

    /// Takes the staged edit to run and marks the panel busy.
    pub fn begin_commit(&mut self, target: Target<'_>) -> Option<Edit> {
        if !self.can_commit(target) {
            return None;
        }
        self.busy = true;
        self.staged.take()
    }

    /// Takes an undo or redo to run at once (no review: both are themselves undoable) and marks
    /// the panel busy; refused while busy or without a workspace.
    pub fn begin_direct(
        &mut self,
        kind: EditKind,
        target: Target<'_>,
    ) -> Result<Option<Edit>, FormError> {
        let edit = self.prepare(kind, target)?;
        if self.busy {
            return Ok(None);
        }
        self.busy = true;
        self.staged = None;
        Ok(Some(edit))
    }

    /// Ends the running edit.
    pub fn finish(&mut self) {
        self.busy = false;
    }
}

/// Tells a worker's lookup result whether it is still wanted. Every new lookup, every
/// invalidation (Esc, an accepted edit) moves the epoch on; a result carries the epoch it was
/// asked under and is dropped when that is no longer the current one, so a late reply can neither
/// bring back a cleared selection nor overwrite a newer one with pre-edit details.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Epoch(u64);

impl Epoch {
    /// Starts a new generation and returns it; every earlier result is now stale.
    pub fn advance(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(1);
        self.0
    }

    /// Whether a result asked for under `asked` is still the wanted one.
    pub fn is_current(&self, asked: u64) -> bool {
        self.0 == asked
    }
}

/// What an accepted edit reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edited {
    /// The action.
    pub kind: EditKind,
    /// The element the edit named or created (a copy reports the new one).
    pub element_id: Option<String>,
    /// The accepted revision's manifest digest, which the view snapshot pins.
    pub manifest_digest: String,
    /// The revision's sequence number.
    pub sequence: u64,
    /// True when the CLI answered an already applied operation again.
    pub replayed: bool,
}

pub(crate) fn sequence_of(revision: &Json) -> Option<u64> {
    let value = revision.get("sequence").ok()?;
    value
        .string()
        .ok()
        .and_then(|s| s.parse().ok())
        .or_else(|| value.unsigned().ok())
}

/// Reads the reply of an edit command: the accepted revision or the CLI's refusal.
pub fn parse_edited(
    kind: EditKind,
    exit_code: Option<i32>,
    stdout: &[u8],
) -> Result<Edited, ToolError> {
    let reply = match decode(stdout) {
        Ok(reply) => reply,
        Err(unreadable) => {
            return Err(match exit_code {
                Some(0) => unreadable,
                _ => ToolError {
                    code: "CLI_FAILED".into(),
                    message: "the CLI failed without a JSON reply".into(),
                },
            });
        }
    };
    if let Some(error) = rejection(&reply, exit_code) {
        return Err(error);
    }
    let outcome = text(&reply, "outcome").unwrap_or_default();
    let replayed = outcome.ends_with("_replayed");
    if outcome != kind.outcome() && !replayed {
        return Err(ToolError {
            code: "UNREADABLE_REPLY".into(),
            message: format!(
                "unexpected outcome \"{}\"",
                outcome.chars().take(40).collect::<String>()
            ),
        });
    }
    let unreadable = || ToolError {
        code: "UNREADABLE_REPLY".into(),
        message: "the CLI reply lacks a field".into(),
    };
    let revision = reply.get("revision").map_err(|_| unreadable())?;
    let element_id = match kind {
        EditKind::Copy => reply
            .get("element")
            .ok()
            .and_then(|e| text(e, "element_id")),
        EditKind::Delete => text(&reply, "element_id"),
        EditKind::Move => reply
            .get("element")
            .ok()
            .and_then(|e| text(e, "element_id")),
        EditKind::Undo | EditKind::Redo => None,
    };
    Ok(Edited {
        kind,
        element_id,
        manifest_digest: text(revision, "manifest_digest").ok_or_else(unreadable)?,
        sequence: sequence_of(revision).ok_or_else(unreadable)?,
        replayed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::Camera;

    fn target() -> Target<'static> {
        Target {
            store: Path::new("S"),
            workspace: None,
        }
    }

    fn workspace() -> Target<'static> {
        Target {
            store: Path::new("W"),
            workspace: Some(Path::new("W")),
        }
    }

    fn info() -> ElementInfo {
        ElementInfo {
            id: "019a7bf0-0000-7000-8000-000000000001".into(),
            kind: "plate".into(),
            name: "P1".into(),
            translation_mm: [100.0, -20.5, 0.0],
        }
    }

    #[test]
    fn a_click_maps_to_device_coordinates_inside_the_rectangle_only() {
        assert_eq!(
            ndc_of([50.0, 25.0], [0.0, 0.0], [100.0, 50.0]),
            Some([0.0, 0.0])
        );
        assert_eq!(
            ndc_of([0.0, 0.0], [0.0, 0.0], [100.0, 50.0]),
            Some([-1.0, 1.0])
        );
        assert_eq!(
            ndc_of([100.0, 50.0], [0.0, 0.0], [100.0, 50.0]),
            Some([1.0, -1.0])
        );
        assert_eq!(ndc_of([110.0, 25.0], [0.0, 0.0], [100.0, 50.0]), None);
        assert_eq!(ndc_of([f32::NAN, 25.0], [0.0, 0.0], [100.0, 50.0]), None);
        assert_eq!(ndc_of([1.0, 1.0], [0.0, 0.0], [0.0, 50.0]), None);
    }

    #[test]
    fn the_ray_through_a_projected_point_hits_that_point() {
        let origin_mm = [10_000.0, -5_000.0, 200.0];
        // Both projections: the ray through a pixel of a point meets the point (V13).
        for orthographic in [false, true] {
            let mut camera = Camera::new([0.0; 3], 4000.0, 1.6).unwrap();
            camera.navigate([0.3, 0.1], [0.0; 2], 0.0).unwrap();
            camera.set_projection(orthographic);
            let view = camera.view(1.6).unwrap();
            for point in [
                [0.0f32, 0.0, 0.0],
                [300.0, -200.0, 100.0],
                [-500.0, 250.0, 0.0],
            ] {
                let [nx, ny, _] = view.project(point);
                let ray = ray_through(&view, [nx, ny], origin_mm).unwrap();
                // The point, back in model millimetres, lies on the ray.
                let p = [
                    f64::from(point[0]) + origin_mm[0],
                    f64::from(point[1]) + origin_mm[1],
                    f64::from(point[2]) + origin_mm[2],
                ];
                let to = [
                    p[0] - ray.origin[0],
                    p[1] - ray.origin[1],
                    p[2] - ray.origin[2],
                ];
                let along: f64 = (0..3).map(|i| to[i] * ray.direction[i]).sum();
                assert!(along > 0.0);
                for (i, (t, d)) in to.iter().zip(&ray.direction).enumerate() {
                    assert!((t - along * d).abs() < 0.05, "{point:?} axis {i}");
                }
                let length: f64 = ray.direction.iter().map(|v| v * v).sum::<f64>().sqrt();
                assert!((length - 1.0).abs() < 1e-9);
                if orthographic {
                    // Parallel rays: the direction is the view's -back (renormalized, so the
                    // f32 basis being only unit to 1e-7 shows up here, not at 1e-9).
                    for (d, b) in ray.direction.iter().zip(&view.back) {
                        assert!((d + f64::from(*b)).abs() < 1e-5);
                    }
                }
            }
        }
        let view = Camera::new([0.0; 3], 4000.0, 1.6)
            .unwrap()
            .view(1.6)
            .unwrap();
        assert!(ray_through(&view, [1.5, 0.0], origin_mm).is_none());
        assert!(ray_through(&view, [f32::NAN, 0.0], origin_mm).is_none());
    }

    #[test]
    fn pick_arguments_are_the_cli_pick_command() {
        let ray = Ray {
            origin: [500.0, 250.5, 1000.0],
            direction: [0.0, 0.0, -1.0],
        };
        assert_eq!(
            pick_args(Path::new("S"), &ray),
            [
                "pick",
                "--store",
                "S",
                "--origin",
                "500,250.5,1000",
                "--direction",
                "0,0,-1",
                "--targets",
                "face",
                "--max-hits",
                "1"
            ]
        );
    }

    #[test]
    fn a_pick_reply_gives_the_element_or_nothing() {
        let hit = br#"{"hits":[{"element_id":"019a7bf0-0000-7000-8000-000000000001","feature_kind":"face"}],"outcome":"picked"}"#;
        assert_eq!(
            parse_pick(Some(0), hit).unwrap().as_deref(),
            Some("019a7bf0-0000-7000-8000-000000000001")
        );
        assert_eq!(
            parse_pick(Some(0), br#"{"hits":[],"outcome":"picked"}"#).unwrap(),
            None
        );
        let refused = br#"{"code":"NOT_FOUND","message":"no revision","outcome":"rejected"}"#;
        assert_eq!(parse_pick(Some(2), refused).unwrap_err().code, "NOT_FOUND");
        assert_eq!(
            parse_pick(Some(0), b"nope").unwrap_err().code,
            "UNREADABLE_REPLY"
        );
        assert!(parse_pick(Some(0), br#"{"outcome":"picked"}"#).is_err());
    }

    #[test]
    fn element_info_is_read_from_the_inspection() {
        let reply = br#"{"plates":[{"element_id":"A","kind":"plate","name":"P1","placement":{"orientation_xyzw":[0,0,0,1],"translation_mm":[100,-20.5,0]}}],"members":[{"element_id":"B","kind":"beam","name":"B1","placement":{"translation_mm":[0,0,3000]}}],"outcome":"inspected"}"#;
        assert_eq!(
            parse_element_info("A", Some(0), reply).unwrap(),
            ElementInfo {
                id: "A".into(),
                kind: "plate".into(),
                name: "P1".into(),
                translation_mm: [100.0, -20.5, 0.0]
            }
        );
        let beam = parse_element_info("B", Some(0), reply).unwrap();
        assert_eq!(
            (beam.kind.as_str(), beam.translation_mm),
            ("beam", [0.0, 0.0, 3000.0])
        );
        assert_eq!(
            parse_element_info("C", Some(0), reply).unwrap_err().code,
            "NOT_FOUND"
        );
    }

    #[test]
    fn the_edit_actions_map_to_the_cli_arguments() {
        let mut editor = Editor::default();
        assert_eq!(
            editor.prepare(EditKind::Move, target()).unwrap_err().field,
            "Selection"
        );
        editor.select(info());
        assert_eq!(editor.move_to, ["100.0", "-20.5", "0.0"]);
        editor.move_to[0] = " 250 ".into();
        assert_eq!(
            editor.prepare(EditKind::Move, target()).unwrap().args,
            [
                "update-element",
                "--store",
                "S",
                "--id",
                "019a7bf0-0000-7000-8000-000000000001",
                "--translation",
                "250,-20.5,0.0"
            ]
        );
        editor.copy_offset = ["0".into(), "800".into(), "0".into()];
        assert_eq!(
            editor.prepare(EditKind::Copy, target()).unwrap().args,
            [
                "copy-element",
                "--store",
                "S",
                "--id",
                "019a7bf0-0000-7000-8000-000000000001",
                "--offset",
                "0,800,0"
            ]
        );
        assert_eq!(
            editor.prepare(EditKind::Delete, target()).unwrap().args,
            [
                "delete-element",
                "--store",
                "S",
                "--id",
                "019a7bf0-0000-7000-8000-000000000001"
            ]
        );
        assert_eq!(
            editor.prepare(EditKind::Undo, workspace()).unwrap().args,
            ["undo-workspace", "--workspace-dir", "W"]
        );
        assert_eq!(
            editor.prepare(EditKind::Redo, workspace()).unwrap().args,
            ["redo-workspace", "--workspace-dir", "W"]
        );
    }

    #[test]
    fn undo_and_redo_need_a_workspace() {
        let editor = Editor::default();
        for kind in [EditKind::Undo, EditKind::Redo] {
            let error = editor.prepare(kind, target()).unwrap_err();
            assert_eq!(error.field, "Workspace");
            assert!(error.message.contains("--workspace"));
        }
    }

    #[test]
    fn the_ui_checks_numbers_and_leaves_engineering_to_the_engine() {
        let mut editor = Editor::default();
        editor.select(info());
        for bad in ["", "abc", "NaN", "inf", &"1".repeat(40)] {
            editor.move_to[1] = bad.into();
            assert_eq!(
                editor.prepare(EditKind::Move, target()).unwrap_err().field,
                "Position"
            );
        }
        // A huge or negative value is the engine's to judge.
        editor.move_to = ["-1e12".into(), "0".into(), "0".into()];
        assert!(editor.prepare(EditKind::Move, target()).is_ok());
    }

    #[test]
    fn a_selection_pre_fills_in_round_trip_form_and_a_reclick_keeps_the_typing() {
        let mut editor = Editor::default();
        // A placement with float noise still fits the number length cap and parses back,
        // because it is written in the shortest round-trip (Debug) form.
        let noisy = ElementInfo {
            translation_mm: [5.551115123125783e-17, 1.5e20, 0.0],
            ..info()
        };
        editor.select(noisy.clone());
        assert_eq!(editor.move_to, ["5.551115123125783e-17", "1.5e20", "0.0"]);
        assert!(editor.move_to.iter().all(|v| v.len() <= 32));
        assert!(editor.prepare(EditKind::Move, target()).is_ok());
        // Selecting the already selected element again keeps the typed coordinates.
        editor.move_to[0] = "250".into();
        editor.select(noisy);
        assert_eq!(editor.move_to[0], "250");
        // Another selection pre-fills its own translation.
        editor.select(info());
        assert_eq!(editor.move_to, ["100.0", "-20.5", "0.0"]);
    }

    #[test]
    fn commit_runs_only_what_was_previewed_and_only_while_unchanged() {
        let mut editor = Editor::default();
        editor.select(info());
        assert!(!editor.can_commit(target()));
        assert!(editor.begin_commit(target()).is_none());
        editor.preview(EditKind::Delete, target()).unwrap();
        assert!(editor.can_commit(target()));
        // Another selection drops the staged edit.
        let mut other = info();
        other.id = "x".into();
        editor.select(other);
        assert!(editor.staged().is_none() && !editor.can_commit(target()));
        editor.move_to[0] = "5".into();
        editor.preview(EditKind::Move, target()).unwrap();
        editor.move_to[0] = "6".into();
        assert!(!editor.can_commit(target()), "edited after the preview");
        editor.move_to[0] = "5".into();
        assert!(editor.can_commit(target()));
        let edit = editor.begin_commit(target()).unwrap();
        assert_eq!(edit.kind, EditKind::Move);
        assert!(editor.busy() && editor.staged().is_none());
        // No second commit while busy.
        editor.preview(EditKind::Move, target()).unwrap();
        assert!(!editor.can_commit(target()));
        editor.finish();
        assert!(editor.can_commit(target()));
        editor.cancel();
        assert!(editor.staged().is_none());
    }

    #[test]
    fn switching_the_action_drops_the_staged_edit() {
        let mut editor = Editor::default();
        editor.select(info());
        editor.set_action(EditKind::Delete);
        editor.preview(EditKind::Delete, target()).unwrap();
        assert!(editor.can_commit(target()));
        editor.set_action(EditKind::Move);
        assert!(editor.staged().is_none() && !editor.can_commit(target()));
        assert!(editor.begin_commit(target()).is_none());
        // Setting the action it already has keeps the preview.
        editor.preview(EditKind::Move, target()).unwrap();
        editor.set_action(EditKind::Move);
        assert!(editor.can_commit(target()));
    }

    #[test]
    fn a_late_lookup_result_is_dropped() {
        let mut epoch = Epoch::default();
        let asked = epoch.advance();
        assert!(epoch.is_current(asked));
        // Esc, or an accepted edit, invalidates what is in flight.
        epoch.advance();
        assert!(!epoch.is_current(asked));
        let refreshed = epoch.advance();
        assert!(epoch.is_current(refreshed) && !epoch.is_current(asked));
    }

    #[test]
    fn undo_runs_directly_and_not_while_busy() {
        let mut editor = Editor::default();
        assert!(editor.begin_direct(EditKind::Undo, target()).is_err());
        let edit = editor
            .begin_direct(EditKind::Undo, workspace())
            .unwrap()
            .unwrap();
        assert_eq!(edit.kind, EditKind::Undo);
        assert!(editor.busy());
        assert!(
            editor
                .begin_direct(EditKind::Redo, workspace())
                .unwrap()
                .is_none()
        );
        editor.finish();
        assert!(
            editor
                .begin_direct(EditKind::Redo, workspace())
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn replies_become_results_or_the_cli_refusal() {
        let moved = br#"{"element":{"element_id":"A"},"outcome":"element_updated","revision":{"manifest_digest":"abc","sequence":"2"}}"#;
        let edited = parse_edited(EditKind::Move, Some(0), moved).unwrap();
        assert_eq!(
            (
                edited.element_id.as_deref(),
                edited.sequence,
                edited.replayed
            ),
            (Some("A"), 2, false)
        );
        let copied = br#"{"element":{"element_id":"N"},"outcome":"element_copied","revision":{"manifest_digest":"abc","sequence":"3"},"source_id":"A"}"#;
        assert_eq!(
            parse_edited(EditKind::Copy, Some(0), copied)
                .unwrap()
                .element_id
                .as_deref(),
            Some("N")
        );
        let deleted = br#"{"element_id":"A","kind":"plate","outcome":"element_deleted","revision":{"manifest_digest":"abc","sequence":"4"}}"#;
        assert_eq!(
            parse_edited(EditKind::Delete, Some(0), deleted)
                .unwrap()
                .element_id
                .as_deref(),
            Some("A")
        );
        let undone = br#"{"outcome":"workspace_undone","redo_units":"1","restored_sequence":"1","revision":{"manifest_digest":"abc","sequence":"5"},"undo_units":"0"}"#;
        assert_eq!(
            parse_edited(EditKind::Undo, Some(0), undone)
                .unwrap()
                .sequence,
            5
        );
        // A reply of another command is not accepted for this action.
        assert_eq!(
            parse_edited(EditKind::Redo, Some(0), undone)
                .unwrap_err()
                .code,
            "UNREADABLE_REPLY"
        );
        let refused = br#"{"code":"DEPENDENCY_CONFLICT","message":"a weld connects it","outcome":"rejected"}"#;
        let error = parse_edited(EditKind::Delete, Some(2), refused).unwrap_err();
        assert_eq!(
            (error.code.as_str(), error.message.as_str()),
            ("DEPENDENCY_CONFLICT", "a weld connects it")
        );
        // A zero exit cannot hide a rejection; a crash without JSON is CLI_FAILED.
        assert_eq!(
            parse_edited(EditKind::Delete, Some(0), refused)
                .unwrap_err()
                .code,
            "DEPENDENCY_CONFLICT"
        );
        assert_eq!(
            parse_edited(EditKind::Delete, Some(1), b"")
                .unwrap_err()
                .code,
            "CLI_FAILED"
        );
        assert_eq!(
            parse_edited(EditKind::Delete, Some(0), b"")
                .unwrap_err()
                .code,
            "UNREADABLE_REPLY"
        );
    }

    #[test]
    fn the_displayed_argument_list_is_one_json_string_per_line() {
        let mut editor = Editor::default();
        let mut quoted = info();
        quoted.id = "a\"b".into();
        editor.select(quoted);
        let edit = editor.prepare(EditKind::Delete, target()).unwrap();
        assert_eq!(edit.argument_list().lines().last(), Some("\"a\\\"b\""));
    }

    #[test]
    fn a_placement_click_that_misses_every_face_lands_on_the_ground_plane() {
        // Down from 100 mm: the crossing is exactly above the origin.
        let ray = Ray {
            origin: [500.0, -250.0, 100.0],
            direction: [0.0, 0.0, -1.0],
        };
        assert_eq!(ground_point(&ray), Some([500.0, -250.0, 0.0]));
        // An oblique ray crosses where its slope says; a negative t is behind the camera.
        let ray = Ray {
            origin: [0.0, 0.0, 100.0],
            direction: [0.6, 0.0, -0.8],
        };
        assert_eq!(ground_point(&ray), Some([75.0, 0.0, 0.0]));
        assert_eq!(
            ground_point(&Ray {
                origin: [0.0, 0.0, 100.0],
                direction: [0.0, 0.0, 1.0]
            }),
            None
        );
        for parallel in [[1.0, 0.0, 0.0], [0.0, 0.0, 0.0]] {
            assert_eq!(
                ground_point(&Ray {
                    origin: [0.0, 0.0, 100.0],
                    direction: parallel
                }),
                None
            );
        }
    }
}
