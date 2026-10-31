// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D02 orthographic wireframe and aligned projected dimension from semantic roles.
//!
//! D04 slice 2 adds two derived views over the same pinned snapshot: a section view cuts the
//! member wireframe with one axis-aligned half-space and carries the exact cut contour, and a
//! detail view magnifies one parent view's circular region at its own scale.

use super::{
    CutSide, DetailSpec, Drawing, MAX_PAPER_MM, Plane, SectionSpec, SourceModelPin, ViewSpec,
};
use crate::query::feature::{
    BodyKind, FeatureGeometry, FeatureId, FeatureKind, list_features, lookup_feature,
};
use crate::{ViewPin, ViewSnapshot};
use beamcraft_geom::tolerance::ABSOLUTE_FLOOR_MM;
use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::ElementId;

/// Signature of fixed-basis f64 projected-distance rules, including collapse tolerance.
pub const ALGORITHM: &str = "drawing-orthographic-box/1;collapse-mm=0.001;section=axis-half-space/1;detail=circle-magnify/1";

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

fn place(
    plane: Plane,
    scale_denominator: f64,
    paper_origin_mm: [f64; 2],
    point: [f64; 3],
) -> Result<[f64; 2], CommandError> {
    let [x, y] = projected(plane, point);
    let [ox, oy] = paper_origin_mm;
    bounded([x / scale_denominator + ox, y / scale_denominator + oy])
}

fn paper(view: &ViewSpec, point: [f64; 3]) -> Result<[f64; 2], CommandError> {
    place(
        view.plane,
        view.scale_denominator,
        view.paper_origin_mm,
        point,
    )
}

fn paper_section(section: &SectionSpec, point: [f64; 3]) -> Result<[f64; 2], CommandError> {
    place(
        section.plane,
        section.scale_denominator,
        section.paper_origin_mm,
        point,
    )
}

fn unresolved(reason: UnresolvedReason) -> Projection {
    Projection {
        body: BodyProjection::Unresolved(reason),
        dimension: DimensionProjection::Unresolved(reason),
    }
}

/// Rejects a snapshot that does not carry the document's exact captured source. This is the
/// whole stale rule: a model change marks every view of the document stale together.
fn check_pin(source: &SourceModelPin, snapshot: &ViewSnapshot) -> Result<(), CommandError> {
    if snapshot.pin() != ViewPin::new(source.manifest, source.generation) {
        return Err(failure(
            ErrorCode::StaleWorkspace,
            "snapshot does not match the captured source pin",
        ));
    }
    Ok(())
}

/// The member's canonical box edges: each with its semantic address and world endpoints.
type MemberEdges = Vec<(FeatureId, [[f64; 3]; 2])>;

/// The member's canonical box edges as semantic (feature id, world endpoints) pairs, or the
/// explicit unresolved state. Fixed work: at most 26 named roles and twelve edges.
fn member_edges(
    member: ElementId,
    snapshot: &ViewSnapshot,
) -> Result<MemberEdges, UnresolvedReason> {
    let Some(owner) = snapshot.element(&member) else {
        return Err(UnresolvedReason::MissingOwner);
    };
    if BodyKind::of(owner.kind) != Some(BodyKind::Member) {
        return Err(UnresolvedReason::UnsupportedBody);
    }
    let Ok(features) = list_features(snapshot, &member) else {
        return Err(UnresolvedReason::UnsupportedBody);
    };
    let mut edges = Vec::with_capacity(12);
    for id in features
        .into_iter()
        .filter(|id| id.kind() == FeatureKind::Edge)
    {
        let Ok(found) = lookup_feature(snapshot, &id.to_string()) else {
            return Err(UnresolvedReason::MissingFeature);
        };
        let FeatureGeometry::Edge { ends, .. } = found.world else {
            return Err(UnresolvedReason::MissingFeature);
        };
        edges.push((id, ends));
    }
    Ok(edges)
}

