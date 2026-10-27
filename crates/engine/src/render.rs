// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! V16 pure presentation selection and pipeline settings. No engineering state is changed.
use crate::{
    error::{ViewCode, ViewError},
    query::{selection::Selection, snapshot::ViewSnapshot},
};
use beamcraft_model::id::ElementId;
use std::collections::BTreeSet;
/// Closed display modes; selection remains explicit viewport state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Edges without surfaces.
    Wireframe,
    /// Lit surfaces with baseline transparency 0.65.
    Transparent,
    /// Uniform grey unlit surfaces.
    Grey,
    /// Lit surfaces.
    #[default]
    Shaded,
    /// Lit surfaces restricted to explicit selection.
    SelectedOnly,
}
impl Mode {
    /// Stable external token.
    pub const fn token(self) -> &'static str {
        match self {
            Self::Wireframe => "wireframe",
            Self::Transparent => "transparent",
            Self::Grey => "grey",
            Self::Shaded => "shaded",
            Self::SelectedOnly => "selected-only",
        }
    }
    /// Parse a closed external token.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "wireframe" => Some(Self::Wireframe),
            "transparent" => Some(Self::Transparent),
            "grey" => Some(Self::Grey),
            "shaded" => Some(Self::Shaded),
            "selected-only" => Some(Self::SelectedOnly),
            _ => None,
        }
    }
}
/// Bounded result for any consumer, including the CLI bundle transport.
#[derive(Debug, Clone, PartialEq)]
pub struct Presentation {
    /// Admitted mode.
    pub mode: Mode,
    /// Render triangle surfaces.
    pub surfaces: bool,
    /// Render edges.
    pub edges: bool,
    /// Apply lighting.
    pub lit: bool,
    /// Base RGB in [0,1].
    pub color: [f64; 3],
    /// Base transparency in [0,1], zero means opaque.
    pub transparency: f64,
    /// Explicit selection, including elements hidden by filters/clipping.
    pub selected: Vec<ElementId>,
    /// Visible identities in ascending order.
    pub visible: Vec<ElementId>,
}
/// Intersect a bounded visibility mask with the selected-only mode. Unknown IDs are errors;
/// repeated selection IDs collapse according to the existing Selection API.
pub fn query(
    snapshot: &ViewSnapshot,
    mode: Mode,
    selected: &[ElementId],
    candidates: &[ElementId],
) -> Result<Presentation, ViewError> {
    let selection = Selection::from_ids(snapshot, selected)?;
    if candidates.len() > snapshot.limits().max_selection() {
        return Err(ViewError::new(ViewCode::SelectionLimit));
    }
    let mut visible = BTreeSet::new();
    for id in candidates {
        if snapshot.element(id).is_none() {
            return Err(ViewError::new(ViewCode::UnknownElement));
        }
        if mode != Mode::SelectedOnly || selection.contains(id) {
            visible.insert(*id);
        }
    }
    Ok(Presentation {
        mode,
        surfaces: mode != Mode::Wireframe,
        edges: mode == Mode::Wireframe,
        lit: !matches!(mode, Mode::Grey | Mode::Wireframe),
        color: [0.65; 3],
        transparency: if mode == Mode::Transparent { 0.65 } else { 0.0 },
        selected: selection.ids().copied().collect(),
        visible: visible.into_iter().collect(),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mode_tokens_are_closed_and_roundtrip() {
        for m in [
            Mode::Wireframe,
            Mode::Transparent,
            Mode::Grey,
            Mode::Shaded,
            Mode::SelectedOnly,
        ] {
            assert_eq!(Mode::parse(m.token()), Some(m));
        }
        assert_eq!(Mode::parse("solid"), None);
    }
}
