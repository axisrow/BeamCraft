// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Pinned read queries (F06 section 9): snapshot, bounds, mesh blobs, deltas and selection.
//!
//! Implemented: `ListElements`, `GetMeshSnapshot` (blobs by digest), `QueryBounds` (element and
//! all, world space), `GetChanges`, the selection API, and (V04) FeatureIds, `LookupFeature`,
//! `Pick` and snaps for boxes and semantically named contour, operation and profile bodies.
//! Local-space bounds and dependency keys are separate work. [`committed`] (P01) turns the
//! authored plates of a committed revision into the snapshot builder's input.

pub mod bounds;
pub mod bundle;
pub mod changes;
pub mod committed;
pub mod feature;
pub mod mesh;
pub mod pick;
pub mod selection;
pub mod snapshot;
