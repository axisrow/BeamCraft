// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Part fittings, cuts, splits and holes (#851):
//! - `Bench::Fit`: trim a part's end flush with a plate (`fit-end-to-part`, A13).
//! - `Bench::Cut`: cut a part by another part (`cut-by-part`), cut by a plane (`cut-by-plane`),
//!   split a part (`split-part`) or combine two parts (`combine-parts`).
//! - `Bench::Hole`: create a round or rectangular through-hole (`add-hole`) or remove one
//!   (`remove-hole`).

use super::App;
use crate::command;
use crate::edit::{Ray, pick_point_in};
use crate::tools::{ToolError, decode, rejection};
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

/// Sub-mode of the Cut bench (`Bench::Cut`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum CutMode {
    /// `cut-by-part`: boolean cut by a tool part.
    #[default]
    ByPart,
    /// `cut-by-plane`: half-space plane cut.
    ByPlane,
    /// `split-part`: split one part across a plane into two parts.
    Split,
    /// `combine-parts`: merge two colinear/coplanar parts into one.
    Combine,
}

impl CutMode {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::ByPart => "Cut by part",
            Self::ByPlane => "Cut by plane",
            Self::Split => "Split part",
            Self::Combine => "Combine parts",
        }
    }
}

/// Sub-mode of the Hole bench (`Bench::Hole`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum HoleKind {
    /// Round hole along a 3D axis (`--axis-point`, `--axis-direction`, `--diameter`).
    #[default]
    AxisRound,
    /// Plate-local round hole (`--hole-round X,Y,D`).
    PlateRound,
    /// Plate-local rectangular hole (`--hole-rect X,Y,W,H`).
    PlateRect,
}

impl HoleKind {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::AxisRound => "Round hole (3D pick)",
            Self::PlateRound => "Plate round (local X, Y)",
            Self::PlateRect => "Plate rect (local X, Y, W, H)",
        }
    }
}

/// The Fit, Cut and Hole benches' state between frames.
pub(crate) struct Panel {
    /// The part to trim, cut or drill: a beam, a column or a plate.
    pub(crate) part: Option<String>,
    /// The tool part (`fit-end-to-part`, `cut-by-part`, `combine-parts`).
    pub(crate) tool: Option<String>,
    /// Sub-mode when on `Bench::Cut`.
    pub(crate) cut_mode: CutMode,
    /// Plane point `[X, Y, Z]` in mm for `cut-by-plane` / `split-part`, or 3D hole point.
    pub(crate) point: [String; 3],
    /// Plane normal or hole axis direction `[X, Y, Z]`.
    pub(crate) normal: [String; 3],
    /// `--keep` for `cut-by-plane` and `split-part`: `positive` or `negative`.
    pub(crate) keep: String,
    /// `--new-name` for `split-part`.
    pub(crate) split_name: String,
    /// Sub-mode when on `Bench::Hole`.
    pub(crate) hole_kind: HoleKind,
    /// Local `[X, Y]` in mm for plate holes.
    pub(crate) local_xy: [String; 2],
    /// Hole diameter in mm.
    pub(crate) diameter: String,
    /// Rectangular hole `[W, H]` in mm.
    pub(crate) rect_wh: [String; 2],
    /// Operation ID for `remove-hole` or `remove-cut`.
    pub(crate) remove_op_id: String,
    pub(super) commit_job: Option<Receiver<Result<String, ToolError>>>,
    /// The last outcome, shown in the status bar.
    pub(crate) note: Option<Result<String, String>>,
}

impl Panel {
    pub(super) fn empty() -> Self {
        Self {
            part: None,
            tool: None,
            cut_mode: CutMode::default(),
            point: ["0".into(), "0".into(), "0".into()],
            normal: ["0".into(), "0".into(), "1".into()],
            keep: "positive".into(),
            split_name: "Split part".into(),
            hole_kind: HoleKind::default(),
            local_xy: ["100".into(), "100".into()],
            diameter: "22".into(),
            rect_wh: ["60".into(), "40".into()],
            remove_op_id: String::new(),
            commit_job: None,
            note: None,
        }
    }