/// One view's body wireframe at the admitted intent's pin, or its explicit unresolved state.
/// All paper output is bounded.
fn body_edges(
    view: &ViewSpec,
    member: ElementId,
    snapshot: &ViewSnapshot,
) -> Result<BodyProjection, CommandError> {
    let world = match member_edges(member, snapshot) {
        Ok(edges) => edges,
        Err(reason) => return Ok(BodyProjection::Unresolved(reason)),
    };
    let mut edges = Vec::with_capacity(world.len());
    for (id, ends) in world {
        edges.push(ProjectedEdge {
            feature: id,
            paper_mm: [paper(view, ends[0])?, paper(view, ends[1])?],
        });
    }
    Ok(BodyProjection::Resolved(edges))
}

/// Resolves one placed or primary view of an admitted document from its exact pinned snapshot:
/// the view shows exactly what its basis shows over the captured source, nothing more. A view
/// outside the document refuses; a stale snapshot refuses before geometry; missing or
/// unsupported geometry is explicitly unresolved. All paper output is bounded.
pub fn resolve_view(
    drawing: &Drawing,
    view: &ViewSpec,
    snapshot: &ViewSnapshot,
) -> Result<BodyProjection, CommandError> {
    let spec = drawing.spec();
    check_pin(&spec.source, snapshot)?;
    if *view != spec.view && !spec.views.contains(view) {
        return Err(failure(
            ErrorCode::InvalidArgument,
            "view does not belong to this document",
        ));
    }
    body_edges(view, spec.member, snapshot)
}

