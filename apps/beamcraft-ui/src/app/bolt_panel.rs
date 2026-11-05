// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The Bolt bench (V19): two picked parts and two points on the active plane commit through the
//! same `create-bolt-group` command a user types (A14.3), and a placed group is loaded from the
//! bench's group list and edited through `update-bolt-group` as one atomic edit (slice 2). The
//! fastener travels by designation through a fastener library file, listed read-only with
//! `list-library`; the groups are listed read-only with `bolts`. The reply is the CLI's; a
//! refusal publishes nothing and is shown next to the form.

use super::{App, PointText, field};
use crate::command;
use crate::edit::{Ray, pick_in};
use crate::tools::{MAX_TEXT, ToolError, decode, rejection, text};
use beamcraft_json::Json;
use egui::RichText;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

/// The longest library path the form accepts, as the CLI bounds one argument.
const MAX_PATH: usize = 4096;

/// The most designations one listing may hold, as `--limit` asks for.
const MAX_ENTRIES: usize = 200;

/// The most placed groups one listing may hold.
const MAX_GROUPS: usize = 200;

/// Where the holes come from, as `--hole-mode` spells it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Holes {
    /// One group-owned hole record per crossing fastener on every connected plate.
    #[default]
    Generate,
    /// The group stores no holes.
    Quiet,
}

impl Holes {
    /// The CLI token.
    pub(crate) const fn token(self) -> &'static str {
        match self {
            Self::Generate => "generate",
            Self::Quiet => "none",
        }
    }

    /// The form label.
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Generate => "Generate holes",
            Self::Quiet => "No holes",
        }
    }
}

/// Which spacing pattern the form sends, as the command spells it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Spacing {
    /// An explicit list of gaps between neighbouring fasteners.
    #[default]
    List,
    /// One pitch repeated `count - 1` times.
    Pitch,
    /// The gap from the row's end, back-computed over `count` fasteners.
    OffsetEnd,
}

impl Spacing {
    /// The form label.
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::List => "Spacings list",
            Self::Pitch => "Pitch",
            Self::OffsetEnd => "Offset end",
        }
    }
}

/// The Bolt bench's state between frames; everything else is on [`App`].
pub(crate) struct Panel {
    /// The picked parts, main first (`--parts`); create needs two, edit keeps the stored ones.
    pub(crate) parts: Vec<String>,
    /// The first point on the work plane: the first fastener's spot.
    pub(crate) point1: Option<[f64; 3]>,
    /// The second point on the work plane: the row runs from point 1 towards it.
    pub(crate) point2: Option<[f64; 3]>,
    /// The group's label (`--name`, required by the command).
    pub(crate) name: String,
    /// The fastener library file (`--fastener-library`), as typed.
    pub(crate) library: String,
    /// The chosen fastener (`--designation`).
    pub(crate) designation: String,
    /// The designations the last listing reported.
    pub(crate) entries: Vec<String>,
    pub(super) entries_job: Option<Receiver<Result<Vec<String>, ToolError>>>,
    /// The placed groups the last `bolts` listing reported.
    pub(crate) groups: Vec<Listing>,
    pub(super) groups_job: Option<Receiver<Result<Vec<Listing>, ToolError>>>,
    /// The group loaded for editing, as its stored identity; `None` builds a new group.
    pub(crate) edit_target: Option<String>,
    /// The plane normal of the loaded group, kept across the edit (`--plane-normal`); `None`
    /// follows the active work plane, `0,0,1` without one.
    pub(crate) normal: Option<[f64; 3]>,
    /// Whether the update follows a changed library entry (`--repin`).
    pub(crate) repin: bool,
    pub(super) commit_job: Option<Receiver<Result<(), ToolError>>>,
    /// `--washer-under-head`, sent explicitly.
    pub(crate) washer_head: bool,
    /// `--washer-under-nut`, sent explicitly.
    pub(crate) washer_nut: bool,
    /// The hole setting.
    pub(crate) holes: Holes,
    /// The hole clearance as typed; empty sends no `--hole-clearance`.
    pub(crate) clearance: String,
    /// `--on-plane` as typed.
    pub(crate) on_plane: String,
    /// `--rotation-deg` as typed.
    pub(crate) rotation: String,
    /// `--depth` as typed.
    pub(crate) depth: String,
    /// `--offset-start` as typed.
    pub(crate) offset_start: String,
    /// Which spacing pattern the form sends.
    pub(crate) spacing: Spacing,
    /// The `--spacings` list as typed.
    pub(crate) spacings: String,
    /// The `--pitch` value as typed.
    pub(crate) pitch: String,
    /// The `--offset-end` value as typed.
    pub(crate) offset_end: String,
    /// The `--count` of the generators as typed.
    pub(crate) count: String,
    /// The last outcome, shown under the form.
    pub(crate) note: Option<Result<String, String>>,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            parts: Vec::new(),
            point1: None,
            point2: None,
            name: "Bolt group".into(),
            library: String::new(),
            designation: String::new(),
            entries: Vec::new(),
            entries_job: None,
            groups: Vec::new(),
            groups_job: None,
            edit_target: None,
            normal: None,
            repin: false,
            commit_job: None,
            washer_head: true,
            washer_nut: true,
            holes: Holes::default(),
            clearance: String::new(),
            on_plane: "0".into(),
            rotation: "0".into(),
            depth: "0".into(),
            offset_start: "50".into(),
            spacing: Spacing::default(),
            spacings: "150".into(),
            pitch: "150".into(),
            offset_end: "50".into(),
            count: "2".into(),
            note: None,
        }
    }
}

/// One placed group of a `bolts` listing, as the command reports it: every value is the
/// store's own text, parsed here for the edit form.
#[derive(Debug, Clone)]
pub(crate) struct Listing {
    /// The group's element identity.
    pub(crate) id: String,
    /// The group's label.
    pub(crate) name: String,
    /// The connected parts, main first.
    pub(crate) parts: Vec<String>,
    /// The row's first point and direction point, world millimetres.
    pub(crate) point1: [f64; 3],
    /// The second point.
    pub(crate) point2: [f64; 3],
    /// The stored plane normal.
    pub(crate) normal: [f64; 3],
    /// The stored rotation, radians.
    pub(crate) rotation_rad: f64,
    /// The stored `--on-plane`, millimetres.
    pub(crate) on_plane: f64,
    /// The stored `--depth`, millimetres.
    pub(crate) depth: f64,
    /// The stored `--offset-start`, millimetres.
    pub(crate) offset_start: f64,
    /// `spacings`, `pitch` or `offset_end`.
    pub(crate) spacing_mode: String,
    /// The generator's value; `0` for an explicit list.
    pub(crate) spacing_value: f64,
    /// The authoritative gaps between neighbouring fasteners.
    pub(crate) spacings: Vec<f64>,
    /// The stored washer switches.
    pub(crate) washer_head: bool,
    /// The stored washer switch under the nut.
    pub(crate) washer_nut: bool,
    /// `generate` or `none`.
    pub(crate) hole_mode: String,
    /// The stored clearance, when the group generates holes.
    pub(crate) hole_clearance: Option<f64>,
}

