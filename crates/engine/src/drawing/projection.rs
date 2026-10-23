// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D02 orthographic wireframe and aligned projected dimension from semantic roles.

use super::{Drawing, MAX_PAPER_MM, Plane};
use crate::query::feature::{
    BodyKind, FeatureGeometry, FeatureId, FeatureKind, list_features, lookup_feature,
};
use crate::{ViewPin, ViewSnapshot};
use beamcraft_geom::tolerance::ABSOLUTE_FLOOR_MM;
use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;

/// Signature of fixed-basis f64 projected-distance rules, including collapse tolerance.
pub const ALGORITHM: &str = "drawing-orthographic-box/1;collapse-mm=0.001";

/// An explicit unresolved condition, with no successful numeric value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnresolvedReason {
    /// Selected owner was deleted or is absent.
    MissingOwner,
    /// Owner is not a member or no longer has canonical box topology.
    UnsupportedBody,
    /// A retained semantic role no longer resolves; never repaired by index/nearest geometry.
    MissingFeature,
    /// Distinct anchors project to less than the 0.001 mm geometric floor.
    CollapsedProjection,
}

impl UnresolvedReason {
    /// Stable wire token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MissingOwner => "missing_owner",
            Self::UnsupportedBody => "unsupported_body",
            Self::MissingFeature => "missing_feature",
            Self::CollapsedProjection => "collapsed_projection",
        }
    }
}

/// One named edge in paper millimetres. Coincident projected edges are retained.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectedEdge {
    /// Semantic source address, never a mesh/triangle index.
    pub feature: FeatureId,
    /// Projected endpoints in semantic order.
    pub paper_mm: [[f64; 2]; 2],
}

/// Body wireframe includes hidden/coincident edges; it is not hidden-line removal.
#[derive(Debug, Clone, PartialEq)]
pub enum BodyProjection {
    /// Twelve named edges in frozen role order.
    Resolved(Vec<ProjectedEdge>),
    /// No fabricated bounding-box projection.
    Unresolved(UnresolvedReason),
}

/// Aligned dimension with a projected model-unit label, separate from paper size.
#[derive(Debug, Clone, PartialEq)]
pub struct DimensionGeometry {
    /// Source vertices after projection, paper scale and origin.
    pub anchors_paper_mm: [[f64; 2]; 2],
    /// Parallel offset line endpoints in paper millimetres.
    pub line_paper_mm: [[f64; 2]; 2],
    /// Projected distance before paper scale.
    pub projected_length_mm: f64,
    /// Bounded decimal label in millimetres; no fabricated position mark.
    pub label_mm: String,
}

/// New verified value or explicit unresolved state, never a cached old value.
#[derive(Debug, Clone, PartialEq)]
pub enum DimensionProjection {
    /// Dimension resolved at this exact source.
    Resolved(DimensionGeometry),
    /// Intent retains its references but has no successful measurement.
    Unresolved(UnresolvedReason),
}

/// Pure geometry at the captured source, independent of UI camera/buffers.
#[derive(Debug, Clone, PartialEq)]
pub struct Projection {
    /// Wireframe or its explicit unresolved state.
    pub body: BodyProjection,
    /// Dimension or its explicit unresolved state.
    pub dimension: DimensionProjection,
}

fn failure(code: ErrorCode, message: &'static str) -> CommandError {
    CommandError::new(code, "drawing_projection", message)
}

fn projected(plane: Plane, [x, y, z]: [f64; 3]) -> [f64; 2] {
    match plane {
        Plane::Front => [x, z],
        Plane::Top => [x, y],
        Plane::Right => [y, z],
    }
}

fn bounded([x, y]: [f64; 2]) -> Result<[f64; 2], CommandError> {
    if !x.is_finite() || !y.is_finite() {
        return Err(failure(
            ErrorCode::NonFiniteNumber,
            "projection produced a nonfinite number",
        ));
    }
    if x.abs() > MAX_PAPER_MM || y.abs() > MAX_PAPER_MM {
        return Err(failure(
            ErrorCode::LimitExceeded,
            "projected drawing exceeds the paper coordinate cap",
        ));
    }
    Ok([
        if x == 0.0 { 0.0 } else { x },
        if y == 0.0 { 0.0 } else { y },
    ])
}

