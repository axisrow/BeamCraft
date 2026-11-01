// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The hook behind `Shape::Discipline`: the part of a created element that belongs to a
//! discipline rather than to the platform (#795).
//!
//! The platform keeps the lifecycle, the label, the pose, the material link and the budgets of
//! an element. Everything else about a created element (its kind token, its attributes, their
//! bounds and the attribute names the create will hold) is answered by the shape's own hook, so
//! the platform never matches on the kind of a discipline's element.

use super::*;
use openusd::sdf;
use std::any::Any;
use std::collections::BTreeSet;
use std::fmt::Debug;
use std::sync::Arc;

/// What a hook needs to write the attributes of a created element.
pub struct AuthorCx<'a> {
    /// The authored layer being built.
    pub data: &'a mut sdf::Data,
    /// The element's prim path.
    pub path: &'a str,
    /// The created element.
    pub element: &'a Element,
    /// The whole batch.
    pub input: &'a Prepare,
    /// The stage the batch applies to.
    pub prims: &'a [Prim],
}

/// What a hook needs to validate a created element.
pub struct ValidateCx<'a> {
    /// The byte and item budgets of the batch.
    pub budgets: Budgets,
    /// The created element.
    pub element: &'a Element,
    /// The whole batch.
    pub input: &'a Prepare,
    /// The stage the batch applies to.
    pub prims: &'a [Prim],
    /// The declared footprint reads; a hook adds the identities its element depends on.
    pub reads: &'a mut BTreeSet<Footprint>,
}

/// The discipline half of a created element.
pub trait ShapeHook: Debug + Send + Sync {
    /// The hook as `Any`, so equality can compare concrete types.
    fn as_any(&self) -> &dyn Any;
    /// Whether `other` is the same concrete shape with equal content.
    fn same_as(&self, other: &dyn ShapeHook) -> bool;
    /// The `bim:elementKind` token of the created element.
    fn kind_token(&self) -> &'static str;
    /// The prim category (and so the path prefix) of the created element.
    fn category(&self) -> ElementCategory;
    /// Writes the element's own attributes and relationships, after the lifecycle attributes
    /// and before the material link, the cut records and the pose.
    fn author(&self, cx: &mut AuthorCx<'_>) -> Result<(), Diagnostic>;
    /// The `(name, type)` of every attribute the create writes beyond the lifecycle and pose.
    fn attributes(&self) -> Vec<(&'static str, &'static str)>;
    /// Engineering bounds of the create; adds the reads the create depends on.
    fn validate(&self, cx: &mut ValidateCx<'_>) -> Result<(), Diagnostic>;
    /// Every number of the shape is finite.
    fn check_finite(&self) -> Result<(), Diagnostic>;
    /// Charges the element's arrays against the batch budget; `count(items, bytes_per_item)`.
    fn charge(
        &self,
        _count: &mut dyn FnMut(usize, usize) -> Result<(), Diagnostic>,
    ) -> Result<(), Diagnostic> {
        Ok(())
    }
    /// The owned `plane_cut` records authored with the element (A03); none by default.
    fn cuts(&self) -> &[CutRecord] {
        &[]
    }
    /// Whether the element carries the authored part class `steel:class`.
    fn classed(&self) -> bool {
        false
    }
}

/// A hook shape as the platform holds it: cheap to clone, comparable.
#[derive(Debug, Clone)]
pub struct HookedShape(pub Arc<dyn ShapeHook>);

impl HookedShape {
    /// Wraps a discipline shape.
    pub fn new(hook: impl ShapeHook + 'static) -> Self {
        Self(Arc::new(hook))
    }
}

impl PartialEq for HookedShape {
    fn eq(&self, other: &Self) -> bool {
        self.0.same_as(other.0.as_ref())
    }
}

/// What a change hook needs to write its edit into the authored layer.
pub struct ChangeAuthorCx<'a> {
    /// The authored layer being edited.
    pub data: &'a mut sdf::Data,
    /// The target element's prim path.
    pub path: &'a str,
    /// The whole batch.
    pub input: &'a Prepare,
    /// The stage the batch applies to.
    pub prims: &'a [Prim],
}

/// What a change hook needs to validate its edit.
pub struct ChangeValidateCx<'a> {
    /// The byte and item budgets of the batch.
    pub budgets: Budgets,
    /// The whole batch.
    pub input: &'a Prepare,
    /// The stage the batch applies to.
    pub prims: &'a [Prim],
    /// The declared footprint reads; a hook adds the identities its edit depends on.
    pub reads: &'a mut BTreeSet<Footprint>,
}

/// The discipline half of an edit of an existing element.
pub trait ChangeHook: Debug + Send + Sync {
    /// The hook as `Any`, so equality can compare concrete types.
    fn as_any(&self) -> &dyn Any;
    /// Whether `other` is the same concrete change with equal content.
    fn same_as(&self, other: &dyn ChangeHook) -> bool;
    /// Full identity of the element the change targets.
    fn target(&self) -> &str;
    /// The write group name used to order and to compare changes of one element.
    fn group_key(&self) -> &'static str;
    /// Whether an element of the stored `bim:elementKind` may take this change.
    fn targets_kind(&self, kind: &str) -> bool;
    /// The semantic groups the change reads and the groups it writes.
    fn groups(&self) -> (&'static [Group], &'static [Group]);
    /// Engineering bounds of the edit; adds the reads the edit depends on.
    fn validate(&self, cx: &mut ChangeValidateCx<'_>) -> Result<(), Diagnostic>;
    /// Every number of the edit is finite.
    fn check_finite(&self) -> Result<(), Diagnostic>;
    /// Writes the edit into the target element's prim.
    fn author(&self, cx: &mut ChangeAuthorCx<'_>) -> Result<(), Diagnostic>;
    /// The `(name, type)` of every attribute the edit writes.
    fn attributes(&self) -> Vec<(&'static str, &'static str)>;
    /// Whether the edit removes the derived result of its element (a shape input changes).
    fn invalidates_derived(&self) -> bool {
        false
    }
    /// Whether the edit also declares the derived mesh fields among the attributes it writes.
    fn declares_mesh(&self) -> bool {
        self.invalidates_derived()
    }
    /// Charges the edit's arrays against the batch budget; `count(items, bytes_per_item)`.
    fn charge(
        &self,
        _count: &mut dyn FnMut(usize, usize) -> Result<(), Diagnostic>,
    ) -> Result<(), Diagnostic> {
        Ok(())
    }
}

/// A hook change as the platform holds it: cheap to clone, comparable.
#[derive(Debug, Clone)]
pub struct HookedChange(pub Arc<dyn ChangeHook>);

impl HookedChange {
    /// Wraps a discipline change.
    pub fn new(hook: impl ChangeHook + 'static) -> Self {
        Self(Arc::new(hook))
    }
}

impl PartialEq for HookedChange {
    fn eq(&self, other: &Self) -> bool {
        self.0.same_as(other.0.as_ref())
    }
}
