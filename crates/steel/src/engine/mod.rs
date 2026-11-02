// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Structural steel in the engine: plates, members, cuts, corners, bends, lofted plates, holes,
//! fabrication, welds, bolts and fasteners, and the synthetic scenes built from them.
//!
//! The rest of this crate (queries, scenes, drawings, views, filters, numbering, schedules,
//! transforms, grids, levels, assemblies, attributes, references) is shared by every
//! discipline. `docs/design/platform-and-steel.md` says how this module becomes a crate of its
//! own, and which platform files still reach into it.

pub mod bend;
pub mod bolt;
pub mod command;
pub mod committed;
pub mod contour_edit;
pub mod copy;
pub mod corners;
pub mod cut;
pub mod fabricate;
pub mod fastener;
pub mod instance;
pub mod loft_plate;
pub mod member;
pub mod member_hole;
pub mod numbering_geometry;
pub mod part;
pub mod plate_draft;
pub mod quantity;
pub mod snap;
pub mod solid_mesh;
pub mod transaction;
pub mod transform;
pub mod update;
pub mod weld;