fn paper(drawing: &Drawing, point: [f64; 3]) -> Result<[f64; 2], CommandError> {
    let view = &drawing.spec().view;
    let [x, y] = projected(view.plane, point);
    let [ox, oy] = view.paper_origin_mm;
    bounded([
        x / view.scale_denominator + ox,
        y / view.scale_denominator + oy,
    ])
}

fn unresolved(reason: UnresolvedReason) -> Projection {
    Projection {
        body: BodyProjection::Unresolved(reason),
        dimension: DimensionProjection::Unresolved(reason),
    }
}

/// Resolves at the admitted intent's exact manifest/generation pin. The adapter must also verify
/// source project/revision/schema and kernel-derived contents before supplying this snapshot.
/// A mismatched pin refuses before geometry. Missing/unsupported geometry is explicitly unresolved.
/// Fixed work: at most 26 named roles, twelve edges and two anchors. All paper output is bounded.
pub fn resolve(drawing: &Drawing, snapshot: &ViewSnapshot) -> Result<Projection, CommandError> {
    let spec = drawing.spec();
    if snapshot.pin() != ViewPin::new(spec.source.manifest, spec.source.generation) {
        return Err(failure(
            ErrorCode::StaleWorkspace,
            "snapshot does not match the captured source pin",
        ));
    }
    let Some(owner) = snapshot.element(&spec.member) else {
        return Ok(unresolved(UnresolvedReason::MissingOwner));
    };
    if BodyKind::of(owner.kind) != Some(BodyKind::Member) {
        return Ok(unresolved(UnresolvedReason::UnsupportedBody));
    }
    let Ok(features) = list_features(snapshot, &spec.member) else {
        return Ok(unresolved(UnresolvedReason::UnsupportedBody));
    };
    let mut edges = Vec::with_capacity(12);
    for id in features
        .into_iter()
        .filter(|id| id.kind() == FeatureKind::Edge)
    {
        let Ok(found) = lookup_feature(snapshot, &id.to_string()) else {
            return Ok(unresolved(UnresolvedReason::MissingFeature));
        };
        let FeatureGeometry::Edge { ends: [a, b], .. } = found.world else {
            return Ok(unresolved(UnresolvedReason::MissingFeature));
        };
        edges.push(ProjectedEdge {
            feature: id,
            paper_mm: [paper(drawing, a)?, paper(drawing, b)?],
        });
    }
    let body = BodyProjection::Resolved(edges);
    let [first, second] = &spec.dimension.anchors;
    let vertex = |id: &FeatureId| match lookup_feature(snapshot, &id.to_string()) {
        Ok(found) => match found.world {
            FeatureGeometry::Vertex { position } => Some(position),
            _ => None,
        },
        Err(_) => None,
    };
    let (Some(a), Some(b)) = (vertex(first), vertex(second)) else {
        return Ok(Projection {
            body,
            dimension: DimensionProjection::Unresolved(UnresolvedReason::MissingFeature),
        });
    };
    let [ax, ay] = projected(spec.view.plane, a);
    let [bx, by] = projected(spec.view.plane, b);
    let (dx, dy) = (bx - ax, by - ay);
    let length = dx.hypot(dy);
    if !length.is_finite() {
        return Err(failure(
            ErrorCode::NonFiniteNumber,
            "dimension produced a nonfinite length",
        ));
    }
    if length < ABSOLUTE_FLOOR_MM {
        return Ok(Projection {
            body,
            dimension: DimensionProjection::Unresolved(UnresolvedReason::CollapsedProjection),
        });
    }
    let a_paper = paper(drawing, a)?;
    let b_paper = paper(drawing, b)?;
    let offset = spec.dimension.paper_offset_mm;
    let (ox, oy) = (-dy / length * offset, dx / length * offset);
    let line = |[x, y]: [f64; 2]| bounded([x + ox, y + oy]);
    Ok(Projection {
        body,
        dimension: DimensionProjection::Resolved(DimensionGeometry {
            anchors_paper_mm: [a_paper, b_paper],
            line_paper_mm: [line(a_paper)?, line(b_paper)?],
            projected_length_mm: length,
            label_mm: format!("{:.*}", usize::from(spec.dimension.decimal_places), length),
        }),
    })
}
