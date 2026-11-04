// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! V07 discipline view presets: a bounded book of named bundles of view display state, each
//! composing existing primitives only — the element-kind filter (whose `reference` kind is the
//! reference visibility), the view depth slab above and below the plane (the underlay), the
//! representation level (U12), the name of a saved colour set (V17) and the display option
//! toggles (grid guides). A preset holds no camera, no work area, no provenance and no model
//! revision: applying one to a view changes only that view's display bundle, so the model and
//! the snapshot token never move.

use crate::element::KindFilter;
use crate::presentation::Representation;
use crate::views::{MAX_DEPTH_MM, ModelView};
use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;

/// Most presets one book admits; more is `LIMIT_EXCEEDED`.
pub const MAX_PRESETS: usize = 64;
/// Most bytes in a preset name and in a named colour set, matching the colour-set file rules.
pub const MAX_NAME_BYTES: usize = 64;

fn invalid(field: &'static str, message: &'static str) -> CommandError {
    CommandError::new(ErrorCode::InvalidArgument, field, message)
}

fn not_found(field: &'static str, message: &'static str) -> CommandError {
    CommandError::new(ErrorCode::NotFound, field, message)
}

/// One named discipline preset: the display bundle a switcher applies to a view, in model
/// millimetres. Every field is view display state; nothing here reads or writes a store.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewPreset {
    /// User-visible name, unique in the book (case-insensitively), 1 to [`MAX_NAME_BYTES`]
    /// bytes without controls or outer spaces.
    pub name: String,
    /// Element kinds the view hides: the kind filter, whose `reference` kind is the visibility
    /// of read-only reference models.
    pub hidden_kinds: KindFilter,
    /// The underlay: visible distance `[above, below]` the view plane; `None` shows everything
    /// in front of and behind the plane.
    pub depth: Option<[f64; 2]>,
    /// Representation level (U12) the view loads for every shown part.
    pub representation: Representation,
    /// The saved colour set (V17) the view colours by, by name; `None` colours by the model.
    /// The name is resolved against the user's colour sets at display time.
    pub colour_set: Option<String>,
    /// Whether grid guides are displayed.
    pub grids_visible: bool,
}

impl ViewPreset {
    /// A preset that hides nothing, shows everything above and below the plane, loads the full
    /// representation, colours by the model and shows grid guides.
    pub fn new(name: &str) -> Self {
        Self {
            name: name.trim().to_owned(),
            hidden_kinds: KindFilter::SHOW_ALL,
            depth: None,
            representation: Representation::Full,
            colour_set: None,
            grids_visible: true,
        }
    }

    /// Admits the preset: a bounded printable name, a bounded depth slab and a bounded colour
    /// set name.
    pub fn validate(&self) -> Result<(), CommandError> {
        if self.name.trim() != self.name
            || self.name.is_empty()
            || self.name.len() > MAX_NAME_BYTES
            || self.name.chars().any(char::is_control)
        {
            return Err(invalid(
                "presets.name",
                "a preset name needs 1 to 64 bytes without outer spaces or controls",
            ));
        }
        if let Some([above, below]) = self.depth
            && (!above.is_finite()
                || !below.is_finite()
                || !(0.0..=MAX_DEPTH_MM).contains(&above)
                || !(0.0..=MAX_DEPTH_MM).contains(&below)
                || above + below <= 0.0)
        {
            return Err(invalid(
                "presets.depth",
                "the underlay needs bounded nonnegative distances and a positive thickness",
            ));
        }
        if let Some(set) = &self.colour_set
            && (set.is_empty() || set.len() > MAX_NAME_BYTES || set.chars().any(char::is_control))
        {
            return Err(invalid(
                "presets.colour_set",
                "a colour set name is 1 to 64 bytes without controls",
            ));
        }
        Ok(())
    }

    /// Applies the display bundle onto `view`: the kind filter, the depth slab, the
    /// representation, the colour set and the grid toggle, and nothing else — the camera, the
    /// span, the render mode, the colour and opacity, the folder, the favourite and the
    /// provenance stay. A preset is display state, so this can never touch a model.
    ///
    /// # Errors
    /// [`ErrorCode::InvalidArgument`] when the preset itself breaks its bounds.
    pub fn apply_to(&self, view: &mut ModelView) -> Result<(), CommandError> {
        self.validate()?;
        view.hidden_kinds = self.hidden_kinds;
        view.depth = self.depth;
        view.representation = self.representation;
        view.colour_set = self.colour_set.clone();
        view.grids_visible = self.grids_visible;
        Ok(())
    }
}

/// The named discipline presets of one user. Presets keep their definition order; names are
/// unique case-insensitively.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PresetBook {
    presets: Vec<ViewPreset>,
}

impl PresetBook {
    /// An empty book.
    pub fn new() -> Self {
        Self::default()
    }

    /// Admits a whole book: every preset, unique names and the preset limit.
    pub fn from_parts(presets: Vec<ViewPreset>) -> Result<Self, CommandError> {
        let mut book = Self::new();
        for preset in presets {
            book.insert(preset)?;
        }
        Ok(book)
    }

    /// The presets in definition order.
    pub fn presets(&self) -> &[ViewPreset] {
        &self.presets
    }

    /// How many presets the book holds.
    pub fn len(&self) -> usize {
        self.presets.len()
    }

    /// Whether the book holds no preset.
    pub fn is_empty(&self) -> bool {
        self.presets.is_empty()
    }

    /// The preset named exactly (case-insensitively).
    pub fn find(&self, name: &str) -> Option<&ViewPreset> {
        let wanted = name.trim().to_lowercase();
        self.presets
            .iter()
            .find(|p| p.name.to_lowercase() == wanted)
    }

