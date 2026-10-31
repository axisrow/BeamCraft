// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! V12 named views: the session sidecar, the saved-views panel state and the extra view
//! windows. A view is display state: the book never touches the loaded snapshot, and a saved
//! view restores exactly the pose, projection and work area it captured.

use crate::camera::Camera;
use crate::grid::GridScene;
use beamcraft_engine::views::{PlaneSpec, ViewBook, WorkArea};
use beamcraft_json::Limits;
use std::path::{Path, PathBuf};

/// The session sidecar of one store: a sibling `<store>.views.json` file, never inside the
/// store itself.
pub fn session_path(store: &Path) -> PathBuf {
    let mut name = store.as_os_str().to_os_string();
    name.push(".views.json");
    PathBuf::from(name)
}

/// Loads the book for a store. A missing file is an empty book; anything unreadable is an
/// error note, never a reason to refuse the model.
pub fn load(store: &Path) -> Result<ViewBook, String> {
    let bytes = match std::fs::read(session_path(store)) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(ViewBook::new()),
        Err(e) => return Err(format!("the saved views could not be read: {}", e.kind())),
    };
    if bytes.is_empty() {
        return Ok(ViewBook::new());
    }
    if bytes.len() > 1024 * 1024 {
        return Err("the saved views file exceeds 1 MiB".to_owned());
    }
    let json = beamcraft_json::decode(&bytes, &Limits::DEFAULT)
        .map_err(|_| "the saved views file is not valid JSON".to_owned())?;
    beamcraft_host::views::book_from_json(&json)
        .map_err(|e| format!("the saved views were refused: {e}"))
}

/// Persists the book through a sibling temporary file, so a failed write keeps the old content.
pub fn persist(store: &Path, book: &ViewBook) -> Result<(), String> {
    let path = session_path(store);
    let bytes = beamcraft_host::views::book_to_json(book)
        .map_err(|e| format!("the saved views were refused: {e}"))?
        .encode(&Limits::DEFAULT)
        .map_err(|_| "the saved views exceed the JSON limits".to_owned())?;
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, &bytes)
        .and_then(|()| std::fs::rename(&temp, &path))
        .map_err(|e| format!("the saved views could not be written: {}", e.kind()))
}

/// A plane view the panel can build from displayed grids, a level or two typed points.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Plane {
    /// On a grid line of the chosen grid.
    #[default]
    GridLine,
    /// Near top-down plan on the frame centre.
    Level,
    /// From the first typed point toward the second.
    TwoPoints,
}

/// An explicit action from the saved-views window; editing fields changes nothing.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Action {
    /// Restore a saved view into the main viewport.
    Apply(String),
    /// Remove a saved view.
    Delete(String),
    /// Capture the current camera and work area under the typed name.
    SaveCurrent,
    /// Resolve and save the typed plane view.
    SavePlane(PlaneSpec),
    /// Install the typed work area as the current one.
    SetWorkArea(WorkArea),
    /// Drop the current work area.
    ClearWorkArea,
    /// Switch the projection of the main viewport.
    ToggleProjection,
}

/// The saved-views window: its list, its save forms and the view properties readout. Fields
/// are text until an explicit action, so typing never moves the camera.
#[derive(Debug, Default)]
pub(crate) struct Panel {
    /// Whether the window is shown.
    pub window_open: bool,
    pub(crate) name: String,
    pub(crate) plane: Plane,
    pub(crate) grid: usize,
    pub(crate) line: usize,
    pub(crate) reverse: bool,
    pub(crate) from: [String; 3],
    pub(crate) to: [String; 3],
    pub(crate) work_min: [String; 3],
    pub(crate) work_max: [String; 3],
}

/// A bounded name or coordinate field.
const FIELDS: usize = 64;

fn error(message: impl std::fmt::Display) -> String {
    message.to_string()
}