    pub(crate) fn busy(&self) -> bool {
        self.commit_job.is_some()
    }

    /// Forgets the picked pair.
    pub(super) fn clear_placement(&mut self) {
        self.part = None;
        self.tool = None;
    }
}

fn triple(label: &str, coords: &[String; 3]) -> Result<String, String> {
    let mut out = [0.0f64; 3];
    for (slot, s) in out.iter_mut().zip(coords) {
        let t = s.trim();
        let v = t
            .parse::<f64>()
            .map_err(|_| format!("{label}: \"{t}\" is not a valid number."))?;
        if !v.is_finite() {
            return Err(format!("{label}: enter finite numbers."));
        }
        *slot = v;
    }
    let [x, y, z] = out;
    Ok(format!("{x},{y},{z}"))
}

fn positive_num(label: &str, text: &str) -> Result<f64, String> {
    let t = text.trim();
    let v = t
        .parse::<f64>()
        .map_err(|_| format!("{label}: \"{t}\" is not a valid number."))?;
    if !v.is_finite() || v <= 0.0 {
        return Err(format!("{label}: enter a positive number of millimetres."));
    }
    Ok(v)
}

/// The `fit-end-to-part` arguments, exactly as the command takes them. `operation` is the fresh
/// identity of the fitting, which later edits and removals address.
pub(crate) fn fit_args(
    store: &Path,
    actor: Option<&str>,
    part: &str,
    tool: &str,
    operation: &str,
) -> Result<Vec<String>, String> {
    let (part, tool) = (part.trim(), tool.trim());
    if part.is_empty() || tool.is_empty() {
        return Err("Pick the part to trim, then the plate it fits to.".into());
    }
    if part == tool {
        return Err("A part cannot be fitted to itself; pick the plate.".into());
    }
    let mut args = vec![
        "fit-end-to-part".into(),
        "--store".into(),
        store.to_string_lossy().into_owned(),
        "--id".into(),
        part.to_owned(),
        "--operation-id".into(),
        operation.to_owned(),
        "--tool".into(),
        tool.to_owned(),
    ];
    if let Some(actor) = actor {
        args.extend(["--actor-id".into(), actor.to_owned()]);
    }
    Ok(args)
}

/// Builds the CLI arguments for `cut-by-part`.
pub(crate) fn cut_by_part_args(
    store: &Path,
    actor: Option<&str>,
    part: &str,
    tool: &str,
    operation: &str,
) -> Result<Vec<String>, String> {
    let (part, tool) = (part.trim(), tool.trim());
    if part.is_empty() || tool.is_empty() {
        return Err("Pick the part to cut, then the cutting part.".into());
    }
    if part == tool {
        return Err("A part cannot cut itself; pick a different cutting part.".into());
    }
    let mut args = vec![
        "cut-by-part".into(),
        "--store".into(),
        store.to_string_lossy().into_owned(),
        "--id".into(),
        part.to_owned(),
        "--operation-id".into(),
        operation.to_owned(),
        "--tool".into(),
        tool.to_owned(),
    ];
    if let Some(actor) = actor {
        args.extend(["--actor-id".into(), actor.to_owned()]);
    }
    Ok(args)
}

/// Builds the CLI arguments for `cut-by-plane`.
pub(crate) fn cut_by_plane_args(
    store: &Path,
    actor: Option<&str>,
    part: &str,
    operation: &str,
    point: &[String; 3],
    normal: &[String; 3],
    keep: &str,
) -> Result<Vec<String>, String> {
    let part = part.trim();
    if part.is_empty() {
        return Err("Pick the part to cut in the view.".into());
    }
    let pt = triple("Point", point)?;
    let nrm = triple("Normal", normal)?;
    let keep = match keep.trim() {
        "negative" => "negative",
        _ => "positive",
    };
    let mut args = vec![
        "cut-by-plane".into(),
        "--store".into(),
        store.to_string_lossy().into_owned(),
        "--id".into(),
        part.to_owned(),
        "--operation-id".into(),
        operation.to_owned(),
        "--point".into(),
        pt,
        "--normal".into(),
        nrm,
        "--keep".into(),
        keep.into(),
    ];
    if let Some(actor) = actor {
        args.extend(["--actor-id".into(), actor.to_owned()]);
    }
    Ok(args)
}

