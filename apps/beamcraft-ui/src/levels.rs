// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The levels form: it prepares `create-levels`, `update-level` and `delete-element` as the same
//! commands the CLI runs. One gesture is one command, hence one revision and one undo unit.

use crate::camera::View;
use crate::stats::Stats;
use crate::tools::{FormError, LevelChoice, ToolError, decode, push_actor, rejection};
use crate::{command, theme};
use beamcraft_json::Json;
use beamcraft_model::id::ElementId;
use beamcraft_model::level::{level_elevations, level_names};
use std::path::Path;

/// The longest name a section caption shows, in characters.
const MAX_CAPTION_CHARS: usize = 32;
/// The screen height of one caption, in points.
const CAPTION_HEIGHT: f32 = 20.0;
/// The distance of a level's mark from the pane's right edge, in points.
const MARK_INSET: f32 = 16.0;
/// How far a level's line runs past the model and the grids on either side, in points.
const LINE_OVERHANG: f32 = 40.0;

/// The text of the level form.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LevelForm {
    /// The level's label.
    pub name: String,
    /// The elevation in millimetres, as typed.
    pub elevation: String,
    /// A new elevation carries the beams and columns on the level with it
    /// (`--move-elements true`).
    pub move_elements: bool,
}

/// The text of the new-levels form: what `create-levels` takes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewLevelsForm {
    /// The elevations: the first one, then spacings, `N*D` for N equal spacings (millimetres).
    pub elevations: String,
    /// Names separated by commas; the rest are generated. Empty generates every name.
    pub names: String,
}

impl NewLevelsForm {
    /// The levels the form creates in a model holding `existing`, lowest first, named as the
    /// engine names them.
    pub fn preview(&self, existing: &[LevelChoice]) -> Result<Vec<(String, f64)>, FormError> {
        let elevations = level_elevations(&self.elevations)
            .map_err(|error| form_error("Elevations", error.message))?;
        let used = existing.iter().map(|level| level.name.clone()).collect();
        let names = level_names(&self.names, elevations.len(), &used)
            .map_err(|error| form_error("Names", error.message))?;
        Ok(names.into_iter().zip(elevations).collect())
    }

    /// The arguments of `create-levels`.
    pub fn prepare(&self, store: &Path, actor: Option<&str>) -> Result<Vec<String>, FormError> {
        self.preview(&[])?;
        let mut args = vec![
            "create-levels".to_owned(),
            "--store".into(),
            store_text(store)?.into(),
            "--elevations".into(),
            self.elevations.trim().to_owned(),
        ];
        let names = self.names.trim();
        if !names.is_empty() {
            args.extend(["--names".into(), names.to_owned()]);
        }
        push_actor(&mut args, actor);
        Ok(args)
    }
}

fn form_error(field: &'static str, message: &str) -> FormError {
    FormError {
        field,
        message: message.to_owned(),
    }
}

fn store_text(store: &Path) -> Result<&str, FormError> {
    store
        .to_str()
        .ok_or_else(|| form_error("Model", "The model path must be UTF-8."))
}

impl LevelForm {
    /// The form showing an existing level.
    pub fn of(level: &LevelChoice) -> Self {
        Self {
            name: level.name.clone(),
            elevation: level.elevation_mm.to_string(),
            move_elements: true,
        }
    }

    fn name(&self) -> Result<&str, FormError> {
        let name = self.name.trim();
        if name.is_empty() || name.len() > 256 {
            return Err(form_error(
                "Name",
                "Enter a level name of at most 256 UTF-8 bytes.",
            ));
        }
        Ok(name)
    }

    fn elevation(&self) -> Result<f64, FormError> {
        self.elevation
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .ok_or_else(|| form_error("Elevation", "Enter a finite number using a decimal point."))
    }

    /// Whether the form differs from `level`, as the values it would send.
    pub fn differs(&self, level: &LevelChoice) -> bool {
        self.name.trim() != level.name
            || !self
                .elevation()
                .is_ok_and(|value| value == level.elevation_mm)
    }

    /// The arguments of `update-level`, naming only the values that changed.
    pub fn prepare_update(
        &self,
        level: &LevelChoice,
        store: &Path,
        actor: Option<&str>,
    ) -> Result<Vec<String>, FormError> {
        let name = self.name()?;
        let elevation = self.elevation()?;
        ElementId::parse(&level.id).map_err(|_| form_error("Level", "Select a level."))?;
        let mut args = vec![
            "update-level".to_owned(),
            "--store".into(),
            store_text(store)?.into(),
            "--id".into(),
            level.id.clone(),
        ];
        if name != level.name {
            args.extend(["--name".into(), name.into()]);
        }
        if elevation != level.elevation_mm {
            args.extend(["--elevation".into(), elevation.to_string()]);
            if self.move_elements {
                args.extend(["--move-elements".into(), "true".into()]);
            }
        }
        push_actor(&mut args, actor);
        Ok(args)
    }
}

