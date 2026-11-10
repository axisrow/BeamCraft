// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The Bolt bench (V19): two picked parts and two points on the active plane commit through the
//! same `create-bolt-group` command a user types (A14.3), and a placed group is loaded from the
//! bench's group list and edited through `update-bolt-group` as one atomic edit (slice 2). The
//! fastener travels by designation through a fastener library file, listed read-only with
//! `list-library`; the groups are listed read-only with `bolts`. The reply is the CLI's; a
//! refusal publishes nothing and is shown next to the form.

use super::{App, PointText};
use crate::camera::View;
use crate::command;
use crate::edit::{Ray, pick_in};
use crate::tools::{MAX_TEXT, ToolError, decode, rejection, text};
use beamcraft_engine::ViewSnapshot;
use beamcraft_json::Json;
use beamcraft_model::command::Placement as PartPlacement;
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::id::ElementId;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use steel::engine::bolt::Bounds;
use steel::engine::bolt::GeneratedHole;
use steel::model::command::{BoltGroupInputs, BoltLength, BoltSpacing};
use steel::model::fastener::{FastenerKind, FastenerSpec, HeadKind, HoleMode, WasherSpec};

/// The longest library path the form accepts, as the CLI bounds one argument.
const MAX_PATH: usize = 4096;

/// The most designations one listing may hold, as `--limit` asks for.
const MAX_ENTRIES: usize = 200;

/// The most placed groups one listing may hold.
const MAX_GROUPS: usize = 200;

/// The most candidate holes the preview decides: past this many fasteners times parts the
/// drawing is noise, and the commit's own caps still govern the real group.
const MAX_PREVIEW_HOLES: usize = 4096;