/// Builds the CLI arguments for `split-part`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn split_part_args(
    store: &Path,
    actor: Option<&str>,
    part: &str,
    new_id: &str,
    new_name: &str,
    operation: &str,
    point: &[String; 3],
    normal: &[String; 3],
    keep: &str,
) -> Result<Vec<String>, String> {
    let part = part.trim();
    if part.is_empty() {
        return Err("Pick the part to split in the view.".into());
    }
    let new_name = new_name.trim();
    if new_name.is_empty() {
        return Err("New name: enter a name for the second part.".into());
    }
    let pt = triple("Point", point)?;
    let nrm = triple("Normal", normal)?;
    let keep = match keep.trim() {
        "negative" => "negative",
        _ => "positive",
    };
    let mut args = vec![
        "split-part".into(),
        "--store".into(),
        store.to_string_lossy().into_owned(),
        "--id".into(),
        part.to_owned(),
        "--new-id".into(),
        new_id.trim().to_owned(),
        "--new-name".into(),
        new_name.to_owned(),
        "--operation-id".into(),
        operation.to_owned(),
        "--point".into(),
        pt,
        "--normal".into(),
        nrm,
        "--keep".into(),
        keep.into(),
    ];
    if let Some(actor) = actor {
        args.extend(["--actor-id".into(), actor.to_owned()]);
    }
    Ok(args)
}

/// Builds the CLI arguments for `combine-parts`.
pub(crate) fn combine_parts_args(
    store: &Path,
    actor: Option<&str>,
    survivor: &str,
    consumed: &str,
    operation: &str,
) -> Result<Vec<String>, String> {
    let (survivor, consumed) = (survivor.trim(), consumed.trim());
    if survivor.is_empty() || consumed.is_empty() {
        return Err("Pick the main (survivor) part, then the part to combine into it.".into());
    }
    if survivor == consumed {
        return Err("Pick two distinct parts to combine.".into());
    }
    let mut args = vec![
        "combine-parts".into(),
        "--store".into(),
        store.to_string_lossy().into_owned(),
        "--survivor".into(),
        survivor.to_owned(),
        "--consumed".into(),
        consumed.to_owned(),
        "--operation-id".into(),
        operation.to_owned(),
    ];
    if let Some(actor) = actor {
        args.extend(["--actor-id".into(), actor.to_owned()]);
    }
    Ok(args)
}

/// Builds the CLI arguments for `add-hole`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn add_hole_args(
    store: &Path,
    actor: Option<&str>,
    part: &str,
    operation: &str,
    kind: HoleKind,
    point: &[String; 3],
    normal: &[String; 3],
    local_xy: &[String; 2],
    diameter: &str,
    rect_wh: &[String; 2],
) -> Result<Vec<String>, String> {
    let part = part.trim();
    if part.is_empty() {
        return Err("Pick the part to drill in the view.".into());
    }
    let mut args = vec![
        "add-hole".into(),
        "--store".into(),
        store.to_string_lossy().into_owned(),
        "--id".into(),
        part.to_owned(),
        "--operation-id".into(),
        operation.to_owned(),
    ];
    match kind {
        HoleKind::AxisRound => {
            let pt = triple("Hole point", point)?;
            let dir = triple("Axis direction", normal)?;
            let d = positive_num("Diameter", diameter)?;
            args.extend([
                "--axis-point".into(),
                pt,
                "--axis-direction".into(),
                dir,
                "--diameter".into(),
                d.to_string(),
            ]);
        }
        HoleKind::PlateRound => {
            let [lx, ly] = local_xy;
            let x = lx
                .trim()
                .parse::<f64>()
                .map_err(|_| "Local X: enter a number.".to_owned())?;
            let y = ly
                .trim()
                .parse::<f64>()
                .map_err(|_| "Local Y: enter a number.".to_owned())?;
            let d = positive_num("Diameter", diameter)?;
            args.extend(["--hole-round".into(), format!("{x},{y},{d}")]);
        }
        HoleKind::PlateRect => {
            let [lx, ly] = local_xy;
            let [rw, rh] = rect_wh;
            let x = lx
                .trim()
                .parse::<f64>()
                .map_err(|_| "Local X: enter a number.".to_owned())?;
            let y = ly
                .trim()
                .parse::<f64>()
                .map_err(|_| "Local Y: enter a number.".to_owned())?;
            let w = positive_num("Width", rw)?;
            let h = positive_num("Height", rh)?;
            args.extend(["--hole-rect".into(), format!("{x},{y},{w},{h}")]);
        }
    }
    if let Some(actor) = actor {
        args.extend(["--actor-id".into(), actor.to_owned()]);
    }
    Ok(args)
}

