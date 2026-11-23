// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The Array bench (#845): create linear (`array-linear`) and polar (`array-polar`) arrays of
//! the selected element through the CLI commands.

use super::App;
use crate::command;
use crate::tools::{MAX_TEXT, ToolError, decode, rejection};
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

/// Which array pattern the form builds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum ArrayKind {
    /// `array-linear`: copies along a direction vector at equal spacing.
    #[default]
    Linear,
    /// `array-polar`: copies rotated around an axis at equal angular step.
    Polar,
}

impl ArrayKind {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Linear => "Linear array",
            Self::Polar => "Polar array",
        }
    }
}

/// State of the Array bench between frames.
pub(crate) struct Panel {
    /// Linear or Polar array.
    pub(crate) kind: ArrayKind,
    /// Number of copies (`--count`).
    pub(crate) count: String,
    /// Optional base label (`--name`).
    pub(crate) name: String,
    /// Linear direction vector `[X, Y, Z]` (`--direction`).
    pub(crate) direction: [String; 3],
    /// Linear spacing in mm (`--spacing`).
    pub(crate) spacing: String,
    /// Polar centre point `[X, Y, Z]` in mm (`--point`).
    pub(crate) point: [String; 3],
    /// Polar rotation axis `[X, Y, Z]` (`--axis`).
    pub(crate) axis: [String; 3],
    /// Polar step angle in degrees (converted to radians for `--angle`).
    pub(crate) angle_deg: String,
    pub(super) commit_job: Option<Receiver<Result<String, ToolError>>>,
    /// Last outcome or refusal message.
    pub(crate) note: Option<Result<String, String>>,
}

impl Panel {
    pub(super) fn empty() -> Self {
        Self {
            kind: ArrayKind::default(),
            count: "3".into(),
            name: String::new(),
            direction: ["1".into(), "0".into(), "0".into()],
            spacing: "3000".into(),
            point: ["0".into(), "0".into(), "0".into()],
            axis: ["0".into(), "0".into(), "1".into()],
            angle_deg: "45".into(),
            commit_job: None,
            note: None,
        }
    }

    pub(crate) fn busy(&self) -> bool {
        self.commit_job.is_some()
    }
}

fn triple(label: &str, coords: &[String; 3]) -> Result<String, String> {
    let mut vals = [0.0f64; 3];
    for (slot, s) in vals.iter_mut().zip(coords) {
        let t = s.trim();
        let v = t
            .parse::<f64>()
            .map_err(|_| format!("{label}: \"{t}\" is not a valid number."))?;
        if !v.is_finite() {
            return Err(format!("{label}: enter finite numbers."));
        }
        *slot = v;
    }
    let [x, y, z] = vals;
    Ok(format!("{x},{y},{z}"))
}

/// Builds the CLI arguments for `array-linear`.
pub(crate) fn linear_args(
    store: &Path,
    actor: Option<&str>,
    id: &str,
    count: &str,
    direction: &[String; 3],
    spacing: &str,
    name: &str,
) -> Result<Vec<String>, String> {
    let id = id.trim();
    if id.is_empty() {
        return Err("Select an element in the view to array.".into());
    }
    let count_n = count
        .trim()
        .parse::<u32>()
        .map_err(|_| "Count: enter a positive integer.".to_owned())?;
    if count_n == 0 {
        return Err("Count: must be at least 1.".into());
    }
    let dir = triple("Direction", direction)?;
    let sp = spacing
        .trim()
        .parse::<f64>()
        .map_err(|_| "Spacing: enter a number of millimetres.".to_owned())?;
    if !sp.is_finite() || sp <= 0.0 {
        return Err("Spacing: enter a positive number of millimetres.".into());
    }
    let name = name.trim();
    if name.len() > MAX_TEXT {
        return Err(format!("Name: at most {MAX_TEXT} bytes."));
    }
    let mut args = vec![
        "array-linear".into(),
        "--store".into(),
        store.to_string_lossy().into_owned(),
        "--id".into(),
        id.to_owned(),
        "--count".into(),
        count_n.to_string(),
        "--direction".into(),
        dir,
        "--spacing".into(),
        sp.to_string(),
    ];
    if !name.is_empty() {
        args.extend(["--name".into(), name.to_owned()]);
    }
    if let Some(actor) = actor {
        args.extend(["--actor-id".into(), actor.to_owned()]);
    }
    Ok(args)
}

