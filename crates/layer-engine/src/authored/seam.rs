// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The seam between the platform and a discipline crate (#795): the narrow, documented surface
//! a discipline's shape and change hooks use to author, validate and read through
//! [`crate::authored`].
//!
//! The seam is split by direction, because authoring and reading each have a `points` and a
//! `relationship`: [`author`] builds layer values, [`read`] inspects composed prims. Everything
//! re-exported here is stable platform behaviour a hook may rely on; anything else in the
//! platform's private modules is not.

/// The authoring half of the seam: writing fields into the layer being built.
pub mod author {
    pub use super::super::edit::{
        attr, attribute, author_record_history, path, points, relationship, relationship_targets,
    };
}

/// The reading half of the seam: inspecting the prims of the stage a batch applies to.
pub mod read {
    pub use super::super::validate::{
        cap, count_points, finite, link, number, optional_number, optional_text, points,
        relationship, required, resource, text,
    };
}

pub use super::{
    element_path, hook::AuthorCx, hook::ChangeAuthorCx, hook::ChangeHook, hook::ChangeValidateCx,
    hook::HookedChange, hook::HookedShape, hook::ShapeHook, hook::ValidateCx, invalid, target_path,
};
pub use crate::{fail, limit, unsupported, uuid7};