/// Resolves the D02 document: the primary view's wireframe and its aligned dimension.
pub fn resolve(drawing: &Drawing, snapshot: &ViewSnapshot) -> Result<Projection, CommandError> {
    let spec = drawing.spec();
    check_pin(&spec.source, snapshot)?;
    let body = match body_edges(&spec.view, spec.member, snapshot)? {
        BodyProjection::Resolved(edges) => BodyProjection::Resolved(edges),
        // A view without its body never reports a successfully measured dimension: both share
        // the same explicit unresolved reason, exactly as the D02 contract resolved them.
        BodyProjection::Unresolved(reason) => return Ok(unresolved(reason)),
    };
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
    let a_paper = paper(&spec.view, a)?;
    let b_paper = paper(&spec.view, b)?;
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

/// One section view's resolved geometry: the visible-half wireframe and the exact cut contour.
#[derive(Debug, Clone, PartialEq)]
pub struct SectionGeometry {
    /// Wireframe edges of the visible half, each clipped to the visible half-space and keeping
    /// its semantic feature address. A crossing edge ends exactly on the cut plane; an edge
    /// wholly behind the cut is dropped.
    pub edges: Vec<ProjectedEdge>,
    /// The cut contour in boundary order, paper millimetres, closed implicitly. Empty when the
    /// cut plane misses the member box.
    pub cut_paper_mm: Vec<[f64; 2]>,
}

/// New verified section geometry or the explicit unresolved state of the sectioned body.
#[derive(Debug, Clone, PartialEq)]
pub enum SectionProjection {
    /// Section resolved at this exact source.
    Resolved(SectionGeometry),
    /// Intent retains its references but has no successful geometry.
    Unresolved(UnresolvedReason),
}

/// One detail view's resolved geometry: the parent wireframe magnified into the region.
#[derive(Debug, Clone, PartialEq)]
pub struct DetailGeometry {
    /// Parent edges mapped into the detail's paper space, each clipped to the region and
    /// keeping the parent view's semantic feature addresses.
    pub edges: Vec<ProjectedEdge>,
    /// The mapped region centre in the detail's paper millimetres.
    pub region_center_mm: [f64; 2],
    /// The mapped region radius in the detail's paper millimetres.
    pub region_radius_mm: f64,
}

/// New verified detail geometry or the explicit unresolved state of the parent's body.
#[derive(Debug, Clone, PartialEq)]
pub enum DetailProjection {
    /// Detail resolved at this exact source.
    Resolved(DetailGeometry),
    /// Intent retains its references but has no successful geometry.
    Unresolved(UnresolvedReason),
}

/// Signed depth of a world point into a section's visible half-space: positive visible,
/// negative hidden, zero exactly on the cut plane. The plane-to-axis mapping lives in
/// [`Plane::depth_axis`] alone.
fn section_depth(section: &SectionSpec, point: [f64; 3]) -> f64 {
    let [x, y, z] = point;
    let depth = match section.plane.depth_axis() {
        0 => x,
        1 => y,
        _ => z,
    } - section.cut_mm;
    match section.side {
        CutSide::Positive => depth,
        CutSide::Negative => -depth,
    }
}

/// The world point at parameter `t` between two world points.
fn world_lerp([ax, ay, az]: [f64; 3], [bx, by, bz]: [f64; 3], t: f64) -> [f64; 3] {
    [ax + t * (bx - ax), ay + t * (by - ay), az + t * (bz - az)]
}

/// Deduplicates near-coincident contour points and orders a convex cut contour by angle
/// around its vertex centroid, deterministically. Fewer than three distinct points is a
/// degenerate sliver and carries no contour.
fn cut_contour(mut points: Vec<[f64; 2]>) -> Vec<[f64; 2]> {
    points.sort_by(|p, q| p[0].total_cmp(&q[0]).then(p[1].total_cmp(&q[1])));
    points.dedup_by(|b, a| (a[0] - b[0]).hypot(a[1] - b[1]) < ABSOLUTE_FLOOR_MM);
    if points.len() < 3 {
        return Vec::new();
    }
    let count = points.len() as f64;
    let (mut sx, mut sy) = (0.0, 0.0);
    for [x, y] in &points {
        sx += x;
        sy += y;
    }
    let (cx, cy) = (sx / count, sy / count);
    points.sort_by(|p, q| {
        let ap = (p[1] - cy).atan2(p[0] - cx);
        let aq = (q[1] - cy).atan2(q[0] - cx);
        ap.total_cmp(&aq)
            .then(p[0].total_cmp(&q[0]))
            .then(p[1].total_cmp(&q[1]))
    });
    points
}

/// Resolves one section view of an admitted document from its exact pinned snapshot: the
/// visible half of the member wireframe and the exact cut contour. A section outside the
/// document refuses; a stale snapshot refuses before geometry; missing or unsupported
/// geometry is explicitly unresolved. All paper output is bounded.
pub fn resolve_section(
    drawing: &Drawing,
    section: &SectionSpec,
    snapshot: &ViewSnapshot,
) -> Result<SectionProjection, CommandError> {
    let spec = drawing.spec();
    check_pin(&spec.source, snapshot)?;
    if !spec.sections.contains(section) {
        return Err(failure(
            ErrorCode::InvalidArgument,
            "section does not belong to this document",
        ));
    }
    let mut edges = Vec::with_capacity(12);
    let mut crossings = Vec::new();
    for (id, ends) in match member_edges(spec.member, snapshot) {
        Ok(edges) => edges,
        Err(reason) => return Ok(SectionProjection::Unresolved(reason)),
    } {
        let [a, b] = ends;
        let (da, db) = (section_depth(section, a), section_depth(section, b));
        // A strict sign change places the crossing point on the cut contour; an endpoint
        // exactly on the plane stays visible without adding a contour vertex.
        if da < 0.0 && db >= 0.0 || da >= 0.0 && db < 0.0 {
            // Both denominators are strictly nonzero by the strict sign change above.
            let t = da / (da - db);
            let crossing = world_lerp(a, b, t);
            crossings.push(paper_section(section, crossing)?);
        }
        let visible = |depth: f64| depth >= 0.0;
        if let Some((from, to)) = match (visible(da), visible(db)) {
            (true, true) => Some((a, b)),
            (false, false) => None,
            (true, false) => {
                let t = da / (da - db);
                Some((a, world_lerp(a, b, t)))
            }
            (false, true) => {
                let t = da / (da - db);
                Some((world_lerp(a, b, t), b))
            }
        } {
            edges.push(ProjectedEdge {
                feature: id,
                paper_mm: [paper_section(section, from)?, paper_section(section, to)?],
            });
        }
    }
    Ok(SectionProjection::Resolved(SectionGeometry {
        edges,
        cut_paper_mm: cut_contour(crossings),
    }))
}

/// Clips the paper segment `a`–`b` to the closed disk (centre, radius): the visible
/// sub-segment, or `None` when the segment stays outside. A zero-length segment is a point.
fn clip_disk_segment(
    a: [f64; 2],
    b: [f64; 2],
    centre: [f64; 2],
    radius: f64,
) -> Option<[[f64; 2]; 2]> {
    let [ax, ay] = a;
    let [bx, by] = b;
    let [cx, cy] = centre;
    let (dx, dy) = (bx - ax, by - ay);
    let (fx, fy) = (ax - cx, ay - cy);
    let q = dx * dx + dy * dy;
    if q <= 0.0 {
        // All inputs are bounded paper coordinates, so this cannot overflow.
        let inside = fx * fx + fy * fy <= radius * radius;
        let point = [ax, ay];
        return inside.then_some([point, point]);
    }
    let r = fx * dx + fy * dy;
    let disc = r * r - q * (fx * fx + fy * fy - radius * radius);
    if disc < 0.0 {
        return None;
    }
    let root = disc.sqrt();
    let t0 = ((-r - root) / q).max(0.0);
    let t1 = ((-r + root) / q).min(1.0);
    if t0 > t1 {
        return None;
    }
    Some([[ax + t0 * dx, ay + t0 * dy], [ax + t1 * dx, ay + t1 * dy]])
}

/// Resolves one detail view of an admitted document from its exact pinned snapshot: the
/// parent view's wireframe magnified by the two scales' ratio around the region centre and
/// clipped to the region. A detail outside the document refuses; a stale snapshot refuses
/// before geometry; the parent's explicit unresolved state is the detail's.
pub fn resolve_detail(
    drawing: &Drawing,
    detail: &DetailSpec,
    snapshot: &ViewSnapshot,
) -> Result<DetailProjection, CommandError> {
    let spec = drawing.spec();
    check_pin(&spec.source, snapshot)?;
    if !spec.details.contains(detail) {
        return Err(failure(
            ErrorCode::InvalidArgument,
            "detail does not belong to this document",
        ));
    }
    // Admission guarantees the parent is the primary view or a placed view.
    let parent = if detail.parent == spec.view.id {
        &spec.view
    } else {
        spec.views
            .iter()
            .find(|placed| placed.id == detail.parent)
            .ok_or_else(|| {
                failure(
                    ErrorCode::InvalidArgument,
                    "detail parent is not a view of this document",
                )
            })?
    };
    let magnification = parent.scale_denominator / detail.scale_denominator;
    let (centre, radius, origin) = (
        detail.center_paper_mm,
        detail.radius_mm,
        detail.paper_origin_mm,
    );
    let map = |[x, y]: [f64; 2]| {
        bounded([
            (x - centre[0]) * magnification + origin[0],
            (y - centre[1]) * magnification + origin[1],
        ])
    };
    let edges = match body_edges(parent, spec.member, snapshot)? {
        BodyProjection::Resolved(edges) => edges,
        BodyProjection::Unresolved(reason) => return Ok(DetailProjection::Unresolved(reason)),
    };
    let mut mapped = Vec::with_capacity(edges.len());
    for edge in edges {
        let ProjectedEdge {
            feature,
            paper_mm: [a, b],
        } = edge;
        if let Some([from, to]) = clip_disk_segment(a, b, centre, radius) {
            mapped.push(ProjectedEdge {
                feature,
                paper_mm: [map(from)?, map(to)?],
            });
        }
    }
    Ok(DetailProjection::Resolved(DetailGeometry {
        edges: mapped,
        region_center_mm: bounded(origin)?,
        region_radius_mm: radius * magnification,
    }))
}
