// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The P11 instance behind the discipline hook (#795): shared inputs are read from a canonical
//! part; only metadata and pose are local. The arms are the ones that replaced this in the
//! platform's `edit.rs` and `validate.rs`, moved unchanged; the facade constructor in [`super`]
//! builds the platform `Shape` from this hook.

use crate::model::instance::PartKind;
use beamcraft_layer_engine::Diagnostic;
use beamcraft_layer_engine::authored::instance;
use beamcraft_layer_engine::authored::{
    AuthorCx, Footprint, Group, ShapeHook, ValidateCx, invalid,
};
use beamcraft_model::prim::ElementCategory;
use std::any::Any;

/// The authored shape of a P11 instance.
#[derive(Debug, Clone, PartialEq)]
pub struct InstanceShape {
    /// Same part kind as the canonical definition.
    pub kind: PartKind,
    /// Canonical part identity, never another instance.
    pub definition: String,
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
        instance::validate_source(definition, self.kind.token(), cx.prims)?;
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
