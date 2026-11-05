// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The Bolt bench (V19 slice 1): two picked parts and two points on the active plane commit
//! through the same `create-bolt-group` command a user types (A14.3). The fastener travels by
//! designation through a fastener library file, listed read-only with `list-library`. The reply
//! is the CLI's; a refusal publishes nothing and is shown next to the form.

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
    /// The part the first click picked, as the engine reported its id.
    pub(crate) main: Option<String>,
    /// The part the second click picked.
    pub(crate) secondary: Option<String>,
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
            main: None,
            secondary: None,
            point1: None,
            point2: None,
            name: "Bolt group".into(),
            library: String::new(),
            designation: String::new(),
            entries: Vec::new(),
            entries_job: None,
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

impl Panel {
    pub(super) fn empty() -> Self {
        Self::default()
    }

    fn busy(&self) -> bool {
        self.commit_job.is_some()
    }

    /// Forgets the picked parts and points; the form keeps its typed values.
    fn clear_placement(&mut self) {
        self.main = None;
        self.secondary = None;
        self.point1 = None;
        self.point2 = None;
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

/// The `create-bolt-group` arguments of the form, exactly as the command takes them. `normal`
/// is the active work plane's normal, `0,0,1` without one, taken when the commit starts.
pub(crate) fn create_args(
    panel: &Panel,
    normal: [f64; 3],
    store: &Path,
    actor: Option<&str>,
) -> Result<Vec<String>, String> {
    let name = panel.name.trim();
    if name.is_empty() {
        return Err("Name: enter a label for the group".into());
    }
    if name.len() > MAX_TEXT {
        return Err(format!("Name: at most {MAX_TEXT} bytes"));
    }
    let (Some(main), Some(secondary)) = (&panel.main, &panel.secondary) else {
        return Err("Pick the main part, then the secondary part, in the model view.".into());
    };
    if main == secondary {
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
    let offset_start = number("Offset start", &panel.offset_start)?;
    let on_plane = number("On plane", &panel.on_plane)?;
    let rotation = number("Rotation", &panel.rotation)?;
    let depth = number("Depth", &panel.depth)?;
    let clearance = match panel.clearance.trim() {
        "" => None,
        value => Some(number("Clearance", value)?),
    };
    let point = |p: [f64; 3]| {
        p.iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>()
            .join(",")
    };
    let mut args = vec![
        "create-bolt-group".to_owned(),
        "--store".into(),
        store.to_string_lossy().into_owned(),
        "--name".into(),
        name.to_owned(),
        "--parts".into(),
        format!("{main},{secondary}"),
        "--fastener-library".into(),
        library.to_owned(),
        "--designation".into(),
        designation.to_owned(),
        "--point1".into(),
        point(point1),
        "--point2".into(),
        point(point2),
        "--plane-normal".into(),
        point(normal),
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
        if self.bolt.main.is_none() || self.bolt.secondary.is_none() {
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
                    let slot = if self.bolt.main.is_none() {
                        &mut self.bolt.main
                    } else {
                        &mut self.bolt.secondary
                    };
                    if slot.as_deref() == Some(id.as_str()) {
                        self.bolt.note = Some(Err(
                            "Already picked; pick a different part for the other side.".into(),
                        ));
                        return;
                    }
                    *slot = Some(id);
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

    /// Starts the committed `create-bolt-group` on a worker: the same command a user types.
    pub(crate) fn start_bolt_commit(&mut self) {
        if self.other_command_running() {
            self.bolt.note = Some(Err(
                "another command is running; wait for it to finish.".into()
            ));
            return;
        }
        let normal = self
            .tools
            .plane
            .map_or([0.0, 0.0, 1.0], |plane| plane.normal());
        let args = match create_args(
            &self.bolt,
            normal,
            &self.source.store,
            self.source.actor.as_deref(),
        ) {
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

    /// Drains the listing and the commit; a success reloads the model the viewport shows.
    pub(crate) fn poll_bolt(&mut self) {
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
                self.bolt.clear_placement();
                self.bolt.note = Some(Ok(
                    "Created bolt group. Pick new parts, or press Esc to finish.".into(),
                ));
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
        ui.label(crate::theme::bold("Bolt properties", 16.0));
        ui.label("A row of library fasteners between two picked parts, through the create-bolt-group command.");
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
        picked(ui, "Main", self.bolt.main.as_ref());
        picked(ui, "Secondary", self.bolt.secondary.as_ref());
        point(ui, "Point 1", self.bolt.point1);
        point(ui, "Point 2", self.bolt.point2);
        ui.label(
            match (
                self.bolt.main.is_some(),
                self.bolt.secondary.is_some(),
                self.bolt.point1.is_some(),
                self.bolt.point2.is_some(),
            ) {
                (false, _, _, _) => "Click the main part in the model view.",
                (true, false, _, _) => "Click the secondary part in the model view.",
                (true, true, false, _) => "Click the first point on the work plane.",
                (true, true, true, false) => {
                    "Click the second point: the row runs from point 1 towards it."
                }
                _ => "Parts and points are picked; fill the form and press Create bolts.",
            },
        );
        if ui
            .add_enabled(
                self.bolt.main.is_some()
                    || self.bolt.secondary.is_some()
                    || self.bolt.point1.is_some()
                    || self.bolt.point2.is_some(),
                egui::Button::new("Clear picks"),
            )
            .clicked()
        {
            self.bolt.clear_placement();
        }
        field(ui, "Name", &mut self.bolt.name, 160.0);
        field(ui, "Fastener library (path)", &mut self.bolt.library, 240.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!busy, egui::Button::new("Load entries"))
                .clicked()
            {
                self.start_entries_load();
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
        ui.separator();
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
        ui.separator();
        ui.add_enabled_ui(!busy, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Create bolts").clicked() {
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
        panel.main = Some(MAIN.into());
        panel.secondary = Some(OTHER.into());
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
        assert!(refused(&|p| p.main = None).contains("Pick the main part"));
        assert!(refused(&|p| p.secondary = None).contains("Pick the main part"));
        assert!(refused(&|p| p.secondary = Some(MAIN.into())).contains("distinct parts"));
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
}
