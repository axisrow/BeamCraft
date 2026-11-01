// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Structural steel: what belongs to the steel discipline and to no other.
//!
//! The commands that create and edit plates, members, holes, bends, welds and bolts, the section
//! families and the fastener definitions. The rest of this crate (identities, digests, error
//! codes, limits, grids, construction objects, annotations, assemblies, levels) serves every
//! discipline. `docs/design/platform-and-steel.md` says how this module becomes a crate of its
//! own.

pub mod command;
pub mod definition;
pub mod fastener;
pub mod instance;
pub mod section;
