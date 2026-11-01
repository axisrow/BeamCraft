// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The connection family behind the discipline hook (#795): the weld (A06), the A05 bolt group
//! and the A14.3 bolt group. Each body is the arm it replaces in `edit.rs` and `validate.rs`,
//! moved unchanged, so the bytes written and the refusals returned are the same.

use super::edit::{attr, attribute, author_bolt_group, path, points, relationship_targets};
use super::hook::{
    AuthorCx, ChangeAuthorCx, ChangeHook, ChangeValidateCx, HookedChange, HookedShape, ShapeHook,
    ValidateCx,
};
use super::target_path;
use super::validate::{
    bolt_group_against, bolt_group_connects, bolt_group_values, cap, fastener_definition, finite,
    weld_path_points, weld_size, weld_tokens,
};
use super::*;
use openusd::sdf;
use std::any::Any;

pub(super) const WELD: &[(&str, &str)] = &[
    ("steel:connection:type", "token"),
    ("steel:weld:type", "token"),
    ("steel:weld:sizeMm", "double"),
    ("steel:weld:location", "token"),
    ("steel:weld:side", "token"),
    ("bim:connects", "rel"),
];
pub(super) const BOLT: &[(&str, &str)] = &[
    ("steel:connection:type", "token"),
    ("steel:bolt:diameterMm", "double"),
    ("steel:bolt:count", "int"),
    ("steel:bolt:pitchMm", "double"),
    ("steel:bolt:originMm", "double3"),
    ("steel:bolt:directionMm", "double3"),
    ("bim:connects", "rel"),
];
/// The seventeen authored attributes of an A14.3 bolt group, replaced as a whole.
pub(super) const BOLT_GROUP: &[(&str, &str)] = &[
    ("steel:bolt:fastener", "string"),
    ("steel:bolt:point1Mm", "double3"),
    ("steel:bolt:point2Mm", "double3"),
    ("steel:bolt:planeNormalMm", "double3"),
    ("steel:bolt:rotationRad", "double"),
    ("steel:bolt:onPlaneMm", "double"),
    ("steel:bolt:depthMm", "double"),
    ("steel:bolt:offsetStartMm", "double"),
    ("steel:bolt:spacingMode", "token"),
    ("steel:bolt:spacingValueMm", "double"),
    ("steel:bolt:spacingsMm", "double[]"),
    ("steel:bolt:lengthMode", "token"),
    ("steel:bolt:lengthMm", "double"),
    ("steel:bolt:washerUnderHead", "bool"),
    ("steel:bolt:washerUnderNut", "bool"),
    ("steel:bolt:holeMode", "token"),
    ("steel:bolt:holeClearanceMm", "double"),
];

impl Shape {
    /// A06 weld: a `connection` element with explicitly authored weld properties and one
    /// `bim:connects` record towards two distinct parts. No material and no derived body; its
    /// placement must be the identity in this slice.
    pub fn weld(weld: Weld) -> Self {
        Self::Discipline(HookedShape::new(weld))
    }

    /// A05 bolt group: a `connection` element with explicitly authored bolt properties and one
    /// `bim:connects` record towards two distinct parts. Like a weld it has no material, no
    /// derived body and the identity placement in this slice.
    pub fn bolt(bolt: Bolt) -> Self {
        Self::Discipline(HookedShape::new(bolt))
    }

    /// A14.3 bolt group (bolt amendment sections 4 and 6): a `connection` element that places
    /// one row of identical fasteners of a pinned definition between 2 to 1024 parts. Like an
    /// A05 group it has no material, no derived body and the identity placement; the definition
    /// travels by its digest. With `hole_mode = generate` (A14.4) the connected plates carry the
    /// group-owned hole records this group decides.
    pub fn bolt_group(group: BoltGroup) -> Self {
        Self::Discipline(HookedShape::new(group))
    }

    /// The weld of a hook shape, when it is one.
    pub fn as_weld(&self) -> Option<&Weld> {
        self.hook::<Weld>()
    }

    /// The A05 bolt group of a hook shape, when it is one.
    pub fn as_bolt(&self) -> Option<&Bolt> {
        self.hook::<Bolt>()
    }

    /// The A14.3 bolt group of a hook shape, when it is one.
    pub fn as_bolt_group(&self) -> Option<&BoltGroup> {
        self.hook::<BoltGroup>()
    }

    fn hook<T: 'static>(&self) -> Option<&T> {
        match self {
            Self::Discipline(h) => h.0.as_any().downcast_ref::<T>(),
            _ => None,
        }
    }
}