fn point_fields(ui: &mut egui::Ui, label: &str, point: &mut [String; 3]) {
    ui.horizontal(|ui| {
        ui.label(label);
        for (axis, text) in ["X", "Y", "Z"].iter().zip(point.iter_mut()) {
            ui.label(*axis);
            ui.add(
                egui::TextEdit::singleline(text)
                    .desired_width(56.0)
                    .char_limit(FIELDS),
            );
        }
    });
}

fn typed_point(text: &[String; 3]) -> Result<[f64; 3], String> {
    let numbers = text
        .join(",")
        .split(',')
        .map(|v| v.trim().parse::<f64>().map_err(|_| error("enter X,Y,Z")))
        .collect::<Result<Vec<_>, _>>()?;
    match numbers.as_slice() {
        [a, b, c] if [a, b, c].iter().all(|v| v.is_finite()) => Ok([*a, *b, *c]),
        _ => Err(error("enter three finite coordinates")),
    }
}

impl Panel {
    /// The typed name of the view being saved.
    pub(crate) fn typed_name(&self) -> &str {
        &self.name
    }

    /// Fills the work-area fields from the current one.
    pub(crate) fn set_work_area(&mut self, area: Option<&WorkArea>) {
        match area {
            Some(area) => {
                for (slot, value) in self.work_min.iter_mut().zip(area.min) {
                    *slot = format!("{value}");
                }
                for (slot, value) in self.work_max.iter_mut().zip(area.max) {
                    *slot = format!("{value}");
                }
            }
            None => {
                self.work_min = Default::default();
                self.work_max = Default::default();
            }
        }
    }

    /// The work area the fields describe; all-empty fields mean none.
    fn typed_work_area(&self) -> Result<Option<WorkArea>, String> {
        if self
            .work_min
            .iter()
            .chain(&self.work_max)
            .all(|t| t.trim().is_empty())
        {
            return Ok(None);
        }
        let number = |text: &str| {
            text.trim()
                .parse::<f64>()
                .map_err(|_| error("work area needs six numbers"))
        };
        let mut min = [0.0f64; 3];
        let mut max = [0.0f64; 3];
        for (slot, text) in min.iter_mut().zip(&self.work_min) {
            *slot = number(text)?;
        }
        for (slot, text) in max.iter_mut().zip(&self.work_max) {
            *slot = number(text)?;
        }
        WorkArea::new(min, max).map(Some).map_err(error)
    }

    /// The plane spec the form describes.
    fn typed_plane(&self, grids: &GridScene) -> Result<PlaneSpec, String> {
        match self.plane {
            Plane::GridLine => {
                let grid = grids
                    .grids
                    .get(self.grid)
                    .ok_or_else(|| error("no grid to place the view on"))?;
                let labels = grids
                    .lines
                    .get(grid.lines.clone())
                    .unwrap_or_default()
                    .iter()
                    .map(|l| l.label.as_str())
                    .collect::<Vec<_>>();
                let line = labels
                    .get(self.line)
                    .ok_or_else(|| error("choose a grid line"))?;
                Ok(PlaneSpec::GridLine {
                    grid: grid.form.name.clone(),
                    line: (*line).to_owned(),
                    reverse: self.reverse,
                })
            }
            Plane::Level => Ok(PlaneSpec::Level),
            Plane::TwoPoints => Ok(PlaneSpec::TwoPoints {
                from_mm: typed_point(&self.from)?,
                to_mm: typed_point(&self.to)?,
            }),
        }
    }