impl Panel {
    pub(super) fn empty() -> Self {
        Self::default()
    }

    fn busy(&self) -> bool {
        self.commit_job.is_some()
    }

    /// Forgets the picked parts and points; the form keeps its typed values.
    fn clear_placement(&mut self) {
        self.parts.clear();
        self.point1 = None;
        self.point2 = None;
    }

    /// Returns the bench to the create flow: the edit target and the stored plane normal go,
    /// the library fields and their listing stay.
    fn exit_edit(&mut self) {
        self.edit_target = None;
        self.normal = None;
        self.repin = false;
        self.clear_placement();
        let fresh = Self::empty();
        self.name = fresh.name;
        self.on_plane = fresh.on_plane;
        self.rotation = fresh.rotation;
        self.depth = fresh.depth;
        self.offset_start = fresh.offset_start;
        self.spacing = fresh.spacing;
        self.spacings = fresh.spacings;
        self.pitch = fresh.pitch;
        self.offset_end = fresh.offset_end;
        self.count = fresh.count;
        self.washer_head = fresh.washer_head;
        self.washer_nut = fresh.washer_nut;
        self.holes = fresh.holes;
        self.clearance = fresh.clearance;
    }

    /// Fills the form from one listed group, as an edit of that group. The stored plane normal
    /// and the pinned length travel inside the panel: the update sends no length arguments, so
    /// the command keeps the stored length mode whatever it is.
    pub(crate) fn load_group(&mut self, listing: &Listing) {
        self.edit_target = Some(listing.id.clone());
        self.parts = listing.parts.clone();
        self.point1 = Some(listing.point1);
        self.point2 = Some(listing.point2);
        self.normal = Some(listing.normal);
        self.name = listing.name.clone();
        self.rotation = clean(listing.rotation_rad.to_degrees());
        self.on_plane = clean(listing.on_plane);
        self.depth = clean(listing.depth);
        self.offset_start = clean(listing.offset_start);
        self.count = (listing.spacings.len().saturating_add(1)).to_string();
        match listing.spacing_mode.as_str() {
            "pitch" => {
                self.spacing = Spacing::Pitch;
                self.pitch = clean(listing.spacing_value);
            }
            "offset_end" => {
                self.spacing = Spacing::OffsetEnd;
                self.offset_end = clean(listing.spacing_value);
            }
            _ => {
                self.spacing = Spacing::List;
                self.spacings = listing
                    .spacings
                    .iter()
                    .map(|gap| clean(*gap))
                    .collect::<Vec<_>>()
                    .join(",");
            }
        }
        self.washer_head = listing.washer_head;
        self.washer_nut = listing.washer_nut;
        self.holes = match listing.hole_mode.as_str() {
            "none" => Holes::Quiet,
            _ => Holes::Generate,
        };
        self.clearance = listing.hole_clearance.map_or_else(String::new, clean);
        self.note = None;
    }
}

/// One stored number, back as the form's text: at most six decimals, no trailing zeros.
fn clean(value: f64) -> String {
    let text = format!("{value:.6}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned();
    if text.is_empty() || text == "-" {
        "0".into()
    } else {
        text
    }
}

/// One typed number of the form: a finite decimal, as the command takes it.
fn number(label: &'static str, value: &str) -> Result<f64, String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 32 {
        return Err(format!("{label}: enter a number"));
    }
    value.parse::<f64>().map_or_else(
        |_| {
            Err(format!(
                "{label}: \"{value}\" is not a number; use a period as the decimal separator"
            ))
        },
        |parsed| {
            if parsed.is_finite() {
                Ok(parsed)
            } else {
                Err(format!("{label}: \"{value}\" is not a finite number"))
            }
        },
    )
}

/// The `--count` of a generator: a whole number.
fn count(label: &'static str, value: &str) -> Result<usize, String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 32 {
        return Err(format!("{label}: enter a whole number"));
    }
    value
        .parse::<usize>()
        .map_err(|_| format!("{label}: \"{value}\" is not a whole number"))
}

