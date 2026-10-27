// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! G09: pilot plane cut and through-hole operations over certified solids.
//!
//! This is a **bounded pilot, not a general boolean kernel**. Two operations exist, both over a
//! [`Body`] that was certified by the G06 shell analysis:
//!
//! * [`plane_cut`]: split a body by a finite plane, keep one closed half-space and cap the
//!   section with a triangulated planar face (one outer loop plus loops inside it);
//! * [`through_hole`]: subtract a straight prism (a G05 round, oval or polygonal cavity along
//!   Z) that fully traverses a Z-prism body (a plate or extrusion), strictly inside its outline.
//!
//! Everything outside those scopes is refused with a typed [`OpFailure`] and no partial result:
//! non-prism bodies for holes, several outer section loops, a section that touches itself,
//! tangent or coplanar contact, holes that touch or cross the outline or another hole, cutters
//! that do not reach past both faces, and any case the exact predicates cannot decide.
//!
//! # Method
//!
//! * Which side of the plane every vertex is on is an exact sign (filtered binary64 first, then
//!   fixed-capacity integers, as in the G06 predicates); nothing is decided by an epsilon.
//! * A mesh edge crossing the plane gets one new vertex, the binary64 rounding of the exact
//!   intersection (each coordinate is rounded once, by exact integer comparisons with the
//!   neighbouring midpoints, so it depends neither on vertex numbering nor on rounded
//!   interpolation). The cap is therefore planar only up to that rounding; the largest
//!   deviation is certified and reported ([`OpResult::plane_deviation_mm`]).
//! * The cap loops are the boundary of the clipped surface; they are triangulated by the G05
//!   admission and ear clipping, in a frame that does not depend on which half is kept.
//! * Every candidate is canonicalised (vertices sorted by coordinates, triangles rotated and
//!   sorted by face), encoded as BCMESH01, parsed back, and certified again by the G06 analysis;
//!   a result that is not a certified single closed outward shell is never returned.
//!
//! Provenance: public mathematics only (half-space clipping of a convex polygon, Euler-Poincare
//! characteristic, the divergence theorem for volume, the winding-number tiling argument). No
//! boolean, clipping or triangulation crate, and no third-party source, is used.
//!
//! The kernel call is the headless API: the CLI commands that wrap these operations (A03 plane
//! cut, A04 hole) and their end-to-end gates are owned by those tasks, as for G02 and G05.

mod body;
mod cut;
mod diagnostic;
mod hole;
mod plane;
pub(crate) mod seal;
mod section;

pub use body::{Body, MAX_LOCAL_ID_BYTES, MAX_ORDINAL};
pub use cut::{Keep, plane_cut, plane_cut_with};
pub use diagnostic::{OpCode, OpDefect, OpFailure, OpField};
pub use hole::{HoleCutter, through_hole, through_hole_with};
pub use plane::CutPlane;
pub use seal::{OpKind, OpResult};

use crate::diagnostic::Stage;
use crate::topology::Limits;

/// Explicit work bounds of one operation; exceeding any gives a typed failure, never a hang
/// and never a partial result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpLimits {
    /// Triangles of the result (also bounded by the F06 cap of 524,288).
    pub max_triangles: usize,
    /// Loops of one cap or section, the outer loop included.
    pub max_loops: usize,
    /// Points of one cap profile (also bounded by the G05 cap of 2048).
    pub max_cap_points: usize,
    /// Exact-integer fallbacks of the operation's own predicates.
    pub max_exact_predicates: u64,
    /// Limits of the G06 certification of the candidate.
    pub analysis: Limits,
}

impl OpLimits {
    /// Production bounds: 524,288 triangles, 257 loops (256 G05 cavities plus the outer
    /// loop), 2048 cap points, 2^18 exact fallbacks and the default G06 limits.
    pub const DEFAULT: Self = Self {
        max_triangles: 524_288,
        max_loops: 257,
        max_cap_points: 2048,
        max_exact_predicates: 1 << 18,
        analysis: Limits::DEFAULT,
    };
}

/// Work counters of a successful operation; the budget tests hit them exactly.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OpStats {
    /// Triangles of the input body.
    pub triangles_in: usize,
    /// Triangles of the result.
    pub triangles_out: usize,
    /// Vertices created by the operation.
    pub new_vertices: usize,
    /// Loops of the cap or section, the outer loop included.
    pub loops: usize,
    /// Points of the cap profile.
    pub cap_points: usize,
    /// Exact-integer fallbacks used by the operation's own predicates.
    pub exact_predicates: u64,
}

pub(crate) fn poll(
    cancelled: &mut impl FnMut(Stage) -> bool,
    stage: Stage,
) -> Result<(), OpFailure> {
    if cancelled(stage) {
        Err(OpFailure::cancelled(stage))
    } else {
        Ok(())
    }
}

/// Counter for the operation's exact-integer fallbacks.
#[derive(Debug)]
pub(crate) struct ExactBudget {
    used: u64,
    limit: u64,
}

impl ExactBudget {
    pub(crate) const fn new(limit: u64) -> Self {
        Self { used: 0, limit }
    }

    pub(crate) const fn used(&self) -> u64 {
        self.used
    }

    pub(crate) fn charge(&mut self) -> Result<(), OpFailure> {
        if self.used >= self.limit {
            return Err(OpFailure::numeric(
                OpField::Limits,
                OpDefect::BudgetExact,
                Stage::Build,
            ));
        }
        self.used += 1;
        Ok(())
    }
}
