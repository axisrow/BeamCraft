// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Structural steel: the discipline for plates, members, holes, bends, welds, bolts and
//! fasteners.
//!
//! The crate has four parts, one per layer it extends:
//!
//! - [`model`]: the commands and their admission, section families, fastener and profile
//!   definitions, and part instances;
//! - [`kernel`]: the steel bodies (plate contours, members from a section, ruled plates,
//!   fasteners, member holes, sweeps);
//! - [`engine`]: the plate and member engines, cuts, corners, bends, fabrication, welds, bolts,
//!   edits, copies, quantities and the committed-plate reader;
//! - [`layer`]: the steel half of the authored layer — the discipline to install with
//!   [`layer::install`], the shapes, changes and definitions its hooks carry, and the readers
//!   of the steel content of a stored layer.
//!
//! Everything a second discipline would also need (identities, digests, error codes, grids,
//! levels, assemblies, queries, scenes, drawings, views, filters, schedules) stays in the
//! platform crates, which do not depend on this one. `docs/design/platform-and-steel.md` says
//! how the work was separated and what remains.

pub mod engine;
pub mod kernel;
pub mod layer;
pub mod model;