/// The arguments of `delete-element` for a level.
pub fn prepare_delete(
    level: &LevelChoice,
    store: &Path,
    actor: Option<&str>,
) -> Result<Vec<String>, FormError> {
    ElementId::parse(&level.id).map_err(|_| form_error("Level", "Select a level."))?;
    let mut args = vec![
        "delete-element".to_owned(),
        "--store".into(),
        store_text(store)?.into(),
        "--id".into(),
        level.id.clone(),
    ];
    push_actor(&mut args, actor);
    Ok(args)
}

/// Reads a command's reply: the number of beams and columns that followed the level. A level
/// still in use gets a message that says what to do.
pub fn parse_reply(exit: Option<i32>, bytes: &[u8]) -> Result<usize, ToolError> {
    let reply = decode(bytes)?;
    match rejection(&reply, exit) {
        Some(mut error) => {
            if error.code == "DEPENDENCY_CONFLICT" {
                "Elements still use this level. Assign them to another level first."
                    .clone_into(&mut error.message);
            }
            Err(error)
        }
        None => Ok(reply
            .get("moved_elements")
            .and_then(Json::array)
            .map_or(0, <[Json]>::len)),
    }
}

/// Runs one prepared level command; the result counts the beams and columns that followed.
pub fn commit(cli: &Path, args: &[String], stats: &Stats) -> Result<usize, ToolError> {
    let out = command::run(cli, args, command::DEADLINE, stats)?;
    parse_reply(out.code, &out.stdout)
}

/// The levels from the highest down; equal elevations keep the order of their names.
pub fn sorted(levels: &[LevelChoice]) -> Vec<&LevelChoice> {
    let mut rows: Vec<&LevelChoice> = levels.iter().collect();
    rows.sort_by(|a, b| {
        b.elevation_mm
            .total_cmp(&a.elevation_mm)
            .then_with(|| a.name.cmp(&b.name))
    });
    rows
}

/// Where a level's line runs across `rect`, in screen points. Only a level seen edge-on is a line:
/// an orthographic view whose screen up is the model's Z. Any other view has none, and neither
/// has a level outside the pane.
pub fn section_row(view: &View, rect: egui::Rect, origin_z: f64, elevation_mm: f64) -> Option<f32> {
    let [.., right_z, _] = view.right;
    let [.., up_z, _] = view.up;
    let [.., back_z, _] = view.back;
    let [eye_x, eye_y, ..] = view.eye;
    let [ortho, ..] = view.mode;
    if ortho < 0.5 || up_z.abs() < 0.999 || right_z.abs() > 0.001 || back_z.abs() > 0.001 {
        return None;
    }
    // Right below the eye the point is at the middle of the screen, whatever the depth.
    let [_, y, _] = view.project([eye_x, eye_y, (elevation_mm - origin_z) as f32]);
    let row = rect.top() + (1.0 - y) * rect.height() / 2.0;
    (row.is_finite() && rect.y_range().contains(row)).then_some(row)
}

/// The span of a level's line across `rect`, in screen points: the model and the grids as the view
/// sees them (`extent`, scene coordinates), with an overhang on either side; the middle half of
/// the pane when there is nothing to measure. `None` when the span misses the pane.
pub fn section_span(
    view: &View,
    rect: egui::Rect,
    extent: Option<([f32; 3], [f32; 3])>,
) -> Option<(f32, f32)> {
    let to_screen = |x: f32| rect.left() + (x + 1.0) * rect.width() / 2.0;
    let mut measured = (f32::INFINITY, f32::NEG_INFINITY);
    if let Some(([lx, ly, lz], [hx, hy, hz])) = extent {
        for x in [lx, hx] {
            for y in [ly, hy] {
                for z in [lz, hz] {
                    let [sx, ..] = view.project([x, y, z]);
                    let sx = to_screen(sx);
                    measured = (measured.0.min(sx), measured.1.max(sx));
                }
            }
        }
    }
    let (left, right) = if measured.0.is_finite() && measured.1.is_finite() {
        (measured.0 - LINE_OVERHANG, measured.1 + LINE_OVERHANG)
    } else {
        (
            rect.left() + rect.width() / 4.0,
            rect.right() - rect.width() / 4.0,
        )
    };
    let left = left.max(rect.left());
    let right = right.min(rect.right() - MARK_INSET);
    (left < right).then_some((left, right))
}

/// The caption of a level's line: its name, cut to fit, and its elevation.
fn caption(level: &LevelChoice) -> String {
    let name: String = level.name.chars().take(MAX_CAPTION_CHARS).collect();
    let cut = if name.len() < level.name.len() {
        "…"
    } else {
        ""
    };
    format!("{name}{cut}  {:+} mm", level.elevation_mm)
}