impl App {
    /// The first click picks the part to trim, the second the plate. Only what the active view
    /// shows can be picked, and the selection switches apply, as they do to selection.
    pub(crate) fn fit_click(&mut self, ray: Ray) {
        if self.other_command_running() || self.fit.busy() {
            return;
        }
        let Some(loaded) = &self.loaded else {
            return;
        };
        let picked = match self.views.active_view() {
            Some(view) => beamcraft_engine::views::pick_visible(
                &loaded.snapshot,
                view,
                ray.origin,
                ray.direction,
                self.selection.filter(),
                false,
            )
            .map(|hit| hit.map(|part| (part.element_id.to_string(), part.point_world_mm)))
            .map_err(|e| ToolError::local("PICK_REFUSED", e.to_string())),
            None => pick_point_in(&loaded.snapshot, &ray).map(|opt| opt.map(|(pt, id)| (id, pt))),
        };
        match picked {
            Ok(Some((id, pt))) => {
                let snapped_pt = self.creation_point(ray, false).unwrap_or(pt);
                if self.bench == super::workbench::Bench::Hole {
                    self.fit.part = Some(id);
                    self.fit.point = snapped_pt.map(|v| format!("{:.1}", v));
                    self.fit.note = Some(Ok(
                        "Part and hole point picked. Adjust parameters and click Create hole."
                            .into(),
                    ));
                    return;
                }
                if self.bench == super::workbench::Bench::Cut
                    && matches!(self.fit.cut_mode, CutMode::ByPlane | CutMode::Split)
                {
                    self.fit.part = Some(id);
                    self.fit.point = snapped_pt.map(|v| format!("{:.1}", v));
                    self.fit.note = Some(Ok(
                        "Part and plane point picked. Set normal and click Apply.".into(),
                    ));
                    return;
                }
                if self.fit.part.as_deref() == Some(id.as_str()) {
                    self.fit.note =
                        Some(Err("That is the first part; pick the second part.".into()));
                    return;
                }
                if self.fit.part.is_none() {
                    self.fit.part = Some(id);
                } else {
                    self.fit.tool = Some(id);
                    if self.bench == super::workbench::Bench::Fit {
                        self.start_fit_commit();
                    } else if self.bench == super::workbench::Bench::Cut {
                        self.start_cut_commit();
                    }
                    return;
                }
                self.fit.note = None;
            }
            Ok(None) => self.fit.note = Some(Err("No part was hit; aim at a model face.".into())),
            Err(error) => self.fit.note = Some(Err(error.to_string())),
        }
    }

    fn run_fit_job(&mut self, args: Vec<String>, ok_message: String) {
        if self.other_command_running() {
            self.fit.note = Some(Err(
                "another command is running; wait for it to finish.".into()
            ));
            return;
        }
        let cli = self.source.cli.clone();
        let stats = Arc::clone(&self.stats);
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new()
            .name("beamcraft-fit-cut-hole".into())
            .spawn(move || {
                let result = command::run(&cli, &args, command::DEADLINE, &stats).and_then(|out| {
                    let reply = decode(&out.stdout)?;
                    rejection(&reply, out.code).map_or(Ok(ok_message), Err)
                });
                let _ = tx.send(result);
            });
        match spawned {
            Ok(_) => {
                self.fit.commit_job = Some(rx);
                self.fit.note = None;
            }
            Err(_) => self.fit.note = Some(Err("The command could not be started.".into())),
        }
    }