/// Builds the CLI arguments for `array-polar`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn polar_args(
    store: &Path,
    actor: Option<&str>,
    id: &str,
    count: &str,
    point: &[String; 3],
    axis: &[String; 3],
    angle_deg: &str,
    name: &str,
) -> Result<Vec<String>, String> {
    let id = id.trim();
    if id.is_empty() {
        return Err("Select an element in the view to array.".into());
    }
    let count_n = count
        .trim()
        .parse::<u32>()
        .map_err(|_| "Count: enter a positive integer.".to_owned())?;
    if count_n == 0 {
        return Err("Count: must be at least 1.".into());
    }
    let pt = triple("Point", point)?;
    let ax = triple("Axis", axis)?;
    let deg = angle_deg
        .trim()
        .parse::<f64>()
        .map_err(|_| "Angle: enter a number of degrees.".to_owned())?;
    if !deg.is_finite() || deg.abs() < 1e-6 {
        return Err("Angle: enter a non-zero angle in degrees.".into());
    }
    let rad = deg.to_radians();
    let name = name.trim();
    if name.len() > MAX_TEXT {
        return Err(format!("Name: at most {MAX_TEXT} bytes."));
    }
    let mut args = vec![
        "array-polar".into(),
        "--store".into(),
        store.to_string_lossy().into_owned(),
        "--id".into(),
        id.to_owned(),
        "--count".into(),
        count_n.to_string(),
        "--point".into(),
        pt,
        "--axis".into(),
        ax,
        "--angle".into(),
        rad.to_string(),
    ];
    if !name.is_empty() {
        args.extend(["--name".into(), name.to_owned()]);
    }
    if let Some(actor) = actor {
        args.extend(["--actor-id".into(), actor.to_owned()]);
    }
    Ok(args)
}

impl App {
    /// Starts the configured linear or polar array command on a worker thread.
    pub(crate) fn start_array_commit(&mut self) {
        if self.other_command_running() || self.array.busy() {
            self.array.note = Some(Err("Another command is running; wait for it.".into()));
            return;
        }
        let id = self
            .editor
            .selected()
            .map_or("", |info| info.id.as_str())
            .to_owned();
        let built = match self.array.kind {
            ArrayKind::Linear => linear_args(
                &self.source.store,
                self.source.actor.as_deref(),
                &id,
                &self.array.count,
                &self.array.direction,
                &self.array.spacing,
                &self.array.name,
            ),
            ArrayKind::Polar => polar_args(
                &self.source.store,
                self.source.actor.as_deref(),
                &id,
                &self.array.count,
                &self.array.point,
                &self.array.axis,
                &self.array.angle_deg,
                &self.array.name,
            ),
        };
        let args = match built {
            Ok(args) => args,
            Err(msg) => {
                self.array.note = Some(Err(msg));
                return;
            }
        };
        let count_str = self.array.count.trim().to_owned();
        let cli = self.source.cli.clone();
        let stats = Arc::clone(&self.stats);
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new()
            .name("beamcraft-array-cmd".into())
            .spawn(move || {
                let res = command::run(&cli, &args, command::DEADLINE, &stats).and_then(|out| {
                    let reply = decode(&out.stdout)?;
                    rejection(&reply, out.code)
                        .map_or_else(|| Ok(format!("Array created ({count_str} copies).")), Err)
                });
                let _ = tx.send(res);
            });
        match spawned {
            Ok(_) => {
                self.array.commit_job = Some(rx);
                self.array.note = None;
            }
            Err(_) => {
                self.array.note = Some(Err("The array command could not be started.".into()));
            }
        }
    }

