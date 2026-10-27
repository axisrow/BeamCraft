// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! View settings and the read-time projection of a snapshot.
//!
//! View settings choose what a viewport loads and draws, not what exists. They are user or
//! workspace state: they never enter the snapshot token, change no authored or derived data and
//! never reach engineering code. The simplified representation is flagged not exact and feeds no
//! volume, mass, bill of materials or snapping; those read
//! [`ViewSnapshot::engineering_representation`] only.
//!
//! The F06 section 7 presentation-key preimage (appearance, visibility, camera) is not
//! implemented here; V01/V03 own it. This module only supplies the visibility and
//! representation inputs of that key.

use crate::element::{ElementId, ElementKind, KindFilter};
use crate::error::{ViewCode, ViewError};
use crate::query::bounds::Aabb;
use crate::query::snapshot::{SnapshotToken, ViewSnapshot};
use beamcraft_model::digest::ContentDigest;

/// Which geometry of a part a view loads. An enumeration, so coarser levels can be added.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum Representation {
    /// Operations applied; exact; the default geometry of the revision.
    #[default]
    Full,
    /// Lightweight stand-in without operations; not exact unless it equals the full mesh.
    Simplified,
}

impl Representation {
    /// Every representation.
    pub const ALL: [Self; 2] = [Self::Full, Self::Simplified];

    /// The stable token.
    pub const fn as_token(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Simplified => "simplified",
        }
    }

    /// Parses an exact token.
    pub fn parse(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.as_token() == token)
    }

    /// Whether engineering quantities and snapping may read this representation.
    pub const fn feeds_engineering(self) -> bool {
        matches!(self, Self::Full)
    }
}

/// How a viewport displays the elements it has loaded (V16). A display style of the window, never
/// model state: switching a mode changes no snapshot token, no authored or derived data and no
/// engineering quantity. One query drives every mode: the renderer keeps one loaded scene and
/// picks the drawing style, and the selected-only mode reads the one element it needs from
/// [`ViewSnapshot::element`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum RenderMode {
    /// Triangle edges only, no filled faces.
    Wireframe,
    /// Filled faces at partial opacity, so hidden geometry shows through.
    Transparent,
    /// Filled, lit faces in one neutral grey without the part colours.
    Grey,
    /// The default: filled, lit faces in the part colours.
    #[default]
    Shaded,
    /// Only the selected element is drawn; nothing, when nothing is selected.
    SelectedOnly,
}

impl RenderMode {
    /// Every mode, in the `Ctrl+1` to `Ctrl+5` order of the default shortcuts.
    pub const ALL: [Self; 5] = [
        Self::Wireframe,
        Self::Transparent,
        Self::Grey,
        Self::Shaded,
        Self::SelectedOnly,
    ];

    /// The stable token, also the name of a `--perf-run` measurement phase.
    pub const fn as_token(self) -> &'static str {
        match self {
            Self::Wireframe => "wireframe",
            Self::Transparent => "transparent",
            Self::Grey => "grey",
            Self::Shaded => "shaded",
            Self::SelectedOnly => "selected",
        }
    }

    /// Parses an exact token.
    pub fn parse(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.as_token() == token)
    }
}

/// The two view parameters V02 carries: an element-kind filter and a representation selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ViewSettings {
    /// Kinds hidden from the view.
    pub hidden_kinds: KindFilter,
    /// Representation loaded for every shown part.
    pub representation: Representation,
}

/// One drawn element of a projection.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ProjectedElement {
    /// The identity; the only thing a consumer may persist or send back.
    pub id: ElementId,
    /// Snapshot-local element index; valid only within this snapshot.
    pub element_index: u32,
    /// Position in this projection, the instance slot of an instanced draw.
    pub instance_index: u32,
    /// The representation drawn.
    pub representation: Representation,
    /// Whether the drawn mesh is exact geometry.
    pub exact: bool,
    /// Slot of the mesh in [`ViewSnapshot::meshes`].
    pub mesh_slot: u32,
    /// Digest of the mesh drawn.
    pub mesh_digest: ContentDigest,
    /// First triangle of this element in a merged buffer built in projection order.
    pub first_triangle: u32,
    /// Triangles of the mesh drawn.
    pub triangle_count: u32,
    /// World bounds of the drawn representation.
    pub world_bounds: Aabb,
    /// Translation placement in millimetres.
    pub placement_mm: [f64; 3],
    /// Rigid orientation in quaternion `(x, y, z, w)` form.
    pub orientation_xyzw: [f64; 4],
}

/// A snapshot seen through view settings. Instance and triangle indices are local to it.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewProjection<'a> {
    snapshot: &'a ViewSnapshot,
    settings: ViewSettings,
    items: Vec<ProjectedElement>,
    total_triangles: u32,
    bounds: Option<Aabb>,
}

