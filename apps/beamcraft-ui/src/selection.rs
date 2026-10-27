// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! V11 selection switches and snap panel, as pure session state: no window, no GPU, no store.
//!
//! The eight switches (parts, assemblies, bolts, welds, holes, grids, construction objects,
//! items) are the engine's [`SelectionFilter`]: a class that is off is left out of the pick
//! queries the window runs. The acceptance is that a switch changes picking only — never the
//! model or the snapshot token — which holds by construction here: the state is consulted when
//! a pick is asked and read by nothing else. The snap panel stores which [`SnapKind`]s are
//! active for the engine's snap query (the engine and CLI surface is V04 and T01.4); the tools
//! that resolve click points through it arrive with T01.6. Nothing here is persisted: the
//! switches are view state, like the camera.

use beamcraft_engine::query::pick::{SelectionClass, SelectionFilter, SnapKind};

/// The state of the bottom bar: the eight selection switches and the active snap kinds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    /// Which classes of objects a pick may select.
    switches: SelectionFilter,
    /// Which snap kinds are active, in [`SnapKind::ALL`] order.
    snap: [bool; SnapKind::ALL.len()],
    /// Whether the snap panel is open. Display state of the panel itself.
    pub snap_open: bool,
}

impl Default for Selection {
    /// Every switch on; the snap kinds the engine's own `snap` default names (vertex, axis end
    /// and axis), grid kinds off until the grid tools ask for them.
    fn default() -> Self {
        Self {
            switches: SelectionFilter::ALL,
            snap: [
                true,  // vertex
                true,  // axis_end
                true,  // axis
                false, // grid_intersection
                false, // grid_end
                false, // grid_line
            ],
            snap_open: false,
        }
    }
}

impl Selection {
    /// Whether the `class` switch is on.
    pub fn is_on(&self, class: SelectionClass) -> bool {
        self.switches.allows_class(class)
    }

    /// Sets the `class` switch. Nothing but the next picks observes this.
    pub fn set(&mut self, class: SelectionClass, on: bool) {
        self.switches = self.switches.with(class, on);
    }

    /// The filter the pick queries of this window carry.
    pub fn filter(&self) -> SelectionFilter {
        self.switches
    }

    /// Whether the snap kind is active.
    pub fn snap_on(&self, kind: SnapKind) -> bool {
        self.snap.get(kind as usize).copied().unwrap_or(false)
    }

    /// Sets the snap kind.
    pub fn set_snap(&mut self, kind: SnapKind, on: bool) {
        if let Some(slot) = self.snap.get_mut(kind as usize) {
            *slot = on;
        }
    }

    /// The active snap kinds, in engine order; empty means the query asks for nothing. At
    /// least one kind stays on in the panel: with none, snapping is simply off.
    pub fn snap_kinds(&self) -> Vec<SnapKind> {
        SnapKind::ALL
            .into_iter()
            .filter(|k| self.snap_on(*k))
            .collect()
    }

    /// The bar label of a switch: short, so eight of them fit the status bar.
    pub const fn label(class: SelectionClass) -> &'static str {
        match class {
            SelectionClass::Parts => "Parts",
            SelectionClass::Assemblies => "Assy",
            SelectionClass::Bolts => "Bolts",
            SelectionClass::Welds => "Welds",
            SelectionClass::Holes => "Holes",
            SelectionClass::Grids => "Grids",
            SelectionClass::ConstructionObjects => "Constr",
            SelectionClass::Items => "Items",
        }
    }

    /// What a switch selects, for its tooltip.
    pub const fn tip(class: SelectionClass) -> &'static str {
        match class {
            SelectionClass::Parts => "Select parts (plates, beams, columns)",
            SelectionClass::Assemblies => "Select assemblies and connections",
            SelectionClass::Bolts => "Select bolts",
            SelectionClass::Welds => "Select welds",
            SelectionClass::Holes => "Select holes",
            SelectionClass::Grids => "Select grids",
            SelectionClass::ConstructionObjects => "Select construction objects",
            SelectionClass::Items => "Select items",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_switch_starts_on_and_toggling_changes_only_the_filter() {
        let mut selection = Selection::default();
        for class in SelectionClass::ALL {
            assert!(selection.is_on(class), "{:?}", class);
        }
        assert_eq!(selection.filter(), SelectionFilter::ALL);
        selection.set(SelectionClass::Parts, false);
        selection.set(SelectionClass::Grids, false);
        assert!(!selection.is_on(SelectionClass::Parts));
        assert!(!selection.is_on(SelectionClass::Grids));
        assert!(selection.is_on(SelectionClass::Bolts));
        assert_eq!(
            selection.filter().off_classes(),
            vec![SelectionClass::Parts, SelectionClass::Grids]
        );
        selection.set(SelectionClass::Parts, true);
        assert!(selection.is_on(SelectionClass::Parts));
    }

    #[test]
    fn the_snap_panel_starts_at_the_engine_default_and_keeps_kinds_in_engine_order() {
        let selection = Selection::default();
        assert_eq!(
            selection.snap_kinds(),
            vec![SnapKind::Vertex, SnapKind::AxisEnd, SnapKind::Axis]
        );
        let mut selection = selection;
        selection.set_snap(SnapKind::GridIntersection, true);
        selection.set_snap(SnapKind::Vertex, false);
        assert_eq!(
            selection.snap_kinds(),
            vec![
                SnapKind::AxisEnd,
                SnapKind::Axis,
                SnapKind::GridIntersection
            ]
        );
        for kind in SnapKind::ALL {
            selection.set_snap(kind, false);
        }
        assert!(selection.snap_kinds().is_empty());
        assert!(!selection.snap_on(SnapKind::Vertex));
    }

    #[test]
    fn every_switch_has_a_bar_label_and_a_tooltip() {
        for class in SelectionClass::ALL {
            assert!(!Selection::label(class).is_empty());
            assert!(Selection::tip(class).starts_with("Select "));
        }
        assert_eq!(Selection::label(SelectionClass::Parts), "Parts");
    }
}