/// Paints the levels seen edge-on: a dashed line over the model and the grids (`extent`, scene
/// coordinates) and, at its right end, a mark with the name and elevation. Levels too close to
/// caption keep their lines.
pub fn paint_sections(
    ui: &egui::Ui,
    rect: egui::Rect,
    view: &View,
    origin_z: f64,
    extent: Option<([f32; 3], [f32; 3])>,
    levels: &[LevelChoice],
    selected: Option<&str>,
) {
    let painter = ui.painter_at(rect);
    let palette = theme::Palette::current(ui);
    let Some((left, right)) = section_span(view, rect, extent) else {
        return;
    };
    let mut last_caption = f32::NEG_INFINITY;
    for level in sorted(levels) {
        let Some(row) = section_row(view, rect, origin_z, level.elevation_mm) else {
            continue;
        };
        let active = selected == Some(level.id.as_str());
        let color = if active {
            theme::BRAND
        } else {
            palette.link.gamma_multiply(0.8)
        };
        let stroke = egui::Stroke::new(if active { 2.0 } else { 1.0 }, color);
        painter.extend(egui::Shape::dashed_line(
            &[egui::pos2(left, row), egui::pos2(right, row)],
            stroke,
            10.0,
            5.0,
        ));
        let tip = egui::pos2(right, row);
        painter.add(egui::Shape::convex_polygon(
            vec![
                tip,
                tip + egui::vec2(5.0, -8.0),
                tip + egui::vec2(-5.0, -8.0),
            ],
            color,
            egui::Stroke::NONE,
        ));
        if row - last_caption < CAPTION_HEIGHT {
            continue;
        }
        last_caption = row;
        let galley = painter.layout_no_wrap(
            caption(level),
            egui::FontId::proportional(12.0),
            palette.text,
        );
        let size = galley.size();
        let corner = egui::pos2(
            (right - 10.0 - size.x).max(rect.left() + 2.0),
            row - size.y - 3.0,
        );
        painter.rect_filled(
            egui::Rect::from_min_size(corner, size).expand(2.0),
            2,
            palette.surface,
        );
        painter.galley(corner, galley, palette.text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::Camera;
    use beamcraft_engine::work_plane::WorkPlane;

    #[test]
    fn a_level_is_a_line_only_when_seen_edge_on() {
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(100.0, 100.0));
        let aligned = |name| {
            let plane = WorkPlane::parallel(name, 0.0).unwrap();
            Camera::aligned_to(plane, [0.0; 3], 1000.0, 1000.0)
                .unwrap()
                .view(1.0)
                .unwrap()
        };
        for name in ["xz", "yz"] {
            let view = aligned(name);
            let row = section_row(&view, rect, 0.0, 250.0).unwrap();
            assert!((row - 25.0).abs() < 0.01, "{name}: {row}");
            // The scene is drawn relative to its origin, and so is the level.
            let shifted = section_row(&view, rect, 250.0, 250.0).unwrap();
            assert!((shifted - 50.0).abs() < 0.01, "{name}: {shifted}");
            assert_eq!(section_row(&view, rect, 0.0, 900.0), None, "off the pane");
        }
        assert_eq!(section_row(&aligned("xy"), rect, 0.0, 0.0), None, "a plan");
        let orbit = Camera::new([0.0; 3], 1000.0, 1.0)
            .unwrap()
            .view(1.0)
            .unwrap();
        assert_eq!(section_row(&orbit, rect, 0.0, 0.0), None, "a 3D view");
    }

    #[test]
    fn a_long_name_is_cut_on_a_character_boundary() {
        let mut long = LevelChoice {
            id: ID.into(),
            name: "Я".repeat(100),
            elevation_mm: -250.5,
            elements: 0,
        };
        let text = caption(&long);
        assert!(text.starts_with(&"Я".repeat(MAX_CAPTION_CHARS)));
        assert!(text.ends_with("…  -250.5 mm"));
        long.name = "L1".into();
        long.elevation_mm = 3000.0;
        assert_eq!(caption(&long), "L1  +3000 mm");
    }

    const ID: &str = "019a7bf0-0000-7000-8000-000000300001";

    fn level() -> LevelChoice {
        LevelChoice {
            id: ID.into(),
            name: "L1".into(),
            elevation_mm: 3000.0,
            elements: 2,
        }
    }

    fn form(name: &str, elevation: &str) -> LevelForm {
        LevelForm {
            name: name.into(),
            elevation: elevation.into(),
            move_elements: false,
        }
    }

    #[test]
    fn new_levels_are_previewed_as_the_engine_names_them_and_sent_as_typed() {
        let store = Path::new("model");
        let batch = NewLevelsForm {
            elevations: " -1000 1000 2*3500 ".into(),
            names: " B1 ".into(),
        };
        let mut existing = level();
        existing.name = "B2".into();
        assert_eq!(
            batch.preview(&[existing]).unwrap(),
            [
                ("B1".to_owned(), -1000.0),
                ("B3".to_owned(), 0.0),
                ("B4".to_owned(), 3500.0),
                ("B5".to_owned(), 7000.0)
            ]
        );
        assert_eq!(
            batch.prepare(store, None).unwrap(),
            [
                "create-levels",
                "--store",
                "model",
                "--elevations",
                "-1000 1000 2*3500",
                "--names",
                "B1"
            ]
        );
        let generated = NewLevelsForm {
            elevations: "0".into(),
            names: "  ".into(),
        };
        assert_eq!(
            generated.prepare(store, None).unwrap(),
            ["create-levels", "--store", "model", "--elevations", "0"]
        );
        for (elevations, names, field) in [
            ("", "", "Elevations"),
            ("-1000 0 6*3500", "", "Elevations"),
            ("0 3000", "A\u{7}", "Names"),
        ] {
            let form = NewLevelsForm {
                elevations: elevations.into(),
                names: names.into(),
            };
            assert_eq!(form.prepare(store, None).unwrap_err().field, field);
        }
    }

    #[test]
    fn a_level_line_spans_the_model_and_the_grids_not_the_pane() {
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, 500.0));
        let view = Camera::aligned_to(
            WorkPlane::parallel("xz", 0.0).unwrap(),
            [0.0; 3],
            1000.0,
            2000.0,
        )
        .unwrap()
        .view(2.0)
        .unwrap();
        let (left, right) = section_span(
            &view,
            rect,
            Some(([-100.0, -50.0, 0.0], [100.0, 50.0, 300.0])),
        )
        .unwrap();
        assert!(
            left > rect.left() && right < rect.right(),
            "{left}..{right}"
        );
        let [a, ..] = view.project([-100.0, 0.0, 0.0]);
        let [b, ..] = view.project([100.0, 0.0, 0.0]);
        let width = (b - a).abs() * rect.width() / 2.0;
        assert!(
            ((right - left) - (width + 2.0 * LINE_OVERHANG)).abs() < 0.5,
            "{left}..{right} for {width}"
        );
        // Nothing to measure: the middle half of the pane.
        assert_eq!(section_span(&view, rect, None), Some((250.0, 750.0)));
        // A model far off the pane has no line on it.
        assert_eq!(
            section_span(&view, rect, Some(([1e6, 0.0, 0.0], [1.1e6, 0.0, 0.0]))),
            None
        );
    }
    #[test]
    fn update_names_only_what_changed() {
        let store = Path::new("model");
        let args = form("L1", "3300")
            .prepare_update(&level(), store, None)
            .unwrap();
        assert_eq!(
            args,
            [
                "update-level",
                "--store",
                "model",
                "--id",
                ID,
                "--elevation",
                "3300"
            ]
        );
        assert!(!form("L1", "3000.0").differs(&level()));
        assert!(form("L2", "3000").differs(&level()));
        assert!(form("L1", "x").differs(&level()));
        // A moved elevation carries the members on the level when the box is ticked; a rename
        // alone moves nothing and sends no flag.
        let mut moving = form("L2", "3300");
        moving.move_elements = true;
        let args = moving.prepare_update(&level(), store, None).unwrap();
        assert!(args.windows(2).any(|p| p == ["--move-elements", "true"]));
        let renamed = LevelForm {
            elevation: "3000".into(),
            ..moving
        };
        let args = renamed.prepare_update(&level(), store, None).unwrap();
        assert!(!args.iter().any(|a| a == "--move-elements"));
        assert!(LevelForm::of(&level()).move_elements, "on by default");
        assert_eq!(
            parse_reply(
                Some(0),
                br#"{"outcome":"level_updated","moved_elements":["a","b"]}"#
            )
            .unwrap(),
            2
        );
    }

    #[test]
    fn a_used_level_is_explained_and_the_rows_run_from_the_top() {
        let reply = br#"{"outcome":"rejected","code":"DEPENDENCY_CONFLICT","message":"m"}"#;
        let error = parse_reply(Some(1), reply).unwrap_err();
        assert_eq!(error.code, "DEPENDENCY_CONFLICT");
        assert!(error.message.contains("another level"));
        let mut low = level();
        low.elevation_mm = -1.0;
        let levels = [low, level()];
        let rows = sorted(&levels);
        assert_eq!(
            rows.iter().map(|l| l.elevation_mm).collect::<Vec<_>>(),
            [3000.0, -1.0]
        );
    }
}