/// A connection takes no material and sits at the identity pose.
fn identity_connection(e: &Element) -> Result<(), Diagnostic> {
    if e.material.is_some()
        || e.placement.translation_mm != [0.0; 3]
        || e.placement.orientation_xyzw != [0.0, 0.0, 0.0, 1.0]
    {
        return Err(unsupported());
    }
    Ok(())
}

fn connects_two(
    connects: &[String; 2],
    e: &Element,
    message: &'static str,
    reads: &mut std::collections::BTreeSet<Footprint>,
) -> Result<(), Diagnostic> {
    let [main, secondary] = connects;
    if !uuid7(main) || !uuid7(secondary) {
        return Err(
            fail(ErrorCode::InvalidId, "invalid full element identity").with_rule("DV-BASE-001")
        );
    }
    if main == secondary || main == &e.id || secondary == &e.id {
        return Err(
            fail(ErrorCode::InvalidArgument, message).with_rule(if main == secondary {
                "DV-REL-007"
            } else {
                "DV-REL-006"
            }),
        );
    }
    // The parts' existence and kinds decide acceptance; they are only read.
    for part in [main, secondary] {
        reads.insert(Footprint {
            identity: part.clone(),
            group: Group::Lifecycle,
        });
    }
    Ok(())
}

fn connect_targets(connects: &[String], cx: &AuthorCx<'_>) -> Result<Vec<sdf::Path>, Diagnostic> {
    connects
        .iter()
        .map(|id| path(&target_path(id, ElementCategory::Part, cx.input, cx.prims)?))
        .collect()
}

impl ShapeHook for Weld {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn same_as(&self, other: &dyn ShapeHook) -> bool {
        other.as_any().downcast_ref::<Self>() == Some(self)
    }

    fn kind_token(&self) -> &'static str {
        "connection"
    }

    fn category(&self) -> ElementCategory {
        ElementCategory::Connection
    }

    fn author(&self, cx: &mut AuthorCx<'_>) -> Result<(), Diagnostic> {
        let (p, w) = (cx.path, self);
        attr(
            cx.data,
            p,
            "steel:connection:type",
            "token",
            sdf::Value::Token("weld".into()),
        )?;
        for (name, value) in [
            ("steel:weld:type", &w.weld_type),
            ("steel:weld:location", &w.location),
            ("steel:weld:side", &w.side),
        ] {
            attr(cx.data, p, name, "token", sdf::Value::token(value.as_str()))?;
        }
        attr(
            cx.data,
            p,
            "steel:weld:sizeMm",
            "double",
            sdf::Value::Double(w.size_mm),
        )?;
        if let Some(geometry) = &w.geometry {
            attr(
                cx.data,
                p,
                "steel:weld:pathMm",
                "double3[]",
                points(&geometry.points_mm),
            )?;
            attr(
                cx.data,
                p,
                "steel:weld:allAround",
                "token",
                sdf::Value::token(if geometry.all_around { "true" } else { "false" }),
            )?;
            attr(
                cx.data,
                p,
                "steel:weld:contour",
                "token",
                sdf::Value::token(geometry.contour.as_str()),
            )?;
        }
        let targets = connect_targets(&w.connects, cx)?;
        relationship_targets(cx.data, p, "bim:connects", Some(targets))
    }

    fn attributes(&self) -> Vec<(&'static str, &'static str)> {
        let mut out = WELD.to_vec();
        if self.geometry.is_some() {
            out.extend([
                ("steel:weld:pathMm", "double3[]"),
                ("steel:weld:allAround", "token"),
                ("steel:weld:contour", "token"),
            ]);
        }
        out
    }

    fn validate(&self, cx: &mut ValidateCx<'_>) -> Result<(), Diagnostic> {
        let w = self;
        identity_connection(cx.element)?;
        weld_tokens(&w.weld_type, &w.location, &w.side)?;
        weld_size(w.size_mm)?;
        if let Some(geometry) = &w.geometry {
            weld_path_points(&geometry.points_mm)?;
            if !matches!(
                geometry.contour.as_str(),
                "none" | "flush" | "concave" | "convex"
            ) {
                return Err(invalid());
            }
        }
        connects_two(
            &w.connects,
            cx.element,
            "a weld connects two distinct other parts",
            cx.reads,
        )
    }

    fn check_finite(&self) -> Result<(), Diagnostic> {
        finite([self.size_mm])
    }
}

impl ShapeHook for Bolt {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn same_as(&self, other: &dyn ShapeHook) -> bool {
        other.as_any().downcast_ref::<Self>() == Some(self)
    }

