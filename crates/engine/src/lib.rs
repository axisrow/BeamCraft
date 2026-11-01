// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The BeamCraft engine API. V02 adds the renderer-agnostic view snapshot and query contract;
//! P01 adds `CreatePlate` application (`steel::engine::command` and `steel::engine::transaction` in the steel crate, and [`footprint`]) and the read
//! side over committed plates (`steel::engine::committed`).
//!
//! The crate holds pure, bounded, deterministic data and no UI, wgpu, egui, I/O or clock. A
//! renderer receives meshes by digest, instance and triangle tables, ids and bounds; it computes
//! no engineering geometry. P01 implements the first command only (a rectangular plate); keys,
//! the other commands and personal workspaces are not implemented.

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
