// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The BeamCraft engine API. V02 adds the renderer-agnostic view snapshot and query contract;
//! P01 adds `CreatePlate` application ([`command`], [`footprint`], [`transaction`]) and the read
//! side over committed plates ([`query::committed`]).
//!
//! The crate holds pure, bounded, deterministic data and no UI, wgpu, egui, I/O or clock. A
//! renderer receives meshes by digest, instance and triangle tables, ids and bounds; it computes
//! no engineering geometry. P01 implements the first command only (a rectangular plate); keys,
//! the other commands and personal workspaces are not implemented.

pub mod assembly;
pub mod attribute;
pub mod bolt;
pub mod bom;
pub mod class_palette;
pub mod command;
pub mod contour_edit;
pub mod coordinate;
pub mod copy;
pub mod corners;
pub mod cut;
pub mod drawing;
pub mod element;
pub mod error;
pub mod fabricate;
pub mod footprint;
pub mod grid;
pub mod grid_edit;
pub mod grid_geometry;
pub mod grid_query;
pub mod grid_update;
pub mod instance;
pub mod level;
pub mod limits;
pub mod measure;
pub mod member;
pub mod numbering;
pub mod numbering_approval;
pub mod plate_draft;
pub mod presentation;
pub mod quantity;
pub mod query;
pub mod rebase;
pub mod reference;
pub mod scene;
pub mod solid;
pub mod synthetic;
pub mod transaction;
pub mod update;
pub mod weld;
pub mod work_plane;

pub use beamcraft_kernel::PlateResult;
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