/// The definition fetch of one library-and-designation pair, as the worker returns it: the
/// digest it pins and the specification the preview draws from.
pub(super) type SpecFetch = Result<(ContentDigest, Arc<FastenerSpec>), ToolError>;

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
    /// The fetched definition of the form's designation, as the read-only `show-library`
    /// reported it: the digest it pins and the specification the preview draws from.
    pub(crate) spec: Option<(ContentDigest, Arc<FastenerSpec>)>,
    /// The library and designation the current `spec` answers; a form that names another pair
    /// has no preview yet.
    pub(crate) spec_for: Option<(String, String)>,
    pub(super) spec_job: Option<Receiver<SpecFetch>>,
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
            spec: None,
            spec_for: None,
            spec_job: None,
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

    /// The library and designation pair whose definition the preview needs, when the form
    /// names a complete pair of the loaded listing that the fetched spec does not answer yet.
    /// A designation the listing does not carry fetches nothing, so typing never chases
    /// half-entered names.
    pub(crate) fn spec_query(&self) -> Option<(String, String)> {
        let library = self.library.trim().to_owned();
        let designation = self.designation.trim().to_owned();
        if library.is_empty()
            || designation.is_empty()
            || !self.entries.iter().any(|listed| listed == &designation)
        {
            return None;
        }
        match &self.spec_for {
            Some((listed_library, listed_designation))
                if listed_library.as_str() == library
                    && listed_designation.as_str() == designation =>
            {
                None
            }
            _ => Some((library, designation)),
        }
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

/// The parsed spacing pattern of the form: what `checked_tail_values` returns for the tail
/// both the commit argv and the preview inputs build from.
#[derive(Debug, Clone, PartialEq)]
enum SpacingValues {
    /// An explicit list of gaps.
    List(Vec<f64>),
    /// One pitch over `count` fasteners.
    Pitch {
        /// The pitch in millimetres.
        pitch: f64,
        /// The fastener count.
        count: usize,
    },
    /// `count` fasteners ending `end` before the row's second point.
    OffsetEnd {
        /// The end offset in millimetres.
        end: f64,
        /// The fastener count.
        count: usize,
    },
}

/// The typed tail of the form: the position values, spacing pattern and hole clearance parsed
/// once, so the commit argv and the preview inputs build from the same numbers and can never
/// disagree about what a field says.
struct TailValues {
    /// The first point.
    point1: [f64; 3],
    /// The second point.
    point2: [f64; 3],
    /// The rotation in degrees, as the command spells it.
    rotation_deg: f64,
    /// The on-plane offset in millimetres.
    on_plane: f64,
    /// The depth in millimetres.
    depth: f64,
    /// The offset start in millimetres.
    offset_start: f64,
    /// The parsed spacing pattern.
    spacing: SpacingValues,
    /// The parsed hole clearance, when the form carries one.
    clearance: Option<f64>,
}

fn checked_tail_values(panel: &Panel) -> Result<TailValues, String> {
    let (Some(point1), Some(point2)) = (panel.point1, panel.point2) else {
        return Err(
            "Pick the two points on the work plane: the first fastener's spot, then the row's direction.".into(),
        );
    };
    let offset_start = number("Offset start", &panel.offset_start)?;
    let on_plane = number("On plane", &panel.on_plane)?;
    let rotation_deg = number("Rotation", &panel.rotation)?;
    let depth = number("Depth", &panel.depth)?;
    let clearance = match panel.clearance.trim() {
        "" => None,
        value => Some(number("Clearance", value)?),
    };
    let spacing = match panel.spacing {
        Spacing::List => SpacingValues::List(spacings(&panel.spacings)?),
        Spacing::Pitch => SpacingValues::Pitch {
            pitch: number("Pitch", &panel.pitch)?,
            count: count("Count", &panel.count)?,
        },
        Spacing::OffsetEnd => SpacingValues::OffsetEnd {
            end: number("Offset end", &panel.offset_end)?,
            count: count("Count", &panel.count)?,
        },
    };
    Ok(TailValues {
        point1,
        point2,
        rotation_deg,
        on_plane,
        depth,
        offset_start,
        spacing,
        clearance,
    })
}

/// The checked position values, spacing pattern and hole setting of the form, as the option
/// tail from `--point1` on; `normal` is the plane normal the commit sends.
fn checked_tail(panel: &Panel, normal: [f64; 3]) -> Result<Vec<String>, String> {
    let tail = checked_tail_values(panel)?;
    let mut args = vec![
        "--point1".into(),
        point_text(tail.point1),
        "--point2".into(),
        point_text(tail.point2),
        "--plane-normal".into(),
        point_text(normal),
        "--rotation-deg".into(),
        tail.rotation_deg.to_string(),
        "--on-plane".into(),
        tail.on_plane.to_string(),
        "--depth".into(),
        tail.depth.to_string(),
        "--offset-start".into(),
        tail.offset_start.to_string(),
    ];
    match tail.spacing {
        SpacingValues::List(gaps) => {
            args.extend([
                "--spacings".into(),
                gaps.iter()
                    .map(|g| g.to_string())
                    .collect::<Vec<_>>()
                    .join(","),
            ]);
        }
        SpacingValues::Pitch { pitch, count } => {
            args.extend([
                "--pitch".into(),
                pitch.to_string(),
                "--count".into(),
                count.to_string(),
            ]);
        }
        SpacingValues::OffsetEnd { end, count } => {
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
    if let Some(clearance) = tail.clearance {
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

/// The engine inputs the preview draws from: the form's values over the fetched definition,
/// through the same parsing the commit argv builds from. The length stays `Auto` — the
/// preview's decisions never read it, and the commit's length is the parts' own decision.
pub(crate) fn preview_inputs(
    panel: &Panel,
    normal: [f64; 3],
    digest: ContentDigest,
    spec: &FastenerSpec,
) -> Result<BoltGroupInputs, String> {
    let (_, parts, _, _) = checked_placement(panel)?;
    let tail = checked_tail_values(panel)?;
    let mut ids = Vec::with_capacity(parts.len());
    for part in &parts {
        ids.push(
            ElementId::parse(part)
                .map_err(|_| format!("Part: \"{part}\" is not an element identity"))?,
        );
    }
    Ok(BoltGroupInputs {
        parts: ids,
        fastener_digest: digest,
        fastener: spec.clone(),
        point1_mm: tail.point1,
        point2_mm: tail.point2,
        plane_normal_mm: normal,
        rotation_rad: tail.rotation_deg.to_radians(),
        on_plane_mm: tail.on_plane,
        depth_mm: tail.depth,
        offset_start_mm: tail.offset_start,
        spacing: match tail.spacing {
            SpacingValues::List(list) => BoltSpacing::Spacings(list),
            SpacingValues::Pitch { pitch, count } => BoltSpacing::Pitch {
                count,
                pitch_mm: pitch,
            },
            SpacingValues::OffsetEnd { end, count } => BoltSpacing::OffsetEnd {
                count,
                offset_end_mm: end,
            },
        },
        washer_under_head: Some(panel.washer_head),
        washer_under_nut: Some(panel.washer_nut),
        length: BoltLength::Auto,
        hole_mode: Some(match panel.holes {
            Holes::Generate => HoleMode::Generate,
            Holes::Quiet => HoleMode::None,
        }),
        hole_clearance_mm: tail.clearance,
    })
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

/// The read-only `show-library` call of the definition fetch: the entry the preview draws from.
pub(crate) fn spec_args(library: &str, designation: &str) -> Vec<String> {
    vec![
        "show-library".into(),
        "--fastener-library".into(),
        library.to_owned(),
        "--designation".into(),
        designation.to_owned(),
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

/// The definition `show-library` names, as the reply's own text and numbers: the digest it
/// pins and the specification the preview draws from. A non-fastener entry, a reply field the
/// entry does not carry, or a definition the model refuses is no spec — the preview draws
/// nothing rather than a guess.
pub(crate) fn parse_spec(
    exit_code: Option<i32>,
    stdout: &[u8],
) -> Result<(ContentDigest, Arc<FastenerSpec>), ToolError> {
    let reply = decode(stdout)?;
    if let Some(error) = rejection(&reply, exit_code) {
        return Err(error);
    }
    let unreadable = |message: &'static str| ToolError::local("LIBRARY_REPLY", message);
    let entry = reply
        .get("entry")
        .map_err(|_| unreadable("the reply carries no entry"))?;
    if text(entry, "kind").as_deref() != Some("fastener") {
        return Err(unreadable("the designation is not a fastener"));
    }
    let digest = entry
        .get("digest")
        .ok()
        .and_then(|d| d.string().ok())
        .and_then(|d| d.parse::<ContentDigest>().ok())
        .ok_or_else(|| unreadable("the entry carries no readable digest"))?;
    let number = |name: &str| -> Result<f64, ToolError> {
        entry
            .get(name)
            .ok()
            .and_then(|v| v.float().ok())
            .ok_or_else(|| unreadable("the entry carries no readable numbers"))
    };
    let optional = |name: &str| -> Result<Option<f64>, ToolError> {
        match entry.get(name) {
            Ok(Json::Null) | Err(_) => Ok(None),
            Ok(value) => value
                .float()
                .ok()
                .map(Some)
                .ok_or_else(|| unreadable("the entry carries no readable numbers")),
        }
    };
    let optional_text = |name: &str| -> Option<String> {
        match entry.get(name) {
            Ok(Json::Null) | Err(_) => None,
            Ok(value) => value.string().ok().map(str::to_owned),
        }
    };
    let flag = |name: &str| -> Result<bool, ToolError> {
        entry
            .get(name)
            .ok()
            .and_then(|v| v.boolean().ok())
            .ok_or_else(|| unreadable("the entry carries no readable switches"))
    };
    // The reply carries the washer as three flat dimensions, or no washer dimensions at all
    // (`washer: null` is the reply's only marker of a bare fastener).
    let washer = if entry.get("washer_outer_diameter_mm").is_ok() {
        Some(WasherSpec {
            outer_diameter_mm: number("washer_outer_diameter_mm")?,
            inner_diameter_mm: number("washer_inner_diameter_mm")?,
            thickness_mm: number("washer_thickness_mm")?,
        })
    } else {
        None
    };
    let lengths = entry
        .get("lengths_mm")
        .ok()
        .and_then(|v| v.array().ok())
        .map(|list| {
            list.iter()
                .filter_map(|v| v.float().ok())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let kind = match text(entry, "fastener_kind").as_deref() {
        Some("bolt") => FastenerKind::Bolt,
        Some("screw") => FastenerKind::Screw,
        _ => return Err(unreadable("the entry names no fastener kind")),
    };
    let head_kind = match text(entry, "head_kind").as_deref() {
        Some("hex") => HeadKind::Hex,
        _ => return Err(unreadable("the entry names no head kind")),
    };
    let hole_mode = match text(entry, "hole_mode").as_deref() {
        Some("none") => HoleMode::None,
        Some("generate") => HoleMode::Generate,
        _ => return Err(unreadable("the entry names no hole mode")),
    };
    let spec = FastenerSpec {
        designation: optional_text("designation"),
        standard: optional_text("standard"),
        grade: optional_text("grade"),
        display_name: optional_text("display_name"),
        kind,
        head_kind,
        nominal_diameter_mm: number("nominal_diameter_mm")?,
        head_across_flats_mm: number("head_across_flats_mm")?,
        head_height_mm: number("head_height_mm")?,
        nut_across_flats_mm: optional("nut_across_flats_mm")?,
        nut_height_mm: optional("nut_height_mm")?,
        washer,
        hole_clearance_mm: number("hole_clearance_mm")?,
        lengths_mm: lengths,
        thread_protrusion_mm: number("thread_protrusion_mm")?,
        washer_under_head: flag("washer_under_head")?,
        washer_under_nut: flag("washer_under_nut")?,
        hole_mode,
        mass_each_kg: optional("mass_each_kg")?,
    };
    spec.validate()
        .map_err(|e| ToolError::local("DEFINITION_INVALID", e.message))?;
    Ok((digest, Arc::new(spec)))
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

/// The preview the Bolt bench draws for the form as filled: the row line, a marker at every
/// fastener bearing and the outline of every generated hole, all world millimetres. The
/// engine decides it over the parts the snapshot shows; it is display only.
pub(crate) struct BoltDraft {
    /// The row's first point.
    pub(crate) point1: [f64; 3],
    /// The row's second point.
    pub(crate) point2: [f64; 3],
    /// The bearing point of every fastener the row places.
    pub(crate) bearings_mm: Vec<[f64; 3]>,
    /// Every generated hole as its world circle polygon at the plate's mid-plane height.
    pub(crate) holes_mm: Vec<Vec<[f64; 3]>>,
}

/// The picked parts' geometry the preview's hole decision reads, as the snapshot on screen
/// reports it.
struct PreviewGeometry {
    /// Every decided part's element-local box (the box of its own mesh before placement: for a
    /// flat plate the contour extents and the material band of its thickness side, the same
    /// box the commit's adapter decides in).
    slabs: BTreeMap<ElementId, Bounds>,
    /// Every shown part's placement.
    placements: BTreeMap<ElementId, PartPlacement>,
    /// The linear members among the parts.
    members: BTreeSet<ElementId>,
    /// The parts whose holes the preview does not decide: ruled-loft plates (bent or lofted).
    /// The commit decides their holes over the authored outline's box (for a bent plate the
    /// developed, unfolded contour), which the shown mesh's box is not and which the snapshot
    /// does not carry; a preview over the mesh box would draw holes the commit does not cut.
    undecided: BTreeSet<ElementId>,
}

/// Reads [`PreviewGeometry`] for the picked parts. The hole decision carries any placement
/// rotation into the element-local frame and refuses a member, as the commit's does (issue 555
/// finding 2). A part whose mesh the snapshot does not carry is left out, so the decision
/// refuses it as unreachable and the preview draws nothing.
fn preview_geometry(snapshot: &ViewSnapshot, parts: &[ElementId]) -> PreviewGeometry {
    let mut geometry = PreviewGeometry {
        slabs: BTreeMap::new(),
        placements: BTreeMap::new(),
        members: BTreeSet::new(),
        undecided: BTreeSet::new(),
    };
    for part in parts {
        let Some(entry) = snapshot.element(part) else {
            continue;
        };
        if steel::kernel::ruled::is_loft_face_table(&entry.full_face_table) {
            geometry.undecided.insert(*part);
        } else if let Some(local) = snapshot
            .mesh(&entry.full.mesh_digest)
            .and_then(|mesh| mesh.local_bounds())
        {
            geometry.slabs.insert(*part, local);
        }
        geometry.placements.insert(
            *part,
            PartPlacement {
                translation_mm: entry.placement_mm,
                orientation_xyzw: entry.orientation_xyzw,
            },
        );
        if entry.kind.is_linear_member() {
            geometry.members.insert(*part);
        }
    }
    geometry
}

/// Lifts the generated holes into world space for the preview: each polygon is the plate's
/// local XY, set on the mid-plane of the plate's local box (where the engine centres it) and
/// carried by the plate's whole placement, rotation included. A hole whose part the decision
/// geometry does not carry is skipped whole.
fn hole_world_rings(
    holes: &[GeneratedHole],
    placements: &BTreeMap<ElementId, PartPlacement>,
    slabs: &BTreeMap<ElementId, Bounds>,
) -> Vec<Vec<[f64; 3]>> {
    holes
        .iter()
        .filter_map(|hole| {
            let placement = placements.get(&hole.part)?;
            let slab = slabs.get(&hole.part)?;
            let mid = (slab[0][2] + slab[1][2]) / 2.0;
            let t = placement.translation_mm;
            Some(
                hole.polygon_mm
                    .iter()
                    .map(|[x, y]| {
                        let turned = beamcraft_engine::solid::rotate_vector(
                            [*x, *y, mid],
                            placement.orientation_xyzw,
                        );
                        [turned[0] + t[0], turned[1] + t[1], turned[2] + t[2]]
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect()
}

/// Paints the bolt preview over one pane, as the member draft paints: projection only, the
/// scene beneath untouched. The row line is soft, the bearings are dots, the holes are
/// outlined circles; a ring any point of which does not project is skipped whole.
pub(crate) fn paint_bolt_draft(
    ui: &egui::Ui,
    rect: egui::Rect,
    view: &View,
    origin: [f64; 3],
    draft: &BoltDraft,
) {
    let project = |point: [f64; 3]| {
        let local = std::array::from_fn(|i| {
            (point.get(i).copied().unwrap_or(0.0) - origin.get(i).copied().unwrap_or(0.0)) as f32
        });
        let [x, y, z] = view.project(local);
        (x.is_finite()
            && y.is_finite()
            && x.abs() < 1e5
            && y.abs() < 1e5
            && (0.0..=1.0).contains(&z))
        .then(|| {
            egui::pos2(
                rect.left() + (x + 1.0) * rect.width() / 2.0,
                rect.top() + (1.0 - y) * rect.height() / 2.0,
            )
        })
    };
    let painter = ui.painter_at(rect);
    let color = crate::theme::Palette::current(ui).link;
    if let (Some(a), Some(b)) = (project(draft.point1), project(draft.point2)) {
        painter.line_segment([a, b], egui::Stroke::new(1.5, color.gamma_multiply(0.6)));
    }
    for bearing in &draft.bearings_mm {
        if let Some(pos) = project(*bearing) {
            painter.circle_filled(pos, 4.0, color);
        }
    }
    let stroke = egui::Stroke::new(1.5, color);
    for hole in &draft.holes_mm {
        let mut ring = hole.iter().filter_map(|p| project(*p)).collect::<Vec<_>>();
        if ring.len() == hole.len()
            && ring.len() >= 3
            && let Some(first) = ring.first().copied()
        {
            ring.push(first);
            painter.add(egui::Shape::line(ring, stroke));
        }
    }
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

    /// The plane normal the bolt commit and the preview share: the loaded group's stored one
    /// in edit mode, else the active work plane's, else world up.
    pub(crate) fn bolt_plane_normal(&self) -> [f64; 3] {
        self.bolt.normal.unwrap_or_else(|| {
            self.tools
                .plane
                .map_or([0.0, 0.0, 1.0], |plane| plane.normal())
        })
    }

    /// Starts the definition fetch on a worker: the read-only `show-library` a user types. The
    /// pair is recorded before the reply arrives, so a slow or failed reply never respawns for
    /// the same pair; a failed fetch draws no preview until the form names another pair.
    pub(crate) fn start_spec_load(&mut self, pair: (String, String)) {
        if self.other_command_running() {
            return;
        }
        let cli = self.source.cli.clone();
        let stats = Arc::clone(&self.stats);
        let args = spec_args(&pair.0, &pair.1);
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new()
            .name("beamcraft-bolt-spec".into())
            .spawn(move || {
                let result = command::run(&cli, &args, command::DEADLINE, &stats)
                    .and_then(|out| parse_spec(out.code, &out.stdout));
                let _ = tx.send(result);
            });
        match spawned {
            Ok(_) => self.bolt.spec_job = Some(rx),
            Err(_) => self.bolt.note = Some(Err("The definition could not be loaded.".into())),
        }
        self.bolt.spec_for = Some(pair);
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
        let normal = self.bolt_plane_normal();
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

    /// Drains the listings, the definition fetch and the commit; a success reloads the model
    /// the viewport shows.
    pub(crate) fn poll_bolt(&mut self) {
        // The definition of the form's designation, fetched once per pair by the read-only
        // `show-library` a user types; the preview draws from it.
        if let Some(pair) = self.bolt.spec_query() {
            self.start_spec_load(pair);
        }
        if let Some(rx) = &self.bolt.spec_job {
            let outcome = match rx.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Err(ToolError::local(
                    "CLI_FAILED",
                    "The definition fetch ended without a result.",
                ))),
            };
            if outcome.is_some() {
                self.bolt.spec_job = None;
            }
            match outcome {
                Some(Ok(spec)) => self.bolt.spec = Some(spec),
                // A pair the reply does not answer draws no preview; the listings' and the
                // commit's own notes stay the panel's voice.
                Some(Err(_)) => self.bolt.spec = None,
                None => {}
            }
        }
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

    /// The preview the Bolt bench draws for the form as filled: the row and the holes the
    /// engine decides over the parts the snapshot shows. `None` draws nothing — a command
    /// running, an incomplete form, a definition the form has not fetched yet, a part the
    /// snapshot does not show, or an engine refusal. Display only: the commit still asks the
    /// engine, like the member ghost.
    pub(crate) fn bolt_draft(&self) -> Option<BoltDraft> {
        if self.bolt.busy() || self.other_command_running() {
            return None;
        }
        let loaded = self.loaded.as_ref()?;
        let library = self.bolt.library.trim().to_owned();
        let designation = self.bolt.designation.trim().to_owned();
        if !self
            .bolt
            .spec_for
            .as_ref()
            .is_some_and(|(listed_library, listed_designation)| {
                listed_library == library.as_str() && listed_designation == designation.as_str()
            })
        {
            return None;
        }
        let (digest, spec) = self.bolt.spec.as_ref()?;
        let normal = self.bolt_plane_normal();
        let inputs = preview_inputs(&self.bolt, normal, *digest, spec).ok()?;
        let row = steel::engine::bolt::preview_row(&inputs).ok()?;
        // The display budget: a preview past this many candidate holes is noise, and the
        // commit's own caps still govern the real group.
        if row.bearings_mm.len().saturating_mul(inputs.parts.len()) > MAX_PREVIEW_HOLES {
            return None;
        }
        let geometry = preview_geometry(&loaded.snapshot, &inputs.parts);
        // The row and its bearings still draw over a ruled-loft plate; only its holes, which
        // the commit decides in a box the snapshot does not carry, are left out.
        let mut decided = inputs.clone();
        decided
            .parts
            .retain(|part| !geometry.undecided.contains(part));
        let holes = steel::engine::bolt::generate_holes(
            &decided,
            &row,
            &geometry.slabs,
            &geometry.placements,
            &geometry.members,
        )
        .ok()?;
        let holes_mm = hole_world_rings(&holes, &geometry.placements, &geometry.slabs);
        Some(BoltDraft {
            point1: inputs.point1_mm,
            point2: inputs.point2_mm,
            bearings_mm: row.bearings_mm,
            holes_mm,
        })
    }

    /// The inspector panel of the Bolt bench.
    /// The bolt editor's action bar, pinned under the form by the inspector.
    pub(crate) fn bolt_actions(&mut self, ui: &mut egui::Ui) {
        let busy = self.bolt.busy() || self.other_command_running();
        let editing = self.bolt.edit_target.is_some();
        let done = crate::kit::actions(
            ui,
            if editing {
                "Update group"
            } else {
                "Create bolts"
            },
            true,
            "Cancel",
            busy,
        );
        if done.primary {
            self.start_bolt_commit();
        }
        if done.cancel {
            self.bolt.clear_placement();
            self.bolt.note = None;
            self.set_bench(super::workbench::Bench::Select);
        }
    }

    pub(crate) fn bolt_panel(&mut self, ui: &mut egui::Ui) {
        use crate::kit;
        let palette = crate::theme::Palette::current(ui);
        ui.set_max_width(ui.available_width());
        let busy = self.bolt.busy() || self.other_command_running();
        let editing = self.bolt.edit_target.is_some();
        kit::type_box(
            ui,
            crate::theme::Icon::Bolt,
            if editing {
                "Bolt group"
            } else {
                "New bolt group"
            },
        );
        // The listing is shown from a local copy: picking a row mutates the panel mid-loop.
        let listed_groups = self.bolt.groups.clone();
        let loaded_id = self.bolt.edit_target.clone();
        kit::section(ui, "bolt", "Group", true, |ui| {
            kit::prop_row(ui, "Groups", |ui| {
                let loaded = listed_groups
                    .iter()
                    .find(|listed| Some(listed.id.as_str()) == loaded_id.as_deref());
                egui::ComboBox::from_id_salt("bolt-groups")
                    .width((ui.available_width() - kit::SPACE_XS).max(60.0))
                    .truncate()
                    .selected_text(match loaded {
                        Some(listed) => listed.name.as_str(),
                        None if listed_groups.is_empty() => "None in the model yet",
                        None => "Pick a group to edit",
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
            });
            if editing {
                kit::prop_row(ui, "", |ui| {
                    if ui.button("New group").clicked() {
                        self.bolt.exit_edit();
                    }
                });
            }
            kit::prop_text(ui, "Name", &mut self.bolt.name);
        });
        kit::section(ui, "bolt", "Parts and points", true, |ui| {
            let parts = self.bolt.parts.clone();
            for (index, part) in parts.iter().enumerate() {
                kit::prop_info(ui, &format!("Part {}", index + 1), Some(part), "");
            }
            if parts.len() < 2 {
                kit::prop_info(
                    ui,
                    &format!("Part {}", parts.len() + 1),
                    None,
                    "Click a part",
                );
            }
            let point1 = self.bolt.point1.map(|p| PointText(p).to_string());
            let point2 = self.bolt.point2.map(|p| PointText(p).to_string());
            kit::prop_info(ui, "Point 1", point1.as_deref(), "Click a point");
            kit::prop_info(ui, "Point 2", point2.as_deref(), "Click a point");
            kit::prop_row(ui, "", |ui| {
                ui.label(
                    egui::RichText::new(
                        match (
                            self.bolt.parts.len(),
                            self.bolt.point1.is_some(),
                            self.bolt.point2.is_some(),
                        ) {
                            (0..=0, _, _) => "Click the main part in the model view.",
                            (1, _, _) => "Click the secondary part in the model view.",
                            (_, false, _) => "Click the first point on the work plane.",
                            (_, true, false) => "Click the second point: the row runs towards it.",
                            _ => "Parts and points are picked.",
                        },
                    )
                    .size(11.0)
                    .color(palette.muted),
                );
            });
            kit::prop_row(ui, "", |ui| {
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
        });
        kit::section(ui, "bolt", "Fastener", true, |ui| {
            kit::prop_text(ui, "Library (path)", &mut self.bolt.library);
            kit::prop_row(ui, "", |ui| {
                if ui
                    .add_enabled(!busy, egui::Button::new("Load entries"))
                    .clicked()
                {
                    self.start_entries_load();
                }
            });
            kit::prop_row(ui, "Designation", |ui| {
                egui::ComboBox::from_id_salt("bolt-designation")
                    .width((ui.available_width() - kit::SPACE_XS).max(60.0))
                    .truncate()
                    .selected_text(if self.bolt.designation.is_empty() {
                        "Load a library first"
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
            if editing {
                kit::prop_check(ui, "Follow library", &mut self.bolt.repin);
            }
        });
        kit::section(ui, "bolt", "Holes and washers", true, |ui| {
            kit::prop_choice(
                ui,
                "bolt-holes",
                "Holes",
                &mut self.bolt.holes,
                &[
                    (Holes::Generate, Holes::Generate.label()),
                    (Holes::Quiet, Holes::Quiet.label()),
                ],
            );
            kit::prop_text(ui, "Clearance, mm", &mut self.bolt.clearance);
            kit::prop_check(ui, "Washer under head", &mut self.bolt.washer_head);
            kit::prop_check(ui, "Washer under nut", &mut self.bolt.washer_nut);
        });
        kit::section(ui, "bolt", "Position", true, |ui| {
            kit::prop_text(ui, "On plane, mm", &mut self.bolt.on_plane);
            kit::prop_text(ui, "Rotation, °", &mut self.bolt.rotation);
            kit::prop_text(ui, "Depth, mm", &mut self.bolt.depth);
            kit::prop_text(ui, "Offset start, mm", &mut self.bolt.offset_start);
        });
        kit::section(ui, "bolt", "Spacing", true, |ui| {
            kit::prop_choice(
                ui,
                "bolt-spacing",
                "Spacing",
                &mut self.bolt.spacing,
                &[
                    (Spacing::List, Spacing::List.label()),
                    (Spacing::Pitch, Spacing::Pitch.label()),
                    (Spacing::OffsetEnd, Spacing::OffsetEnd.label()),
                ],
            );
            match self.bolt.spacing {
                Spacing::List => {
                    kit::prop_text(ui, "Spacings, mm", &mut self.bolt.spacings);
                }
                Spacing::Pitch => {
                    kit::prop_text(ui, "Pitch, mm", &mut self.bolt.pitch);
                }
                Spacing::OffsetEnd => {
                    kit::prop_text(ui, "Offset end, mm", &mut self.bolt.offset_end);
                }
            }
            if self.bolt.spacing != Spacing::List {
                kit::prop_text(ui, "Count", &mut self.bolt.count);
            }
        });
        // Its outcome is told in the status bar (`App::status_note`).
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

    /// One `show-library` reply of a washered bolt entry, as the CLI's definition view spells
    /// it: the washer as three flat dimensions, the switches as booleans.
    fn spec_reply() -> Vec<u8> {
        br#"{"outcome":"library_entry",
            "library":{"id":"lib","digest":"aa"},"library_path":"fasteners.usda",
            "entry":{
              "digest":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
              "kind":"fastener","units":"mm","schema_version":"0.1.0","display_name":null,
              "fastener_kind":"bolt","head_kind":"hex",
              "nominal_diameter_mm":12,"head_across_flats_mm":18,"head_height_mm":7.5,
              "nut_across_flats_mm":18,"nut_height_mm":9,
              "washer_outer_diameter_mm":24,"washer_inner_diameter_mm":13,"washer_thickness_mm":2,
              "hole_clearance_mm":1.5,"lengths_mm":[30,40,50,55],"thread_protrusion_mm":3,
              "mass_each_kg":null,"washer_under_head":true,"washer_under_nut":true,
              "hole_mode":"generate","designation":"SYN-BOLT-12x50","standard":null,"grade":null
            },
            "shadowed":null}"#
            .to_vec()
    }

    #[test]
    fn parse_spec_reads_the_reply_the_cli_spells() {
        let (digest, spec) = parse_spec(Some(0), &spec_reply()).unwrap();
        assert_eq!(
            digest.to_string(),
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        );
        assert_eq!(spec.designation.as_deref(), Some("SYN-BOLT-12x50"));
        assert_eq!(spec.nominal_diameter_mm, 12.0);
        assert_eq!(spec.washer.as_ref().map(|w| w.thickness_mm), Some(2.0));
        assert!(spec.washer_under_head && spec.washer_under_nut);
        assert_eq!(spec.lengths_mm, [30.0, 40.0, 50.0, 55.0]);
        assert_eq!(spec.hole_mode, HoleMode::Generate);
        // Refusals: not json, a rejected command, a non-fastener entry, an unreadable digest,
        // a definition the model refuses.
        assert!(parse_spec(Some(0), b"not json").is_err());
        let refused = br#"{"outcome":"rejected","code":"LIBRARY_UNREADABLE","message":"no"}"#;
        assert!(parse_spec(Some(2), refused).is_err());
        let profile = br#"{"outcome":"library_entry","entry":{"kind":"section_profile"}}"#;
        assert!(parse_spec(Some(0), profile).is_err());
        let bare = br#"{"outcome":"library_entry","entry":{"kind":"fastener","digest":"aa"}}"#;
        assert!(parse_spec(Some(0), bare).is_err());
        let nutless = br#"{"outcome":"library_entry","entry":{"kind":"fastener",
            "digest":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "fastener_kind":"bolt","head_kind":"hex","nominal_diameter_mm":12,
            "head_across_flats_mm":18,"head_height_mm":7.5,"hole_clearance_mm":1.5,
            "lengths_mm":[30],"thread_protrusion_mm":3,"washer_under_head":false,
            "washer_under_nut":false,"hole_mode":"generate"}}"#;
        assert!(parse_spec(Some(0), nutless).is_err());
    }

    #[test]
    fn the_preview_inputs_match_the_commit_argv_across_the_patterns() {
        let (digest, spec) = parse_spec(Some(0), &spec_reply()).unwrap();
        // List, pitch and offset-end: the argv the commit sends and the inputs the preview
        // builds read the same numbers from the same form.
        for spacing in [Spacing::List, Spacing::Pitch, Spacing::OffsetEnd] {
            let mut panel = form();
            panel.spacing = spacing;
            panel.rotation = "45".into();
            panel.on_plane = "2.5".into();
            panel.depth = "-4".into();
            panel.holes = Holes::Quiet;
            panel.clearance = "1.5".into();
            let normal = [0.0, 1.0, 0.0];
            let argv = create_args(&panel, normal, Path::new("S"), None).unwrap();
            let inputs = preview_inputs(&panel, normal, digest, &spec).unwrap();
            let pair = |key: &str| {
                let at = argv
                    .iter()
                    .position(|arg| arg == key)
                    .unwrap_or_else(|| panic!("{key} is in the argv"));
                argv[at + 1].clone()
            };
            assert_eq!(pair("--point1"), point_text(inputs.point1_mm));
            assert_eq!(pair("--point2"), point_text(inputs.point2_mm));
            assert_eq!(pair("--plane-normal"), point_text(inputs.plane_normal_mm));
            let rotation = pair("--rotation-deg").parse::<f64>().unwrap();
            assert_eq!(inputs.rotation_rad, rotation.to_radians());
            assert_eq!(inputs.on_plane_mm.to_string(), pair("--on-plane"));
            assert_eq!(inputs.depth_mm.to_string(), pair("--depth"));
            assert_eq!(inputs.offset_start_mm.to_string(), pair("--offset-start"));
            match (&inputs.spacing, spacing) {
                (BoltSpacing::Spacings(list), Spacing::List) => {
                    assert_eq!(
                        pair("--spacings"),
                        list.iter()
                            .map(f64::to_string)
                            .collect::<Vec<_>>()
                            .join(",")
                    );
                }
                (
                    BoltSpacing::Pitch {
                        count,
                        pitch_mm: pitch,
                    },
                    Spacing::Pitch,
                ) => {
                    assert_eq!(pitch.to_string(), pair("--pitch"));
                    assert_eq!(count.to_string(), pair("--count"));
                }
                (
                    BoltSpacing::OffsetEnd {
                        count,
                        offset_end_mm: end,
                    },
                    Spacing::OffsetEnd,
                ) => {
                    assert_eq!(end.to_string(), pair("--offset-end"));
                    assert_eq!(count.to_string(), pair("--count"));
                }
                _ => panic!("the pattern does not round-trip"),
            }
            assert_eq!(inputs.washer_under_head, Some(panel.washer_head));
            assert_eq!(inputs.washer_under_nut, Some(panel.washer_nut));
            assert_eq!(inputs.hole_mode, Some(HoleMode::None));
            assert_eq!(inputs.hole_clearance_mm, Some(1.5));
            assert_eq!(inputs.length, BoltLength::Auto);
            // A malformed field refuses both the argv and the preview inputs.
            panel.offset_start = "wide".into();
            assert!(create_args(&panel, normal, Path::new("S"), None).is_err());
            assert!(preview_inputs(&panel, normal, digest, &spec).is_err());
        }
    }

    #[test]
    fn spec_query_fetches_only_complete_listed_pairs() {
        let mut panel = Panel::empty();
        assert_eq!(panel.spec_query(), None);
        panel.library = "fasteners.usda".into();
        panel.designation = "SYN-BOLT-12x50".into();
        // A designation the loaded listing does not carry fetches nothing: typing never
        // chases half-entered names.
        assert_eq!(panel.spec_query(), None);
        panel.entries = vec!["SYN-BOLT-12x50".into()];
        assert_eq!(
            panel.spec_query(),
            Some(("fasteners.usda".to_owned(), "SYN-BOLT-12x50".to_owned()))
        );
        panel.spec_for = Some(("fasteners.usda".to_owned(), "SYN-BOLT-12x50".to_owned()));
        assert_eq!(panel.spec_query(), None);
        // The trimmed form answers.
        panel.designation = " SYN-BOLT-12x50 ".into();
        assert_eq!(panel.spec_query(), None);
    }

    #[test]
    fn hole_rings_lift_the_plate_local_polygon_by_the_part_placement() {
        // The polygon is local XY on the mid-plane of the plate's local box (0..10 -> 5),
        // carried by the whole placement: a ring centred on local (0,0) on a plate set down at
        // (40, 30, 12) draws at (40, 30, 17) without rotation.
        let part = ElementId::parse("019a7bf0-0000-7000-8000-000000510001").unwrap();
        let mut placements = BTreeMap::new();
        placements.insert(
            part,
            PartPlacement {
                translation_mm: [40.0, 30.0, 12.0],
                orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
            },
        );
        let mut bounds = BTreeMap::new();
        bounds.insert(part, [[-10.0, -5.0, 0.0], [10.0, 5.0, 10.0]]);
        let hole = GeneratedHole {
            part,
            polygon_mm: vec![[0.0, 0.0], [4.0, 0.0], [0.0, 4.0]],
        };
        let rings = hole_world_rings(std::slice::from_ref(&hole), &placements, &bounds);
        assert_eq!(rings.len(), 1);
        assert_eq!(
            rings[0],
            vec![[40.0, 30.0, 17.0], [44.0, 30.0, 17.0], [40.0, 34.0, 17.0]]
        );
        // A quarter turn about +X (a plate standing in the world XZ plane) maps local y to +Z
        // and the local mid-plane z = 5 to world -Y (issue 555 finding 2).
        let half = core::f64::consts::FRAC_1_SQRT_2;
        placements.insert(
            part,
            PartPlacement {
                translation_mm: [40.0, 30.0, 12.0],
                orientation_xyzw: [half, 0.0, 0.0, half],
            },
        );
        let turned = hole_world_rings(&[hole], &placements, &bounds);
        for (got, want) in
            turned[0]
                .iter()
                .zip([[40.0, 25.0, 12.0], [44.0, 25.0, 12.0], [40.0, 25.0, 16.0]])
        {
            for (g, w) in got.iter().zip(want) {
                assert!((g - w).abs() <= 1e-9, "{got:?} != {want:?}");
            }
        }
        // A hole on a part the decision geometry does not carry is skipped whole.
        let stranger = ElementId::parse("019a7bf0-0000-7000-8000-000000510002").unwrap();
        let rings = hole_world_rings(
            &[GeneratedHole {
                part: stranger,
                polygon_mm: vec![[0.0, 0.0]],
            }],
            &placements,
            &bounds,
        );
        assert!(rings.is_empty());
    }

    #[test]
    fn a_bent_plate_gets_no_preview_holes_from_its_shown_mesh_box() {
        // #557 review: the commit decides a plate's holes over the authored contour's box,
        // which for a bent plate is the developed (unfolded) outline, while its shown mesh is
        // folded out of plane; the preview leaves a bent plate's holes undecided instead of
        // drawing holes over a box the commit never uses. A flat plate keeps the mesh box,
        // which is the commit's.
        use steel::engine::command::AuthoredPlate;
        use steel::model::command::{BendRecord, BendSpec};
        let flat_id = ElementId::parse(MAIN).unwrap();
        let bent_id = ElementId::parse(OTHER).unwrap();
        let flat = AuthoredPlate {
            id: flat_id,
            name: "Flat".into(),
            class: 0,
            contour_points_mm: vec![
                [0.0, 0.0, 0.0],
                [100.0, 0.0, 0.0],
                [100.0, 200.0, 0.0],
                [0.0, 200.0, 0.0],
            ],
            thickness_mm: 10.0,
            thickness_side: None,
            corners: Vec::new(),
            placement: PartPlacement {
                translation_mm: [0.0; 3],
                orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
            },
            material: None,
            holes_mm: Vec::new(),
            hole_records: Vec::new(),
            cuts: Vec::new(),
            bends: Vec::new(),
        };
        let mut bent = flat.clone();
        bent.id = bent_id;
        bent.name = "Bent".into();
        bent.bends = vec![BendRecord {
            operation_id: beamcraft_model::id::OpId::parse("0197aabb-ccdd-7ebb-9beb-59df63a5fc13")
                .unwrap(),
            spec: BendSpec::Fold {
                a_mm: [0.0, 100.0, 0.0],
                b_mm: [100.0, 100.0, 0.0],
                radius_mm: 8.0,
                angle_rad: core::f64::consts::FRAC_PI_2,
            },
        }];
        let pin = beamcraft_engine::ViewPin::new(
            beamcraft_model::digest::Sha256::digest(b"revision").unwrap(),
            1,
        );
        // The committed-model path the window's snapshot comes from.
        let records = steel::engine::committed::evaluate_committed(
            &[flat.clone(), bent.clone()],
            &steel::engine::command::no_materials,
        )
        .unwrap();
        let snapshot = ViewSnapshot::build(
            pin,
            steel::engine::committed::scene_elements(&records).unwrap(),
            &beamcraft_engine::Limits::default(),
        )
        .unwrap();
        // The shown bent mesh leaves the contour's box: its box is not the commit's.
        let bent_mesh = snapshot
            .mesh(&snapshot.element(&bent_id).unwrap().full.mesh_digest)
            .and_then(|mesh| mesh.local_bounds())
            .unwrap();
        assert_ne!(Some(bent_mesh), bent.local_box_mm());

        let geometry = preview_geometry(&snapshot, &[flat_id, bent_id]);
        assert_eq!(geometry.undecided, BTreeSet::from([bent_id]));
        assert!(!geometry.slabs.contains_key(&bent_id));
        assert_eq!(geometry.slabs.get(&flat_id).copied(), flat.local_box_mm());
        assert!(geometry.placements.contains_key(&bent_id));
        assert!(geometry.members.is_empty());
    }

    #[test]
    fn the_preview_paints_the_row_the_bearings_and_the_hole_rings() {
        let draft = BoltDraft {
            point1: [0.0, 0.0, 0.0],
            point2: [100.0, 0.0, 0.0],
            bearings_mm: vec![[25.0, 0.0, 0.0], [75.0, 0.0, 0.0]],
            holes_mm: vec![vec![[20.0, 5.0, 0.0], [30.0, 5.0, 0.0], [25.0, -4.0, 0.0]]],
        };
        let view = crate::camera::Camera::new([50.0, 0.0, 0.0], 200.0, 268.0 / 184.0)
            .unwrap()
            .view(268.0 / 184.0)
            .unwrap();
        let context = egui::Context::default();
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(268.0, 184.0));
        let output = context.run_ui(Default::default(), |ui| {
            paint_bolt_draft(ui, rect, &view, [0.0; 3], &draft);
        });
        let mut segments = 0;
        let mut dots = 0;
        let mut rings = 0;
        for shape in &output.shapes {
            match &shape.shape {
                egui::Shape::LineSegment { .. } => segments += 1,
                egui::Shape::Circle(circle) if circle.fill != egui::Color32::TRANSPARENT => {
                    dots += 1
                }
                egui::Shape::Path(_) => rings += 1,
                _ => {}
            }
        }
        output.drop_without_applying_deltas();
        // The soft row line, one dot per bearing, the closed hole ring.
        assert_eq!((segments, dots, rings), (1, 2, 1));
    }
}
