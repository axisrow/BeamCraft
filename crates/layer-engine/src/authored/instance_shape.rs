// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The P11 instance behind the discipline hook (#795): shared inputs are read from a canonical
//! part; only metadata and pose are local. The arms are the ones this replaces in `edit.rs` and
//! `validate.rs`, moved unchanged.

use super::hook::{AuthorCx, HookedShape, ShapeHook, ValidateCx};
use super::*;
use std::any::Any;
use steel::model::instance::PartKind;

/// The authored shape of a P11 instance.
#[derive(Debug, Clone, PartialEq)]
pub struct InstanceShape {
    /// Same part kind as the canonical definition.
    pub kind: PartKind,
    /// Canonical part identity, never another instance.
    pub definition: String,
}

impl Shape {
    /// An instance of the given canonical part; see [`InstanceShape`].
    pub fn instance(kind: PartKind, definition: String) -> Self {
        Self::Discipline(HookedShape::new(InstanceShape { kind, definition }))
    }
}

impl ShapeHook for InstanceShape {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn same_as(&self, other: &dyn ShapeHook) -> bool {
        other.as_any().downcast_ref::<Self>() == Some(self)
    }

    // An instance is a component; its part kind lives only in `bim:elementKind`.
    fn kind_token(&self) -> &'static str {
        self.kind.token()
    }

    fn category(&self) -> ElementCategory {
        ElementCategory::Part
    }

    fn author(&self, cx: &mut AuthorCx<'_>) -> Result<(), Diagnostic> {
        instance::author(cx.data, cx.path, &self.definition, cx.input, cx.prims)
    }

    fn attributes(&self) -> Vec<(&'static str, &'static str)> {
        vec![(instance::RELATIONSHIP, "rel")]
    }

    fn validate(&self, cx: &mut ValidateCx<'_>) -> Result<(), Diagnostic> {
        let definition = &self.definition;
        if cx.element.id == *definition || cx.element.material.is_some() {
            return Err(invalid());
        }
        instance::validate_source(definition, self.kind, cx.prims)?;
        for group in [
            Group::Lifecycle,
            Group::Label,
            Group::ShapeInputs,
            Group::Placement,
            Group::Material,
            Group::Relationship,
        ] {
            cx.reads.insert(Footprint {
                identity: definition.clone(),
                group,
            });
        }
        Ok(())
    }

    fn check_finite(&self) -> Result<(), Diagnostic> {
        Ok(())
    }
}
