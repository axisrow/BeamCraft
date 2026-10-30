// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Bounded, headless analytic geometry. G02 builds origin rectangular plates;
//! the G05 [`profile`] module extrudes planar profiles with cavities; the G09 [`ops`] module
//! is a bounded pilot of plane cuts and through holes over certified solids (not a general
//! boolean kernel).
//!
//! The G02 builder is dimension-based, not the complete F06 contour/placement
//! request adapter. No rotation, I/O or dependency keys are exposed.

pub mod build;
pub mod diagnostic;
pub mod fastener;
pub mod feature_id;
pub mod input;
pub mod member;
pub mod mesh;
pub mod plate_contour;
pub mod profile;
pub mod result;
pub mod ruled;
pub mod section;
pub mod sweep;

// G06: shell certification, exact predicates, measurements and a bounded box index.
mod bigint;
pub mod enclosure;
mod exact;
mod intersect;
pub mod measure;
pub mod ops;
pub mod spatial;
pub mod topology;
pub mod trimesh;
pub mod verdict;

pub use build::{rectangular_plate, rectangular_plate_with_cancel};
pub use input::RectangularPlate;
pub use member::{StraightMember, straight_member};
pub use result::PlateResult;