    /// Draws the window body and reports the explicit action, or the reason an action could
    /// not be built.
    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        book: &ViewBook,
        camera: &Camera,
        grids: &GridScene,
    ) -> Result<Option<Action>, String> {
        let mut action: Option<Result<Option<Action>, String>> = None;
        let pose = camera.pose();
        ui.label("View properties");
        ui.monospace(format!(
            "target {:.0}, {:.0}, {:.0} mm · yaw {:.2} · pitch {:.2} · distance {:.0} mm",
            pose.target_mm[0],
            pose.target_mm[1],
            pose.target_mm[2],
            pose.yaw,
            pose.pitch,
            pose.distance_mm,
        ));
        ui.horizontal(|ui| {
            ui.label(if pose.orthographic {
                "orthographic"
            } else {
                "perspective"
            });
            if ui.button("switch").clicked() {
                action = Some(Ok(Some(Action::ToggleProjection)));
            }
        });
        ui.separator();
        ui.label("Saved views");
        if book.is_empty() {
            ui.label("Nothing saved yet.");
        }
        let mut pick = None;
        for view in book.iter() {
            ui.horizontal(|ui| {
                let kind = match &view.plane {
                    None => "3D",
                    Some(PlaneSpec::GridLine { .. }) => "grid line",
                    Some(PlaneSpec::Level) => "level",
                    Some(PlaneSpec::TwoPoints { .. }) => "two points",
                };
                if ui.selectable_label(false, &view.name).clicked() {
                    pick = Some(view.name.clone());
                }
                ui.label(kind);
                if ui.button("×").on_hover_text("Remove view").clicked() {
                    pick = None;
                    action = Some(Ok(Some(Action::Delete(view.name.clone()))));
                }
            });
        }
        if let Some(name) = pick {
            action = Some(Ok(Some(Action::Apply(name))));
        }
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Name");
            ui.add(
                egui::TextEdit::singleline(&mut self.name)
                    .hint_text("view name")
                    .desired_width(120.0)
                    .char_limit(FIELDS),
            );
            if ui.button("Save current view").clicked() {
                action = Some(Ok(Some(Action::SaveCurrent)));
            }
        });
        ui.add_space(4.0);
        ui.label("Save a plane view");
        egui::ComboBox::from_id_salt("view-plane-kind")
            .selected_text(match self.plane {
                Plane::GridLine => "On a grid line",
                Plane::Level => "Level (plan)",
                Plane::TwoPoints => "Through two points",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.plane, Plane::GridLine, "On a grid line");
                ui.selectable_value(&mut self.plane, Plane::Level, "Level (plan)");
                ui.selectable_value(&mut self.plane, Plane::TwoPoints, "Through two points");
            });
        match self.plane {
            Plane::GridLine => {
                self.grid = self.grid.min(grids.grids.len().saturating_sub(1));
                let names: Vec<String> = grids.grids.iter().map(|g| g.form.name.clone()).collect();
                combo(ui, "view-grid", "Grid", &mut self.grid, &names);
                self.line = self.line.min(
                    grids
                        .grids
                        .get(self.grid)
                        .map(|g| g.lines.len())
                        .unwrap_or(1)
                        .saturating_sub(1),
                );
                let labels: Vec<String> = grids
                    .grids
                    .get(self.grid)
                    .and_then(|g| grids.lines.get(g.lines.clone()))
                    .map(|lines| lines.iter().map(|l| l.label.clone()).collect())
                    .unwrap_or_default();
                combo(ui, "view-line", "Line", &mut self.line, &labels);
                ui.checkbox(&mut self.reverse, "Look from the other side");
            }
            Plane::Level => {
                ui.label("Centred on the model bounds, near top-down.");
            }
            Plane::TwoPoints => {
                point_fields(ui, "From", &mut self.from);
                point_fields(ui, "To", &mut self.to);
            }
        }
        ui.horizontal(|ui| {
            if ui.button("Save plane view").clicked() {
                action = Some(
                    self.typed_plane(grids)
                        .map(|spec| Some(Action::SavePlane(spec))),
                );
            }
        });
        ui.separator();
        ui.label("Work area of the current view");
        point_fields(ui, "Min", &mut self.work_min);
        point_fields(ui, "Max", &mut self.work_max);
        ui.horizontal(|ui| {
            if ui.button("Set work area").clicked() {
                action = Some(
                    self.typed_work_area()
                        .map(|area| area.map(Action::SetWorkArea)),
                );
            }
            if ui.button("Clear").clicked() {
                action = Some(Ok(Some(Action::ClearWorkArea)));
            }
        });
        // The payload is already optional: `None` means nothing was clicked.
        action.unwrap_or(Ok(None))
    }
}