    fn kind_token(&self) -> &'static str {
        "connection"
    }

    fn category(&self) -> ElementCategory {
        ElementCategory::Connection
    }

    fn author(&self, cx: &mut AuthorCx<'_>) -> Result<(), Diagnostic> {
        let (p, b) = (cx.path, self);
        attr(
            cx.data,
            p,
            "steel:connection:type",
            "token",
            sdf::Value::Token("bolt_group".into()),
        )?;
        attr(
            cx.data,
            p,
            "steel:bolt:diameterMm",
            "double",
            sdf::Value::Double(b.diameter_mm),
        )?;
        // The engine admits 1 to 512 bolts, so the count always fits an int.
        attr(
            cx.data,
            p,
            "steel:bolt:count",
            "int",
            sdf::Value::Int(i32::try_from(b.count).map_err(|_| limit())?),
        )?;
        attr(
            cx.data,
            p,
            "steel:bolt:pitchMm",
            "double",
            sdf::Value::Double(b.pitch_mm),
        )?;
        let [ox, oy, oz] = b.origin_mm;
        attr(
            cx.data,
            p,
            "steel:bolt:originMm",
            "double3",
            sdf::Value::vec3d(ox, oy, oz),
        )?;
        let [dx, dy, dz] = b.direction_mm;
        attr(
            cx.data,
            p,
            "steel:bolt:directionMm",
            "double3",
            sdf::Value::vec3d(dx, dy, dz),
        )?;
        let targets = connect_targets(&b.connects, cx)?;
        relationship_targets(cx.data, p, "bim:connects", Some(targets))
    }

    fn attributes(&self) -> Vec<(&'static str, &'static str)> {
        BOLT.to_vec()
    }

    fn validate(&self, cx: &mut ValidateCx<'_>) -> Result<(), Diagnostic> {
        let b = self;
        identity_connection(cx.element)?;
        finite([b.diameter_mm])?;
        if b.diameter_mm <= 0.0 {
            return Err(invalid());
        }
        cap([b.diameter_mm], 1000.0)?;
        if b.count == 0 || b.count > 512 {
            return Err(invalid());
        }
        finite([b.pitch_mm])?;
        if b.pitch_mm <= 0.0 {
            return Err(invalid());
        }
        cap([b.pitch_mm], 1e6)?;
        finite(b.origin_mm)?;
        cap(b.origin_mm, 1e9)?;
        finite(b.direction_mm)?;
        let squared = b.direction_mm.iter().map(|v| v * v).sum::<f64>();
        if (squared - 1.0).abs() > 1e-6 {
            return Err(invalid());
        }
        connects_two(
            &b.connects,
            cx.element,
            "a bolt group connects two distinct other parts",
            cx.reads,
        )
    }

    fn check_finite(&self) -> Result<(), Diagnostic> {
        finite(
            [self.diameter_mm, self.pitch_mm]
                .into_iter()
                .chain(self.origin_mm)
                .chain(self.direction_mm),
        )
    }
}

impl ShapeHook for BoltGroup {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn same_as(&self, other: &dyn ShapeHook) -> bool {
        other.as_any().downcast_ref::<Self>() == Some(self)
    }

    fn kind_token(&self) -> &'static str {
        "connection"
    }

    fn category(&self) -> ElementCategory {
        ElementCategory::Connection
    }

    fn author(&self, cx: &mut AuthorCx<'_>) -> Result<(), Diagnostic> {
        author_bolt_group(cx.data, cx.path, self)?;
        let targets = connect_targets(&self.connects, cx)?;
        relationship_targets(cx.data, cx.path, "bim:connects", Some(targets))
    }

    fn attributes(&self) -> Vec<(&'static str, &'static str)> {
        let mut out = vec![("steel:connection:type", "token"), ("bim:connects", "rel")];
        for &(name, ty) in BOLT_GROUP {
            if name != "steel:bolt:holeClearanceMm" || self.hole_clearance_mm.is_some() {
                out.push((name, ty));
            }
        }
        out
    }

    fn validate(&self, cx: &mut ValidateCx<'_>) -> Result<(), Diagnostic> {
        let b = self;
        bolt_group_connects(&b.connects, &cx.element.id)?;
        bolt_group_values(b)?;
        // The pinned definition travels by digest: either this batch pins it as a resource or
        // the model already holds it. Either way the stored record must agree with the
        // definition's own values.
        let spec = fastener_definition(cx.input, cx.prims, b.fastener)?;
        bolt_group_against(&spec, b)?;
        for part in &b.connects {
            cx.reads.insert(Footprint {
                identity: part.clone(),
                group: Group::Lifecycle,
            });
        }
        cx.reads.insert(Footprint {
            identity: b.fastener.to_string(),
            group: Group::Relationship,
        });
        Ok(())
    }

    fn check_finite(&self) -> Result<(), Diagnostic> {
        finite(
            self.point1_mm
                .into_iter()
                .chain(self.point2_mm)
                .chain(self.plane_normal_mm)
                .chain([
                    self.rotation_rad,
                    self.on_plane_mm,
                    self.depth_mm,
                    self.offset_start_mm,
                    self.spacing_value_mm,
                    self.length_mm,
                ])
                .chain(self.spacings_mm.iter().copied()),
        )
    }
}