    /// Polls any running array command.
    pub(crate) fn poll_array(&mut self) {
        let Some(rx) = &self.array.commit_job else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(res) => res,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(ToolError::local(
                "CLI_FAILED",
                "The array command ended without a result.",
            )),
        };
        self.array.commit_job = None;
        match result {
            Ok(msg) => {
                self.array.note = Some(Ok(msg));
                self.reload();
            }
            Err(error) => {
                if error.code == "OUTCOME_UNKNOWN" {
                    self.reload();
                }
                self.array.note = Some(Err(error.to_string()));
            }
        }
    }

    /// Bottom action bar for `Bench::Array`.
    pub(crate) fn array_actions(&mut self, ui: &mut egui::Ui) {
        let busy = self.array.busy() || self.other_command_running();
        let has_sel = self.editor.selected().is_some();
        let done = crate::kit::actions(ui, "Create array", !busy && has_sel, "Cancel", busy);
        if done.primary {
            self.start_array_commit();
        }
        if done.cancel {
            self.set_bench(super::workbench::Bench::Select);
        }
    }

    /// Renders the Array inspector panel (`Bench::Array`).
    pub(crate) fn array_panel(&mut self, ui: &mut egui::Ui) {
        use crate::kit;
        use crate::theme::Icon;
        ui.set_max_width(ui.available_width());
        kit::type_box(ui, Icon::Array, "Array copies");
        let selected_label = self
            .editor
            .selected()
            .map(|info| format!("{} {}", info.kind, info.name));

        kit::section(ui, "array", "Pattern", true, |ui| {
            kit::prop_info(
                ui,
                "Source part",
                selected_label.as_deref(),
                "Select a part in view",
            );
            kit::prop_row(ui, "Mode", |ui| {
                egui::ComboBox::from_id_salt("array-mode")
                    .width((ui.available_width() - kit::FIT).max(80.0))
                    .selected_text(self.array.kind.label())
                    .show_ui(ui, |ui| {
                        for k in [ArrayKind::Linear, ArrayKind::Polar] {
                            ui.selectable_value(&mut self.array.kind, k, k.label());
                        }
                    });
            });
            super::field(ui, "Copies count", &mut self.array.count, 90.0);
            super::field(ui, "Name prefix", &mut self.array.name, 130.0);
        });

        kit::section(ui, "array", "Geometry", true, |ui| match self.array.kind {
            ArrayKind::Linear => {
                for (label, val) in ["Direction X", "Direction Y", "Direction Z"]
                    .into_iter()
                    .zip(&mut self.array.direction)
                {
                    super::field(ui, label, val, 110.0);
                }
                super::field(ui, "Spacing, mm", &mut self.array.spacing, 110.0);
            }
            ArrayKind::Polar => {
                for (label, val) in ["Center X, mm", "Center Y, mm", "Center Z, mm"]
                    .into_iter()
                    .zip(&mut self.array.point)
                {
                    super::field(ui, label, val, 110.0);
                }
                for (label, val) in ["Axis X", "Axis Y", "Axis Z"]
                    .into_iter()
                    .zip(&mut self.array.axis)
                {
                    super::field(ui, label, val, 110.0);
                }
                super::field(ui, "Step angle, °", &mut self.array.angle_deg, 110.0);
            }
        });
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn array_builders_match_cli_contract() {
        let store = Path::new("m");
        let dir = ["1".to_owned(), "0".to_owned(), "0".to_owned()];
        assert_eq!(
            linear_args(store, Some("act"), "p1", "4", &dir, "6000", "Bay").unwrap(),
            [
                "array-linear",
                "--store",
                "m",
                "--id",
                "p1",
                "--count",
                "4",
                "--direction",
                "1,0,0",
                "--spacing",
                "6000",
                "--name",
                "Bay",
                "--actor-id",
                "act"
            ]
        );
        let pt = ["0".to_owned(), "0".to_owned(), "0".to_owned()];
        let ax = ["0".to_owned(), "0".to_owned(), "1".to_owned()];
        let polar = polar_args(store, None, "p1", "6", &pt, &ax, "60", "").unwrap();
        assert_eq!(polar[0], "array-polar");
        assert_eq!(polar[6], "6");
    }
}