fn combo(ui: &mut egui::Ui, id: &str, label: &str, current: &mut usize, choices: &[String]) {
    ui.horizontal(|ui| {
        ui.label(label);
        let selected = choices
            .get(*current)
            .map_or_else(|| "choose…".to_owned(), Clone::clone);
        egui::ComboBox::from_id_salt(id)
            .selected_text(selected)
            .show_ui(ui, |ui| {
                for (index, choice) in choices.iter().enumerate() {
                    ui.selectable_value(current, index, choice);
                }
            });
    });
}

/// One extra 3D window: the same loaded scene through its own camera, on its own targets. A
/// window draws the model and navigates; selection and tools stay in the main viewport.
pub(crate) struct ViewWindow {
    pub(crate) camera: Camera,
    pub(crate) open: bool,
    title: String,
    renderer: Option<crate::render::Renderer>,
    texture: Option<egui::TextureId>,
}

/// Extra windows per model; the main viewport is the first window and not counted here.
pub(crate) const MAX_WINDOWS: usize = 3;

impl ViewWindow {
    pub(crate) fn new(camera: Camera, index: usize) -> Self {
        Self {
            camera,
            open: true,
            title: format!("View {}", index + 2),
            renderer: None,
            texture: None,
        }
    }

    pub(crate) fn title(&self) -> &str {
        &self.title
    }

    /// Resizes or creates the offscreen targets, keeping the egui texture in step. Returns the
    /// aspect of the drawn image.
    pub(crate) fn resize(
        &mut self,
        state: &eframe::egui_wgpu::RenderState,
        width: u32,
        height: u32,
        samples: Option<u32>,
        stats: &crate::stats::Stats,
    ) -> Result<f32, crate::Error> {
        if width == 0 || height == 0 {
            // egui reports a zero-size content rect before the window lays out; wait for real
            // pixels rather than building empty targets.
            return Err(crate::Error::Input("window not laid out yet"));
        }
        if let Some(renderer) = &mut self.renderer {
            if (renderer.width != width || renderer.height != height)
                && renderer.resize(&state.device, width, height, stats)?
            {
                // The targets changed: egui's copy of the texture follows them.
                if let Some(texture) = self.texture {
                    state
                        .renderer
                        .write()
                        .update_egui_texture_from_wgpu_texture(
                            &state.device,
                            &renderer.view,
                            wgpu::FilterMode::Linear,
                            texture,
                        );
                }
            }
        } else {
            let renderer = crate::render::Renderer::with_samples(
                &state.adapter,
                &state.device,
                width,
                height,
                samples,
                stats,
            )?;
            let texture = state.renderer.write().register_native_texture(
                &state.device,
                &renderer.view,
                wgpu::FilterMode::Linear,
            );
            self.texture = Some(texture);
            self.renderer = Some(renderer);
        }
        let renderer = self.renderer.as_ref().ok_or(crate::Error::NoAdapter)?;
        Ok((renderer.width as f32 / renderer.height as f32).clamp(0.05, 20.0))
    }

    /// The egui texture of the drawn image, once targets exist.
    pub(crate) fn texture(&self) -> Option<egui::TextureId> {
        self.texture
    }