/// The replacement of the complete authored record of an A14.3 bolt group as a whole (bolt
/// amendment section 6): the seventeen `steel:bolt:*` attributes and the `bim:connects` record,
/// with the five A05 annotation attributes removed. The engine has already decided the
/// placement; this writes it. It is also the upgrade path of a stored A05 group.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplaceBoltGroup {
    /// Full target UUIDv7.
    pub id: String,
    /// The complete new record.
    pub group: BoltGroup,
}

impl Change {
    /// Replace the complete authored record of an A14.3 bolt group; see [`ReplaceBoltGroup`].
    pub fn bolt_group(id: String, group: BoltGroup) -> Self {
        Self::Discipline(HookedChange::new(ReplaceBoltGroup { id, group }))
    }
}

impl ChangeHook for ReplaceBoltGroup {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn same_as(&self, other: &dyn ChangeHook) -> bool {
        other.as_any().downcast_ref::<Self>() == Some(self)
    }

    fn target(&self) -> &str {
        &self.id
    }

    fn group_key(&self) -> &'static str {
        "steel:bolt"
    }

    fn targets_kind(&self, kind: &str) -> bool {
        kind == "connection"
    }

    // The complete authored inputs and the `connects` record are replaced as a whole (A14.3);
    // a connection derives no body, so no derived group is written.
    fn groups(&self) -> (&'static [Group], &'static [Group]) {
        (
            &[Group::Lifecycle, Group::ShapeInputs, Group::Relationship],
            &[Group::ShapeInputs, Group::Relationship],
        )
    }

    fn validate(&self, cx: &mut ChangeValidateCx<'_>) -> Result<(), Diagnostic> {
        let group = &self.group;
        bolt_group_connects(&group.connects, &self.id)?;
        bolt_group_values(group)?;
        // A repin in the same batch or the pinned definition the model already holds.
        let spec = fastener_definition(cx.input, cx.prims, group.fastener)?;
        bolt_group_against(&spec, group)?;
        for part in &group.connects {
            cx.reads.insert(Footprint {
                identity: part.clone(),
                group: Group::Lifecycle,
            });
        }
        cx.reads.insert(Footprint {
            identity: group.fastener.to_string(),
            group: Group::Relationship,
        });
        Ok(())
    }

    fn check_finite(&self) -> Result<(), Diagnostic> {
        self.group.check_finite()
    }

    fn author(&self, cx: &mut ChangeAuthorCx<'_>) -> Result<(), Diagnostic> {
        // An A05 annotation group is upgraded by removing its five attributes; a group of this
        // slice replaces its own attributes as a whole.
        for (name, ty) in [
            ("steel:bolt:diameterMm", "double"),
            ("steel:bolt:count", "int"),
            ("steel:bolt:pitchMm", "double"),
            ("steel:bolt:originMm", "double3"),
            ("steel:bolt:directionMm", "double3"),
        ] {
            attribute(cx.data, cx.path, name, ty, Opinion::Remove, true)?;
        }
        author_bolt_group(cx.data, cx.path, &self.group)?;
        let targets = self
            .group
            .connects
            .iter()
            .map(|id| path(&target_path(id, ElementCategory::Part, cx.input, cx.prims)?))
            .collect::<Result<Vec<_>, _>>()?;
        relationship_targets(cx.data, cx.path, "bim:connects", Some(targets))
    }

    fn attributes(&self) -> Vec<(&'static str, &'static str)> {
        let mut out = vec![("steel:connection:type", "token"), ("bim:connects", "rel")];
        for &(name, ty) in BOLT_GROUP {
            if name != "steel:bolt:holeClearanceMm" || self.group.hole_clearance_mm.is_some() {
                out.push((name, ty));
            }
        }
        out
    }
}