    /// Starts the committed `fit-end-to-part` on a worker.
    pub(crate) fn start_fit_commit(&mut self) {
        let Some(operation) = crate::sheet::fresh_id() else {
            self.fit.note = Some(Err("No identity could be made for the fitting.".into()));
            return;
        };
        match fit_args(
            &self.source.store,
            self.source.actor.as_deref(),
            self.fit.part.as_deref().unwrap_or(""),
            self.fit.tool.as_deref().unwrap_or(""),
            &operation,
        ) {
            Ok(args) => self.run_fit_job(
                args,
                "End fitted. Pick the next part, or press Esc to finish.".into(),
            ),
            Err(message) => self.fit.note = Some(Err(message)),
        }
    }

    /// Starts the active `Bench::Cut` operation on a worker.
    pub(crate) fn start_cut_commit(&mut self) {
        let Some(operation) = crate::sheet::fresh_id() else {
            self.fit.note = Some(Err("No identity could be made for the operation.".into()));
            return;
        };
        let built = match self.fit.cut_mode {
            CutMode::ByPart => cut_by_part_args(
                &self.source.store,
                self.source.actor.as_deref(),
                self.fit.part.as_deref().unwrap_or(""),
                self.fit.tool.as_deref().unwrap_or(""),
                &operation,
            )
            .map(|a| (a, "Part cut applied.".to_owned())),
            CutMode::ByPlane => cut_by_plane_args(
                &self.source.store,
                self.source.actor.as_deref(),
                self.fit.part.as_deref().unwrap_or(""),
                &operation,
                &self.fit.point,
                &self.fit.normal,
                &self.fit.keep,
            )
            .map(|a| (a, "Plane cut applied.".to_owned())),
            CutMode::Split => {
                let Some(new_id) = crate::sheet::fresh_id() else {
                    self.fit.note = Some(Err("No identity could be made for the split.".into()));
                    return;
                };
                split_part_args(
                    &self.source.store,
                    self.source.actor.as_deref(),
                    self.fit.part.as_deref().unwrap_or(""),
                    &new_id,
                    &self.fit.split_name,
                    &operation,
                    &self.fit.point,
                    &self.fit.normal,
                    &self.fit.keep,
                )
                .map(|a| (a, "Part split into two parts.".to_owned()))
            }
            CutMode::Combine => combine_parts_args(
                &self.source.store,
                self.source.actor.as_deref(),
                self.fit.part.as_deref().unwrap_or(""),
                self.fit.tool.as_deref().unwrap_or(""),
                &operation,
            )
            .map(|a| (a, "Parts combined.".to_owned())),
        };
        match built {
            Ok((args, msg)) => self.run_fit_job(args, msg),
            Err(msg) => self.fit.note = Some(Err(msg)),
        }
    }

    /// Starts the active `Bench::Hole` operation on a worker.
    pub(crate) fn start_hole_commit(&mut self) {
        let Some(operation) = crate::sheet::fresh_id() else {
            self.fit.note = Some(Err("No identity could be made for the hole.".into()));
            return;
        };
        match add_hole_args(
            &self.source.store,
            self.source.actor.as_deref(),
            self.fit.part.as_deref().unwrap_or(""),
            &operation,
            self.fit.hole_kind,
            &self.fit.point,
            &self.fit.normal,
            &self.fit.local_xy,
            &self.fit.diameter,
            &self.fit.rect_wh,
        ) {
            Ok(args) => self.run_fit_job(args, "Hole created.".into()),
            Err(msg) => self.fit.note = Some(Err(msg)),
        }
    }

