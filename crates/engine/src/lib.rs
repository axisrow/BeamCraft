// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The BeamCraft engine API: the platform half of the engine that every discipline shares.
//!
//! It holds the renderer-agnostic view snapshot and its queries (bounds, changes, selection,
//! picking and features, snapping input), footprints, grids, levels, work planes, assemblies,
//! attributes, references, numbering, drawings, views, filters and schedules. The steel commands
//! (plates, members, holes, cuts, bends, welds, bolts, copies and edits) and their transactions
//! live in the `steel` discipline crate (`steel::engine`), which builds on this one (#795);
//! personal workspaces, publication and undo live with the host and the application layer.
//!
//! The crate holds pure, bounded, deterministic data and no UI, wgpu, egui, I/O or clock. A
//! renderer receives meshes by digest, instance and triangle tables, ids and bounds; it computes
//! no engineering geometry.

pub mod annotation;
pub mod assembly;
pub mod attribute;
pub mod bom;
pub mod class_palette;
pub mod construction;
pub mod coordinate;
pub mod drawing;
pub mod element;
pub mod error;
pub mod filter;
pub mod footprint;
pub mod grid;
pub mod grid_edit;
pub mod grid_geometry;
pub mod grid_query;
pub mod grid_update;
pub mod level;
pub mod limits;
pub mod measure;
pub mod numbering;
pub mod numbering_approval;
pub mod presentation;
pub mod query;
pub mod rebase;
pub mod reference;
pub mod scene;
pub mod scope;
pub mod solid;
pub mod synthetic;
pub mod view_clipping;
pub mod views;
pub mod work_plane;

pub use element::{ElementId, ElementKind, KindFilter};
pub use error::{ViewCode, ViewError};
pub use limits::Limits;
pub use presentation::{
    ProjectedElement, RenderMode, Representation, ViewProjection, ViewSettings,
};
pub use query::bounds::Aabb;
pub use query::changes::{Change, ChangeKind, ChangeSet, ChangedFields, SnapshotHistory, diff};
pub use query::mesh::MeshEntry;
pub use query::selection::Selection;
pub use query::snapshot::{
    ElementEntry, ElementPage, RepresentationRef, SnapshotToken, ViewPin, ViewSnapshot,
};
pub use scene::SceneElement;

/// Schedules by attributes (I05).
pub mod schedule;