/// The `--spacings` list: comma-separated finite numbers.
fn spacings(value: &str) -> Result<Vec<f64>, String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 256 {
        return Err("Spacings: enter comma-separated numbers".into());
    }
    let parsed = value
        .split(',')
        .map(|part| {
            part.trim().parse::<f64>().map_err(|_| {
                format!("Spacings: \"{value}\" is not a comma-separated list of numbers")
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if parsed.is_empty() || parsed.iter().any(|n| !n.is_finite()) {
        return Err("Spacings: enter comma-separated numbers".into());
    }
    Ok(parsed)
}

/// Three world millimetres, as a command's point option takes them.
fn point_text(p: [f64; 3]) -> String {
    p.iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

/// The checked name, parts and points of the form, shared by both commands.
type Placement = (String, Vec<String>, [f64; 3], [f64; 3]);

fn checked_placement(panel: &Panel) -> Result<Placement, String> {
    let name = panel.name.trim();
    if name.is_empty() {
        return Err("Name: enter a label for the group".into());
    }
    if name.len() > MAX_TEXT {
        return Err(format!("Name: at most {MAX_TEXT} bytes"));
    }
    if panel.parts.len() < 2 {
        return Err("Pick the main part, then the secondary part, in the model view.".into());
    }
    if panel
        .parts
        .windows(2)
        .any(|pair| pair.first() == pair.last())
    {
        return Err("A group connects distinct parts; pick a different secondary part.".into());
    }
    let (Some(point1), Some(point2)) = (panel.point1, panel.point2) else {
        return Err(
            "Pick the two points on the work plane: the first fastener's spot, then the row's direction.".into(),
        );
    };
    if point1
        .iter()
        .zip(point2)
        .all(|(a, b)| (a - b).abs() <= 1e-9)
    {
        return Err("The two points coincide; pick a different second point.".into());
    }
    Ok((name.to_owned(), panel.parts.clone(), point1, point2))
}

/// The checked fastener library entry of the form: path and designation.
fn checked_fastener(panel: &Panel) -> Result<(String, String), String> {
    let library = panel.library.trim();
    if library.is_empty() {
        return Err("Library: enter the path of the fastener library file.".into());
    }
    if library.len() > MAX_PATH {
        return Err(format!("Library: at most {MAX_PATH} bytes"));
    }
    let designation = panel.designation.trim();
    if designation.is_empty() {
        return Err(
            "Designation: pick a fastener from the library (Load entries lists them).".into(),
        );
    }
    if designation.len() > MAX_TEXT {
        return Err(format!("Designation: at most {MAX_TEXT} bytes"));
    }
    Ok((library.to_owned(), designation.to_owned()))
}

/// The checked position values, spacing pattern and hole setting of the form, as the option
/// tail from `--point1` on; `normal` is the plane normal the commit sends.
fn checked_tail(panel: &Panel, normal: [f64; 3]) -> Result<Vec<String>, String> {
    let (Some(point1), Some(point2)) = (panel.point1, panel.point2) else {
        return Err(
            "Pick the two points on the work plane: the first fastener's spot, then the row's direction.".into(),
        );
    };
    let offset_start = number("Offset start", &panel.offset_start)?;
    let on_plane = number("On plane", &panel.on_plane)?;
    let rotation = number("Rotation", &panel.rotation)?;
    let depth = number("Depth", &panel.depth)?;
    let clearance = match panel.clearance.trim() {
        "" => None,
        value => Some(number("Clearance", value)?),
    };
    let mut args = vec![
        "--point1".into(),
        point_text(point1),
        "--point2".into(),
        point_text(point2),
        "--plane-normal".into(),
        point_text(normal),
        "--rotation-deg".into(),
        rotation.to_string(),
        "--on-plane".into(),
        on_plane.to_string(),
        "--depth".into(),
        depth.to_string(),
        "--offset-start".into(),
        offset_start.to_string(),
    ];
    match panel.spacing {
        Spacing::List => {
            let gaps = spacings(&panel.spacings)?;
            args.extend([
                "--spacings".into(),
                gaps.iter()
                    .map(|g| g.to_string())
                    .collect::<Vec<_>>()
                    .join(","),
            ]);
        }
        Spacing::Pitch => {
            let pitch = number("Pitch", &panel.pitch)?;
            let count = count("Count", &panel.count)?;
            args.extend([
                "--pitch".into(),
                pitch.to_string(),
                "--count".into(),
                count.to_string(),
            ]);
        }
        Spacing::OffsetEnd => {
            let end = number("Offset end", &panel.offset_end)?;
            let count = count("Count", &panel.count)?;
            args.extend([
                "--offset-end".into(),
                end.to_string(),
                "--count".into(),
                count.to_string(),
            ]);
        }
    }
    args.extend([
        "--washer-under-head".into(),
        panel.washer_head.to_string(),
        "--washer-under-nut".into(),
        panel.washer_nut.to_string(),
        "--hole-mode".into(),
        panel.holes.token().into(),
    ]);
    if let Some(clearance) = clearance {
        args.extend(["--hole-clearance".into(), clearance.to_string()]);
    }
    Ok(args)
}

/// The `create-bolt-group` arguments of the form, exactly as the command takes them. `normal`
/// is the active work plane's normal, `0,0,1` without one, taken when the commit starts.
pub(crate) fn create_args(
    panel: &Panel,
    normal: [f64; 3],
    store: &Path,
    actor: Option<&str>,
) -> Result<Vec<String>, String> {
    let (name, parts, _, _) = checked_placement(panel)?;
    let (library, designation) = checked_fastener(panel)?;
    let mut args = vec![
        "create-bolt-group".to_owned(),
        "--store".into(),
        store.to_string_lossy().into_owned(),
        "--name".into(),
        name,
        "--parts".into(),
        parts.join(","),
        "--fastener-library".into(),
        library,
        "--designation".into(),
        designation,
    ];
    args.extend(checked_tail(panel, normal)?);
    if let Some(actor) = actor {
        args.extend(["--actor-id".into(), actor.to_owned()]);
    }
    Ok(args)
}

/// The `update-bolt-group` arguments of the loaded group's edit: the complete new inputs
/// replace the stored ones, and `--repin` follows the library entry the form names. The
/// length arguments stay absent on purpose, so the command keeps the stored length mode.
pub(crate) fn update_args(
    panel: &Panel,
    normal: [f64; 3],
    store: &Path,
    actor: Option<&str>,
) -> Result<Vec<String>, String> {
    let Some(target) = panel.edit_target.as_deref() else {
        return Err("No group is loaded for editing.".into());
    };
    let (name, parts, _, _) = checked_placement(panel)?;
    let mut args = vec![
        "update-bolt-group".to_owned(),
        "--store".into(),
        store.to_string_lossy().into_owned(),
        "--target".into(),
        target.to_owned(),
        "--name".into(),
        name,
        "--parts".into(),
        parts.join(","),
    ];
    if panel.repin {
        let (library, designation) = checked_fastener(panel)?;
        args.extend([
            "--repin".into(),
            "true".into(),
            "--fastener-library".into(),
            library,
            "--designation".into(),
            designation,
        ]);
    }
    args.extend(checked_tail(panel, normal)?);
    if let Some(actor) = actor {
        args.extend(["--actor-id".into(), actor.to_owned()]);
    }
    Ok(args)
}

/// The read-only `list-library` call of the fastener listing.
pub(crate) fn list_args(library: &str) -> Vec<String> {
    vec![
        "list-library".into(),
        "--fastener-library".into(),
        library.to_owned(),
        "--family".into(),
        "fastener".into(),
        "--limit".into(),
        MAX_ENTRIES.to_string(),
    ]
}

/// The designations of a listing reply.
pub(crate) fn parse_entries(
    exit_code: Option<i32>,
    stdout: &[u8],
) -> Result<Vec<String>, ToolError> {
    let reply = decode(stdout)?;
    if let Some(error) = rejection(&reply, exit_code) {
        return Err(error);
    }
    Ok(reply
        .get("entries")
        .and_then(Json::array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| text(entry, "designation"))
                .take(MAX_ENTRIES)
                .collect()
        })
        .unwrap_or_default())
}

/// The read-only `bolts` call of the groups listing.
pub(crate) fn group_list_args(store: &Path) -> Vec<String> {
    vec![
        "bolts".into(),
        "--store".into(),
        store.to_string_lossy().into_owned(),
    ]
}

/// The placed groups of a `bolts` listing; an entry whose fields do not parse is skipped, so
/// one unreadable group never hides the others.
pub(crate) fn parse_groups(
    exit_code: Option<i32>,
    stdout: &[u8],
) -> Result<Vec<Listing>, ToolError> {
    let reply = decode(stdout)?;
    if let Some(error) = rejection(&reply, exit_code) {
        return Err(error);
    }
    let f64s = |json: &Json, key: &str| -> Option<[f64; 3]> {
        let values = json.get(key).ok().and_then(|v| v.array().ok())?;
        let parsed = values
            .iter()
            .filter_map(|v| v.string().ok()?.parse::<f64>().ok())
            .collect::<Vec<_>>();
        <[f64; 3]>::try_from(parsed.as_slice()).ok()
    };
    let number = |json: &Json, key: &str| -> Option<f64> {
        json.get(key)
            .ok()
            .and_then(|v| v.string().ok())?
            .parse::<f64>()
            .ok()
    };
    let flag = |json: &Json, key: &str| -> Option<bool> {
        match text(json, key)?.as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        }
    };
    Ok(reply
        .get("bolt_groups")
        .and_then(Json::array)
        .map(|groups| {
            groups
                .iter()
                .take(MAX_GROUPS)
                .filter_map(|group| {
                    let spacings = group
                        .get("spacings_mm")
                        .ok()?
                        .array()
                        .ok()?
                        .iter()
                        .filter_map(|v| v.string().ok()?.parse::<f64>().ok())
                        .collect::<Vec<_>>();
                    Some(Listing {
                        id: text(group, "id")?,
                        name: text(group, "name")?,
                        parts: group
                            .get("parts")
                            .ok()?
                            .array()
                            .ok()?
                            .iter()
                            .filter_map(|v| v.string().ok().map(str::to_owned))
                            .collect(),
                        point1: f64s(group, "point1_mm")?,
                        point2: f64s(group, "point2_mm")?,
                        normal: f64s(group, "plane_normal_mm")?,
                        rotation_rad: number(group, "rotation_rad")?,
                        on_plane: number(group, "on_plane_mm")?,
                        depth: number(group, "depth_mm")?,
                        offset_start: number(group, "offset_start_mm")?,
                        spacing_mode: text(group, "spacing_mode")?,
                        spacing_value: number(group, "spacing_value_mm")?,
                        spacings,
                        washer_head: flag(group, "washer_under_head")?,
                        washer_nut: flag(group, "washer_under_nut")?,
                        hole_mode: text(group, "hole_mode")?,
                        hole_clearance: match group.get("hole_clearance_mm") {
                            Ok(Json::Null) | Err(_) => None,
                            Ok(_) => number(group, "hole_clearance_mm"),
                        },
                    })
                })
                .collect()
        })
        .unwrap_or_default())
}

impl App {
    /// Resolves a placement click on the viewport: the first two clicks pick the main and the
    /// secondary part (as the Weld bench does), the next two pick the points on the work plane.
    /// With an active model view only what the view shows can be picked (V12), as selection does.
    pub(crate) fn bolt_click(&mut self, ray: Ray) {
        if self.other_command_running() || self.bolt.busy() {
            return;
        }
        let Some(loaded) = &self.loaded else {
            return;
        };
        if self.bolt.parts.len() < 2 {
            let view = self.views.active_view().cloned();
            let picked = match view.as_ref() {
                Some(view) => beamcraft_engine::views::pick_visible(
                    &loaded.snapshot,
                    view,
                    ray.origin,
                    ray.direction,
                    self.selection.filter(),
                )
                .map(|hit| hit.map(|part| part.element_id.to_string()))
                .map_err(|e| ToolError::local("PICK_REFUSED", e.to_string())),
                None => pick_in(&loaded.snapshot, &ray, self.selection.filter()),
            };
            match picked {
                Ok(Some(id)) => {
                    if self.bolt.parts.iter().any(|picked| picked == &id) {
                        self.bolt.note = Some(Err(
                            "Already picked; pick a different part for the other side.".into(),
                        ));
                        return;
                    }
                    self.bolt.parts.push(id);
                    self.bolt.note = None;
                }
                Ok(None) => {
                    self.bolt.note = Some(Err("No part was hit; aim at a model face.".into()));
                }
                Err(error) => self.bolt.note = Some(Err(error.to_string())),
            }
            return;
        }
        let point = match self.creation_point(ray, false) {
            Ok(point) => point,
            Err(error) => {
                self.bolt.note = Some(Err(error.to_string()));
                return;
            }
        };
        let slot = if self.bolt.point1.is_none() {
            &mut self.bolt.point1
        } else {
            &mut self.bolt.point2
        };
        if slot
            .as_ref()
            .is_some_and(|picked| picked.iter().zip(point).all(|(a, b)| (a - b).abs() <= 1e-9))
        {
            self.bolt.note = Some(Err("The points coincide; pick a different spot.".into()));
            return;
        }
        *slot = Some(point);
        self.bolt.note = None;
    }

    /// Starts the fastener listing on a worker: the read-only `list-library` a user types.
    pub(crate) fn start_entries_load(&mut self) {
        if self.other_command_running() {
            self.bolt.note = Some(Err(
                "another command is running; wait for it to finish.".into()
            ));
            return;
        }
        let library = self.bolt.library.trim().to_owned();
        if library.is_empty() {
            self.bolt.note = Some(Err(
                "Library: enter the path of the fastener library file.".into()
            ));
            return;
        }
        let cli = self.source.cli.clone();
        let stats = Arc::clone(&self.stats);
        let args = list_args(&library);
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new()
            .name("beamcraft-bolt-entries".into())
            .spawn(move || {
                let result = command::run(&cli, &args, command::DEADLINE, &stats)
                    .and_then(|out| parse_entries(out.code, &out.stdout));
                let _ = tx.send(result);
            });
        match spawned {
            Ok(_) => self.bolt.entries_job = Some(rx),
            Err(_) => {
                self.bolt.note = Some(Err("The library listing could not be started.".into()))
            }
        }
    }

    /// Starts the groups listing on a worker: the read-only `bolts` a user types. Quiet when
    /// another command runs (the bench's opening load), loud when the caller has a note.
    pub(crate) fn bolt_load(&mut self) {
        if self.other_command_running() || self.bolt.groups_job.is_some() {
            return;
        }
        let cli = self.source.cli.clone();
        let stats = Arc::clone(&self.stats);
        let args = group_list_args(&self.source.store);
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new()
            .name("beamcraft-bolt-groups".into())
            .spawn(move || {
                let result = command::run(&cli, &args, command::DEADLINE, &stats)
                    .and_then(|out| parse_groups(out.code, &out.stdout));
                let _ = tx.send(result);
            });
        if spawned.is_ok() {
            self.bolt.groups_job = Some(rx);
        }
    }

    /// Starts the committed `create-bolt-group` or `update-bolt-group` on a worker: the same
    /// command a user types.
    pub(crate) fn start_bolt_commit(&mut self) {
        if self.other_command_running() {
            self.bolt.note = Some(Err(
                "another command is running; wait for it to finish.".into()
            ));
            return;
        }
        let normal = self.bolt.normal.unwrap_or_else(|| {
            self.tools
                .plane
                .map_or([0.0, 0.0, 1.0], |plane| plane.normal())
        });
        let editing = self.bolt.edit_target.is_some();
        let args = match if editing {
            update_args(
                &self.bolt,
                normal,
                &self.source.store,
                self.source.actor.as_deref(),
            )
        } else {
            create_args(
                &self.bolt,
                normal,
                &self.source.store,
                self.source.actor.as_deref(),
            )
        } {
            Ok(args) => args,
            Err(message) => {
                self.bolt.note = Some(Err(message));
                return;
            }
        };
        let cli = self.source.cli.clone();
        let stats = Arc::clone(&self.stats);
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new()
            .name("beamcraft-bolt-create".into())
            .spawn(move || {
                let result = command::run(&cli, &args, command::DEADLINE, &stats).and_then(|out| {
                    let reply = decode(&out.stdout)?;
                    rejection(&reply, out.code).map_or(Ok(()), Err)
                });
                let _ = tx.send(result);
            });
        match spawned {
            Ok(_) => {
                self.bolt.commit_job = Some(rx);
                self.bolt.note = None;
            }
            Err(_) => self.bolt.note = Some(Err("The bolt command could not be started.".into())),
        }
    }

    /// Drains the listings and the commit; a success reloads the model the viewport shows.
    pub(crate) fn poll_bolt(&mut self) {
        if let Some(rx) = &self.bolt.groups_job {
            let outcome = match rx.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Err(ToolError::local(
                    "CLI_FAILED",
                    "The groups listing ended without a result.",
                ))),
            };
            if outcome.is_some() {
                self.bolt.groups_job = None;
            }
            match outcome {
                Some(Ok(groups)) => self.bolt.groups = groups,
                Some(Err(error)) => self.bolt.note = Some(Err(error.to_string())),
                None => {}
            }
        }
        if let Some(rx) = &self.bolt.entries_job {
            let outcome = match rx.try_recv() {
                Ok(result) => result,
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => Err(ToolError::local(
                    "CLI_FAILED",
                    "The library listing ended without a result.",
                )),
            };
            self.bolt.entries_job = None;
            match outcome {
                Ok(entries) if entries.is_empty() => {
                    self.bolt.entries = Vec::new();
                    self.bolt.note = Some(Err("The library lists no fasteners.".into()));
                }
                Ok(entries) => {
                    let count = entries.len();
                    if !self
                        .bolt
                        .entries
                        .iter()
                        .any(|listed| listed == &self.bolt.designation)
                        && let Some(first) = entries.first().cloned()
                    {
                        self.bolt.designation = first;
                    }
                    self.bolt.entries = entries;
                    self.bolt.note = Some(Ok(format!("{count} fasteners listed.")));
                }
                Err(error) => self.bolt.note = Some(Err(error.to_string())),
            }
        }
        let Some(rx) = &self.bolt.commit_job else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(ToolError::local(
                "CLI_FAILED",
                "The bolt command ended without a result.",
            )),
        };
        self.bolt.commit_job = None;
        match result {
            Ok(()) => {
                let updated = self.bolt.edit_target.is_some();
                self.bolt.clear_placement();
                self.bolt.note = Some(Ok(if updated {
                    "Updated bolt group.".into()
                } else {
                    "Created bolt group. Pick new parts, or press Esc to finish.".into()
                }));
                self.bolt_load();
                self.reload();
            }
            Err(error) => {
                if error.code == "OUTCOME_UNKNOWN" {
                    self.reload();
                }
                self.bolt.note = Some(Err(error.to_string()));
            }
        }
    }

    /// The inspector panel of the Bolt bench.
    pub(crate) fn bolt_panel(&mut self, ui: &mut egui::Ui) {
        let palette = crate::theme::Palette::current(ui);
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
        ui.set_max_width(ui.available_width());
        let busy = self.bolt.busy() || self.other_command_running();
        let editing = self.bolt.edit_target.is_some();
        ui.label(crate::theme::bold(
            if editing {
                "Edit bolt group"
            } else {
                "Bolt properties"
            },
            16.0,
        ));
        // The listing is shown from a local copy: picking a row mutates the panel mid-loop.
        let listed_groups = self.bolt.groups.clone();
        let loaded_id = self.bolt.edit_target.clone();
        ui.horizontal(|ui| {
            ui.add_sized(
                egui::vec2(70.0, 28.0),
                egui::Label::new(egui::RichText::new("Groups").size(11.0)),
            );
            let loaded = listed_groups
                .iter()
                .find(|listed| Some(listed.id.as_str()) == loaded_id.as_deref());
            // In edit mode the row also holds the New-group button: a fixed combo width keeps
            // both on one line.
            let combo_width = if editing {
                110.0
            } else {
                ui.available_width().max(80.0)
            };
            egui::ComboBox::from_id_salt("bolt-groups")
                .width(combo_width)
                .selected_text(match loaded {
                    Some(listed) => listed.name.as_str(),
                    None if listed_groups.is_empty() => "— none in the model yet",
                    None => "— pick a group to edit",
                })
                .show_ui(ui, |ui| {
                    for listed in &listed_groups {
                        let label = format!("{} · {}", listed.name, listed.spacings.len() + 1);
                        if ui
                            .selectable_label(loaded.is_some_and(|l| l.id == listed.id), label)
                            .clicked()
                        {
                            let loaded = listed.clone();
                            self.bolt.load_group(&loaded);
                        }
                    }
                });
            if editing && ui.button("New group").clicked() {
                self.bolt.exit_edit();
            }
        });
        let picked = |ui: &mut egui::Ui, label: &str, value: Option<&String>| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(label).size(11.0));
                match value {
                    Some(id) => ui.label(RichText::new(id.as_str()).size(11.0)),
                    None => ui.label(
                        RichText::new("— click a part")
                            .size(11.0)
                            .color(palette.muted),
                    ),
                };
            });
        };
        let point = |ui: &mut egui::Ui, label: &str, value: Option<[f64; 3]>| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(label).size(11.0));
                match value {
                    Some(p) => ui.label(RichText::new(PointText(p).to_string()).size(11.0)),
                    None => ui.label(
                        RichText::new("— click a point")
                            .size(11.0)
                            .color(palette.muted),
                    ),
                };
            });
        };
        for (index, part) in self.bolt.parts.iter().enumerate() {
            picked(ui, &format!("Part {}", index + 1), Some(part));
        }
        if self.bolt.parts.len() < 2 {
            picked(ui, &format!("Part {}", self.bolt.parts.len() + 1), None);
        }
        point(ui, "Point 1", self.bolt.point1);
        point(ui, "Point 2", self.bolt.point2);
        ui.horizontal(|ui| {
            ui.label(
                match (
                    self.bolt.parts.len(),
                    self.bolt.point1.is_some(),
                    self.bolt.point2.is_some(),
                ) {
                    (0..=0, _, _) => "Click the main part in the model view.",
                    (1, _, _) => "Click the secondary part in the model view.",
                    (_, false, _) => "Click the first point on the work plane.",
                    (_, true, false) => {
                        "Click the second point: the row runs from point 1 towards it."
                    }
                    _ => "Parts and points are picked; fill the form.",
                },
            );
            if ui
                .add_enabled(
                    !self.bolt.parts.is_empty()
                        || self.bolt.point1.is_some()
                        || self.bolt.point2.is_some(),
                    egui::Button::new("Clear picks"),
                )
                .clicked()
            {
                self.bolt.clear_placement();
            }
        });
        field(ui, "Name", &mut self.bolt.name, 160.0);
        field(ui, "Fastener library (path)", &mut self.bolt.library, 240.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!busy, egui::Button::new("Load entries"))
                .clicked()
            {
                self.start_entries_load();
            }
            if editing {
                ui.checkbox(&mut self.bolt.repin, "Follow library (--repin)");
            }
            egui::ComboBox::from_id_salt("bolt-designation")
                .width(ui.available_width().max(80.0))
                .selected_text(if self.bolt.designation.is_empty() {
                    "— load a library first"
                } else {
                    self.bolt.designation.as_str()
                })
                .show_ui(ui, |ui| {
                    for listed in &self.bolt.entries {
                        if ui
                            .selectable_label(self.bolt.designation == *listed, listed.as_str())
                            .clicked()
                        {
                            self.bolt.designation = listed.clone();
                        }
                    }
                });
        });
        let small_label = |ui: &mut egui::Ui, width: f32, label: &str| {
            ui.add_sized(
                egui::vec2(width, 28.0),
                egui::Label::new(egui::RichText::new(label).size(11.0)),
            );
        };
        let small_field = |ui: &mut egui::Ui, width: f32, text: &mut String| {
            ui.add(
                egui::TextEdit::singleline(text)
                    .desired_width(width.min(ui.available_width()))
                    .char_limit(MAX_TEXT),
            );
        };
        ui.horizontal(|ui| {
            small_label(ui, 44.0, "Holes");
            egui::ComboBox::from_id_salt("bolt-holes")
                .width(120.0)
                .selected_text(self.bolt.holes.label())
                .show_ui(ui, |ui| {
                    for choice in [Holes::Generate, Holes::Quiet] {
                        if ui
                            .selectable_label(self.bolt.holes == choice, choice.label())
                            .clicked()
                        {
                            self.bolt.holes = choice;
                        }
                    }
                });
            small_label(ui, 70.0, "Clearance (mm)");
            small_field(ui, 50.0, &mut self.bolt.clearance);
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.bolt.washer_head, "Washer under head");
            ui.checkbox(&mut self.bolt.washer_nut, "under nut");
        });
        ui.horizontal(|ui| {
            small_label(ui, 70.0, "On plane (mm)");
            small_field(ui, 50.0, &mut self.bolt.on_plane);
            small_label(ui, 56.0, "Rot (deg)");
            small_field(ui, 44.0, &mut self.bolt.rotation);
            small_label(ui, 44.0, "Depth (mm)");
            small_field(ui, 50.0, &mut self.bolt.depth);
        });
        ui.horizontal(|ui| {
            small_label(ui, 70.0, "Offset start (mm)");
            small_field(ui, 50.0, &mut self.bolt.offset_start);
            small_label(ui, 56.0, "Spacing");
            egui::ComboBox::from_id_salt("bolt-spacing")
                .width(ui.available_width().max(70.0))
                .selected_text(self.bolt.spacing.label())
                .show_ui(ui, |ui| {
                    for choice in [Spacing::List, Spacing::Pitch, Spacing::OffsetEnd] {
                        if ui
                            .selectable_label(self.bolt.spacing == choice, choice.label())
                            .clicked()
                        {
                            self.bolt.spacing = choice;
                        }
                    }
                });
        });
        ui.horizontal(|ui| {
            match self.bolt.spacing {
                Spacing::List => {
                    small_label(ui, 70.0, "Spacings (mm)");
                    small_field(ui, 120.0, &mut self.bolt.spacings);
                }
                Spacing::Pitch => {
                    small_label(ui, 70.0, "Pitch (mm)");
                    small_field(ui, 50.0, &mut self.bolt.pitch);
                }
                Spacing::OffsetEnd => {
                    small_label(ui, 70.0, "Offset end (mm)");
                    small_field(ui, 50.0, &mut self.bolt.offset_end);
                }
            };
            if self.bolt.spacing != Spacing::List {
                small_label(ui, 44.0, "Count");
                small_field(ui, 44.0, &mut self.bolt.count);
            }
        });
        ui.add_enabled_ui(!busy, |ui| {
            ui.horizontal(|ui| {
                let label = if editing {
                    "Update group"
                } else {
                    "Create bolts"
                };
                if ui.button(label).clicked() {
                    self.start_bolt_commit();
                }
                if ui.button("Cancel").clicked() {
                    self.bolt.clear_placement();
                    self.bolt.note = None;
                    self.set_bench(super::workbench::Bench::Select);
                }
            });
        });
        if busy {
            ui.spinner();
            ui.label("Saving bolt group…");
        }
        match &self.bolt.note {
            Some(Ok(message)) => {
                ui.label(message);
            }
            Some(Err(message)) => {
                ui.colored_label(palette.error, message);
            }
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    const MAIN: &str = "0197aabb-ccdd-7ebb-9beb-59df63a5fc11";
    const OTHER: &str = "0197aabb-ccdd-7ebb-9beb-59df63a5fc12";

    fn form() -> Panel {
        let mut panel = Panel::empty();
        panel.parts = vec![MAIN.to_owned(), OTHER.to_owned()];
        panel.point1 = Some([100.0, 250.0, 0.0]);
        panel.point2 = Some([600.0, 250.0, 0.0]);
        panel.library = "fasteners.usda".into();
        panel.designation = "SYN-BOLT-12x50".into();
        panel
    }

    fn args(spacing: Spacing) -> Vec<String> {
        let mut panel = form();
        panel.spacing = spacing;
        create_args(&panel, [0.0, 0.0, 1.0], Path::new("S"), None).unwrap()
    }

    #[test]
    fn the_form_builds_the_exact_create_bolt_group_command() {
        assert_eq!(
            args(Spacing::List),
            vec![
                "create-bolt-group",
                "--store",
                "S",
                "--name",
                "Bolt group",
                "--parts",
                &format!("{MAIN},{OTHER}"),
                "--fastener-library",
                "fasteners.usda",
                "--designation",
                "SYN-BOLT-12x50",
                "--point1",
                "100,250,0",
                "--point2",
                "600,250,0",
                "--plane-normal",
                "0,0,1",
                "--rotation-deg",
                "0",
                "--on-plane",
                "0",
                "--depth",
                "0",
                "--offset-start",
                "50",
                "--spacings",
                "150",
                "--washer-under-head",
                "true",
                "--washer-under-nut",
                "true",
                "--hole-mode",
                "generate",
            ]
        );
        let mut pitched = form();
        pitched.spacing = Spacing::Pitch;
        pitched.rotation = "90".into();
        pitched.depth = "-10".into();
        pitched.holes = Holes::Quiet;
        pitched.clearance = "1.5".into();
        let pitched =
            create_args(&pitched, [0.0, 1.0, 0.0], Path::new("S"), Some("actor")).unwrap();
        for expected in [
            &["--pitch", "150"],
            &["--count", "2"],
            &["--rotation-deg", "90"],
            &["--depth", "-10"],
            &["--plane-normal", "0,1,0"],
            &["--hole-mode", "none"],
            &["--hole-clearance", "1.5"],
            &["--actor-id", "actor"],
        ] {
            assert!(
                pitched.windows(2).any(|pair| pair == expected),
                "{expected:?}"
            );
        }
        let mut ended = form();
        ended.spacing = Spacing::OffsetEnd;
        let ended = create_args(&ended, [0.0, 0.0, 1.0], Path::new("S"), None).unwrap();
        assert!(ended.windows(2).any(|pair| pair == ["--offset-end", "50"]));
        assert!(ended.windows(2).any(|pair| pair == ["--count", "2"]));
        assert!(!ended.windows(2).any(|pair| pair == ["--spacings", "150"]));
    }

    #[test]
    fn an_incomplete_or_malformed_form_refuses_without_a_command() {
        let refused = |change: &dyn Fn(&mut Panel)| {
            let mut panel = form();
            change(&mut panel);
            create_args(&panel, [0.0, 0.0, 1.0], Path::new("S"), None)
                .err()
                .unwrap_or_else(|| panic!("the form must refuse"))
        };
        assert!(refused(&|p| p.name = "  ".into()).contains("Name"));
        assert!(refused(&|p| p.name = "x".repeat(MAX_TEXT + 1)).contains("Name"));
        assert!(refused(&|p| p.parts.clear()).contains("Pick the main part"));
        assert!(refused(&|p| p.parts.truncate(1)).contains("Pick the main part"));
        assert!(
            refused(&|p| p.parts = vec![MAIN.to_owned(), MAIN.to_owned()])
                .contains("distinct parts")
        );
        assert!(refused(&|p| p.point1 = None).contains("two points"));
        assert!(refused(&|p| p.point2 = None).contains("two points"));
        assert!(refused(&|p| p.point2 = Some([100.0, 250.0, 0.0])).contains("coincide"));
        assert!(refused(&|p| p.library = " ".into()).contains("Library"));
        assert!(refused(&|p| p.designation = String::new()).contains("Designation"));
        assert!(refused(&|p| p.offset_start = "wide".into()).contains("Offset start"));
        assert!(refused(&|p| p.on_plane = "wide".into()).contains("On plane"));
        assert!(refused(&|p| p.rotation = "wide".into()).contains("Rotation"));
        assert!(refused(&|p| p.depth = "wide".into()).contains("Depth"));
        assert!(refused(&|p| p.clearance = "wide".into()).contains("Clearance"));
        assert!(refused(&|p| p.spacings = "150,".into()).contains("Spacings"));
        assert!(refused(&|p| p.spacings = String::new()).contains("Spacings"));
        let pitched = |change: &dyn Fn(&mut Panel)| {
            let mut panel = form();
            panel.spacing = Spacing::Pitch;
            change(&mut panel);
            create_args(&panel, [0.0, 0.0, 1.0], Path::new("S"), None)
                .err()
                .unwrap_or_else(|| panic!("the form must refuse"))
        };
        assert!(pitched(&|p| p.pitch = "wide".into()).contains("Pitch"));
        assert!(pitched(&|p| p.count = "-1".into()).contains("Count"));
        assert!(pitched(&|p| p.count = "2.5".into()).contains("Count"));
        // An engine-level value (a non-positive spacing) reaches the real command.
        assert!(
            create_args(
                &{
                    let mut panel = form();
                    panel.spacings = "0".into();
                    panel
                },
                [0.0, 0.0, 1.0],
                Path::new("S"),
                None,
            )
            .is_ok()
        );
    }

    #[test]
    fn parse_entries_reads_the_designations_and_refuses_a_bad_reply() {
        let listing =
            br#"{"outcome":"library","entries":[{"designation":"A"},{"designation":"B"}]}"#;
        let parsed = parse_entries(Some(0), listing).unwrap();
        assert_eq!(parsed, ["A", "B"]);
        assert!(parse_entries(Some(0), b"not json").is_err());
        let refused = br#"{"outcome":"rejected","code":"LIBRARY_UNREADABLE","message":"no"}"#;
        assert!(parse_entries(Some(2), refused).is_err());
        assert!(
            parse_entries(Some(0), br#"{"outcome":"library"}"#)
                .unwrap()
                .is_empty()
        );
    }

    /// The form of one loaded group, as `bolts` reports it for the shipped synthetic library.
    fn listed() -> Listing {
        Listing {
            id: "019a7bf0-0000-7000-8000-00000071c001".into(),
            name: "G1".into(),
            parts: vec![MAIN.to_owned(), OTHER.to_owned()],
            point1: [100.0, 250.0, 0.0],
            point2: [600.0, 250.0, 0.0],
            normal: [0.0, 0.0, 1.0],
            rotation_rad: 0.0,
            on_plane: 0.0,
            depth: 0.0,
            offset_start: 50.0,
            spacing_mode: "spacings".into(),
            spacing_value: 0.0,
            spacings: vec![150.0],
            washer_head: true,
            washer_nut: true,
            hole_mode: "generate".into(),
            hole_clearance: Some(1.5),
        }
    }

    #[test]
    fn update_args_sends_the_whole_edit_and_repins_only_when_asked() {
        let mut panel = form();
        panel.edit_target = Some("019a7bf0-0000-7000-8000-00000071c001".into());
        panel.normal = Some([0.0, 0.0, 1.0]);
        let updated = update_args(&panel, [0.0, 0.0, 1.0], Path::new("S"), None).unwrap();
        assert_eq!(
            updated[..10],
            [
                "update-bolt-group".to_owned(),
                "--store".into(),
                "S".into(),
                "--target".into(),
                "019a7bf0-0000-7000-8000-00000071c001".to_owned(),
                "--name".into(),
                "Bolt group".into(),
                "--parts".into(),
                format!("{MAIN},{OTHER}"),
                "--point1".into(),
            ]
        );
        assert!(!updated.iter().any(|arg| arg == "--repin"));
        assert!(!updated.iter().any(|arg| arg == "--fastener-library"));
        assert!(!updated.iter().any(|arg| arg == "--length-mode"));
        assert!(updated.windows(2).any(|pair| pair == ["--spacings", "150"]));
        // A repin names the library entry it follows, and travels without one the form refuses.
        panel.repin = true;
        panel.library = String::new();
        assert_eq!(
            update_args(&panel, [0.0, 0.0, 1.0], Path::new("S"), None).unwrap_err(),
            "Library: enter the path of the fastener library file."
        );
        panel.library = "fasteners.usda".into();
        panel.designation = String::new();
        assert!(
            update_args(&panel, [0.0, 0.0, 1.0], Path::new("S"), None)
                .unwrap_err()
                .contains("Designation")
        );
        panel.designation = "SYN-BOLT-12x50".into();
        let repinned = update_args(&panel, [0.0, 0.0, 1.0], Path::new("S"), None).unwrap();
        assert!(repinned.windows(2).any(|pair| pair == ["--repin", "true"]));
        assert!(
            repinned
                .windows(2)
                .any(|pair| pair == ["--fastener-library", "fasteners.usda"])
        );
        // No target, no update.
        panel.edit_target = None;
        assert_eq!(
            update_args(&panel, [0.0, 0.0, 1.0], Path::new("S"), None).unwrap_err(),
            "No group is loaded for editing."
        );
    }

    #[test]
    fn a_loaded_listing_fills_the_form_across_the_stored_patterns() {
        // The listing itself parses from the reply's own text shapes.
        let reply = br#"{"outcome":"bolts","bolt_groups":[
            {"id":"019a7bf0-0000-7000-8000-00000071c001","name":"G1",
             "parts":["0197aabb-ccdd-7ebb-9beb-59df63a5fc11","0197aabb-ccdd-7ebb-9beb-59df63a5fc12"],
             "fastener":"ab","count":"2",
             "point1_mm":["100","250","0"],"point2_mm":["600","250","0"],
             "plane_normal_mm":["0","0","1"],"rotation_rad":"0.017453292519943295",
             "on_plane_mm":"2.5","depth_mm":"-4","offset_start_mm":"50",
             "spacing_mode":"pitch","spacing_value_mm":"150","spacings_mm":["150","150"],
             "length_mode":"fixed","length_mm":"60",
             "washer_under_head":"true","washer_under_nut":"false",
             "hole_mode":"none","hole_clearance_mm":null}]}"#;
        let groups = parse_groups(Some(0), reply).unwrap();
        assert_eq!(groups.len(), 1);
        assert!(parse_groups(Some(0), b"no").is_err());
        // The stored values come back as the form's texts, one edit away from the command.
        let mut panel = Panel::empty();
        panel.load_group(&groups[0]);
        assert_eq!(panel.edit_target.as_deref(), Some(groups[0].id.as_str()));
        assert_eq!(panel.parts, groups[0].parts);
        assert_eq!(panel.point1, Some([100.0, 250.0, 0.0]));
        assert_eq!(panel.rotation, "1");
        assert_eq!(panel.on_plane, "2.5");
        assert_eq!(panel.depth, "-4");
        assert_eq!(panel.offset_start, "50");
        assert_eq!(panel.spacing, Spacing::Pitch);
        assert_eq!(panel.pitch, "150");
        assert_eq!(panel.count, "3");
        assert!(!panel.washer_nut);
        assert_eq!(panel.holes, Holes::Quiet);
        assert_eq!(panel.clearance, "");
        // The loaded form round-trips through the update arguments: the stored radians come
        // back within the degree the form holds, and the normal is the stored one.
        panel.repin = false;
        let updated = update_args(&panel, panel.normal.unwrap(), Path::new("S"), None).unwrap();
        let rotation = updated
            .iter()
            .position(|arg| arg == "--rotation-deg")
            .map(|index| updated[index + 1].parse::<f64>().unwrap())
            .unwrap();
        assert!((rotation - groups[0].rotation_rad.to_degrees()).abs() < 1e-6);
        assert!(
            updated
                .windows(2)
                .any(|pair| pair == ["--plane-normal", "0,0,1"])
        );
        // An explicit list and an offset-end group load into their own patterns.
        let mut list = listed();
        list.spacing_mode = "offset_end".into();
        list.spacing_value = 45.0;
        let mut panel = Panel::empty();
        panel.load_group(&list);
        assert_eq!(panel.spacing, Spacing::OffsetEnd);
        assert_eq!(panel.offset_end, "45");
        assert_eq!(panel.count, "2");
    }

    #[test]
    fn stored_numbers_come_back_without_trailing_zeros() {
        assert_eq!(clean(0.0), "0");
        assert_eq!(clean(150.0), "150");
        assert_eq!(clean(1.5), "1.5");
        assert_eq!(clean(-10.0), "-10");
        assert_eq!(clean(2.25), "2.25");
    }
}