    /// Drains the fit/cut/hole commit; a success reloads the model and readies the next pair.
    pub(crate) fn poll_fit(&mut self) {
        let Some(rx) = &self.fit.commit_job else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(ToolError::local(
                "CLI_FAILED",
                "The command ended without a result.",
            )),
        };
        self.fit.commit_job = None;
        if self.bench == super::workbench::Bench::Fit
            || (self.bench == super::workbench::Bench::Cut
                && matches!(self.fit.cut_mode, CutMode::ByPart | CutMode::Combine))
        {
            self.fit.clear_placement();
        }
        match result {
            Ok(msg) => {
                self.fit.note = Some(Ok(msg));
                self.reload();
            }
            Err(error) => {
                if error.code == "OUTCOME_UNKNOWN" {
                    self.reload();
                }
                self.fit.note = Some(Err(error.to_string()));
            }
        }
    }

    /// Bottom action bar for `Bench::Fit`.
    pub(crate) fn fit_actions(&mut self, ui: &mut egui::Ui) {
        let busy = self.fit.busy() || self.other_command_running();
        let can_apply = !busy && self.fit.part.is_some() && self.fit.tool.is_some();
        let done = crate::kit::actions(ui, "Fit end", can_apply, "Cancel", busy);
        if done.primary {
            self.start_fit_commit();
        }
        if done.cancel {
            self.fit.clear_placement();
            self.set_bench(super::workbench::Bench::Select);
        }
    }

    /// Bottom action bar for `Bench::Cut`.
    pub(crate) fn cut_part_actions(&mut self, ui: &mut egui::Ui) {
        let busy = self.fit.busy() || self.other_command_running();
        let primary = match self.fit.cut_mode {
            CutMode::ByPart | CutMode::ByPlane => "Apply cut",
            CutMode::Split => "Split part",
            CutMode::Combine => "Combine",
        };
        let can_apply = !busy && self.fit.part.is_some();
        let done = crate::kit::actions(ui, primary, can_apply, "Cancel", busy);
        if done.primary {
            self.start_cut_commit();
        }
        if done.cancel {
            self.fit.clear_placement();
            self.set_bench(super::workbench::Bench::Select);
        }
    }

    /// Bottom action bar for `Bench::Hole`.
    pub(crate) fn hole_actions(&mut self, ui: &mut egui::Ui) {
        let busy = self.fit.busy() || self.other_command_running();
        let can_apply = !busy && self.fit.part.is_some();
        let done = crate::kit::actions(ui, "Create hole", can_apply, "Cancel", busy);
        if done.primary {
            self.start_hole_commit();
        }
        if done.cancel {
            self.fit.clear_placement();
            self.set_bench(super::workbench::Bench::Select);
        }
    }

    /// The inspector panel of the Fit end bench (`Bench::Fit`).
    pub(crate) fn fit_panel(&mut self, ui: &mut egui::Ui) {
        use crate::kit;
        let palette = crate::theme::Palette::current(ui);
        ui.set_max_width(ui.available_width());
        kit::type_box(ui, crate::theme::Icon::End, "Fit end to part");
        kit::section(ui, "fit", "Parts", true, |ui| {
            kit::prop_info(ui, "Part to trim", self.fit.part.as_deref(), "Click a part");
            kit::prop_info(
                ui,
                "Fit to plate",
                self.fit.tool.as_deref(),
                "Click a plate",
            );
            kit::prop_row(ui, "", |ui| {
                ui.label(
                    egui::RichText::new(if self.fit.part.is_none() {
                        "Click the beam, column or plate whose end is trimmed."
                    } else {
                        "Click the plate the end fits to; the end nearer the plate is trimmed \
                         flush with its nearest face."
                    })
                    .size(11.0)
                    .color(palette.muted),
                );
            });
            kit::prop_row(ui, "", |ui| {
                if ui
                    .add_enabled(
                        self.fit.part.is_some() && !self.fit.busy(),
                        egui::Button::new("Clear picks"),
                    )
                    .clicked()
                {
                    self.fit.clear_placement();
                }
            });
        });
    }

    /// The inspector panel of the Cut & Split bench (`Bench::Cut`).
    pub(crate) fn cut_part_panel(&mut self, ui: &mut egui::Ui) {
        use crate::kit;
        use crate::theme::Icon;
        ui.set_max_width(ui.available_width());
        kit::type_box(ui, Icon::Cut, "Cut & split part");
        if self.fit.part.is_none()
            && let Some(sel) = self.editor.selected()
        {
            self.fit.part = Some(sel.id.clone());
        }
        kit::section(ui, "cut-part", "Operation", true, |ui| {
            kit::prop_row(ui, "Mode", |ui| {
                egui::ComboBox::from_id_salt("cut-part-mode")
                    .width((ui.available_width() - kit::FIT).max(80.0))
                    .selected_text(self.fit.cut_mode.label())
                    .show_ui(ui, |ui| {
                        for mode in [
                            CutMode::ByPart,
                            CutMode::ByPlane,
                            CutMode::Split,
                            CutMode::Combine,
                        ] {
                            ui.selectable_value(&mut self.fit.cut_mode, mode, mode.label());
                        }
                    });
            });
            kit::prop_info(
                ui,
                if self.fit.cut_mode == CutMode::Combine {
                    "Survivor part"
                } else {
                    "Target part"
                },
                self.fit.part.as_deref(),
                "Click a part in view",
            );
            match self.fit.cut_mode {
                CutMode::ByPart | CutMode::Combine => {
                    kit::prop_info(
                        ui,
                        if self.fit.cut_mode == CutMode::Combine {
                            "Part to merge"
                        } else {
                            "Cutting part"
                        },
                        self.fit.tool.as_deref(),
                        "Click second part",
                    );
                }
                CutMode::ByPlane | CutMode::Split => {
                    if self.fit.cut_mode == CutMode::Split {
                        super::field(ui, "New part name", &mut self.fit.split_name, 130.0);
                    }
                    for (label, val) in ["Point X, mm", "Point Y, mm", "Point Z, mm"]
                        .into_iter()
                        .zip(&mut self.fit.point)
                    {
                        super::field(ui, label, val, 110.0);
                    }
                    for (label, val) in ["Normal X", "Normal Y", "Normal Z"]
                        .into_iter()
                        .zip(&mut self.fit.normal)
                    {
                        super::field(ui, label, val, 110.0);
                    }
                    super::creation_panel::token_combo(
                        ui,
                        "cut-keep-side",
                        "Keep side",
                        &mut self.fit.keep,
                        &[("positive", "Positive"), ("negative", "Negative")],
                    );
                }
            }
        });
    }

    /// The inspector panel of the Hole bench (`Bench::Hole`).
    pub(crate) fn hole_panel(&mut self, ui: &mut egui::Ui) {
        use crate::kit;
        use crate::theme::{self, Icon, Kind};
        ui.set_max_width(ui.available_width());
        kit::type_box(ui, Icon::Hole, "Bolt / part holes");
        let busy = self.fit.busy() || self.other_command_running();
        if self.fit.part.is_none()
            && let Some(sel) = self.editor.selected()
        {
            self.fit.part = Some(sel.id.clone());
        }
        kit::section(ui, "hole", "Create hole", true, |ui| {
            kit::prop_info(
                ui,
                "Target part",
                self.fit.part.as_deref(),
                "Click a part in view",
            );
            kit::prop_row(ui, "Shape", |ui| {
                egui::ComboBox::from_id_salt("hole-shape-kind")
                    .width((ui.available_width() - kit::FIT).max(80.0))
                    .selected_text(self.fit.hole_kind.label())
                    .show_ui(ui, |ui| {
                        for kind in [
                            HoleKind::AxisRound,
                            HoleKind::PlateRound,
                            HoleKind::PlateRect,
                        ] {
                            ui.selectable_value(&mut self.fit.hole_kind, kind, kind.label());
                        }
                    });
            });
            match self.fit.hole_kind {
                HoleKind::AxisRound => {
                    for (label, val) in ["Point X, mm", "Point Y, mm", "Point Z, mm"]
                        .into_iter()
                        .zip(&mut self.fit.point)
                    {
                        super::field(ui, label, val, 110.0);
                    }
                    for (label, val) in ["Axis X", "Axis Y", "Axis Z"]
                        .into_iter()
                        .zip(&mut self.fit.normal)
                    {
                        super::field(ui, label, val, 110.0);
                    }
                    super::field(ui, "Diameter, mm", &mut self.fit.diameter, 110.0);
                }
                HoleKind::PlateRound => {
                    let [lx, ly] = &mut self.fit.local_xy;
                    super::field(ui, "Local X, mm", lx, 110.0);
                    super::field(ui, "Local Y, mm", ly, 110.0);
                    super::field(ui, "Diameter, mm", &mut self.fit.diameter, 110.0);
                }
                HoleKind::PlateRect => {
                    let [lx, ly] = &mut self.fit.local_xy;
                    let [rw, rh] = &mut self.fit.rect_wh;
                    super::field(ui, "Local X, mm", lx, 110.0);
                    super::field(ui, "Local Y, mm", ly, 110.0);
                    super::field(ui, "Width, mm", rw, 110.0);
                    super::field(ui, "Height, mm", rh, 110.0);
                }
            }
        });

        kit::section(ui, "hole", "Remove hole", false, |ui| {
            super::field(ui, "Operation ID", &mut self.fit.remove_op_id, 130.0);
            kit::prop_row(ui, "", |ui| {
                let can_remove =
                    !busy && self.fit.part.is_some() && !self.fit.remove_op_id.trim().is_empty();
                if theme::button(ui, Kind::Secondary, "Remove hole", can_remove).clicked() {
                    let mut args = vec![
                        "remove-hole".to_owned(),
                        "--store".into(),
                        self.source.store.to_string_lossy().into_owned(),
                        "--id".into(),
                        self.fit.part.clone().unwrap_or_default(),
                        "--operation-id".into(),
                        self.fit.remove_op_id.trim().to_owned(),
                    ];
                    if let Some(actor) = self.source.actor.as_deref() {
                        args.extend(["--actor-id".into(), actor.to_owned()]);
                    }
                    self.run_fit_job(args, "Hole removed.".into());
                }
            });
        });
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn the_fit_runs_the_cli_command_with_both_parts_and_a_fresh_operation() {
        let args = fit_args(Path::new("m"), Some("actor"), " a ", "b", "op").unwrap();
        assert_eq!(
            args,
            [
                "fit-end-to-part",
                "--store",
                "m",
                "--id",
                "a",
                "--operation-id",
                "op",
                "--tool",
                "b",
                "--actor-id",
                "actor"
            ]
        );
        assert!(fit_args(Path::new("m"), None, "", "b", "op").is_err());
        assert!(fit_args(Path::new("m"), None, "a", "a", "op").is_err());
    }

    #[test]
    fn cut_and_hole_builders_match_cli_contract() {
        let store = Path::new("m");
        let pt = ["100".to_owned(), "200".to_owned(), "0".to_owned()];
        let nrm = ["0".to_owned(), "0".to_owned(), "1".to_owned()];
        assert_eq!(
            cut_by_part_args(store, None, "p1", "p2", "op1").unwrap(),
            [
                "cut-by-part",
                "--store",
                "m",
                "--id",
                "p1",
                "--operation-id",
                "op1",
                "--tool",
                "p2"
            ]
        );
        assert_eq!(
            cut_by_plane_args(store, None, "p1", "op2", &pt, &nrm, "positive").unwrap(),
            [
                "cut-by-plane",
                "--store",
                "m",
                "--id",
                "p1",
                "--operation-id",
                "op2",
                "--point",
                "100,200,0",
                "--normal",
                "0,0,1",
                "--keep",
                "positive"
            ]
        );
        let xy = ["50".to_owned(), "60".to_owned()];
        let wh = ["40".to_owned(), "30".to_owned()];
        assert_eq!(
            add_hole_args(
                store,
                None,
                "p1",
                "op3",
                HoleKind::PlateRound,
                &pt,
                &nrm,
                &xy,
                "22",
                &wh
            )
            .unwrap(),
            [
                "add-hole",
                "--store",
                "m",
                "--id",
                "p1",
                "--operation-id",
                "op3",
                "--hole-round",
                "50,60,22"
            ]
        );
    }
}
