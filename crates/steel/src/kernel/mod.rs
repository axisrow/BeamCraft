// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Structural steel bodies: rectangular and contoured plates, straight members from a steel
//! section, ruled and lofted plates, fasteners and member holes.
//!
//! The rest of this crate (exact arithmetic, topology, meshes, profiles, sweeps, plane cuts and
//! the seal) is geometry for any discipline. `docs/design/platform-and-steel.md` says how this
//! module becomes a crate of its own.

pub mod fastener;
pub mod member;
pub mod member_hole;
pub mod plate_contour;
pub mod ruled;
pub mod section;

pub use member::{StraightMember, straight_member};
pub mod sweep;