    pub(crate) fn renderer(&self) -> Option<&crate::render::Renderer> {
        self.renderer.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use beamcraft_engine::views::NamedView;

    fn pose() -> beamcraft_engine::views::ViewPose {
        beamcraft_engine::views::ViewPose {
            target_mm: [0.0; 3],
            yaw: 0.5,
            pitch: 0.4,
            distance_mm: 1000.0,
            orthographic: false,
        }
    }

    fn camera() -> Camera {
        Camera::new([0.0; 3], 1000.0, 1.6).unwrap()
    }

    #[test]
    fn the_sidecar_round_trips_and_reports_unreadable_files() {
        let dir = std::env::temp_dir().join(format!("beamcraft-views-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = dir.join("m.store");
        std::fs::create_dir_all(&store).unwrap();
        assert!(load(&store).unwrap().is_empty());
        assert_eq!(session_path(&store), dir.join("m.store.views.json"));
        let mut book = ViewBook::new();
        book.save(
            NamedView::new(
                "Plan",
                Some(PlaneSpec::Level),
                pose(),
                Some(WorkArea::new([0.0; 3], [10.0; 3]).unwrap()),
            )
            .unwrap(),
        )
        .unwrap();
        persist(&store, &book).unwrap();
        assert_eq!(load(&store).unwrap(), book);
        // A corrupt file is a note, not an empty book silently.
        std::fs::write(session_path(&store), b"{not json").unwrap();
        assert!(load(&store).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_panel_builds_plane_specs_and_work_areas_only_from_explicit_actions() {
        let mut panel = Panel::default();
        let camera = camera();
        let book = ViewBook::new();
        let grids = GridScene::default();
        let ctx = egui::Context::default();
        let mut action = None;
        let mut labels: Vec<(String, egui::Pos2)> = Vec::new();
        let frame = |panel: &mut Panel,
                     events: Vec<egui::Event>,
                     action: &mut Option<Action>,
                     labels: &mut Vec<(String, egui::Pos2)>| {
            let output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 600.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    *action = panel.show(ui, &book, &camera, &grids).ok().flatten();
                },
            );
            *labels = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => Some((
                        text.galley.text().to_owned(),
                        text.pos + text.galley.size() / 2.0,
                    )),
                    _ => None,
                })
                .collect();
            output.drop_without_applying_deltas();
        };
        let click = |pos: egui::Pos2| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        // The empty scene has no grids: a grid-line save refuses with a note, level resolves.
        panel.plane = Plane::GridLine;
        assert!(panel.typed_plane(&grids).is_err());
        panel.plane = Plane::Level;
        frame(&mut panel, vec![], &mut action, &mut labels);
        let save_plane = labels
            .iter()
            .find(|(text, _)| text == "Save plane view")
            .map(|(_, pos)| *pos)
            .unwrap_or_else(|| panic!("the plane save button was not drawn: {labels:?}"));
        frame(&mut panel, click(save_plane), &mut action, &mut labels);
        assert_eq!(action, Some(Action::SavePlane(PlaneSpec::Level)));
        // Two points build a spec; a broken point is a note.
        panel.plane = Plane::TwoPoints;
        panel.from = ["0".into(), "0".into(), "5000".into()];
        panel.to = ["4000".into(), "0".into(), "0".into()];
        assert!(matches!(
            panel.typed_plane(&grids),
            Ok(PlaneSpec::TwoPoints { .. })
        ));
        panel.to = ["x".into(), "0".into(), "0".into()];
        assert!(panel.typed_plane(&grids).is_err());
        // The work area is data only until Set, and all-empty means none.
        assert_eq!(panel.typed_work_area().unwrap(), None);
        panel.set_work_area(Some(&WorkArea::new([1.0; 3], [9.0; 3]).unwrap()));
        assert_eq!(
            panel.typed_work_area().unwrap(),
            Some(WorkArea::new([1.0; 3], [9.0; 3]).unwrap())
        );
        panel.work_max[0] = "oops".into();
        assert!(panel.typed_work_area().is_err());
        panel.set_work_area(None);
        assert_eq!(panel.typed_work_area().unwrap(), None);
        // A cleared work area never produces a Set action.
        frame(&mut panel, vec![], &mut action, &mut labels);
        let set = labels
            .iter()
            .find(|(text, _)| text == "Set work area")
            .map(|(_, pos)| *pos)
            .unwrap_or_else(|| panic!("the set button was not drawn"));
        frame(&mut panel, click(set), &mut action, &mut labels);
        assert_eq!(action, None);
        // With numbers in the fields, Set carries the box.
        panel.work_min = ["0".into(), "0".into(), "0".into()];
        panel.work_max = ["10".into(), "20".into(), "30".into()];
        frame(&mut panel, click(set), &mut action, &mut labels);
        assert_eq!(
            action,
            Some(Action::SetWorkArea(
                WorkArea::new([0.0; 3], [10.0, 20.0, 30.0]).unwrap()
            ))
        );
    }
}