    /// Adds a new preset; refuses a taken name and the preset limit.
    pub fn insert(&mut self, preset: ViewPreset) -> Result<(), CommandError> {
        preset.validate()?;
        if self.find(&preset.name).is_some() {
            return Err(CommandError::new(
                ErrorCode::DuplicateId,
                "presets.name",
                "preset names must be unique",
            ));
        }
        if self.presets.len() >= MAX_PRESETS {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "presets.count",
                "the user already has the maximum number of presets",
            ));
        }
        self.presets.push(preset);
        Ok(())
    }

    /// Removes the preset named by `selector`, returning it.
    pub fn remove(&mut self, selector: &str) -> Result<ViewPreset, CommandError> {
        let at = self
            .presets
            .iter()
            .position(|p| p.name.to_lowercase() == selector.trim().to_lowercase())
            .ok_or_else(|| not_found("presets.name", "PRESET_UNKNOWN"))?;
        Ok(self.presets.remove(at))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::element::ElementKind;
    use crate::views::WorkArea;
    use crate::work_plane::WorkPlane;
    use beamcraft_model::id::Uuid7;

    fn id(n: u64) -> Uuid7 {
        Uuid7::parse(&format!("019a7bf0-0000-7000-8000-{n:012}")).unwrap()
    }

    fn view(n: u64, name: &str) -> ModelView {
        ModelView::new(id(n), name, WorkPlane::default(), 5000.0)
    }

    fn preset(name: &str) -> ViewPreset {
        let mut p = ViewPreset::new(name);
        p.hidden_kinds = KindFilter::SHOW_ALL.hiding(ElementKind::Reference);
        p.depth = Some([0.0, 3000.0]);
        p.representation = Representation::Simplified;
        p.colour_set = Some("Part classes".into());
        p.grids_visible = false;
        p
    }

    #[test]
    fn presets_admit_their_bounds() {
        let mut p = ViewPreset::new("  Fab  ");
        assert_eq!(p.name, "Fab");
        p.validate().unwrap();
        p.name = String::new();
        assert!(p.validate().is_err());
        p.name = "a\nb".into();
        assert!(p.validate().is_err());
        p.name = "x".repeat(MAX_NAME_BYTES + 1);
        assert!(p.validate().is_err());
        p.name = "Fab".into();
        p.depth = Some([0.0, 0.0]);
        assert!(p.validate().is_err());
        p.depth = Some([100.0, 3000.0]);
        p.validate().unwrap();
        p.colour_set = Some(String::new());
        assert!(p.validate().is_err());
        p.colour_set = Some("a\tb".into());
        assert!(p.validate().is_err());
        p.colour_set = Some("x".repeat(MAX_NAME_BYTES + 1));
        assert!(p.validate().is_err());
        p.colour_set = None;
        p.validate().unwrap();
    }

    #[test]
    fn applying_changes_only_the_display_bundle() {
        let mut v = view(1, "Plan");
        v.projection = crate::views::Projection::Perspective;
        v.span_mm = 7000.0;
        v.render_mode = crate::RenderMode::Wireframe;
        v.folder = "Plans".into();
        v.favorite = true;
        v.work_area = Some(WorkArea::new([0.0; 3], [100.0; 3]).unwrap());
        v.opacity = 0.5;
        v.color = Some([0.2, 0.3, 0.4]);
        v.stamp_provenance("Ana", "capture", id(9), 7).unwrap();
        let before = v.clone();
        preset("Fab").apply_to(&mut v).unwrap();
        assert_eq!(
            v.hidden_kinds,
            KindFilter::SHOW_ALL.hiding(ElementKind::Reference)
        );
        assert_eq!(v.depth, Some([0.0, 3000.0]));
        assert_eq!(v.representation, Representation::Simplified);
        assert_eq!(v.colour_set.as_deref(), Some("Part classes"));
        assert!(!v.grids_visible);
        // The camera, the display styling and the provenance stay untouched.
        assert_eq!(v.projection, before.projection);
        assert_eq!(v.span_mm, before.span_mm);
        assert_eq!(v.render_mode, before.render_mode);
        assert_eq!(v.work_area, before.work_area);
        assert_eq!(v.opacity, before.opacity);
        assert_eq!(v.color, before.color);
        assert_eq!(v.folder, before.folder);
        assert_eq!(v.favorite, before.favorite);
        assert_eq!(v.provenance, before.provenance);
        assert_eq!(v.camera_plane().unwrap(), before.camera_plane().unwrap());
    }

    #[test]
    fn a_book_keeps_names_unique_and_bounded() {
        let mut book = PresetBook::new();
        book.insert(preset("Fabrication")).unwrap();
        assert_eq!(book.find("fabrication").unwrap().name, "Fabrication");
        assert!(book.find("Erection").is_none());
        assert!(
            book.insert(preset("FABRICATION")).is_err(),
            "names are case-insensitive"
        );
        assert_eq!(book.remove("fabrication").unwrap().name, "Fabrication");
        assert!(book.remove("Fabrication").is_err(), "the preset is gone");
        let mut full = PresetBook::new();
        let batch: Vec<ViewPreset> = (0..MAX_PRESETS)
            .map(|n| ViewPreset::new(&format!("p{n}")))
            .collect();
        for p in batch {
            full.insert(p).unwrap();
        }
        assert!(
            full.insert(ViewPreset::new("one more")).is_err(),
            "the book is bounded"
        );
        assert!(PresetBook::from_parts(vec![preset("a"), preset("A")]).is_err());
    }
}