impl ViewSnapshot {
    /// Applies view settings as a read-time projection; the snapshot and its token are unchanged.
    ///
    /// Fails with `GEO_BUFFER_LIMIT` when the shown triangles exceed the snapshot's limit.
    pub fn project(&self, settings: &ViewSettings) -> Result<ViewProjection<'_>, ViewError> {
        let mut items = Vec::new();
        let mut running = 0u32;
        let mut bounds: Option<Aabb> = None;
        for entry in self.elements() {
            if !settings.hidden_kinds.is_visible(entry.kind) {
                continue;
            }
            let rep = entry.representation(settings.representation);
            let triangle_count = self
                .mesh_at(rep.mesh_slot)
                .ok_or(ViewError::new(ViewCode::BlobInvalid))?
                .triangle_count();
            let first_triangle = running;
            running = running
                .checked_add(triangle_count)
                .filter(|n| *n <= self.limits().max_projected_triangles())
                .ok_or(ViewError::new(ViewCode::BufferLimit))?;
            bounds = Some(bounds.map_or(rep.world_bounds, |b| b.union(rep.world_bounds)));
            items.push(ProjectedElement {
                id: entry.id,
                element_index: entry.index,
                instance_index: u32::try_from(items.len())
                    .map_err(|_| ViewError::new(ViewCode::ElementLimit))?,
                representation: settings.representation,
                exact: rep.exact,
                mesh_slot: rep.mesh_slot,
                mesh_digest: rep.mesh_digest,
                first_triangle,
                triangle_count,
                world_bounds: rep.world_bounds,
                placement_mm: entry.placement_mm,
                orientation_xyzw: entry.orientation_xyzw,
            });
        }
        Ok(ViewProjection {
            snapshot: self,
            settings: *settings,
            items,
            total_triangles: running,
            bounds,
        })
    }
}

impl<'a> ViewProjection<'a> {
    /// The snapshot this projection reads.
    pub const fn snapshot(&self) -> &'a ViewSnapshot {
        self.snapshot
    }

    /// The token of the snapshot, unchanged by the settings.
    pub const fn token(&self) -> &'a SnapshotToken {
        self.snapshot.token()
    }

    /// The settings applied.
    pub const fn settings(&self) -> ViewSettings {
        self.settings
    }

    /// The shown elements in ascending id order; `instance_index` is the position.
    pub fn items(&self) -> &[ProjectedElement] {
        &self.items
    }

    /// Number of shown elements.
    pub const fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether nothing is shown.
    pub const fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Elements of the snapshot hidden by the kind filter.
    pub const fn hidden_count(&self) -> usize {
        self.snapshot.len().saturating_sub(self.items.len())
    }

    /// Triangles of the merged buffer of the shown elements.
    pub const fn total_triangles(&self) -> u32 {
        self.total_triangles
    }

    /// Display bounds of what is shown; `None` when nothing is. Not an engineering quantity.
    pub const fn bounds(&self) -> Option<Aabb> {
        self.bounds
    }

    /// Whether every shown mesh is exact geometry.
    pub fn all_exact(&self) -> bool {
        self.items.iter().all(|p| p.exact)
    }

    /// The distinct mesh digests this view must load, ascending. Hidden elements and the
    /// representation not selected contribute nothing.
    pub fn required_meshes(&self) -> Vec<ContentDigest> {
        let mut slots: Vec<u32> = self.items.iter().map(|p| p.mesh_slot).collect();
        slots.sort_unstable();
        slots.dedup();
        slots
            .into_iter()
            .filter_map(|s| self.snapshot.mesh_at(s).map(|m| m.digest()))
            .collect()
    }

    /// Bytes of [`Self::required_meshes`]; `None` only if the sum overflows.
    pub fn required_mesh_bytes(&self) -> Option<usize> {
        self.required_meshes()
            .iter()
            .filter_map(|d| self.snapshot.mesh(d))
            .try_fold(0usize, |sum, m| sum.checked_add(m.bytes().len()))
    }

    /// The instance slot of a shown element.
    pub fn instance_of(&self, id: &ElementId) -> Option<u32> {
        self.items
            .binary_search_by(|p| p.id.cmp(id))
            .ok()
            .and_then(|i| self.items.get(i))
            .map(|p| p.instance_index)
    }

    /// The shown element of an instance slot. An unknown slot is `VIEW_ARGUMENT_INVALID`.
    pub fn element_for_instance(&self, instance: u32) -> Result<ElementId, ViewError> {
        usize::try_from(instance)
            .ok()
            .and_then(|i| self.items.get(i))
            .map(|p| p.id)
            .ok_or(ViewError::new(ViewCode::ArgumentInvalid))
    }

    /// The shown element owning a triangle of the merged buffer (a display-pick hint made
    /// engineering-meaningful only through this table). Out of range is `VIEW_ARGUMENT_INVALID`.
    pub fn element_for_triangle(&self, triangle: u32) -> Result<ElementId, ViewError> {
        let after = self.items.partition_point(|p| p.first_triangle <= triangle);
        after
            .checked_sub(1)
            .and_then(|i| self.items.get(i))
            .filter(|p| triangle - p.first_triangle < p.triangle_count)
            .map(|p| p.id)
            .ok_or(ViewError::new(ViewCode::ArgumentInvalid))
    }

    /// Whether a kind is shown by the current settings.
    pub const fn shows(&self, kind: ElementKind) -> bool {
        self.settings.hidden_kinds.is_visible(kind)
    }
}

#[cfg(test)]
mod render_mode_tests {
    use super::*;

    #[test]
    fn modes_round_trip_through_tokens_and_have_the_default() {
        for mode in RenderMode::ALL {
            assert_eq!(RenderMode::parse(mode.as_token()), Some(mode));
        }
        assert_eq!(RenderMode::parse("shaded"), Some(RenderMode::Shaded));
        assert_eq!(RenderMode::parse("wireframe"), Some(RenderMode::Wireframe));
        assert_eq!(
            RenderMode::parse("selected"),
            Some(RenderMode::SelectedOnly)
        );
        assert_eq!(RenderMode::parse("secret"), None);
        assert_eq!(RenderMode::default(), RenderMode::Shaded);
        assert_eq!(
            RenderMode::ALL,
            [
                RenderMode::Wireframe,
                RenderMode::Transparent,
                RenderMode::Grey,
                RenderMode::Shaded,
                RenderMode::SelectedOnly,
            ]
        );
    }
}
