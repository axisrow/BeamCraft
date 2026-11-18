// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Q03 slice 2: the validated geometric method behind `check-model`, and the
//! clash verdict.
//!
//! The candidate method of #634 — decompose a part into convex pieces, then
//! compare piece by piece — was rejected in favour of the simpler method the
//! contract of `docs/contracts/q03-model-checks-draft-v0.1.md` section 7
//! actually demands: compare the two **final evaluated closed meshes**
//! directly. The kernel decides the contact facts (`kernel::contact`) with its
//! exact predicates, so the relation of two parts is a fact about their
//! surfaces and cannot depend on a triangulation or on an artificial
//! decomposition; there are no internal faces to offset. The method was
//! validated on the adversarial cases of #634: face contact, edge contact,
//! separation, full containment without a surface meeting, corner overlap,
//! plane-cut and holed bodies, rotated placements, and re-triangulations that
//! keep the vertex set (`kernel::contact::tests`).
//!
//! Two tolerance layers stay separate (contract section 7.2): the kernel's
//! exact predicates decide the contact facts, and the physical contact
//! tolerance below only decides what counts as touching, through the
//! near-threshold rule of section 7.3. The mesh approximation bound is the
//! per-body arc-deviation value recorded at construction; a bound below half
//! the contact tolerance can never decide a verdict alone, because
//! `delta = max(bound, t/2)`.

use crate::engine::committed::{MemberRecord, PlateRecord};
use crate::kernel::section::detailed_sagitta_bound;
use beamcraft_engine::solid::SolidMesh;
use beamcraft_kernel::contact::{self, ContactError, Relation};
use beamcraft_kernel::topology::Limits;
use beamcraft_kernel::trimesh::TriMesh;
use beamcraft_model::command::Placement;
use beamcraft_model::id::ElementId;

/// The product contact tolerance `t` in millimetres: what counts as touching
/// (contract section 7.3). It is a product decision, not a normative
/// dimension; v0.1 has no command to change it and reports it with every
/// result.
pub const CONTACT_TOLERANCE_MM: f64 = 0.5;

/// The kernel work caps of one contact decision.
pub const CONTACT_LIMITS: Limits = Limits::DEFAULT;

/// One evaluated part in the world frame, ready for a pair decision.
#[derive(Debug, Clone)]
pub struct ElementBody {
    /// The element.
    pub id: ElementId,
    /// The element label.
    pub name: String,
    /// The stable kind token (`plate`, `beam`, `column`).
    pub kind: &'static str,
    /// The certified mesh in world coordinates.
    pub mesh: TriMesh,
    /// The world corner extents of the mesh, cached once so a pair prefilter
    /// never rescans either mesh.
    pub bounds: ([f64; 3], [f64; 3]),
    /// The mesh approximation bound in millimetres: the largest chord
    /// deviation any faceted arc of this body keeps.
    pub mesh_bound_mm: f64,
}

impl ElementBody {
    /// The world mesh of an evaluated plate: the kernel body rotated by the
    /// authored orientation, translated, and re-admitted as a mesh.
    pub fn of_plate(record: &PlateRecord) -> Result<Self, ContactError> {
        let mesh_bound = record.body.arc_deviation_mm;
        Self::of_placed(
            record.authored.id,
            record.authored.name.clone(),
            "plate",
            &record.body.solid,
            &record.authored.placement,
            mesh_bound,
        )
    }

    /// The world mesh of an evaluated member. The section's faceted arcs add
    /// their own bound to the member's recorded path bound.
    pub fn of_member(record: &MemberRecord) -> Result<Self, ContactError> {
        let kind = match record.authored.kind {
            crate::model::command::MemberKind::Beam => "beam",
            crate::model::command::MemberKind::Column => "column",
        };
        let section_bound = match &record.body.profile {
            crate::engine::member::ProfileInfo::Section { shape, .. } => {
                detailed_sagitta_bound(shape).map_err(|_| ContactError::Range)?
            }
            crate::engine::member::ProfileInfo::Rectangle { .. } => 0.0,
        };
        let mesh_bound = record
            .body
            .arc_deviation_mm
            .unwrap_or(0.0)
            .max(section_bound);
        Self::of_placed(
            record.authored.id,
            record.authored.name.clone(),
            kind,
            &record.body.solid,
            &record.authored.placement,
            mesh_bound,
        )
    }

    fn of_placed(
        id: ElementId,
        name: String,
        kind: &'static str,
        solid: &SolidMesh,
        placement: &Placement,
        mesh_bound_mm: f64,
    ) -> Result<Self, ContactError> {
        placement.validate().map_err(|_| ContactError::Range)?;
        let [x, y, z] = placement.translation_mm;
        let rotated = solid
            .clone()
            .rotated_by(placement.orientation_xyzw)
            .ok_or(ContactError::Range)?;
        let world = rotated
            .vertices()
            .iter()
            .map(|v| [v[0] + x, v[1] + y, v[2] + z])
            .collect::<Vec<_>>();
        let mesh = TriMesh::from_buffers(&world, solid.triangles())?;
        let bounds = Self::extent(&mesh);
        Ok(Self {
            id,
            name,
            kind,
            mesh,
            bounds,
            mesh_bound_mm,
        })
    }

    /// The world corner extents of a mesh.
    fn extent(mesh: &TriMesh) -> ([f64; 3], [f64; 3]) {
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for v in mesh.vertices() {
            lo[0] = lo[0].min(v[0]);
            hi[0] = hi[0].max(v[0]);
            lo[1] = lo[1].min(v[1]);
            hi[1] = hi[1].max(v[1]);
            lo[2] = lo[2].min(v[2]);
            hi[2] = hi[2].max(v[2]);
        }
        (lo, hi)
    }

    /// The cheap world-bounds overlap with another body, before the exact
    /// pair decision.
    pub fn bounds_overlap(&self, other: &Self) -> bool {
        let (alo, ahi) = (&self.bounds.0, &self.bounds.1);
        let (blo, bhi) = (&other.bounds.0, &other.bounds.1);
        (alo[0] <= bhi[0] && blo[0] <= ahi[0])
            && (alo[1] <= bhi[1] && blo[1] <= ahi[1])
            && (alo[2] <= bhi[2] && blo[2] <= ahi[2])
    }
}

/// What the clash rule concludes for one element pair.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClashState {
    /// No proven or suspected penetration: separated, or contact within the
    /// contact tolerance.
    Clear,
    /// The comparison cannot decide: the measured depth's uncertainty
    /// interval straddles the tolerance band, or a proven crossing left no
    /// measurable contained point.
    Indeterminate,
    /// Proven penetration beyond the contact tolerance, near-threshold rule
    /// applied.
    Penetrated,
}

/// The full outcome of one pair decision, reported with its tolerances.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClashOutcome {
    /// The clash conclusion.
    pub state: ClashState,
    /// The contact fact behind it: `separated`, `face_contact`,
    /// `edge_or_point_touch`, `penetrating`, or `contained` (an interior
    /// overlap without a strict surface crossing: full containment, a flush
    /// burial, or a coincident face such as a duplicate part, #741).
    pub contact: &'static str,
    /// The measured penetration depth in millimetres, when one was measured.
    pub measured_mm: Option<f64>,
    /// The mesh approximation bound the measurement read, in millimetres: the
    /// largest chord deviation among the evaluated bodies the measurement
    /// used (contract section 7.2).
    pub mesh_bound_mm: f64,
    /// The uncertainty radius of the measurement in millimetres:
    /// `max(mesh bound of the pair, t/2)`.
    pub delta_mm: f64,
    /// The contact tolerance the verdict used.
    pub tolerance_mm: f64,
}

/// The near-threshold rule of the contract (section 7.3): compare a measured
/// value against a bound whose acceptance region is `m <= bound + tolerance`,
/// with the uncertainty interval `[m - delta, m + delta]`. A decision only
/// when the interval lies strictly on one side of the region edge.
pub fn near_threshold(measured: f64, bound: f64, tolerance: Option<f64>, delta: f64) -> ClashState {
    let edge = bound + tolerance.unwrap_or(0.0);
    if measured + delta < edge {
        ClashState::Clear
    } else if measured - delta > edge {
        ClashState::Penetrated
    } else {
        ClashState::Indeterminate
    }
}

/// The near-threshold rule against a recorded permitted range with a
/// tolerance that widens both band edges (contract sections 4.1 and 7.3):
/// the acceptance region is `[min - tolerance, max + tolerance]`. `Clear`
/// reads as "within the permitted range", `Penetrated` as outside it.
///
/// A measured gap is never negative: a lower band edge at or below zero
/// cannot be straddled, because no physical value lies beyond it, so only
/// the upper edge decides there (contract section 7.3). A measured zero still
/// carries the uncertainty `delta`, so it passes `[0, max]` only when
/// `max + tolerance` exceeds `delta`; an exact contact at the weld, decided
/// by the predicates rather than measured, is judged before this rule and
/// passes `[0, max]` with any non-negative tolerance (#724).
///
/// Precondition: `min <= max`; the recorded-inputs admission refuses ranges
/// that violate it, so the rules never guess on a malformed band.
pub fn range_threshold(
    measured: f64,
    min: f64,
    max: f64,
    tolerance: f64,
    delta: f64,
) -> ClashState {
    let lo = min - tolerance;
    let hi = max + tolerance;
    if lo <= 0.0 {
        if measured + delta < hi {
            ClashState::Clear
        } else if measured - delta > hi {
            ClashState::Penetrated
        } else {
            ClashState::Indeterminate
        }
    } else if measured - delta > lo && measured + delta < hi {
        ClashState::Clear
    } else if measured - delta > hi || measured + delta < lo {
        ClashState::Penetrated
    } else {
        ClashState::Indeterminate
    }
}

/// The recorded-input types of the contract (sections 4.1 and 5): the joint
/// intent, its declared connections and the gap requirements. They are model
/// data (`beamcraft_model::record`, kind `record`), stored through explicit
/// commands and read back as plain rule input; the engine never infers them.
pub use beamcraft_model::record::{
    ConnectionKind, CornerCutRequirement, CutPurpose, DeclaredConnection, GapRequirement,
    JointIntent, UpperBound,
};

/// The weld facts the connectivity and clearance rules consume: which weld
/// element connects which two parts, its recorded leg (a model fact, never a
/// normative limit — contract section 4.4), and the authored path when the
/// weld carries one (the A06 record). Existence, size and path are plain
/// inputs; the recorded-inputs slice reads them from the store.
#[derive(Debug, Clone, PartialEq)]
pub struct WeldFact {
    /// The weld element.
    pub element: ElementId,
    /// The main part of the weld.
    pub main: ElementId,
    /// The part welded to the main part.
    pub secondary: ElementId,
    /// The recorded weld leg in millimetres (the authored size).
    pub size_mm: f64,
    /// The ordered world path the weld runs along, when authored. A weld
    /// with no path has no placed solid at the joint, so it is never locally
    /// attached (contract section 5.1).
    pub path_mm: Option<Vec<[f64; 3]>>,
}

/// One named bound of a comparison, for findings where one measurement is
/// judged against several recorded bounds (contract section 8.1).
#[derive(Debug, Clone, PartialEq)]
pub struct NamedBound {
    /// The name of the bound within the requirement (`permitted_min`,
    /// `permitted_max`).
    pub name: &'static str,
    /// The bound value in millimetres.
    pub bound_mm: f64,
    /// The tolerance that widens this entry's edge: the recorded tolerance of a
    /// bound, or the measurement uncertainty of a measured leg (`source` is
    /// `"measured"`).
    pub tolerance_mm: f64,
    /// The basis of the entry: the recorded requirement's source, or
    /// `"measured"` for a measured leg.
    pub source: &'static str,
}

/// The recorded inputs of one `check-model` run (contract sections 4-5): the
/// joint intent, the recorded gap requirements and the weld facts. A caller
/// without recorded inputs passes the default — the connectivity rules then
/// stay not applicable, and the report says so in coverage.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RecordedInputs {
    /// The declared joint intent, when one is provided.
    pub intent: Option<JointIntent>,
    /// The recorded gap requirements, one per judged weld connection.
    pub gaps: Vec<GapRequirement>,
    /// The weld facts of the model.
    pub welds: Vec<WeldFact>,
    /// The recorded corner-cut requirements, one per required rib corner.
    pub corner_cuts: Vec<CornerCutRequirement>,
}

/// One finding of the connectivity rules: the same shape the command layer
/// reports, with element ids only (the layer maps labels).
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectivityFinding {
    /// The rule token.
    pub rule: &'static str,
    /// `fail` or `unknown`.
    pub status: &'static str,
    /// The element ids the finding is about.
    pub elements: Vec<ElementId>,
    /// What was found.
    pub message: String,
    /// The basis of the result: the declared intent, the recorded
    /// requirement, or the geometric identity.
    pub basis: String,
    /// The measured value in millimetres, where the rule measures one.
    pub measured_mm: Option<f64>,
    /// The bound the measurement was compared against, where one applies.
    pub required_mm: Option<f64>,
    /// The tolerance the comparison used, where one applies.
    pub tolerance_mm: Option<f64>,
    /// The named bounds of a range comparison (contract section 8.1).
    pub bounds: Vec<NamedBound>,
    /// The mesh approximation bound the measurement read, in millimetres: the
    /// largest chord deviation among the evaluated bodies the measurement
    /// used (contract section 7.2); absent where nothing was measured.
    pub mesh_bound_mm: Option<f64>,
    /// The effective comparison tolerance the comparison applied, in
    /// millimetres: `max(mesh bound, t/2)` (contract section 7.3); absent
    /// where no comparison ran.
    pub delta_mm: Option<f64>,
    /// The inputs whose absence kept the check from deciding.
    pub missing_inputs: Vec<&'static str>,
}

/// How close two parts are, decided with the near-threshold rule against the
/// product contact tolerance (contract sections 5 and 7.3).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Proximity {
    /// The parts meet — exact surface contact, an edge or point touch, or
    /// interpenetration (which the clash rule reports separately).
    Touching,
    /// The parts are separated; the measured surface distance in millimetres
    /// is strictly outside the tolerance band.
    Separated {
        /// The measured surface-to-surface distance in millimetres.
        gap_mm: f64,
    },
    /// The measured distance straddles the tolerance band edge.
    Indeterminate {
        /// The measured surface-to-surface distance in millimetres.
        gap_mm: f64,
    },
}

/// Whether a weld is locally attached at the declared contact of its two
/// parts (contract section 5.1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Attachment {
    /// Every path point lies at the declared contact: within the product
    /// tolerance of both surfaces, or between the two facing surfaces.
    Attached,
    /// The placed path provably misses the declared contact: a weld with no
    /// path has no placed solid at the joint at all.
    NotAttached,
    /// A measured point distance straddles the tolerance band edge, or the
    /// contact decision could not be completed.
    Indeterminate,
}

/// Whether the two evaluated parts are in contact within the product
/// tolerance: the exact contact facts first, then the near-threshold rule on
/// the measured surface distance (contract section 7.3). Interpenetrating
/// parts touch — the clash rule reports the penetration separately.
pub fn proximity_verdict(
    a: &ElementBody,
    b: &ElementBody,
    tolerance_mm: f64,
) -> Result<Proximity, ContactError> {
    let report = contact::contact(&a.mesh, &b.mesh, CONTACT_LIMITS)?;
    if report.relation != Relation::Separated {
        return Ok(Proximity::Touching);
    }
    let gap = contact::surface_gap(&a.mesh, &b.mesh, CONTACT_LIMITS)?;
    let delta = a.mesh_bound_mm.max(b.mesh_bound_mm).max(tolerance_mm / 2.0);
    if gap + delta < tolerance_mm {
        Ok(Proximity::Touching)
    } else if gap - delta > tolerance_mm {
        Ok(Proximity::Separated { gap_mm: gap })
    } else {
        Ok(Proximity::Indeterminate { gap_mm: gap })
    }
}

/// The local attachment verdict of one weld against its two evaluated parts
/// (contract section 5.1): every path point must lie at the declared contact
/// — within the product tolerance of both surfaces (a tight fit), or between
/// the two facing surfaces of the parts. The second case is what lets an
/// attached weld bridge an opening of any size: the opening it spans is not
/// capped at the contact tolerance, and the gap rule judges it. A weld with
/// no (or an empty) path has no placed solid at the joint and is never
/// locally attached.
pub fn attachment_verdict(
    weld: &WeldFact,
    a: &ElementBody,
    b: &ElementBody,
    tolerance_mm: f64,
) -> Result<Attachment, ContactError> {
    let Some(path) = weld.path_mm.as_ref().filter(|p| !p.is_empty()) else {
        return Ok(Attachment::NotAttached);
    };
    let mut verdict = Attachment::Attached;
    for point in path {
        match path_point_contact(*point, a, b, tolerance_mm)? {
            PointContact::On => {}
            PointContact::Off => return Ok(Attachment::NotAttached),
            PointContact::Straddles => verdict = Attachment::Indeterminate,
        }
    }
    Ok(verdict)
}

/// Whether one weld path point sits at the declared contact of its two parts
/// (contract section 5.1): either within the product tolerance of both
/// surfaces (a tight fit), or between the two facing surfaces — outside the
/// material of both parts, with the opening it spans (the summed distances to
/// the two surfaces) agreeing within the product tolerance with the distance
/// between the nearest surface points, and those two surface points facing
/// each other — each the other's nearest point on its own part, within the
/// product tolerance. A point inside either part's material, beside the joint
/// in the air, or on one part's surface away from the joint (#724) satisfies
/// neither and is provably off.
#[derive(Debug, Clone, Copy, PartialEq)]
enum PointContact {
    On,
    Off,
    Straddles,
}

fn path_point_contact(
    point: [f64; 3],
    a: &ElementBody,
    b: &ElementBody,
    tolerance_mm: f64,
) -> Result<PointContact, ContactError> {
    // The tight fit: within the tolerance band of both surfaces.
    let mut tight = PointContact::On;
    for body in [a, b] {
        let measured = contact::point_gap(point, &body.mesh)?;
        let delta = body.mesh_bound_mm.max(tolerance_mm / 2.0);
        match touch_state(measured, tolerance_mm, delta) {
            Touch::On => {}
            Touch::Off => tight = PointContact::Off,
            Touch::Straddles => {
                if tight == PointContact::On {
                    tight = PointContact::Straddles;
                }
            }
        }
    }
    // The bridged opening: the point spans the two facing surfaces. A point
    // inside either part's material is never at the contact; on the surface
    // is outside the material, as the near-threshold band on the span below
    // still decides.
    let mut bridged = PointContact::On;
    for body in [a, b] {
        if contact::contains(&body.mesh, point, CONTACT_LIMITS)? == Some(true) {
            bridged = PointContact::Off;
        }
    }
    let (gap_a, closest_a) = contact::point_gap_closest(point, &a.mesh)?;
    let (gap_b, closest_b) = contact::point_gap_closest(point, &b.mesh)?;
    let delta = a.mesh_bound_mm.max(b.mesh_bound_mm).max(tolerance_mm / 2.0);
    let span = (gap_a + gap_b) - distance(closest_a, closest_b);
    // The two ends of the opening face each other (#724): each end is the
    // other end's nearest point on its own part. A point lying on one part's
    // surface away from the joint spans nothing — its own surface point is
    // the point itself, and the other part's nearest point lies elsewhere on
    // that surface, not across an opening — and a point on the far face of a
    // part reaches the other part through that part's own material.
    let (_, back_a) = contact::point_gap_closest(closest_b, &a.mesh)?;
    let (_, back_b) = contact::point_gap_closest(closest_a, &b.mesh)?;
    let facing = distance(back_a, closest_a).max(distance(back_b, closest_b));
    for measured in [span, facing] {
        match touch_state(measured, tolerance_mm, delta) {
            Touch::Off => bridged = PointContact::Off,
            Touch::Straddles => {
                if bridged == PointContact::On {
                    bridged = PointContact::Straddles;
                }
            }
            Touch::On => {}
        }
    }
    Ok(match (tight, bridged) {
        (PointContact::On, _) | (_, PointContact::On) => PointContact::On,
        (PointContact::Off, PointContact::Off) => PointContact::Off,
        _ => PointContact::Straddles,
    })
}

/// The Euclidean distance between two points.
fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

/// The least distance from a point to a polyline: over its segments, the
/// distance to the closest point of each segment. A path with fewer than two
/// points measures unbounded — the caller treats only validated paths as
/// poly lines (contract section 4.4).
fn polyline_distance(point: [f64; 3], path: &[[f64; 3]]) -> f64 {
    let mut best = f64::INFINITY;
    for segment in path.windows(2) {
        let &[from, to] = segment else {
            continue;
        };
        let edge = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
        let len2 = edge[0] * edge[0] + edge[1] * edge[1] + edge[2] * edge[2];
        let offset = [point[0] - from[0], point[1] - from[1], point[2] - from[2]];
        let along = if len2 <= 0.0 {
            0.0
        } else {
            let projection =
                (offset[0] * edge[0] + offset[1] * edge[1] + offset[2] * edge[2]) / len2;
            projection.clamp(0.0, 1.0)
        };
        let foot = [
            from[0] + along * edge[0],
            from[1] + along * edge[1],
            from[2] + along * edge[2],
        ];
        let gap = distance(point, foot);
        if gap < best {
            best = gap;
        }
    }
    best
}

/// Exact zero requires a surface proof for every segment, not just its ends:
/// a straight path may leave the material between two valid endpoints.
fn exact_contact_at(
    weld: &WeldFact,
    a: &ElementBody,
    b: &ElementBody,
) -> Result<bool, ContactError> {
    let Some(path) = weld.path_mm.as_ref().filter(|p| !p.is_empty()) else {
        return Ok(false);
    };
    // Share one weld's proof budget across both bodies and all path segments.
    let queries = u64::try_from(path.len())
        .unwrap_or(u64::MAX)
        .saturating_mul(2)
        .max(1);
    let limits = Limits {
        max_candidate_pairs: CONTACT_LIMITS.max_candidate_pairs / queries,
        max_exact_predicates: CONTACT_LIMITS.max_exact_predicates / queries,
        ..CONTACT_LIMITS
    };
    let Some(first) = path.first() else {
        return Ok(false);
    };
    for body in [a, b] {
        if !contact::segment_on_surface(*first, *first, &body.mesh, limits)? {
            return Ok(false);
        }
    }
    for segment in path.windows(2) {
        let [from, to] = segment else { continue };
        for body in [a, b] {
            if !contact::segment_on_surface(*from, *to, &body.mesh, limits)? {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// The largest sampled local gap of an attached weld, in millimetres: the largest
/// summed point-to-surface distance of the path points against both bodies —
/// zero where the path runs on the meeting surfaces of touching parts, the
/// air width where the weld bridges an opening. `None` for a weld without a
/// usable path.
pub fn weld_gap_mm(
    weld: &WeldFact,
    a: &ElementBody,
    b: &ElementBody,
) -> Result<Option<f64>, ContactError> {
    let Some(path) = weld.path_mm.as_ref().filter(|p| !p.is_empty()) else {
        return Ok(None);
    };
    let mut best: Option<f64> = None;
    for point in path {
        let gap = contact::point_gap(*point, &a.mesh)? + contact::point_gap(*point, &b.mesh)?;
        best = Some(match best {
            Some(current) if current >= gap => current,
            _ => gap,
        });
    }
    Ok(best)
}

/// Judge the complete path, retaining a measured witness for a failure and
/// refusing to pass a segment whose unsampled interior is not bounded.
fn whole_weld_gap(
    weld: &WeldFact,
    a: &ElementBody,
    b: &ElementBody,
    requirement: &GapRequirement,
    tolerance_mm: f64,
) -> Result<(ClashState, f64, &'static str), ContactError> {
    let path = weld.path_mm.as_ref().ok_or(ContactError::Range)?;
    let delta = a.mesh_bound_mm.max(b.mesh_bound_mm).max(tolerance_mm / 2.0);
    let lo = requirement.permitted_min_mm - requirement.tolerance_mm;
    let hi = requirement.permitted_max_mm + requirement.tolerance_mm;
    let measure = |point| -> Result<f64, ContactError> {
        Ok(contact::point_gap(point, &a.mesh)? + contact::point_gap(point, &b.mesh)?)
    };
    let judge = |gap| {
        range_threshold(
            gap,
            requirement.permitted_min_mm,
            requirement.permitted_max_mm,
            requirement.tolerance_mm,
            delta,
        )
    };
    let mut unknown = false;
    let mut reason = "indeterminate";
    let mut witness = 0.0f64;
    // Count conservative triangle visits as well as samples: a large mesh
    // must not multiply the subdivision cap into unbounded surface scans.
    let triangles = a
        .mesh
        .triangles()
        .len()
        .saturating_add(b.mesh.triangles().len())
        .max(1);
    let sample_limit = 4096usize
        .min(usize::try_from(CONTACT_LIMITS.max_candidate_pairs).unwrap_or(0) / triangles / 16);
    let mut spent = 0usize;
    if path.is_empty() || sample_limit == 0 {
        return Err(ContactError::Budget);
    }
    for point in path {
        if spent >= sample_limit {
            return Ok((ClashState::Indeterminate, witness, reason));
        }
        spent += 1;
        let gap = measure(*point)?;
        witness = witness.max(gap);
        match judge(gap) {
            ClashState::Penetrated => return Ok((ClashState::Penetrated, gap, reason)),
            ClashState::Indeterminate => unknown = true,
            ClashState::Clear => {}
        }
    }
    for segment in path.windows(2) {
        let [from, to] = segment else { continue };
        let g0 = measure(*from)?;
        let g1 = measure(*to)?;
        let mut pending = vec![(*from, *to, g0, g1, 0u8)];
        while let Some((from, to, g0, g1, depth)) = pending.pop() {
            for gap in [g0, g1] {
                witness = witness.max(gap);
                if judge(gap) == ClashState::Penetrated {
                    return Ok((ClashState::Penetrated, gap, reason));
                }
            }
            if spent >= sample_limit {
                unknown = true;
                continue;
            }
            spent += 1;
            // Each surface distance is 1-Lipschitz. Their sum is 2-Lipschitz;
            // the endpoint cones give a lower bound everywhere on the segment.
            let length = distance(from, to);
            let lower = ((g0 + g1) * 0.5 - length).max(0.0);
            let upper = (g0.max(g1) + length).min(
                contact::segment_gap_upper(from, to, &a.mesh)?
                    + contact::segment_gap_upper(from, to, &b.mesh)?,
            );
            if (lo <= 0.0 || lower - delta > lo) && upper + delta < hi {
                continue;
            }
            if depth >= 32 {
                unknown = true;
                continue;
            }
            let Some(mid) = contact::segment_midpoint(from, to, CONTACT_LIMITS)? else {
                unknown = true;
                continue;
            };
            // Additional samples are subject to the same local-attachment test.
            // An invalid or undecidable interior attachment cannot certify a gap.
            if path_point_contact(mid, a, b, tolerance_mm)? != PointContact::On {
                unknown = true;
                reason = "attachment_unvalidated";
                continue;
            }
            let gap = measure(mid)?;
            pending.push((mid, to, gap, g1, depth + 1));
            pending.push((from, mid, g0, gap, depth + 1));
        }
    }
    Ok((
        if unknown {
            ClashState::Indeterminate
        } else {
            ClashState::Clear
        },
        witness,
        reason,
    ))
}

/// The connectivity rules over the recorded inputs (contract sections 2.1
/// and 5). With no joint intent the rules are not applicable for any element
/// and no finding is produced — the report says so in coverage. A connection
/// is judged when its parts are inside the checked scope, or when a part is
/// a declared joint member the model does not carry at all — that one
/// resolves to `unknown` (`target_unresolved`) with the inputs named, never
/// a guessed verdict (contract section 4.3). A declared member that exists
/// in the model but lies outside the checked scope leaves the connection
/// unjudged, like any element outside it: a subtree check of one joint
/// member never reports findings about its other members.
pub fn connectivity_findings(
    inputs: &RecordedInputs,
    bodies: &[ElementBody],
    scope: &[ElementId],
    model_elements: &[ElementId],
    tolerance_mm: f64,
) -> Vec<ConnectivityFinding> {
    let Some(intent) = &inputs.intent else {
        return Vec::new();
    };
    let declared = |id: &ElementId| intent.scope.iter().any(|member| member == id);
    let mut findings = Vec::new();
    for connection in &intent.connections {
        // Inside the checked scope, or declared and absent from the model —
        // the ghost case the resolution below reports unresolved. A declared
        // member the model does carry but the checked scope omits is outside
        // the checked scope like any element, not unresolved.
        let judged = |id: &ElementId| {
            scope.binary_search(id).is_ok()
                || (declared(id) && model_elements.binary_search(id).is_err())
        };
        if !judged(&connection.a) || !judged(&connection.b) {
            continue;
        }
        let body_of = |id: &ElementId| bodies.iter().find(|b| &b.id == id);
        let (body_a, body_b) = match (body_of(&connection.a), body_of(&connection.b)) {
            (Some(a), Some(b)) => (a, b),
            _ => {
                findings.push(ConnectivityFinding {
                    rule: "PART_UNCONNECTED",
                    status: "unknown",
                    elements: vec![connection.a, connection.b],
                    message: format!(
                        "the declared {} connection targets elements that are not evaluated \
                         parts of the checked revision, so its connectivity stays unjudged \
                         and the requirement stays recorded",
                        connection.kind.token(),
                    ),
                    basis: "declared joint intent connection (contract 5); \
                            unresolved target (contract 4.3)"
                        .into(),
                    measured_mm: None,
                    required_mm: None,
                    tolerance_mm: None,
                    bounds: Vec::new(),
                    mesh_bound_mm: None,
                    delta_mm: None,
                    missing_inputs: vec!["target_unresolved"],
                });
                continue;
            }
        };
        // The pair's mesh approximation bound and the effective comparison
        // tolerance it feeds (contract sections 7.2 and 7.3), reported with
        // every finding whose measurement used them.
        let pair_bound = body_a.mesh_bound_mm.max(body_b.mesh_bound_mm);
        let pair_delta = pair_bound.max(tolerance_mm / 2.0);
        let proximity = match proximity_verdict(body_a, body_b, tolerance_mm) {
            Ok(proximity) => proximity,
            Err(_) => {
                findings.push(ConnectivityFinding {
                    rule: "PART_UNCONNECTED",
                    status: "unknown",
                    elements: vec![connection.a, connection.b],
                    message: "the contact decision could not be completed for the declared \
                              connection"
                        .into(),
                    basis: "declared joint intent connection (contract 5)".into(),
                    measured_mm: None,
                    required_mm: None,
                    tolerance_mm: Some(tolerance_mm),
                    bounds: Vec::new(),
                    mesh_bound_mm: None,
                    delta_mm: None,
                    missing_inputs: vec!["unresolved contact predicates"],
                });
                continue;
            }
        };
        let welds = inputs
            .welds
            .iter()
            .filter(|w| {
                (w.main == connection.a && w.secondary == connection.b)
                    || (w.main == connection.b && w.secondary == connection.a)
            })
            .collect::<Vec<_>>();
        match connection.kind {
            ConnectionKind::Weld => weld_connection(
                inputs,
                connection,
                &welds,
                proximity,
                body_a,
                body_b,
                tolerance_mm,
                &mut findings,
            ),
            ConnectionKind::Contact => match proximity {
                // The declared contact exists: the connection is satisfied.
                Proximity::Touching => {}
                Proximity::Separated { gap_mm } => findings.push(ConnectivityFinding {
                    rule: "PART_UNCONNECTED",
                    status: "fail",
                    elements: vec![connection.a, connection.b],
                    message: format!(
                        "the declared contact connection has the parts {} mm apart, beyond \
                         the contact tolerance",
                        gap_mm,
                    ),
                    basis: "declared joint intent connection (contract 5); \
                            geometric contact fact"
                        .into(),
                    measured_mm: Some(gap_mm),
                    required_mm: Some(tolerance_mm),
                    tolerance_mm: Some(tolerance_mm),
                    bounds: Vec::new(),
                    mesh_bound_mm: Some(pair_bound),
                    delta_mm: Some(pair_delta),
                    missing_inputs: Vec::new(),
                }),
                Proximity::Indeterminate { gap_mm } => findings.push(ConnectivityFinding {
                    rule: "PART_UNCONNECTED",
                    status: "unknown",
                    elements: vec![connection.a, connection.b],
                    message: format!(
                        "the measured distance {} mm sits at the contact tolerance band \
                         edge, so the declared contact stays undecidable",
                        gap_mm,
                    ),
                    basis: "declared joint intent connection (contract 5); \
                            geometric contact fact"
                        .into(),
                    measured_mm: Some(gap_mm),
                    required_mm: Some(tolerance_mm),
                    tolerance_mm: Some(tolerance_mm),
                    bounds: Vec::new(),
                    mesh_bound_mm: Some(pair_bound),
                    delta_mm: Some(pair_delta),
                    missing_inputs: vec!["indeterminate"],
                }),
            },
        }
    }
    findings
}

/// The rules of one declared weld connection: `PART_UNCONNECTED` when no
/// weld element connects the parts, `CONTACT_WITHOUT_CONNECTION` when the
/// parts touch but the declared weld is missing, and `WELD_WITHOUT_CONTACT`
/// for each existing weld's judged or unjudgeable local gap.
#[allow(clippy::too_many_arguments)]
fn weld_connection(
    inputs: &RecordedInputs,
    connection: &DeclaredConnection,
    welds: &[&WeldFact],
    proximity: Proximity,
    body_a: &ElementBody,
    body_b: &ElementBody,
    tolerance_mm: f64,
    findings: &mut Vec<ConnectivityFinding>,
) {
    if welds.is_empty() {
        findings.push(ConnectivityFinding {
            rule: "PART_UNCONNECTED",
            status: "fail",
            elements: vec![connection.a, connection.b],
            message: "the declared weld connection has no weld element connecting the parts".into(),
            basis: "declared joint intent connection (contract 5)".into(),
            measured_mm: None,
            required_mm: None,
            tolerance_mm: None,
            bounds: Vec::new(),
            mesh_bound_mm: None,
            delta_mm: None,
            missing_inputs: Vec::new(),
        });
        if proximity == Proximity::Touching {
            findings.push(ConnectivityFinding {
                rule: "CONTACT_WITHOUT_CONNECTION",
                status: "fail",
                elements: vec![connection.a, connection.b],
                message: "the parts touch but the declared weld connection has no weld at \
                          that contact; contact alone never proves a weld is needed, this \
                          one is declared"
                    .into(),
                basis: "declared joint intent connection (contract 5); \
                        geometric contact fact"
                    .into(),
                measured_mm: None,
                required_mm: None,
                tolerance_mm: Some(tolerance_mm),
                bounds: Vec::new(),
                mesh_bound_mm: None,
                delta_mm: None,
                missing_inputs: Vec::new(),
            });
        }
        return;
    }
    // The weld exists: the connection is satisfied; the local gap question
    // is judged per weld against the recorded requirement, and stays
    // unknown where only a normative rule would decide (contract 3.3).
    let Some(requirement) = inputs.gaps.iter().find(|g| {
        (g.a == connection.a && g.b == connection.b) || (g.a == connection.b && g.b == connection.a)
    }) else {
        findings.push(ConnectivityFinding {
            rule: "WELD_WITHOUT_CONTACT",
            status: "unknown",
            elements: vec![connection.a, connection.b],
            message: "the declared weld connection has a weld but neither a recorded \
                      permitted gap requirement nor a supported normative gap rule, so \
                      the local gap stays unjudged"
                .into(),
            basis: "declared joint intent connection (contract 5); no supported rule \
                    (contract 3.3)"
                .into(),
            measured_mm: None,
            required_mm: None,
            tolerance_mm: None,
            bounds: Vec::new(),
            mesh_bound_mm: None,
            delta_mm: None,
            missing_inputs: vec!["no_supported_rule"],
        });
        return;
    };
    for weld in welds {
        let attached = match attachment_verdict(weld, body_a, body_b, tolerance_mm) {
            Ok(attached) => attached,
            Err(_) => Attachment::Indeterminate,
        };
        if attached != Attachment::Attached {
            findings.push(ConnectivityFinding {
                rule: "WELD_WITHOUT_CONTACT",
                status: "unknown",
                elements: vec![connection.a, connection.b],
                message: "the weld is not locally attached at the declared contact of its \
                          parts, so its local gap stays unjudged"
                    .into(),
                basis: "declared joint intent connection (contract 5); local weld \
                        attachment (contract 5.1)"
                    .into(),
                measured_mm: None,
                required_mm: None,
                tolerance_mm: None,
                bounds: Vec::new(),
                mesh_bound_mm: None,
                delta_mm: None,
                missing_inputs: vec!["attachment_unvalidated"],
            });
            continue;
        }
        // An exact contact at the weld (#724): every segment of the path is
        // proven to lie on both parts' surfaces by exact coverage, decided
        // by the exact predicates — not merely its endpoints, since a
        // straight path may leave the material between two valid points.
        // The opening there is exactly zero — a fact the
        // predicates decided, so no physical tolerance band applies to it —
        // and the recorded range passes whenever its lower edge
        // `min - tolerance` is at or below zero, whatever its maximum and
        // tolerance. A recorded minimum above that fails: the opening the
        // owner requires is absent. Anything short of the exact fact (a path
        // in a thin opening, a stepped joint) keeps the measured rule below.
        if matches!(exact_contact_at(weld, body_a, body_b), Ok(true)) {
            let lower_edge = requirement.permitted_min_mm - requirement.tolerance_mm;
            if lower_edge > 0.0 {
                findings.push(ConnectivityFinding {
                    rule: "WELD_WITHOUT_CONTACT",
                    status: "fail",
                    elements: vec![connection.a, connection.b],
                    message: "the parts meet exactly along the weld path, so the local gap \
                              is 0 mm and stays below the permitted minimum"
                        .into(),
                    basis: "recorded gap requirement (contract 4.1); exact contact at the \
                            weld (contract 7.3)"
                        .into(),
                    measured_mm: Some(0.0),
                    required_mm: None,
                    tolerance_mm: Some(requirement.tolerance_mm),
                    bounds: vec![
                        NamedBound {
                            name: "permitted_min",
                            bound_mm: requirement.permitted_min_mm,
                            tolerance_mm: requirement.tolerance_mm,
                            source: "owner",
                        },
                        NamedBound {
                            name: "permitted_max",
                            bound_mm: requirement.permitted_max_mm,
                            tolerance_mm: requirement.tolerance_mm,
                            source: "owner",
                        },
                    ],
                    mesh_bound_mm: Some(body_a.mesh_bound_mm.max(body_b.mesh_bound_mm)),
                    delta_mm: Some(0.0),
                    missing_inputs: Vec::new(),
                });
            }
            continue;
        }
        let (state, gap, reason) =
            match whole_weld_gap(weld, body_a, body_b, requirement, tolerance_mm) {
                Ok(outcome) => outcome,
                Err(_) => {
                    findings.push(ConnectivityFinding {
                        rule: "WELD_WITHOUT_CONTACT",
                        status: "unknown",
                        elements: vec![connection.a, connection.b],
                        message: "the gap decision could not be completed for this weld".into(),
                        basis: "recorded gap requirement (contract 4.1)".into(),
                        measured_mm: None,
                        required_mm: None,
                        tolerance_mm: Some(requirement.tolerance_mm),
                        bounds: Vec::new(),
                        mesh_bound_mm: None,
                        delta_mm: None,
                        missing_inputs: vec!["unresolved contact predicates"],
                    });
                    continue;
                }
            };
        let bound = body_a.mesh_bound_mm.max(body_b.mesh_bound_mm);
        let delta = bound.max(tolerance_mm / 2.0);
        match state {
            // A permissible weld gap passes its applicable gap rule.
            ClashState::Clear => {}
            ClashState::Indeterminate => findings.push(ConnectivityFinding {
                rule: "WELD_WITHOUT_CONTACT",
                status: "unknown",
                elements: vec![connection.a, connection.b],
                message: if reason == "attachment_unvalidated" {
                    "the weld path leaves its validated attachment between authored points, so its local gap stays unjudged".into()
                } else { format!(
                    "the whole weld gap could not be bounded inside the permitted range (largest sample {} mm)", gap,
                ) },
                basis: "recorded gap requirement (contract 4.1)".into(),
                measured_mm: (reason != "attachment_unvalidated").then_some(gap),
                required_mm: None,
                tolerance_mm: Some(requirement.tolerance_mm),
                bounds: vec![
                    NamedBound {
                        name: "permitted_min",
                        bound_mm: requirement.permitted_min_mm,
                        tolerance_mm: requirement.tolerance_mm,
                        source: "owner",
                    },
                    NamedBound {
                        name: "permitted_max",
                        bound_mm: requirement.permitted_max_mm,
                        tolerance_mm: requirement.tolerance_mm,
                        source: "owner",
                    },
                ],
                mesh_bound_mm: Some(bound),
                delta_mm: Some(delta),
                missing_inputs: vec![reason],
            }),
            ClashState::Penetrated => {
                let side = if gap - delta > requirement.permitted_max_mm + requirement.tolerance_mm
                {
                    "exceeds the permitted maximum"
                } else {
                    "stays below the permitted minimum"
                };
                findings.push(ConnectivityFinding {
                    rule: "WELD_WITHOUT_CONTACT",
                    status: "fail",
                    elements: vec![connection.a, connection.b],
                    message: format!("the measured local gap {} mm {}", gap, side),
                    basis: "recorded gap requirement (contract 4.1)".into(),
                    measured_mm: Some(gap),
                    required_mm: None,
                    tolerance_mm: Some(requirement.tolerance_mm),
                    bounds: vec![
                        NamedBound {
                            name: "permitted_min",
                            bound_mm: requirement.permitted_min_mm,
                            tolerance_mm: requirement.tolerance_mm,
                            source: "owner",
                        },
                        NamedBound {
                            name: "permitted_max",
                            bound_mm: requirement.permitted_max_mm,
                            tolerance_mm: requirement.tolerance_mm,
                            source: "owner",
                        },
                    ],
                    mesh_bound_mm: Some(bound),
                    delta_mm: Some(delta),
                    missing_inputs: Vec::new(),
                });
            }
        }
    }
}

/// One finding of the corner-cut rules: the shared finding shape plus the derived
/// corner. The corner point is a computed value (contract section 4.2): re-derived on
/// every run from the final evaluated geometry, it anchors the fix hints back to the
/// authored contour, and it never replaces a recorded input.
#[derive(Debug, Clone, PartialEq)]
pub struct CornerCutFinding {
    /// The finding itself: rule, status, elements, message, basis, measurements.
    pub finding: ConnectivityFinding,
    /// The derived corner point in world coordinates, when the derivation resolved
    /// exactly one candidate corner (contract sections 4.1 and 4.3).
    pub corner_point_mm: Option<[f64; 3]>,
    /// True for the dependent weld-clearance finding of contract section 4.4 (#723),
    /// false for the size findings of the recorded requirement. A clearance finding
    /// never carries a fix: a larger cut and a smaller weld both repair it, and
    /// geometry does not choose between them (contract section 9).
    pub clearance: bool,
}

/// Whether a measured distance puts its point strictly inside the touching band (the
/// product tolerance), strictly outside it, or at the band edge — the near-threshold
/// rule of contract section 7.3, applied to one contact question.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Touch {
    On,
    Off,
    Straddles,
}

fn touch_state(gap: f64, tolerance_mm: f64, delta: f64) -> Touch {
    if gap + delta < tolerance_mm {
        Touch::On
    } else if gap - delta > tolerance_mm {
        Touch::Off
    } else {
        Touch::Straddles
    }
}

/// What the corner derivation of one requirement concludes (contract section 4.3).
enum Corner {
    /// Exactly one candidate corner adjacent to both neighbours, in world coordinates.
    Resolved([f64; 3]),
    /// A mesh vertex sits at a contact band edge, so the candidate set cannot be
    /// established.
    Ambiguous,
    /// Zero candidate corners.
    Absent,
    /// Several candidate corners; the harness never picks among them.
    Several,
}

/// One authored contour edge's supporting line, in the rib plate's local 2D
/// contour frame.
#[derive(Clone, Copy)]
struct ContourLine {
    /// A point on the line, local contour coordinates.
    point: [f64; 2],
    /// The unit direction of the line.
    direction: [f64; 2],
}

impl ContourLine {
    /// The supporting line of the contour edge from `from` to `to`, or `None`
    /// for a degenerate edge.
    fn of(from: [f64; 2], to: [f64; 2]) -> Option<ContourLine> {
        let (dx, dy) = (to[0] - from[0], to[1] - from[1]);
        let len = dx.hypot(dy);
        if len <= 1e-6 || !len.is_finite() {
            return None;
        }
        Some(ContourLine {
            point: from,
            direction: [dx / len, dy / len],
        })
    }

    /// Whether the local point lies on the line. The mesh vertices of a plate
    /// wall lie exactly on their contour edge, so the margin only absorbs the
    /// placement frame round-trip.
    fn holds(&self, point: [f64; 2]) -> bool {
        let off = (point[0] - self.point[0]) * self.direction[1]
            - (point[1] - self.point[1]) * self.direction[0];
        off.abs() <= 1e-6 && off.is_finite()
    }

    /// The intersection with another line, when they are not parallel.
    fn intersection(&self, other: &ContourLine) -> Option<[f64; 2]> {
        let cross = self.direction[0] * other.direction[1] - self.direction[1] * other.direction[0];
        if cross.abs() <= 1e-12 {
            return None;
        }
        let (dx, dy) = (
            other.point[0] - self.point[0],
            other.point[1] - self.point[1],
        );
        let along = (dx * other.direction[1] - dy * other.direction[0]) / cross;
        Some([
            self.point[0] + along * self.direction[0],
            self.point[1] + along * self.direction[1],
        ])
    }
}

/// What the corner derivation of one requirement concludes (contract section 4.3):
/// the corner verdict plus, when the corner resolved, the measured setback legs.
struct CornerDerivation {
    corner: Corner,
    /// The per-neighbour setbacks: the least distance, in the local contour
    /// frame, from the derived corner to an evaluated vertex provably touching
    /// that neighbour and lying on one of its contact lines. The largest
    /// setback is the larger of the two.
    setback_a: Option<f64>,
    setback_b: Option<f64>,
    /// The admitted leg: the largest `k` for which the triangle with one vertex
    /// at the derived corner and the other two at distance `k` along each
    /// neighbour's contact line contains no rib material — the least, over the
    /// evaluated mesh vertices inside the corner's forward quadrant, of the
    /// sum of the projections onto the two contact lines, scaled by the corner
    /// angle (`#725`'s measure).
    admitted: Option<f64>,
}

/// Derives the rib's corner adjacent to both recorded neighbours from the final
/// evaluated geometry and the two neighbour bodies (contract sections 4.1 and 4.3,
/// as amended for #725): an authored contour edge is a contact line of the
/// neighbour its wall provably touches along a stretch in the final body (two
/// distinct touching contour points at least, #746) — an authored vertex is
/// never read, so a cut authored as contour points or as a plane cut is judged
/// like a parameterized treatment of the same final shape — and every
/// intersection of one line touching each neighbour, itself within the contact
/// tolerance of both neighbour surfaces, is a candidate corner. Exactly one
/// candidate is judged; zero or several are unresolved and nothing picks among
/// them; a mesh vertex at a contact band edge keeps the candidate set from being
/// established.
///
/// `touches` carries, per evaluated rib mesh vertex, its world position and its
/// contact states against the two neighbours, as gathered by the caller's single
/// pass over the mesh (one `point_gap` decision per vertex and neighbour, shared
/// by the derivation and the measurement).
fn derive_corner(
    plate: &PlateRecord,
    touches: &[(Touch, Touch, [f64; 3])],
    a: &ElementBody,
    b: &ElementBody,
    tolerance_mm: f64,
) -> Result<CornerDerivation, ContactError> {
    let delta = a.mesh_bound_mm.max(b.mesh_bound_mm).max(tolerance_mm / 2.0);
    let to_frame = |world: [f64; 3]| -> [f64; 2] {
        let local = crate::engine::transform::placement_to_local(&plate.authored.placement, world);
        [local[0], local[1]]
    };
    // The contact sets in the local contour frame; the thickness coordinate
    // drops out, so the two depth faces of one wall collapse onto the same line.
    let contour: Vec<[f64; 2]> = plate
        .authored
        .contour_points_mm
        .iter()
        .map(|p| [p[0], p[1]])
        .collect();
    // The contour is a closed polygon: each point paired with its successor, the
    // last with the first.
    let contour_lines: Vec<ContourLine> = contour
        .iter()
        .zip(contour.iter().skip(1).chain(contour.first()))
        .filter_map(|(from, to)| ContourLine::of(*from, *to))
        .collect();
    let mut all_local: Vec<[f64; 2]> = Vec::with_capacity(touches.len());
    let mut on_a: Vec<[f64; 2]> = Vec::new();
    let mut on_b: Vec<[f64; 2]> = Vec::new();
    for (state_a, state_b, world) in touches {
        let local = to_frame(*world);
        all_local.push(local);
        // A wall vertex at a contact band edge could or could not be a second
        // candidate corner: the candidate set cannot be established. A vertex on
        // no contour line — a faceted arc vertex between the tangent points —
        // never holds a line, measures a setback, or forms a corner, so its
        // near-threshold state poisons nothing.
        let on_a_line = contour_lines.iter().any(|line| line.holds(local));
        match (*state_a, *state_b) {
            (Touch::Straddles, _) | (_, Touch::Straddles) if on_a_line => {
                return Ok(CornerDerivation {
                    corner: Corner::Ambiguous,
                    setback_a: None,
                    setback_b: None,
                    admitted: None,
                });
            }
            // A vertex provably on both neighbours belongs to both contact sets.
            (Touch::On, Touch::On) => {
                on_a.push(local);
                on_b.push(local);
            }
            (Touch::On, _) => on_a.push(local),
            (_, Touch::On) => on_b.push(local),
            // A straddling state off every contour line, or both off.
            _ => {}
        }
    }
    // The contact lines: authored contour edge lines whose wall lies on the
    // neighbour along a stretch, not at a point — at least two distinct contour
    // points of the line provably touch it. A line holding a single touching
    // point is no contact wall: a chamfer authored as contour points touches the
    // base at one end and the column at the other, and its extension met the
    // extension of the rib's top edge at a phantom point 0.5 mm off an I-section
    // web, which stalled the whole requirement as indeterminate (#746). The two
    // thickness faces of one wall collapse onto the same contour point, so they
    // count once.
    let contact_lines = |on: &[[f64; 2]]| -> Vec<ContourLine> {
        contour_lines
            .iter()
            .copied()
            .filter(|line| {
                let mut distinct: Vec<[f64; 2]> = Vec::new();
                for point in on.iter().filter(|p| line.holds(**p)) {
                    if !distinct
                        .iter()
                        .any(|seen| (seen[0] - point[0]).hypot(seen[1] - point[1]) <= 1e-6)
                    {
                        distinct.push(*point);
                    }
                }
                distinct.len() >= 2
            })
            .collect()
    };
    let lines_a = contact_lines(&on_a);
    let lines_b = contact_lines(&on_b);
    // Candidate corners: intersections of one line touching each neighbour,
    // kept where both neighbour surfaces still sandwich the point — the ideal
    // corner of the joint, not a phantom of extended walls. Each candidate
    // carries the line pair that produced it: the corner's two contact lines,
    // the axes of the admitted-leg measure.
    let mut candidates: Vec<([f64; 2], ContourLine, ContourLine)> = Vec::new();
    for line_a in &lines_a {
        for line_b in &lines_b {
            let Some(point) = line_a.intersection(line_b) else {
                continue;
            };
            let world = crate::engine::transform::placement_to_world(
                &plate.authored.placement,
                [point[0], point[1], 0.0],
            );
            let gap_a = contact::point_gap(world, &a.mesh)?;
            let gap_b = contact::point_gap(world, &b.mesh)?;
            match (
                touch_state(gap_a, tolerance_mm, delta),
                touch_state(gap_b, tolerance_mm, delta),
            ) {
                (Touch::On, Touch::On) => {
                    // The same ideal corner can arise from several line pairs
                    // (a vertex dual-touching both neighbours puts its wall
                    // lines on both sides): one candidate, not several.
                    let fresh =
                        !candidates
                            .iter()
                            .any(|c: &([f64; 2], ContourLine, ContourLine)| {
                                (c.0[0] - point[0]).hypot(c.0[1] - point[1]) <= 1e-6
                            });
                    if fresh {
                        candidates.push((point, *line_a, *line_b));
                    }
                }
                (Touch::Straddles, _) | (_, Touch::Straddles) => {
                    return Ok(CornerDerivation {
                        corner: Corner::Ambiguous,
                        setback_a: None,
                        setback_b: None,
                        admitted: None,
                    });
                }
                _ => {}
            }
        }
    }
    let [(corner, line_a, line_b)] = candidates.as_slice() else {
        return Ok(CornerDerivation {
            corner: if candidates.is_empty() {
                Corner::Absent
            } else {
                Corner::Several
            },
            setback_a: None,
            setback_b: None,
            admitted: None,
        });
    };
    // Orient each contact line from the corner towards the material: the
    // contour edge that produced it may run either way (the rib's back edge
    // runs from the far vertex towards the corner), and the projections below
    // measure distances into the rib.
    let orient = |line: &ContourLine, on: &[[f64; 2]]| -> ContourLine {
        for point in on {
            if line.holds(*point) {
                let along = (point[0] - corner[0]) * line.direction[0]
                    + (point[1] - corner[1]) * line.direction[1];
                if along < 0.0 {
                    return ContourLine {
                        point: line.point,
                        direction: [-line.direction[0], -line.direction[1]],
                    };
                }
                return *line;
            }
        }
        *line
    };
    let line_a = orient(line_a, &on_a);
    let line_b = orient(line_b, &on_b);
    // The setbacks: the least distance from the corner to an evaluated vertex
    // provably touching the neighbour on one of its contact lines — zero where
    // the rib still reaches the corner itself, the setback (a chamfer leg, a
    // rounding's tangent setback) where a treatment or cut removed it. A
    // rounding's tangent setbacks are exact: the tangent points sit on the
    // authored edge lines, while the arc facets between them lie on no line, so
    // the measure never follows the arc faceting.
    let setback = |on: &[[f64; 2]], lines: &[ContourLine]| -> Option<f64> {
        let mut least: Option<f64> = None;
        for point in on {
            if lines.iter().any(|line| line.holds(*point)) {
                let distance = (point[0] - corner[0]).hypot(point[1] - corner[1]);
                least = Some(least.map_or(distance, |current: f64| current.min(distance)));
            }
        }
        least
    };
    // The admitted leg (#725): the largest `k` for which the triangle with one
    // vertex at the corner and the other two at distance `k` along each contact
    // line contains no rib material. The triangle's hypotenuse is the level set
    // of the projection sum onto the two lines, so the bound is the least
    // projection sum over the body's vertices inside the corner's forward
    // quadrant — over all evaluated vertices, because the binding point of a
    // rounding is an arc facet vertex that touches neither neighbour — scaled
    // by the corner angle (1 + cos of the turn between the lines; 1 at a right
    // angle).
    let cos = line_a.direction[0] * line_b.direction[0] + line_a.direction[1] * line_b.direction[1];
    let admitted = if 1.0 + cos <= 1e-9 {
        None
    } else {
        let mut least: Option<f64> = None;
        for point in &all_local {
            let along_a = (point[0] - corner[0]) * line_a.direction[0]
                + (point[1] - corner[1]) * line_a.direction[1];
            let along_b = (point[0] - corner[0]) * line_b.direction[0]
                + (point[1] - corner[1]) * line_b.direction[1];
            if along_a < -1e-9 || along_b < -1e-9 {
                continue;
            }
            let sum = along_a + along_b;
            least = Some(least.map_or(sum, |current: f64| current.min(sum)));
        }
        least.map(|least| (least / (1.0 + cos)).max(0.0))
    };
    Ok(CornerDerivation {
        corner: Corner::Resolved(crate::engine::transform::placement_to_world(
            &plate.authored.placement,
            [corner[0], corner[1], 0.0],
        )),
        setback_a: setback(&on_a, &lines_a),
        setback_b: setback(&on_b, &lines_b),
        admitted,
    })
}

/// The corner-cut rules over the recorded requirements (contract sections 2.1, 4.1 and
/// 7.3). The corner itself is never stored: each run re-derives the rib's corner
/// adjacent to both recorded neighbours from the final evaluated geometry and the two
/// neighbour bodies (#725) — the authored contour edge lines whose walls provably
/// touch a neighbour along a stretch in the final body (two distinct touching
/// contour points at least, #746) are that neighbour's contact lines, and every
/// intersection of one line touching each neighbour, itself within the contact
/// tolerance of both neighbour surfaces, is a candidate corner. No authored vertex is
/// read, so a cut authored as contour points or as a plane cut is judged like a
/// parameterized treatment producing the same final shape. Zero or several candidate
/// corners are `unknown` (`target_unresolved`), never a guessed pick (contract
/// section 4.3); a mesh vertex at a contact band edge keeps the candidate set from
/// being established (`indeterminate`).
///
/// The cut is measured on the final body by two values taken from the rib's
/// evaluated mesh over its full thickness (#725's measure):
///
/// - the **admitted leg** — the largest `k` for which the triangle with one
///   vertex at the derived corner and the other two at distance `k` along each
///   neighbour's contact line contains no rib material: the smaller leg of a
///   chamfer, `R·(2 − √2)` for a rounding of radius `R` at a right angle;
/// - the **largest setback** — the larger of the two distances from the corner
///   to where the rib's final body first meets each neighbour: the larger leg
///   of a chamfer, `R` for a rounding. A rounding's tangent setbacks are
///   exact: the tangent points sit on the authored edge lines and the arc
///   facets between them lie on no line, so no value follows the arc faceting.
///
/// The admitted leg goes against the acceptance band `[required − tolerance,
/// upper edge]`'s lower edge — determinately below is `CORNER_NOT_CUT` with
/// `measured_mm` the admitted leg — and the largest setback against its upper
/// edge, where the recorded standalone `upper_bound` replaces the upper edge —
/// determinately above is `CORNER_OVERCUT` with `measured_mm` the largest
/// setback; both findings are reported when both hold, and an interval
/// straddling a band edge stays `unknown` (`indeterminate`). Both values are
/// reported as the named `admitted_leg` and `largest_setback` entries of the
/// finding's `bounds` (contract section 8.1). With no recorded corner-cut
/// requirement the rules are not applicable for any element and the report
/// says so in coverage.
pub fn corner_cut_findings(
    inputs: &RecordedInputs,
    bodies: &[ElementBody],
    plates: &[PlateRecord],
    scope: &[ElementId],
    model_elements: &[ElementId],
    tolerance_mm: f64,
) -> Vec<CornerCutFinding> {
    let mut findings = Vec::new();
    for requirement in &inputs.corner_cuts {
        let named = [
            requirement.target,
            requirement.neighbours[0],
            requirement.neighbours[1],
        ];
        let push_unknown = |status_reason: &'static str,
                            message: String,
                            corner: Option<[f64; 3]>,
                            findings: &mut Vec<CornerCutFinding>| {
            findings.push(CornerCutFinding {
                finding: ConnectivityFinding {
                    rule: "CORNER_NOT_CUT",
                    status: "unknown",
                    elements: named.to_vec(),
                    message,
                    basis: "recorded corner-cut requirement (contract 4.1); corner \
                            derivation (contract 4.3)"
                        .into(),
                    measured_mm: None,
                    required_mm: Some(requirement.required_mm),
                    tolerance_mm: Some(requirement.tolerance_mm),
                    bounds: Vec::new(),
                    mesh_bound_mm: None,
                    delta_mm: None,
                    missing_inputs: vec![status_reason],
                },
                corner_point_mm: corner,
                clearance: false,
            });
        };
        // The judgement set: the named elements inside the checked scope, plus the
        // ghost case — a named element absent from the model altogether is judged as
        // unresolved, while one the model carries but the scope omits is simply outside
        // the checked scope, like any element of a subtree check.
        let judged = |id: &ElementId| {
            scope.binary_search(id).is_ok()
                || (id == &requirement.target && model_elements.binary_search(id).is_err())
                || requirement
                    .neighbours
                    .iter()
                    .any(|n| n == id && model_elements.binary_search(n).is_err())
        };
        if !named.iter().all(judged) {
            continue;
        }
        let body_of = |id: &ElementId| bodies.iter().find(|b| &b.id == id);
        let (rib, a, b) = match (body_of(&named[0]), body_of(&named[1]), body_of(&named[2])) {
            (Some(rib), Some(a), Some(b)) => (rib, a, b),
            _ => {
                push_unknown(
                    "target_unresolved",
                    "the corner-cut requirement targets elements that are not evaluated \
                     parts of the checked revision, so the corner stays unjudged and the \
                     requirement stays recorded"
                        .into(),
                    None,
                    &mut findings,
                );
                continue;
            }
        };
        // The rib plate frame: the derivation works in the local contour frame, and
        // the fix hint maps the derived corner back into it. A rib that is not an
        // authored plate carries no such frame and stays unresolved (contract 7.1).
        let Some(plate) = plates.iter().find(|p| p.authored.id == rib.id) else {
            push_unknown(
                "target_unresolved",
                "the corner derivation works in the authored plate contour frame, and \
                 the required rib carries none, so the corner stays unjudged and the \
                 requirement stays recorded"
                    .into(),
                None,
                &mut findings,
            );
            continue;
        };
        // One pass over the final evaluated body: per mesh vertex, the contact
        // decisions against the two neighbours feed the corner derivation, which
        // also measures the two legs (contract 7.1).
        let bound = rib.mesh_bound_mm.max(a.mesh_bound_mm).max(b.mesh_bound_mm);
        let delta = bound.max(tolerance_mm / 2.0);
        let upper_edge = match &requirement.upper_bound {
            Some(bound) => bound.max_mm + bound.tolerance_mm,
            None => requirement.required_mm + requirement.tolerance_mm,
        };
        let mut touches: Vec<(Touch, Touch, [f64; 3])> = Vec::new();
        // One requirement's measurement failure is that requirement's unknown:
        // the remaining recorded requirements are still judged.
        let mut unmeasured = false;
        for vertex in rib.mesh.vertices() {
            let gap_a = match contact::point_gap(*vertex, &a.mesh) {
                Ok(gap) => gap,
                Err(_) => {
                    push_unknown(
                        "unresolved contact predicates",
                        "the corner derivation and leg measurement could not complete \
                         their contact decisions against the recorded neighbours"
                            .into(),
                        None,
                        &mut findings,
                    );
                    unmeasured = true;
                    break;
                }
            };
            let gap_b = match contact::point_gap(*vertex, &b.mesh) {
                Ok(gap) => gap,
                Err(_) => {
                    push_unknown(
                        "unresolved contact predicates",
                        "the corner derivation and leg measurement could not complete \
                         their contact decisions against the recorded neighbours"
                            .into(),
                        None,
                        &mut findings,
                    );
                    unmeasured = true;
                    break;
                }
            };
            touches.push((
                touch_state(gap_a, tolerance_mm, delta),
                touch_state(gap_b, tolerance_mm, delta),
                *vertex,
            ));
        }
        if unmeasured {
            continue;
        }
        let derivation = match derive_corner(plate, &touches, a, b, tolerance_mm) {
            Ok(derivation) => derivation,
            Err(_) => {
                push_unknown(
                    "unresolved contact predicates",
                    "the corner derivation could not complete its contact decisions \
                     against the recorded neighbours"
                        .into(),
                    None,
                    &mut findings,
                );
                continue;
            }
        };
        let corner = match derivation.corner {
            Corner::Resolved(corner) => corner,
            Corner::Ambiguous => {
                push_unknown(
                    "indeterminate",
                    "a rib vertex sits at a neighbour's contact band edge, so the \
                     corner candidate set cannot be established"
                        .into(),
                    None,
                    &mut findings,
                );
                continue;
            }
            Corner::Absent => {
                push_unknown(
                    "target_unresolved",
                    "the corner derivation finds no corner of the rib adjacent to \
                     both recorded neighbours, so the requirement stays recorded and \
                     nothing auto-retargets it"
                        .into(),
                    None,
                    &mut findings,
                );
                continue;
            }
            Corner::Several => {
                push_unknown(
                    "target_unresolved",
                    "the corner derivation finds several candidate corners adjacent \
                     to both recorded neighbours, so the requirement stays unresolved \
                     and nothing picks among them"
                        .into(),
                    None,
                    &mut findings,
                );
                continue;
            }
        };
        let setback_a = derivation.setback_a;
        let setback_b = derivation.setback_b;
        let basis: String = "recorded corner-cut requirement (contract 4.1); final-body \
                     admitted-leg and largest-setback measurement (contract 7.1)"
            .into();
        let mut bounds = vec![NamedBound {
            name: "required",
            bound_mm: requirement.required_mm,
            tolerance_mm: requirement.tolerance_mm,
            source: "owner",
        }];
        if let Some(bound) = &requirement.upper_bound {
            bounds.push(NamedBound {
                name: "upper_bound",
                bound_mm: bound.max_mm,
                tolerance_mm: bound.tolerance_mm,
                source: "owner",
            });
        }
        // Both measured values, named beside the recorded bounds (#725, contract
        // 8.1); an entry's tolerance is its measurement uncertainty, not a
        // recorded tolerance.
        let measurable = match (derivation.admitted, setback_a, setback_b) {
            (Some(admitted), Some(setback_a), Some(setback_b)) => {
                bounds.push(NamedBound {
                    name: "admitted_leg",
                    bound_mm: admitted,
                    tolerance_mm: delta,
                    source: "measured",
                });
                bounds.push(NamedBound {
                    name: "largest_setback",
                    bound_mm: setback_a.max(setback_b),
                    tolerance_mm: delta,
                    source: "measured",
                });
                Some((admitted, setback_a.max(setback_b)))
            }
            _ => None,
        };
        let finish = |bounds: Vec<NamedBound>,
                      rule: &'static str,
                      status: &'static str,
                      measured: Option<f64>,
                      message: String,
                      missing: Vec<&'static str>,
                      findings: &mut Vec<CornerCutFinding>| {
            findings.push(CornerCutFinding {
                finding: ConnectivityFinding {
                    rule,
                    status,
                    elements: named.to_vec(),
                    message,
                    basis: basis.clone(),
                    measured_mm: measured,
                    required_mm: Some(requirement.required_mm),
                    tolerance_mm: Some(requirement.tolerance_mm),
                    bounds,
                    mesh_bound_mm: Some(bound),
                    delta_mm: Some(delta),
                    missing_inputs: missing,
                },
                corner_point_mm: Some(corner),
                clearance: false,
            });
        };
        let (admitted, largest_setback) = match measurable {
            Some(values) => values,
            None => {
                finish(
                    bounds,
                    "CORNER_NOT_CUT",
                    "unknown",
                    None,
                    "the rib does not provably touch both recorded neighbours at the \
                     derived corner, so the cut cannot be measured"
                        .into(),
                    vec!["indeterminate"],
                    &mut findings,
                );
                continue;
            }
        };
        let lower_edge = requirement.required_mm - requirement.tolerance_mm;
        // The admitted leg against the band's lower edge, the largest setback
        // against its upper edge (#725): each value judged independently, and
        // both findings reported when both hold.
        let admitted_fails = admitted + delta < lower_edge;
        let admitted_straddles = !admitted_fails && admitted - delta <= lower_edge;
        let setback_exceeds = largest_setback - delta > upper_edge;
        let setback_straddles = !setback_exceeds && largest_setback + delta >= upper_edge;
        if admitted_fails {
            finish(
                bounds.clone(),
                "CORNER_NOT_CUT",
                "fail",
                Some(admitted),
                format!(
                    "the admitted leg {} mm stays below the required {} mm acceptance \
                     band ({:?} purpose), so the corner provides too little clearance",
                    admitted,
                    requirement.required_mm,
                    requirement.purpose.token(),
                ),
                Vec::new(),
                &mut findings,
            );
        } else if admitted_straddles {
            finish(
                bounds.clone(),
                "CORNER_NOT_CUT",
                "unknown",
                Some(admitted),
                format!(
                    "the admitted leg {} mm sits at the acceptance band's lower edge, \
                     so the required {} mm cut stays undecidable",
                    admitted, requirement.required_mm,
                ),
                vec!["indeterminate"],
                &mut findings,
            );
        }
        if setback_exceeds {
            finish(
                bounds,
                "CORNER_OVERCUT",
                "fail",
                Some(largest_setback),
                format!(
                    "the largest setback {} mm exceeds the acceptance band's upper edge \
                     {} mm (required {} mm, tolerance {} mm)",
                    largest_setback, upper_edge, requirement.required_mm, requirement.tolerance_mm,
                ),
                Vec::new(),
                &mut findings,
            );
        } else if setback_straddles {
            finish(
                bounds,
                "CORNER_NOT_CUT",
                "unknown",
                Some(largest_setback),
                format!(
                    "the largest setback {} mm sits at the acceptance band edge, so the \
                     required {} mm cut stays undecidable",
                    largest_setback, requirement.required_mm,
                ),
                vec!["indeterminate"],
                &mut findings,
            );
        }
        // The dependent weld-clearance judgement (contract section 4.4): the
        // qualifying weld between the two recorded neighbours — local
        // attachment validated (contract 5.1), its path passing the derived
        // corner within the product tolerance — has its recorded leg compared
        // with the admitted leg through the near-threshold rule, with the
        // product contact tolerance as the comparison's band tolerance. The
        // finding rides this rule's token with its own basis naming the weld
        // record; the recorded requirement above stays untouched.
        let neighbours = |weld: &WeldFact| {
            (weld.main == named[1] && weld.secondary == named[2])
                || (weld.main == named[2] && weld.secondary == named[1])
        };
        let mut qualified: Vec<&WeldFact> = Vec::new();
        let mut unvalidated: Vec<&WeldFact> = Vec::new();
        let mut straddling: Vec<&WeldFact> = Vec::new();
        for weld in inputs.welds.iter().filter(|w| neighbours(w)) {
            let attached = match attachment_verdict(weld, a, b, tolerance_mm) {
                Ok(attached) => attached,
                Err(_) => Attachment::Indeterminate,
            };
            if attached != Attachment::Attached {
                unvalidated.push(weld);
                continue;
            }
            let Some(path) = weld.path_mm.as_ref() else {
                unvalidated.push(weld);
                continue;
            };
            let corner_gap = polyline_distance(corner, path);
            match touch_state(corner_gap, tolerance_mm, delta) {
                Touch::On => qualified.push(weld),
                Touch::Off => {}
                Touch::Straddles => straddling.push(weld),
            }
        }
        // The basis names the judged weld records (#723), so a clearance finding
        // can never be mistaken for the recorded size requirement.
        let clearance_basis = |welds: &[&WeldFact]| -> String {
            let ids = welds
                .iter()
                .map(|weld| weld.element.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "recorded corner-cut requirement (contract 4.1); derived corner \
                 (contract 4.3); rebuilt weld fillet of weld {ids} (contract 4.4)"
            )
        };
        let clearance_bounds = |weld: &WeldFact| -> Vec<NamedBound> {
            vec![
                NamedBound {
                    name: "required",
                    bound_mm: requirement.required_mm,
                    tolerance_mm: requirement.tolerance_mm,
                    source: "owner",
                },
                NamedBound {
                    name: "admitted_leg",
                    bound_mm: admitted,
                    tolerance_mm: delta,
                    source: "measured",
                },
                NamedBound {
                    name: "weld_leg",
                    bound_mm: weld.size_mm,
                    tolerance_mm,
                    source: "weld record",
                },
            ]
        };
        for weld in &qualified {
            let state = near_threshold(weld.size_mm, admitted, Some(tolerance_mm), delta);
            let mut elements = named.to_vec();
            elements.push(weld.element);
            let (status, message, missing) = match state {
                ClashState::Clear => continue,
                ClashState::Indeterminate => (
                    "unknown",
                    format!(
                        "the weld of leg {} mm sits at the band edge of the admitted leg \
                         {} mm at the derived corner, so the weld clearance stays \
                         undecidable; the recorded requirement stays {} mm",
                        weld.size_mm, admitted, requirement.required_mm,
                    ),
                    vec!["indeterminate"],
                ),
                ClashState::Penetrated => {
                    // A cut c clears this weld when K + δ − t < c, and passes the size
                    // check when c + δ < upper edge; no cut does both once
                    // K ≥ upper edge + t − 2δ. Then the recorded requirement and the
                    // weld leg conflict, and the model does not pick the repair (#723).
                    let conflict = weld.size_mm >= upper_edge + tolerance_mm - 2.0 * delta;
                    let repair = if conflict {
                        format!(
                            "; no cut the recorded requirement accepts (upper edge {} mm) \
                             frees room for this leg, so the requirement and the weld leg \
                             conflict: a smaller weld or a changed requirement repairs it, \
                             and the model does not choose between them",
                            upper_edge,
                        )
                    } else {
                        "; a larger cut inside the recorded acceptance band or a smaller \
                         weld repairs it, and the model does not choose between them"
                            .to_owned()
                    };
                    (
                        "fail",
                        format!(
                            "the weld of leg {} mm exceeds the clearance the cut admits at \
                             the derived corner: the admitted leg is {} mm, so the fillet of \
                             the recorded leg does not fit the freed space{}; the recorded \
                             requirement stays {} mm",
                            weld.size_mm, admitted, repair, requirement.required_mm,
                        ),
                        Vec::new(),
                    )
                }
            };
            findings.push(CornerCutFinding {
                finding: ConnectivityFinding {
                    rule: "CORNER_NOT_CUT",
                    status,
                    elements,
                    message,
                    basis: clearance_basis(&[weld]),
                    measured_mm: Some(admitted),
                    required_mm: Some(weld.size_mm),
                    tolerance_mm: Some(tolerance_mm),
                    bounds: clearance_bounds(weld),
                    mesh_bound_mm: Some(bound),
                    delta_mm: Some(delta),
                    missing_inputs: missing,
                },
                corner_point_mm: Some(corner),
                clearance: true,
            });
        }
        if qualified.is_empty() && !unvalidated.is_empty() {
            let mut elements = named.to_vec();
            elements.extend(unvalidated.iter().map(|weld| weld.element));
            findings.push(CornerCutFinding {
                finding: ConnectivityFinding {
                    rule: "CORNER_NOT_CUT",
                    status: "unknown",
                    elements,
                    message: "the weld between the recorded neighbours has an unvalidated \
                              local attachment, so the model cannot establish whether its \
                              path passes the derived corner and the weld clearance stays \
                              unjudged"
                        .into(),
                    basis: clearance_basis(&unvalidated),
                    measured_mm: None,
                    required_mm: None,
                    tolerance_mm: None,
                    bounds: Vec::new(),
                    mesh_bound_mm: None,
                    delta_mm: None,
                    missing_inputs: vec!["attachment_unvalidated"],
                },
                corner_point_mm: Some(corner),
                clearance: true,
            });
        } else if qualified.is_empty() && !straddling.is_empty() {
            let mut elements = named.to_vec();
            elements.extend(straddling.iter().map(|weld| weld.element));
            findings.push(CornerCutFinding {
                finding: ConnectivityFinding {
                    rule: "CORNER_NOT_CUT",
                    status: "unknown",
                    elements,
                    message: "a weld between the recorded neighbours runs at the corner's \
                              tolerance band edge, so whether its path passes the derived \
                              corner stays undecidable and the weld clearance stays \
                              unjudged"
                        .into(),
                    basis: clearance_basis(&straddling),
                    measured_mm: None,
                    required_mm: None,
                    tolerance_mm: Some(tolerance_mm),
                    bounds: Vec::new(),
                    mesh_bound_mm: Some(bound),
                    delta_mm: Some(delta),
                    missing_inputs: vec!["indeterminate"],
                },
                corner_point_mm: Some(corner),
                clearance: true,
            });
        }
        if !admitted_fails && !admitted_straddles && !setback_exceeds && !setback_straddles {
            // The admitted leg clears the lower edge and the largest setback the
            // upper edge: the required corner cut is provided.
            continue;
        }
    }
    findings
}

/// The `PART_CLASH` decision for one pair of evaluated parts: proven
/// penetration (including full containment) beyond the product contact
/// tolerance, decided by the exact contact facts and the near-threshold rule.
pub fn clash_verdict(
    a: &ElementBody,
    b: &ElementBody,
    tolerance_mm: f64,
) -> Result<ClashOutcome, ContactError> {
    let delta = a.mesh_bound_mm.max(b.mesh_bound_mm).max(tolerance_mm / 2.0);
    let report = contact::contact(&a.mesh, &b.mesh, CONTACT_LIMITS)?;
    let (state, contact_kind, measured) = match report.relation {
        Relation::Separated => (ClashState::Clear, "separated", None),
        Relation::Touching => {
            let kind = if report.face_contact {
                "face_contact"
            } else {
                "edge_or_point_touch"
            };
            (ClashState::Clear, kind, None)
        }
        Relation::Overlapping { surface_crossing } => {
            // The sampled measure decides first (#740): the vertices, the
            // face probes and the edge traversal samples together read the
            // deepest strictly-contained point's distance to the surface —
            // the kernel's documented deterministic lower bound, invariant
            // under re-triangulations that keep the vertex set. The samples
            // reach a last-place placement dip (#721) and a
            // through-penetration whose overlap carries no vertex or probe
            // (#722) alike, so the crossing run of #722 stays the last
            // resort for a crossing the samples cannot read at all. A
            // crossing neither measure reads stays unmeasured, never
            // guessed.
            let measured = match penetration_depth_pair(&a.mesh, &b.mesh)? {
                Some(depth) => Some(depth),
                // The run refuses on flush walls and grazes; a crossing
                // neither measure reads stays unmeasured.
                None if surface_crossing => {
                    contact::crossing_depth(&a.mesh, &b.mesh, CONTACT_LIMITS)?
                }
                None => None,
            };
            // A coincident face (#741) — a duplicate, or a copy shifted by
            // less than the tolerance — keeps every vertex and face probe on
            // the other surface, so the samples above read nothing or only
            // the slivers. Its own measure starts on the shared face; the
            // deeper value wins, and a refused coincident measure keeps the
            // other one.
            let measured = if report.coincident {
                match contact::coincident_depth(&a.mesh, &b.mesh, CONTACT_LIMITS) {
                    Ok(Some(depth)) => Some(measured.map_or(depth, |m: f64| m.max(depth))),
                    Ok(None) | Err(_) => measured,
                }
            } else {
                measured
            };
            let kind = if surface_crossing {
                "penetrating"
            } else {
                "contained"
            };
            let state = match measured {
                Some(depth) => near_threshold(depth, tolerance_mm, None, delta),
                // A proven crossing whose overlap touches no mesh vertex of
                // either body: the depth is unmeasured, never guessed.
                None => ClashState::Indeterminate,
            };
            (state, kind, measured)
        }
    };
    Ok(ClashOutcome {
        state,
        contact: contact_kind,
        measured_mm: measured,
        mesh_bound_mm: a.mesh_bound_mm.max(b.mesh_bound_mm),
        delta_mm: delta,
        tolerance_mm,
    })
}

/// The sampled fallback depth: the deepest contained point of either side,
/// measured as its distance to the other shell's surface by the vertex and
/// face probes.
fn penetration_depth_pair(a: &TriMesh, b: &TriMesh) -> Result<Option<f64>, ContactError> {
    Ok(contact::penetration_depth(a, b, CONTACT_LIMITS)?
        .into_iter()
        .chain(contact::penetration_depth(b, a, CONTACT_LIMITS)?)
        .fold(None, |best: Option<f64>, depth| {
            Some(match best {
                Some(x) if x >= depth => x,
                _ => depth,
            })
        }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::command::{AuthoredPlate, evaluate_plate};
    use beamcraft_model::command::Placement;
    use beamcraft_model::id::ElementId;

    fn id(n: u8) -> ElementId {
        ElementId::parse(&format!("019a7bf0-0000-7000-8000-0000000000{n:02x}")).unwrap()
    }

    fn plate_at(n: u8, x: f64, y: f64, z: f64) -> Result<ElementBody, ContactError> {
        let authored = AuthoredPlate {
            id: id(n),
            name: format!("Plate {n}"),
            class: 0,
            contour_points_mm: vec![
                [0.0, 0.0, 0.0],
                [1000.0, 0.0, 0.0],
                [1000.0, 500.0, 0.0],
                [0.0, 500.0, 0.0],
            ],
            thickness_mm: 10.0,
            thickness_side: None,
            corners: Vec::new(),
            placement: Placement {
                translation_mm: [x, y, z],
                ..Placement::IDENTITY
            },
            material: None,
            holes_mm: Vec::new(),
            hole_records: Vec::new(),
            cuts: Vec::new(),
            bends: Vec::new(),
        };
        plate_body(authored)
    }

    fn plate_body(authored: AuthoredPlate) -> Result<ElementBody, ContactError> {
        let body = evaluate_plate(&authored).unwrap();
        let quantities = body.quantities();
        let record = crate::engine::committed::PlateRecord {
            authored,
            body,
            quantities,
        };
        ElementBody::of_plate(&record)
    }

    #[test]
    fn clear_pairs_report_what_they_are() {
        let a = plate_at(1, 0.0, 0.0, 0.0).unwrap();
        let b = plate_at(2, 5000.0, 0.0, 0.0).unwrap();
        let out = clash_verdict(&a, &b, CONTACT_TOLERANCE_MM).unwrap();
        assert_eq!(out.state, ClashState::Clear);
        assert_eq!(out.contact, "separated");
        assert_eq!(out.measured_mm, None);
        // The exact stack touches face to face and stays clear.
        let b = plate_at(2, 0.0, 0.0, 10.0).unwrap();
        let out = clash_verdict(&a, &b, CONTACT_TOLERANCE_MM).unwrap();
        assert_eq!(out.state, ClashState::Clear);
        assert_eq!(out.contact, "face_contact");
    }

    #[test]
    fn deep_overlap_penetrates_beyond_the_tolerance() {
        let a = plate_at(1, 0.0, 0.0, 0.0).unwrap();
        let b = plate_at(2, 100.0, 0.0, 5.0).unwrap();
        let out = clash_verdict(&a, &b, CONTACT_TOLERANCE_MM).unwrap();
        assert_eq!(out.state, ClashState::Penetrated);
        assert_eq!(out.contact, "penetrating");
        assert_eq!(out.measured_mm, Some(5.0));
    }

    #[test]
    fn full_containment_is_a_clash_without_a_surface_meeting() {
        // A thick base plate holds a smaller plate entirely inside.
        let big = plate_body(AuthoredPlate {
            id: id(5),
            name: "Base".into(),
            class: 0,
            contour_points_mm: vec![
                [0.0, 0.0, 0.0],
                [1000.0, 0.0, 0.0],
                [1000.0, 500.0, 0.0],
                [0.0, 500.0, 0.0],
            ],
            thickness_mm: 100.0,
            thickness_side: None,
            corners: Vec::new(),
            placement: Placement::IDENTITY,
            material: None,
            holes_mm: Vec::new(),
            hole_records: Vec::new(),
            cuts: Vec::new(),
            bends: Vec::new(),
        })
        .unwrap();
        let small = plate_body(AuthoredPlate {
            id: id(2),
            name: "Insert".into(),
            class: 0,
            contour_points_mm: vec![
                [0.0, 0.0, 0.0],
                [400.0, 0.0, 0.0],
                [400.0, 300.0, 0.0],
                [0.0, 300.0, 0.0],
            ],
            thickness_mm: 10.0,
            thickness_side: None,
            corners: Vec::new(),
            placement: Placement {
                translation_mm: [100.0, 100.0, 3.0],
                ..Placement::IDENTITY
            },
            material: None,
            holes_mm: Vec::new(),
            hole_records: Vec::new(),
            cuts: Vec::new(),
            bends: Vec::new(),
        })
        .unwrap();
        let out = clash_verdict(&big, &small, CONTACT_TOLERANCE_MM).unwrap();
        assert_eq!(out.state, ClashState::Penetrated);
        assert_eq!(out.contact, "contained");
        // The measurement is the deepest contained point of the insert: its
        // top face sits 13 mm above the base's bottom wall.
        assert_eq!(out.measured_mm, Some(13.0));
    }

    #[test]
    fn a_flush_buried_insert_is_a_clash_even_without_a_surface_crossing() {
        // The insert shares both side walls with the base (flush at x=0 and
        // x=1000), lies fully inside, and its flush-face region dodges the
        // base wall diagonals, so the surface facts alone read as touching.
        // The contained samples promote it, and the clash verdict fires.
        let big = plate_body(AuthoredPlate {
            id: id(6),
            name: "Pocket".into(),
            class: 0,
            contour_points_mm: vec![
                [0.0, 0.0, 0.0],
                [1000.0, 0.0, 0.0],
                [1000.0, 500.0, 0.0],
                [0.0, 500.0, 0.0],
            ],
            thickness_mm: 300.0,
            thickness_side: None,
            corners: Vec::new(),
            placement: Placement::IDENTITY,
            material: None,
            holes_mm: Vec::new(),
            hole_records: Vec::new(),
            cuts: Vec::new(),
            bends: Vec::new(),
        })
        .unwrap();
        let insert = plate_body(AuthoredPlate {
            id: id(7),
            name: "Liner".into(),
            class: 0,
            contour_points_mm: vec![
                [0.0, 0.0, 0.0],
                [1000.0, 0.0, 0.0],
                [1000.0, 200.0, 0.0],
                [0.0, 200.0, 0.0],
            ],
            thickness_mm: 10.0,
            thickness_side: None,
            corners: Vec::new(),
            placement: Placement {
                translation_mm: [0.0, 30.0, 150.0],
                ..Placement::IDENTITY
            },
            material: None,
            holes_mm: Vec::new(),
            hole_records: Vec::new(),
            cuts: Vec::new(),
            bends: Vec::new(),
        })
        .unwrap();
        let out = clash_verdict(&big, &insert, CONTACT_TOLERANCE_MM).unwrap();
        assert_eq!(out.state, ClashState::Penetrated);
        assert_eq!(out.contact, "contained");
        let measured = out.measured_mm.unwrap_or_default();
        assert!(measured > 30.0, "{measured}");
    }

    #[test]
    fn a_shallow_overlap_is_indeterminate_at_the_band() {
        // 0.5 mm of measured overlap against t = 0.5: the uncertainty interval
        // [0.25, 0.75] straddles the band edge, so the pair stays undecidable.
        let a = plate_at(1, 0.0, 0.0, 0.0).unwrap();
        let b = plate_at(2, 100.0, 0.0, 9.5).unwrap();
        let out = clash_verdict(&a, &b, CONTACT_TOLERANCE_MM).unwrap();
        assert_eq!(out.state, ClashState::Indeterminate);
        let measured = out.measured_mm.unwrap_or_default();
        assert!((measured - 0.5).abs() <= 1e-9, "{measured}");
    }

    #[test]
    fn a_duplicate_plate_and_a_sub_tolerance_copy_are_clashes() {
        // #741: a duplicate shares every face with the same orientation, so
        // no edge crosses and every sample sits on the other surface. The
        // coincident faces prove the overlap and measure it from the shared
        // caps: the 10 mm plate reads its mid-thickness, 5 mm.
        let a = plate_at(1, 0.0, 0.0, 0.0).unwrap();
        let duplicate = plate_at(2, 0.0, 0.0, 0.0).unwrap();
        let out = clash_verdict(&a, &duplicate, CONTACT_TOLERANCE_MM).unwrap();
        assert_eq!(out.state, ClashState::Penetrated);
        assert_eq!(out.contact, "contained");
        let measured = out.measured_mm.unwrap_or_default();
        assert!((measured - 5.0).abs() <= 1e-9, "{measured}");
        // A copy 0.2 mm off: the samples alone read the 0.2 mm slivers and
        // stayed clear; the coincident caps read the real overlap.
        let shifted = plate_at(2, 0.2, 0.0, 0.0).unwrap();
        let out = clash_verdict(&a, &shifted, CONTACT_TOLERANCE_MM).unwrap();
        assert_eq!(out.state, ClashState::Penetrated);
        let measured = out.measured_mm.unwrap_or_default();
        assert!((measured - 5.0).abs() <= 1e-9, "{measured}");
        // The exact stack and a plate beside the first share faces with
        // opposite orientations only: face contact, still clear.
        let stacked = plate_at(2, 0.0, 0.0, 10.0).unwrap();
        let out = clash_verdict(&a, &stacked, CONTACT_TOLERANCE_MM).unwrap();
        assert_eq!(out.state, ClashState::Clear);
        assert_eq!(out.contact, "face_contact");
        let beside = plate_at(2, 1000.0, 0.0, 0.0).unwrap();
        let out = clash_verdict(&a, &beside, CONTACT_TOLERANCE_MM).unwrap();
        assert_eq!(out.state, ClashState::Clear);
        assert_eq!(out.contact, "face_contact");
    }

    #[test]
    fn a_thin_overlap_stays_within_the_contact_tolerance() {
        let a = plate_at(1, 0.0, 0.0, 0.0).unwrap();
        let b = plate_at(2, 100.0, 0.0, 9.8).unwrap();
        let out = clash_verdict(&a, &b, CONTACT_TOLERANCE_MM).unwrap();
        assert_eq!(out.state, ClashState::Clear);
        let measured = out.measured_mm.unwrap_or_default();
        assert!((measured - 0.2).abs() <= 1e-9, "{measured}");
    }

    #[test]
    fn a_rotated_placement_keeps_the_verdict() {
        let a = plate_at(1, 0.0, 0.0, 0.0).unwrap();
        // The plate stands in the XZ plane after a quarter turn about X, with
        // its thickness running along Y into the first plate.
        let s = core::f64::consts::FRAC_1_SQRT_2;
        let upright = AuthoredPlate {
            id: id(3),
            name: "Wall".into(),
            class: 0,
            contour_points_mm: vec![
                [0.0, 0.0, 0.0],
                [1000.0, 0.0, 0.0],
                [1000.0, 500.0, 0.0],
                [0.0, 500.0, 0.0],
            ],
            thickness_mm: 10.0,
            thickness_side: None,
            corners: Vec::new(),
            placement: Placement {
                translation_mm: [100.0, 5.0, 0.0],
                orientation_xyzw: [s, 0.0, 0.0, s],
            },
            material: None,
            holes_mm: Vec::new(),
            hole_records: Vec::new(),
            cuts: Vec::new(),
            bends: Vec::new(),
        };
        let wall = plate_body(upright).unwrap();
        let out = clash_verdict(&a, &wall, CONTACT_TOLERANCE_MM).unwrap();
        assert_eq!(out.state, ClashState::Penetrated);
        // The identical geometry without the rotation clashes the same way.
        let plain = plate_at(4, 100.0, 5.0, 0.0).unwrap();
        let out_plain = clash_verdict(&a, &plain, CONTACT_TOLERANCE_MM).unwrap();
        assert_eq!(out_plain.state, ClashState::Penetrated);
    }

    #[test]
    fn near_threshold_never_resolves_the_straddling_interval() {
        let t = CONTACT_TOLERANCE_MM;
        // No recorded tolerance: the bare bound is the only edge.
        assert_eq!(near_threshold(0.2, t, None, 0.25), ClashState::Clear);
        assert_eq!(
            near_threshold(0.5, t, None, 0.25),
            ClashState::Indeterminate
        );
        assert_eq!(near_threshold(0.76, t, None, 0.25), ClashState::Penetrated);
        // A recorded tolerance widens the accepted band.
        assert_eq!(near_threshold(1.2, t, Some(1.0), 0.25), ClashState::Clear);
        assert_eq!(
            near_threshold(2.3, t, Some(1.0), 0.25),
            ClashState::Penetrated
        );
    }

    #[test]
    fn range_threshold_decides_both_band_edges() {
        // The permitted range [0, 0.2] with tolerance 0.05: the acceptance
        // region is [-0.05, 0.25].
        assert_eq!(range_threshold(0.1, 0.0, 0.2, 0.05, 0.1), ClashState::Clear);
        assert_eq!(
            range_threshold(0.6, 0.0, 0.2, 0.05, 0.25),
            ClashState::Penetrated
        );
        assert_eq!(
            range_threshold(0.25, 0.0, 0.2, 0.05, 0.25),
            ClashState::Indeterminate
        );
        // Below the minimum edge is outside too.
        assert_eq!(
            range_threshold(0.5, 1.0, 2.0, 0.1, 0.25),
            ClashState::Penetrated
        );
        assert_eq!(
            range_threshold(0.9, 1.0, 2.0, 0.1, 0.25),
            ClashState::Indeterminate
        );
        // A lower edge at or below zero cannot straddle: the physical gap
        // is never negative, so a tight fit passes the owner's range
        // [0, 2] with no recorded tolerance at all (contract section 7.3,
        // #724). A range whose upper edge sits inside the uncertainty band
        // (max 0.2, delta 0.25) still straddles that upper edge honestly.
        assert_eq!(range_threshold(0.0, 0.0, 2.0, 0.0, 0.25), ClashState::Clear);
        assert_eq!(
            range_threshold(0.0, 0.0, 2.0, 0.05, 0.25),
            ClashState::Clear
        );
        assert_eq!(
            range_threshold(0.0, 0.0, 0.2, 0.0, 0.25),
            ClashState::Indeterminate
        );
    }

    /// Two touching 1000x500x10 plates: a at z in [0, 10], b at z in
    /// [10, 20]; the shared plane z = 10 is the declared contact.
    fn stacked_pair() -> (ElementBody, ElementBody) {
        let a = plate_at(1, 0.0, 0.0, 0.0).unwrap();
        let b = plate_at(2, 0.0, 0.0, 10.0).unwrap();
        (a, b)
    }

    fn weld_at(path: Option<Vec<[f64; 3]>>) -> WeldFact {
        WeldFact {
            element: id(4),
            main: id(1),
            secondary: id(2),
            size_mm: 6.0,
            path_mm: path,
        }
    }

    #[test]
    fn a_path_on_the_declared_contact_is_attached() {
        let (a, b) = stacked_pair();
        let weld = weld_at(Some(vec![[100.0, 100.0, 10.0], [300.0, 100.0, 10.0]]));
        let attached = attachment_verdict(&weld, &a, &b, CONTACT_TOLERANCE_MM).unwrap();
        assert_eq!(attached, Attachment::Attached);
        // A path point off both surfaces is provably not attached.
        let weld = weld_at(Some(vec![[100.0, 100.0, 30.0]]));
        let attached = attachment_verdict(&weld, &a, &b, CONTACT_TOLERANCE_MM).unwrap();
        assert_eq!(attached, Attachment::NotAttached);
        // A weld with no path has no placed solid at the joint at all.
        let attached = attachment_verdict(&weld_at(None), &a, &b, CONTACT_TOLERANCE_MM).unwrap();
        assert_eq!(attached, Attachment::NotAttached);
        // A path point whose distance straddles the tolerance band edge
        // (0.6 mm off both surfaces against t = 0.5, delta = 0.25) stays
        // unresolved, never guessed.
        let weld = weld_at(Some(vec![[100.0, 100.0, 10.6]]));
        let attached = attachment_verdict(&weld, &a, &b, CONTACT_TOLERANCE_MM).unwrap();
        assert_eq!(attached, Attachment::Indeterminate);
        // A path point between the two facing surfaces of a 5 mm opening is
        // attached too: local attachment does not cap the measurable gap at
        // the contact tolerance (contract section 5.1, #724).
        let a = plate_at(1, 0.0, 0.0, 0.0).unwrap();
        let b = plate_at(2, 0.0, 0.0, 15.0).unwrap();
        let weld = weld_at(Some(vec![[100.0, 100.0, 12.5]]));
        let attached = attachment_verdict(&weld, &a, &b, CONTACT_TOLERANCE_MM).unwrap();
        assert_eq!(attached, Attachment::Attached);
        // The same opening with the path beside the plate edge in the open
        // air stays provably not attached.
        let weld = weld_at(Some(vec![[1100.0, 250.0, 12.5]]));
        let attached = attachment_verdict(&weld, &a, &b, CONTACT_TOLERANCE_MM).unwrap();
        assert_eq!(attached, Attachment::NotAttached);
    }

    #[test]
    fn proximity_classifies_touching_separated_and_band_edge() {
        let (a, b) = stacked_pair();
        let near = proximity_verdict(&a, &b, CONTACT_TOLERANCE_MM).unwrap();
        assert_eq!(near, Proximity::Touching);
        let (a, far) = (plate_at(1, 0.0, 0.0, 0.0).unwrap(), {
            plate_at(2, 0.0, 0.0, 12.0).unwrap()
        });
        let near = proximity_verdict(&a, &far, CONTACT_TOLERANCE_MM).unwrap();
        assert_eq!(near, Proximity::Separated { gap_mm: 2.0 });
        // A 0.5 mm air slab against t = 0.5: the band edge straddles.
        let (a, lifted) = (plate_at(1, 0.0, 0.0, 0.0).unwrap(), {
            plate_at(2, 0.0, 0.0, 10.5).unwrap()
        });
        let near = proximity_verdict(&a, &lifted, CONTACT_TOLERANCE_MM).unwrap();
        assert_eq!(near, Proximity::Indeterminate { gap_mm: 0.5 });
    }

    #[test]
    fn weld_gap_measures_the_air_at_the_path() {
        // A 0.3 mm air slab; the path runs mid-air on the declared contact,
        // 0.15 mm from each surface, so the summed gap is the air width.
        let a = plate_at(1, 0.0, 0.0, 0.0).unwrap();
        let b = plate_at(2, 0.0, 0.0, 10.3).unwrap();
        let weld = weld_at(Some(vec![[100.0, 100.0, 10.15], [300.0, 100.0, 10.15]]));
        let gap = weld_gap_mm(&weld, &a, &b).unwrap();
        let gap = gap.unwrap_or_default();
        assert!((gap - 0.3).abs() <= 1e-9, "{gap}");
    }

    fn intent(connections: Vec<DeclaredConnection>) -> RecordedInputs {
        RecordedInputs {
            intent: Some(JointIntent {
                scope: vec![id(1), id(2), id(3)],
                connections,
            }),
            gaps: Vec::new(),
            welds: Vec::new(),
            corner_cuts: Vec::new(),
        }
    }

    fn weld_pair() -> DeclaredConnection {
        DeclaredConnection {
            a: id(1),
            b: id(2),
            kind: ConnectionKind::Weld,
        }
    }

    #[test]
    fn no_intent_produces_no_connectivity_finding() {
        let (a, b) = stacked_pair();
        let findings = connectivity_findings(
            &RecordedInputs::default(),
            &[a, b],
            &[id(1), id(2)],
            &[id(1), id(2)],
            CONTACT_TOLERANCE_MM,
        );
        assert!(findings.is_empty());
    }

    #[test]
    fn a_missing_weld_with_touching_parts_is_unconnected_and_contacted_without_connection() {
        let (a, b) = stacked_pair();
        let findings = connectivity_findings(
            &intent(vec![weld_pair()]),
            &[a.clone(), b.clone()],
            &[id(1), id(2)],
            &[id(1), id(2)],
            CONTACT_TOLERANCE_MM,
        );
        let rules = findings.iter().map(|f| f.rule).collect::<Vec<_>>();
        assert!(rules.contains(&"PART_UNCONNECTED"));
        assert!(rules.contains(&"CONTACT_WITHOUT_CONNECTION"));
        let unconnected = findings
            .iter()
            .find(|f| f.rule == "PART_UNCONNECTED")
            .unwrap();
        assert_eq!(unconnected.status, "fail");
        assert_eq!(unconnected.elements, vec![id(1), id(2)]);
    }

    #[test]
    fn a_missing_weld_over_separated_parts_is_unconnected_only() {
        let a = plate_at(1, 0.0, 0.0, 0.0).unwrap();
        let b = plate_at(2, 0.0, 0.0, 500.0).unwrap();
        let findings = connectivity_findings(
            &intent(vec![weld_pair()]),
            &[a, b],
            &[id(1), id(2)],
            &[id(1), id(2)],
            CONTACT_TOLERANCE_MM,
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule, "PART_UNCONNECTED");
        assert_eq!(findings[0].status, "fail");
    }

    #[test]
    fn an_existing_weld_without_a_gap_requirement_stays_unknown() {
        let (a, b) = stacked_pair();
        let mut inputs = intent(vec![weld_pair()]);
        inputs.welds = vec![weld_at(Some(vec![[100.0, 100.0, 10.0]]))];
        let findings = connectivity_findings(
            &inputs,
            &[a, b],
            &[id(1), id(2)],
            &[id(1), id(2)],
            CONTACT_TOLERANCE_MM,
        );
        // The connection is satisfied: no PART_UNCONNECTED, and the gap
        // question stays unknown with the missing rule named.
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule, "WELD_WITHOUT_CONTACT");
        assert_eq!(findings[0].status, "unknown");
        assert_eq!(findings[0].missing_inputs, vec!["no_supported_rule"]);
    }

    #[test]
    fn a_pathless_weld_with_a_requirement_reports_unvalidated_attachment() {
        let (a, b) = stacked_pair();
        let mut inputs = intent(vec![weld_pair()]);
        inputs.welds = vec![weld_at(None)];
        inputs.gaps = vec![GapRequirement {
            a: id(1),
            b: id(2),
            permitted_min_mm: 0.0,
            permitted_max_mm: 0.2,
            tolerance_mm: 0.05,
        }];
        let findings = connectivity_findings(
            &inputs,
            &[a, b],
            &[id(1), id(2)],
            &[id(1), id(2)],
            CONTACT_TOLERANCE_MM,
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule, "WELD_WITHOUT_CONTACT");
        assert_eq!(findings[0].missing_inputs, vec!["attachment_unvalidated"]);
    }

    #[test]
    fn a_permitted_gap_passes_and_an_excessive_gap_fails_with_named_bounds() {
        // The owner requires contact at the weld: the permitted range is
        // [0, 0] with no tolerance, so any measured opening outside the
        // uncertainty band fails. The bridged opening here is 5 mm — ten
        // times the contact tolerance — and the weld is still locally
        // attached, because its path point lies between the two facing
        // surfaces (contract section 5.1, #724).
        let gap_requirement = GapRequirement {
            a: id(1),
            b: id(2),
            permitted_min_mm: 0.0,
            permitted_max_mm: 0.0,
            tolerance_mm: 0.0,
        };
        let a = plate_at(1, 0.0, 0.0, 0.0).unwrap();
        let b = plate_at(2, 0.0, 0.0, 15.0).unwrap();
        let mut inputs = intent(vec![weld_pair()]);
        inputs.welds = vec![weld_at(Some(vec![[100.0, 100.0, 12.5]]))];
        inputs.gaps = vec![gap_requirement.clone()];
        let findings = connectivity_findings(
            &inputs,
            &[a, b],
            &[id(1), id(2)],
            &[id(1), id(2)],
            CONTACT_TOLERANCE_MM,
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule, "WELD_WITHOUT_CONTACT");
        assert_eq!(findings[0].status, "fail");
        let measured = findings[0].measured_mm.unwrap_or_default();
        assert!((measured - 5.0).abs() <= 1e-6, "{measured}");
        assert_eq!(findings[0].bounds.len(), 2);
        assert_eq!(findings[0].bounds[0].name, "permitted_min");
        assert_eq!(findings[0].bounds[1].name, "permitted_max");
        // A 1.5 mm opening inside a permitted range [0, 2] passes too: the
        // weld is attached at the joint and the range admits it.
        inputs.gaps = vec![GapRequirement {
            permitted_min_mm: 0.0,
            permitted_max_mm: 2.0,
            tolerance_mm: 0.3,
            ..gap_requirement
        }];
        inputs.welds = vec![weld_at(Some(vec![[100.0, 100.0, 10.75]]))];
        let a = plate_at(1, 0.0, 0.0, 0.0).unwrap();
        let b = plate_at(2, 0.0, 0.0, 11.5).unwrap();
        let findings = connectivity_findings(
            &inputs,
            &[a, b],
            &[id(1), id(2)],
            &[id(1), id(2)],
            CONTACT_TOLERANCE_MM,
        );
        assert!(findings.is_empty());
        // The same geometry inside a wider permitted range passes: no finding.
        inputs.gaps = vec![GapRequirement {
            permitted_max_mm: 6.0,
            tolerance_mm: 0.05,
            ..gap_requirement
        }];
        inputs.welds = vec![weld_at(Some(vec![[100.0, 100.0, 12.5]]))];
        let a = plate_at(1, 0.0, 0.0, 0.0).unwrap();
        let b = plate_at(2, 0.0, 0.0, 15.0).unwrap();
        let findings = connectivity_findings(
            &inputs,
            &[a, b],
            &[id(1), id(2)],
            &[id(1), id(2)],
            CONTACT_TOLERANCE_MM,
        );
        assert!(findings.is_empty());
    }

    #[test]
    fn a_tight_fit_passes_a_zero_minimum_range_with_any_non_negative_tolerance() {
        // Touching plates, the path on the declared contact: the measured
        // gap is zero, and a range [0, max] passes with no recorded
        // tolerance at all — a lower band edge at or below the physical
        // floor of a gap cannot straddle (contract section 7.3, #724).
        let (a, b) = stacked_pair();
        let mut inputs = intent(vec![weld_pair()]);
        inputs.welds = vec![weld_at(Some(vec![[100.0, 100.0, 10.0]]))];
        inputs.gaps = vec![GapRequirement {
            a: id(1),
            b: id(2),
            permitted_min_mm: 0.0,
            permitted_max_mm: 2.0,
            tolerance_mm: 0.0,
        }];
        let findings = connectivity_findings(
            &inputs,
            &[a.clone(), b.clone()],
            &[id(1), id(2)],
            &[id(1), id(2)],
            CONTACT_TOLERANCE_MM,
        );
        assert!(
            findings.is_empty(),
            "a tight fit passes [0, max] with tolerance 0: {findings:?}"
        );
        inputs.gaps[0].tolerance_mm = 0.05;
        let findings = connectivity_findings(
            &inputs,
            &[a, b],
            &[id(1), id(2)],
            &[id(1), id(2)],
            CONTACT_TOLERANCE_MM,
        );
        assert!(findings.is_empty());
    }

    /// A 200x100x10 block with its origin corner at `origin`: the part
    /// standing on the 1000x500 plate of `plate_at` in the attachment cases.
    fn block_at(n: u8, origin: [f64; 3]) -> ElementBody {
        plate_body(AuthoredPlate {
            id: id(n),
            name: format!("Block {n}"),
            class: 0,
            contour_points_mm: vec![
                [0.0, 0.0, 0.0],
                [200.0, 0.0, 0.0],
                [200.0, 100.0, 0.0],
                [0.0, 100.0, 0.0],
            ],
            thickness_mm: 10.0,
            thickness_side: None,
            corners: Vec::new(),
            placement: Placement {
                translation_mm: origin,
                ..Placement::IDENTITY
            },
            material: None,
            holes_mm: Vec::new(),
            hole_records: Vec::new(),
            cuts: Vec::new(),
            bends: Vec::new(),
        })
        .unwrap()
    }

    #[test]
    fn a_path_on_one_surface_away_from_the_joint_is_not_attached() {
        // #724: the block stands on the plate over x 100..300, y 100..200.
        // A path point lying on the plate's own surface spans no opening:
        // the block's nearest point lies elsewhere on that same surface, not
        // across from it. Far on the plate top, on the plate's bottom face
        // under the block, or one point at the joint and one far away — none
        // is attached; the path along the joint is.
        let plate = plate_at(1, 0.0, 0.0, 0.0).unwrap();
        let block = block_at(2, [100.0, 100.0, 10.0]);
        let attached = |path: Vec<[f64; 3]>| {
            attachment_verdict(&weld_at(Some(path)), &plate, &block, CONTACT_TOLERANCE_MM).unwrap()
        };
        assert_eq!(
            attached(vec![[600.0, 400.0, 10.0], [700.0, 400.0, 10.0]]),
            Attachment::NotAttached
        );
        assert_eq!(
            attached(vec![[150.0, 150.0, 0.0], [250.0, 150.0, 0.0]]),
            Attachment::NotAttached
        );
        assert_eq!(
            attached(vec![[150.0, 150.0, 10.0], [700.0, 400.0, 10.0]]),
            Attachment::NotAttached
        );
        assert_eq!(
            attached(vec![[150.0, 100.0, 10.0], [250.0, 100.0, 10.0]]),
            Attachment::Attached
        );
    }

    #[test]
    fn a_gapped_joint_keeps_its_bridging_paths_attached() {
        // The block lifted 2 mm off the plate: a path on the block's bottom
        // edge and a path mid-gap both span the opening between facing
        // surfaces, so both stay attached and measure the 2 mm gap.
        let plate = plate_at(1, 0.0, 0.0, 0.0).unwrap();
        let block = block_at(2, [100.0, 100.0, 12.0]);
        for path in [
            vec![[100.0, 150.0, 12.0], [100.0, 180.0, 12.0]],
            vec![[150.0, 150.0, 11.0], [250.0, 150.0, 11.0]],
        ] {
            let weld = weld_at(Some(path));
            assert_eq!(
                attachment_verdict(&weld, &plate, &block, CONTACT_TOLERANCE_MM).unwrap(),
                Attachment::Attached
            );
            let gap = weld_gap_mm(&weld, &plate, &block)
                .unwrap()
                .unwrap_or_default();
            assert!((gap - 2.0).abs() <= 1e-9, "{gap}");
        }
    }

    fn gap_findings(
        a: &ElementBody,
        b: &ElementBody,
        path: Vec<[f64; 3]>,
        min: f64,
        max: f64,
        tolerance: f64,
    ) -> Vec<ConnectivityFinding> {
        let mut inputs = intent(vec![weld_pair()]);
        inputs.welds = vec![weld_at(Some(path))];
        inputs.gaps = vec![GapRequirement {
            a: id(1),
            b: id(2),
            permitted_min_mm: min,
            permitted_max_mm: max,
            tolerance_mm: tolerance,
        }];
        connectivity_findings(
            &inputs,
            &[a.clone(), b.clone()],
            &[id(1), id(2)],
            &[id(1), id(2)],
            CONTACT_TOLERANCE_MM,
        )
    }

    #[test]
    fn an_exact_contact_at_the_weld_passes_any_zero_minimum_range() {
        // #724: the plates meet exactly along the path, decided by the exact
        // predicates, so the opening is 0 with no tolerance band around it:
        // "contact required" ([0, 0], no tolerance) and [0, 0.2] with 0.05
        // pass; a recorded minimum opening fails with the measured 0.
        let (a, b) = stacked_pair();
        let path = vec![[100.0, 100.0, 10.0], [300.0, 100.0, 10.0]];
        assert!(gap_findings(&a, &b, path.clone(), 0.0, 0.0, 0.0).is_empty());
        assert!(gap_findings(&a, &b, path.clone(), 0.0, 0.2, 0.05).is_empty());
        let findings = gap_findings(&a, &b, path, 0.5, 2.0, 0.0);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].rule, "WELD_WITHOUT_CONTACT");
        assert_eq!(findings[0].status, "fail");
        assert_eq!(findings[0].measured_mm, Some(0.0));
        assert!(findings[0].message.contains("below the permitted minimum"));
    }

    #[test]
    fn a_thin_opening_is_measured_not_an_exact_contact() {
        // A 0.2 mm opening with the path mid-gap: every point lies within
        // the contact band of both surfaces, but outside both materials, so
        // it is no exact contact. The measured 0.2 mm against [0, 0] stays
        // at the band edge; a 0.3 mm opening fails it.
        let a = plate_at(1, 0.0, 0.0, 0.0).unwrap();
        let thin = plate_at(2, 0.0, 0.0, 10.2).unwrap();
        let findings = gap_findings(&a, &thin, vec![[100.0, 100.0, 10.1]], 0.0, 0.0, 0.0);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].status, "unknown");
        assert_eq!(findings[0].missing_inputs, vec!["indeterminate"]);
        let wider = plate_at(2, 0.0, 0.0, 10.3).unwrap();
        let findings = gap_findings(&a, &wider, vec![[100.0, 100.0, 10.15]], 0.0, 0.0, 0.0);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].status, "fail");
        let measured = findings[0].measured_mm.unwrap_or_default();
        assert!((measured - 0.3).abs() <= 1e-6, "{measured}");
    }

    #[test]
    fn a_weld_path_beside_the_joint_stays_unvalidated() {
        // The path runs in the open air beside the plate edge, mid-gap: it
        // neither touches the surfaces nor spans the contact between them,
        // so the weld is not locally attached and its gap stays unjudged.
        let a = plate_at(1, 0.0, 0.0, 0.0).unwrap();
        let b = plate_at(2, 0.0, 0.0, 15.0).unwrap();
        let mut inputs = intent(vec![weld_pair()]);
        inputs.welds = vec![weld_at(Some(vec![[1100.0, 250.0, 12.5]]))];
        inputs.gaps = vec![GapRequirement {
            a: id(1),
            b: id(2),
            permitted_min_mm: 0.0,
            permitted_max_mm: 2.0,
            tolerance_mm: 0.3,
        }];
        let findings = connectivity_findings(
            &inputs,
            &[a, b],
            &[id(1), id(2)],
            &[id(1), id(2)],
            CONTACT_TOLERANCE_MM,
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule, "WELD_WITHOUT_CONTACT");
        assert_eq!(findings[0].status, "unknown");
        assert_eq!(findings[0].missing_inputs, vec!["attachment_unvalidated"]);
    }

    #[test]
    fn a_band_edge_gap_stays_indeterminate() {
        // The range [0, 0.2] with tolerance 0.1 has its upper edge at 0.3;
        // the measured 0.3 mm with delta 0.25 straddles it.
        let a = plate_at(1, 0.0, 0.0, 0.0).unwrap();
        let b = plate_at(2, 0.0, 0.0, 10.3).unwrap();
        let mut inputs = intent(vec![weld_pair()]);
        inputs.welds = vec![weld_at(Some(vec![[100.0, 100.0, 10.15]]))];
        inputs.gaps = vec![GapRequirement {
            a: id(1),
            b: id(2),
            permitted_min_mm: 0.0,
            permitted_max_mm: 0.2,
            tolerance_mm: 0.1,
        }];
        let findings = connectivity_findings(
            &inputs,
            &[a, b],
            &[id(1), id(2)],
            &[id(1), id(2)],
            CONTACT_TOLERANCE_MM,
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].status, "unknown");
        assert_eq!(findings[0].missing_inputs, vec!["indeterminate"]);
        let measured = findings[0].measured_mm.unwrap_or_default();
        assert!((measured - 0.3).abs() <= 1e-6, "{measured}");
    }

    #[test]
    fn a_declared_contact_connection_judges_the_proximity() {
        let (a, b) = stacked_pair();
        let contact_pair = DeclaredConnection {
            kind: ConnectionKind::Contact,
            ..weld_pair()
        };
        // Touching: the declared contact exists, no finding.
        let findings = connectivity_findings(
            &intent(vec![contact_pair.clone()]),
            &[a, b],
            &[id(1), id(2)],
            &[id(1), id(2)],
            CONTACT_TOLERANCE_MM,
        );
        assert!(findings.is_empty());
        // 2 mm apart: the declared contact is absent.
        let a = plate_at(1, 0.0, 0.0, 0.0).unwrap();
        let b = plate_at(2, 0.0, 0.0, 12.0).unwrap();
        let findings = connectivity_findings(
            &intent(vec![contact_pair]),
            &[a, b],
            &[id(1), id(2)],
            &[id(1), id(2)],
            CONTACT_TOLERANCE_MM,
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule, "PART_UNCONNECTED");
        assert_eq!(findings[0].status, "fail");
        assert_eq!(findings[0].measured_mm, Some(2.0));
    }

    #[test]
    fn an_unresolved_target_stays_unknown_and_out_of_scope_connections_are_skipped() {
        let (a, b) = stacked_pair();
        let mut connections = vec![weld_pair()];
        connections.push(DeclaredConnection {
            a: id(1),
            b: id(9),
            kind: ConnectionKind::Weld,
        });
        // The first connection is inside the scope and judged; the second
        // targets an element outside the checked scope and is not checked.
        let findings = connectivity_findings(
            &intent(connections.clone()),
            &[a.clone(), b.clone()],
            &[id(1), id(2)],
            &[id(1), id(2)],
            CONTACT_TOLERANCE_MM,
        );
        assert_eq!(findings.len(), 2);
        // A scoped connection whose target is missing from the evaluated
        // parts is unresolved, never guessed.
        connections.push(DeclaredConnection {
            a: id(1),
            b: id(3),
            kind: ConnectionKind::Contact,
        });
        let findings = connectivity_findings(
            &intent(connections),
            &[a, b],
            &[id(1), id(2), id(3)],
            &[id(1), id(2)],
            CONTACT_TOLERANCE_MM,
        );
        let unresolved = findings
            .iter()
            .find(|f| f.missing_inputs == vec!["target_unresolved"])
            .unwrap();
        assert_eq!(unresolved.rule, "PART_UNCONNECTED");
        assert_eq!(unresolved.status, "unknown");
        assert_eq!(unresolved.elements, vec![id(1), id(3)]);
    }

    #[test]
    fn a_declared_member_outside_the_checked_scope_is_never_unresolved() {
        // Subtree semantics: the checked scope holds one joint member, the
        // other exists and is evaluated in the model — the connection is not
        // judged at all, like any connection outside the checked scope, and
        // the narrowed check stays quiet about its other members.
        let (a, b) = stacked_pair();
        let findings = connectivity_findings(
            &intent(vec![weld_pair()]),
            &[a.clone(), b.clone()],
            &[id(1)],
            &[id(1), id(2)],
            CONTACT_TOLERANCE_MM,
        );
        assert!(findings.is_empty());
        // The ghost stays unresolved under the same subtree scope: declared
        // and absent from the model is judged and reports unknown.
        let connections = vec![
            weld_pair(),
            DeclaredConnection {
                a: id(1),
                b: id(3),
                kind: ConnectionKind::Contact,
            },
        ];
        let findings = connectivity_findings(
            &intent(connections),
            &[a, b],
            &[id(1)],
            &[id(1), id(2)],
            CONTACT_TOLERANCE_MM,
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].missing_inputs, vec!["target_unresolved"]);
        assert_eq!(findings[0].elements, vec![id(1), id(3)]);
    }

    // The corner-cut fixture (contract section 3): a base plate on the world
    // floor, a column standing on it, and a rib perpendicular to the column
    // face. The rib's welded edge runs on the column face, its bottom runs on
    // the plate, and exactly one authored contour vertex — the ideal corner —
    // lies on both neighbours, whatever cut the rib carries.
    mod corner {
        use super::super::super::command::AuthoredPlate;
        use super::*;
        use crate::model::command::{CornerTreatment, PlateCorner};

        const COLUMN_FACE_X: f64 = 150.0;
        const PLATE_TOP_Z: f64 = 10.0;

        fn quarter_turn() -> [f64; 4] {
            let s = core::f64::consts::FRAC_1_SQRT_2;
            [s, 0.0, 0.0, s]
        }

        fn slab_plate(
            n: u8,
            name: &str,
            contour: Vec<[f64; 3]>,
            thickness: f64,
            placement: Placement,
        ) -> ElementBody {
            let authored = AuthoredPlate {
                id: id(n),
                name: name.into(),
                class: 0,
                contour_points_mm: contour,
                thickness_mm: thickness,
                thickness_side: None,
                corners: Vec::new(),
                placement,
                material: None,
                holes_mm: Vec::new(),
                hole_records: Vec::new(),
                cuts: Vec::new(),
                bends: Vec::new(),
            };
            super::plate_body(authored).unwrap()
        }

        fn plate_record(authored: AuthoredPlate) -> crate::engine::committed::PlateRecord {
            let body = crate::engine::command::evaluate_plate(&authored).unwrap();
            let quantities = body.quantities();
            crate::engine::committed::PlateRecord {
                authored,
                body,
                quantities,
            }
        }

        /// The standing slab: a contour in the local XY plane turned upright,
        /// so the contour plane faces +X and the thickness runs along −Y.
        fn standing(
            n: u8,
            name: &str,
            contour: Vec<[f64; 3]>,
            thickness: f64,
            at: [f64; 3],
        ) -> ElementBody {
            slab_plate(
                n,
                name,
                contour,
                thickness,
                Placement {
                    translation_mm: at,
                    orientation_xyzw: quarter_turn(),
                },
            )
        }

        fn base_plate() -> ElementBody {
            // The floor plate: slab z in [0, 10].
            slab_plate(
                1,
                "Base plate",
                vec![
                    [0.0, 0.0, 0.0],
                    [300.0, 0.0, 0.0],
                    [300.0, 200.0, 0.0],
                    [0.0, 200.0, 0.0],
                ],
                10.0,
                Placement::IDENTITY,
            )
        }

        fn column() -> ElementBody {
            // The column face at x = 150 (40 mm deep along y), standing on the
            // plate top.
            standing(
                2,
                "Column",
                vec![
                    [0.0, 0.0, 0.0],
                    [100.0, 0.0, 0.0],
                    [100.0, 200.0, 0.0],
                    [0.0, 200.0, 0.0],
                ],
                40.0,
                [50.0, 40.0, 10.0],
            )
        }

        /// The rib plate record, with the given corner treatment on vertex 0
        /// (the ideal corner). A leg of 0 leaves the corner sharp.
        fn rib_record(leg: f64) -> crate::engine::committed::PlateRecord {
            rib_with_corners(if leg > 0.0 {
                vec![PlateCorner {
                    vertex: 0,
                    treatment: CornerTreatment::Chamfer {
                        d1_mm: leg,
                        d2_mm: leg,
                    },
                }]
            } else {
                Vec::new()
            })
        }

        /// The rib plate record with the given corner treatments on the sharp
        /// 80×100 contour.
        fn rib_with_corners(corners: Vec<PlateCorner>) -> crate::engine::committed::PlateRecord {
            rib_authored(
                vec![
                    [0.0, 0.0, 0.0],
                    [80.0, 0.0, 0.0],
                    [80.0, 100.0, 0.0],
                    [0.0, 100.0, 0.0],
                ],
                corners,
            )
        }

        /// The rib plate record from an authored contour, with no treatments:
        /// the cut, when any, is part of the contour itself.
        fn rib_authored(
            contour_points_mm: Vec<[f64; 3]>,
            corners: Vec<PlateCorner>,
        ) -> crate::engine::committed::PlateRecord {
            let authored = AuthoredPlate {
                id: id(3),
                name: "Rib".into(),
                class: 0,
                contour_points_mm,
                thickness_mm: 10.0,
                thickness_side: None,
                corners,
                placement: Placement {
                    translation_mm: [COLUMN_FACE_X, 10.0, PLATE_TOP_Z],
                    orientation_xyzw: quarter_turn(),
                },
                material: None,
                holes_mm: Vec::new(),
                hole_records: Vec::new(),
                cuts: Vec::new(),
                bends: Vec::new(),
            };
            plate_record(authored)
        }

        fn requirement(required: f64, tolerance: f64) -> CornerCutRequirement {
            CornerCutRequirement {
                target: id(3),
                neighbours: [id(1), id(2)],
                required_mm: required,
                tolerance_mm: tolerance,
                upper_bound: None,
                purpose: CutPurpose::Clearance,
            }
        }

        fn inputs(requirement: CornerCutRequirement) -> RecordedInputs {
            RecordedInputs {
                intent: None,
                gaps: Vec::new(),
                welds: Vec::new(),
                corner_cuts: vec![requirement],
            }
        }

        fn run(
            requirement: CornerCutRequirement,
            rib: &crate::engine::committed::PlateRecord,
            scope: &[u8],
        ) -> Vec<CornerCutFinding> {
            let bodies = vec![base_plate(), column(), ElementBody::of_plate(rib).unwrap()];
            let ids: Vec<ElementId> = scope.iter().map(|n| id(*n)).collect();
            corner_cut_findings(
                &inputs(requirement),
                &bodies,
                std::slice::from_ref(rib),
                &ids,
                &[id(1), id(2), id(3)],
                CONTACT_TOLERANCE_MM,
            )
        }

        fn with_upper_bound(
            mut requirement: CornerCutRequirement,
            max: f64,
            tol: f64,
        ) -> CornerCutRequirement {
            requirement.upper_bound = Some(UpperBound {
                max_mm: max,
                tolerance_mm: tol,
            });
            requirement
        }

        #[test]
        fn an_uncut_corner_is_red_with_measured_zero() {
            // The acceptance red case: the model without the cut reports
            // CORNER_NOT_CUT with the measured leg 0 mm.
            let rib = rib_record(0.0);
            let findings = run(requirement(10.0, 1.0), &rib, &[1, 2, 3]);
            assert_eq!(findings.len(), 1);
            let finding = &findings[0].finding;
            assert_eq!(finding.rule, "CORNER_NOT_CUT");
            assert_eq!(finding.status, "fail");
            let measured = finding.measured_mm.unwrap_or_default();
            assert!(measured.abs() <= 1e-9, "{measured}");
            assert_eq!(finding.required_mm, Some(10.0));
            assert_eq!(finding.missing_inputs, Vec::<&'static str>::new());
            // The finding names the rib first, then both neighbours, reports
            // the required bound with the recorded tolerance, and names both
            // measured values beside it (contract 8.1): the admitted leg and
            // the largest setback, both 0 for the uncut corner.
            assert_eq!(finding.elements, vec![id(3), id(1), id(2)]);
            assert_eq!(finding.bounds.len(), 3);
            assert_eq!(finding.bounds[0].name, "required");
            assert_eq!(finding.bounds[1].name, "admitted_leg");
            assert_eq!(finding.bounds[1].source, "measured");
            assert_eq!(finding.bounds[2].name, "largest_setback");
            let admitted = finding.bounds[1].bound_mm;
            let setback = finding.bounds[2].bound_mm;
            assert!(
                admitted.abs() <= 1e-9 && setback.abs() <= 1e-9,
                "{admitted} {setback}"
            );
            // The derived corner is the kept sharp authored vertex.
            let corner = findings[0].corner_point_mm.unwrap();
            assert!((corner[0] - COLUMN_FACE_X).abs() <= 1e-9, "{corner:?}");
            assert!((corner[2] - PLATE_TOP_Z).abs() <= 1e-9, "{corner:?}");
        }

        #[test]
        fn the_correct_cut_is_green_without_a_finding() {
            // The acceptance green case: a 10 mm chamfer against the recorded
            // 10 mm requirement provides the required cut.
            let rib = rib_record(10.0);
            let findings = run(requirement(10.0, 1.0), &rib, &[1, 2, 3]);
            assert!(findings.is_empty());
        }

        #[test]
        fn a_twenty_millimetre_cut_reports_the_overcut() {
            // The acceptance red case: 20 mm instead of the recorded 10 mm is
            // CORNER_OVERCUT, measured 20 mm, required 10 mm — the recorded
            // size is the acceptance band's upper edge (contract 7.3).
            let rib = rib_record(20.0);
            let findings = run(requirement(10.0, 1.0), &rib, &[1, 2, 3]);
            assert_eq!(findings.len(), 1);
            let finding = &findings[0].finding;
            assert_eq!(finding.rule, "CORNER_OVERCUT");
            assert_eq!(finding.status, "fail");
            let measured = finding.measured_mm.unwrap_or_default();
            assert!((measured - 20.0).abs() <= 1e-9, "{measured}");
            assert_eq!(finding.required_mm, Some(10.0));
            assert_eq!(finding.tolerance_mm, Some(1.0));
            assert_eq!(finding.missing_inputs, Vec::<&'static str>::new());
        }

        #[test]
        fn an_undersized_cut_is_still_not_cut() {
            // A 5 mm chamfer against the recorded 10 mm requirement is
            // undersized: the same rule, the measured leg reported.
            let rib = rib_record(5.0);
            let findings = run(requirement(10.0, 1.0), &rib, &[1, 2, 3]);
            assert_eq!(findings.len(), 1);
            let finding = &findings[0].finding;
            assert_eq!(finding.rule, "CORNER_NOT_CUT");
            assert_eq!(finding.status, "fail");
            let measured = finding.measured_mm.unwrap_or_default();
            assert!((measured - 5.0).abs() <= 1e-9, "{measured}");
        }

        #[test]
        fn a_recorded_upper_bound_replaces_the_band_upper_edge() {
            // A 12 mm chamfer exceeds the required 10 mm band (upper edge 11
            // mm with the recorded 1 mm tolerance)...
            let rib = rib_record(12.0);
            let findings = run(requirement(10.0, 1.0), &rib, &[1, 2, 3]);
            assert_eq!(findings.len(), 1);
            assert_eq!(findings[0].finding.rule, "CORNER_OVERCUT");
            // ...but stays within the owner's standalone maximum of 12 ± 2 mm,
            // which replaces the band's upper edge (contract 4.1).
            let rib = rib_record(12.0);
            let findings = run(
                with_upper_bound(requirement(10.0, 1.0), 12.0, 2.0),
                &rib,
                &[1, 2, 3],
            );
            assert!(findings.is_empty());
            // The bound is named in the finding when it fires.
            let rib = rib_record(20.0);
            let findings = run(
                with_upper_bound(requirement(10.0, 1.0), 12.0, 2.0),
                &rib,
                &[1, 2, 3],
            );
            assert_eq!(findings.len(), 1);
            assert_eq!(findings[0].finding.bounds.len(), 4);
            assert_eq!(findings[0].finding.bounds[1].name, "upper_bound");
        }

        #[test]
        fn a_band_edge_leg_stays_indeterminate() {
            // A chamfer of 11 mm against the band [9, 11]: the uncertainty
            // interval straddles the upper edge, so the cut stays undecidable.
            let rib = rib_record(11.0);
            let findings = run(requirement(10.0, 1.0), &rib, &[1, 2, 3]);
            assert_eq!(findings.len(), 1);
            let finding = &findings[0].finding;
            assert_eq!(finding.status, "unknown");
            assert_eq!(finding.missing_inputs, vec!["indeterminate"]);
            let measured = finding.measured_mm.unwrap_or_default();
            assert!((measured - 11.0).abs() <= 1e-9, "{measured}");
        }

        #[test]
        fn an_asymmetric_chamfer_is_judged_on_both_legs() {
            // #725: a chamfer of 10 mm on one leg and 40 mm on the other is
            // CORNER_OVERCUT measured 40 — the largest setback — in either
            // order of the two distances: the admitted leg (the smaller leg)
            // stays inside the band, the largest setback exceeds its upper
            // edge, and both measured values are named.
            for (d1, d2) in [(10.0, 40.0), (40.0, 10.0)] {
                let rib = rib_with_corners(vec![PlateCorner {
                    vertex: 0,
                    treatment: CornerTreatment::Chamfer {
                        d1_mm: d1,
                        d2_mm: d2,
                    },
                }]);
                let findings = run(requirement(10.0, 1.0), &rib, &[1, 2, 3]);
                assert_eq!(findings.len(), 1, "{d1}/{d2}");
                let finding = &findings[0].finding;
                assert_eq!(finding.rule, "CORNER_OVERCUT", "{d1}/{d2}");
                assert_eq!(finding.status, "fail");
                let measured = finding.measured_mm.unwrap_or_default();
                assert!((measured - 40.0).abs() <= 1e-9, "{d1}/{d2}: {measured}");
                let admitted = finding.bounds[1].bound_mm;
                let setback = finding.bounds[2].bound_mm;
                assert_eq!(finding.bounds[1].name, "admitted_leg");
                assert_eq!(finding.bounds[2].name, "largest_setback");
                assert!(
                    (admitted - 10.0).abs() <= 1e-9 && (setback - 40.0).abs() <= 1e-9,
                    "{d1}/{d2}: {admitted} {setback}"
                );
            }
        }

        #[test]
        fn an_undersized_and_an_oversized_leg_report_both_findings() {
            // #725: a chamfer of 5 mm on one leg and 40 mm on the other fails
            // both edges at once — CORNER_NOT_CUT with the admitted leg 5 mm
            // and CORNER_OVERCUT with the largest setback 40 mm.
            let rib = rib_with_corners(vec![PlateCorner {
                vertex: 0,
                treatment: CornerTreatment::Chamfer {
                    d1_mm: 5.0,
                    d2_mm: 40.0,
                },
            }]);
            let findings = run(requirement(10.0, 1.0), &rib, &[1, 2, 3]);
            assert_eq!(findings.len(), 2);
            let not_cut = findings
                .iter()
                .find(|f| f.finding.rule == "CORNER_NOT_CUT")
                .unwrap_or_else(|| panic!("no CORNER_NOT_CUT in {findings:?}"));
            assert_eq!(not_cut.finding.status, "fail");
            let admitted = not_cut.finding.measured_mm.unwrap_or_default();
            assert!((admitted - 5.0).abs() <= 1e-9, "{admitted}");
            let overcut = findings
                .iter()
                .find(|f| f.finding.rule == "CORNER_OVERCUT")
                .unwrap_or_else(|| panic!("no CORNER_OVERCUT in {findings:?}"));
            assert_eq!(overcut.finding.status, "fail");
            let setback = overcut.finding.measured_mm.unwrap_or_default();
            assert!((setback - 40.0).abs() <= 1e-9, "{setback}");
        }

        #[test]
        fn a_contour_authored_cut_is_judged_like_a_parameterized_one() {
            // #725: a 10×10 cut authored as contour points leaves no authored
            // vertex at the corner; the derivation reads the final contact
            // edges, so the cut is green exactly like the parameterized chamfer
            // of the same size, and a contour-authored 20×20 cut is
            // CORNER_OVERCUT with measured 20 mm.
            let rib = rib_authored(
                vec![
                    [10.0, 0.0, 0.0],
                    [80.0, 0.0, 0.0],
                    [80.0, 100.0, 0.0],
                    [0.0, 100.0, 0.0],
                    [0.0, 10.0, 0.0],
                ],
                Vec::new(),
            );
            let findings = run(requirement(10.0, 1.0), &rib, &[1, 2, 3]);
            assert!(findings.is_empty());
            let rib = rib_authored(
                vec![
                    [20.0, 0.0, 0.0],
                    [80.0, 0.0, 0.0],
                    [80.0, 100.0, 0.0],
                    [0.0, 100.0, 0.0],
                    [0.0, 20.0, 0.0],
                ],
                Vec::new(),
            );
            let findings = run(requirement(10.0, 1.0), &rib, &[1, 2, 3]);
            assert_eq!(findings.len(), 1);
            let finding = &findings[0].finding;
            assert_eq!(finding.rule, "CORNER_OVERCUT");
            assert_eq!(finding.status, "fail");
            let measured = finding.measured_mm.unwrap_or_default();
            assert!((measured - 20.0).abs() <= 1e-9, "{measured}");
            // The corner is resolved from the final geometry and anchored.
            assert!(findings[0].corner_point_mm.is_some());
        }

        #[test]
        fn a_rounding_is_judged_by_its_admitted_leg_and_largest_setback() {
            // #725: a rounding of radius R at a right angle admits the
            // R·(2 − √2) chamfer leg and sets back R — both exact, so the
            // measure never follows the arc faceting. Against the recorded
            // 10 ± 1 mm band: round:10 is CORNER_NOT_CUT measured 5.858 (the
            // rounding is not the chamfer its setback suggests), round:5
            // measured 2.929, round:20 CORNER_OVERCUT measured 20.
            let admitted_of = |radius: f64| radius * (2.0 - std::f64::consts::SQRT_2);
            for (radius, rule, measured) in [
                (10.0, "CORNER_NOT_CUT", admitted_of(10.0)),
                (5.0, "CORNER_NOT_CUT", admitted_of(5.0)),
                (20.0, "CORNER_OVERCUT", 20.0),
            ] {
                let rib = rib_with_corners(vec![PlateCorner {
                    vertex: 0,
                    treatment: CornerTreatment::Round { radius_mm: radius },
                }]);
                let findings = run(requirement(10.0, 1.0), &rib, &[1, 2, 3]);
                assert_eq!(findings.len(), 1, "{radius}");
                let finding = &findings[0].finding;
                assert_eq!(finding.rule, rule, "{radius}");
                assert_eq!(finding.status, "fail", "{radius}");
                let value = finding.measured_mm.unwrap_or_default();
                // The admitted leg is sampled on the faceted arc: exact where
                // the 45° arc point is a facet vertex, otherwise within the
                // arc's 0.05 mm chord-deviation bound (#725).
                assert!(
                    (value - measured).abs() <= crate::engine::corners::ARC_CHORD_DEVIATION_MM,
                    "{radius}: {value} vs {measured}"
                );
            }
        }

        #[test]
        fn a_flush_rib_with_two_candidate_corners_stays_unresolved() {
            // A rib whose whole attached face is flush with the column face
            // has two lower corners adjacent to both neighbours: the
            // requirement cannot say which one it means, so nothing picks.
            let flush = AuthoredPlate {
                id: id(3),
                name: "Flush rib".into(),
                class: 0,
                contour_points_mm: vec![
                    [0.0, 0.0, 0.0],
                    [6.0, 0.0, 0.0],
                    [6.0, 100.0, 0.0],
                    [0.0, 100.0, 0.0],
                ],
                thickness_mm: 10.0,
                thickness_side: None,
                corners: Vec::new(),
                placement: Placement {
                    // The contour plane on the column face, the width along
                    // the column face (inside its 40 mm depth), the thickness
                    // outward: both lower contour corners touch both
                    // neighbours.
                    translation_mm: [COLUMN_FACE_X, 0.0, PLATE_TOP_Z],
                    orientation_xyzw: [0.5, 0.5, 0.5, 0.5],
                },
                material: None,
                holes_mm: Vec::new(),
                hole_records: Vec::new(),
                cuts: Vec::new(),
                bends: Vec::new(),
            };
            let rib = plate_record(flush);
            let findings = run(requirement(10.0, 1.0), &rib, &[1, 2, 3]);
            assert_eq!(findings.len(), 1);
            let finding = &findings[0].finding;
            assert_eq!(finding.rule, "CORNER_NOT_CUT");
            assert_eq!(finding.status, "unknown");
            assert_eq!(finding.missing_inputs, vec!["target_unresolved"]);
            assert_eq!(findings[0].corner_point_mm, None);
        }

        #[test]
        fn a_ghost_target_stays_unresolved_and_out_of_scope_neighbours_are_skipped() {
            // A requirement naming an element the model does not carry is the
            // ghost case: judged and unresolved, the requirement kept.
            let rib = rib_record(10.0);
            let ghost = requirement(10.0, 1.0);
            let mut ghost = ghost;
            ghost.target = id(9);
            let bodies = vec![base_plate(), column(), ElementBody::of_plate(&rib).unwrap()];
            let ids = vec![id(1), id(2), id(3), id(9)];
            let findings = corner_cut_findings(
                &inputs(ghost),
                &bodies,
                std::slice::from_ref(&rib),
                &ids,
                &[id(1), id(2), id(3)],
                CONTACT_TOLERANCE_MM,
            );
            assert_eq!(findings.len(), 1);
            assert_eq!(
                findings[0].finding.missing_inputs,
                vec!["target_unresolved"]
            );
            // A neighbour that exists but lies outside the checked scope
            // leaves the requirement unjudged, like any subtree omission.
            let findings = run(requirement(10.0, 1.0), &rib, &[1, 3]);
            assert!(findings.is_empty());
            // With no recorded requirement at all the rules produce nothing.
            let findings = corner_cut_findings(
                &RecordedInputs::default(),
                &bodies,
                &[rib],
                &[id(1), id(2), id(3)],
                &[id(1), id(2), id(3)],
                CONTACT_TOLERANCE_MM,
            );
            assert!(findings.is_empty());
        }

        /// A weld between the recorded neighbours with the given leg and path.
        fn weld_between(size: f64, path: Option<Vec<[f64; 3]>>) -> WeldFact {
            WeldFact {
                element: id(4),
                main: id(1),
                secondary: id(2),
                size_mm: size,
                path_mm: path,
            }
        }

        fn run_with_welds(
            requirement: CornerCutRequirement,
            rib: &crate::engine::committed::PlateRecord,
            welds: Vec<WeldFact>,
            scope: &[u8],
        ) -> Vec<CornerCutFinding> {
            let bodies = vec![base_plate(), column(), ElementBody::of_plate(rib).unwrap()];
            let ids: Vec<ElementId> = scope.iter().map(|n| id(*n)).collect();
            let mut recorded = inputs(requirement);
            recorded.welds = welds;
            corner_cut_findings(
                &recorded,
                &bodies,
                std::slice::from_ref(rib),
                &ids,
                &[id(1), id(2), id(3)],
                CONTACT_TOLERANCE_MM,
            )
        }

        #[test]
        fn a_weld_leg_within_the_cut_stays_green_and_one_past_it_turns_the_clearance_red() {
            // Contract 4.4: the weld that passes the derived corner with a leg
            // within the cut changes nothing; raising the leg past the cut is
            // the clearance finding riding this rule's token, with the
            // admitted leg as the measurement and the recorded weld leg as
            // the bound — the recorded requirement stays 10 mm.
            let rib = rib_record(10.0);
            let along_the_joint = vec![
                [COLUMN_FACE_X, 10.0, PLATE_TOP_Z],
                [COLUMN_FACE_X, 40.0, PLATE_TOP_Z],
            ];
            let findings = run_with_welds(
                requirement(10.0, 1.0),
                &rib,
                vec![weld_between(6.0, Some(along_the_joint.clone()))],
                &[1, 2, 3],
            );
            assert!(
                findings.is_empty(),
                "a 6 mm weld fits the 10 mm cut: {findings:?}"
            );
            let findings = run_with_welds(
                requirement(10.0, 1.0),
                &rib,
                vec![weld_between(14.0, Some(along_the_joint))],
                &[1, 2, 3],
            );
            assert_eq!(findings.len(), 1, "{findings:?}");
            let finding = &findings[0].finding;
            assert_eq!(finding.rule, "CORNER_NOT_CUT");
            assert_eq!(finding.status, "fail");
            let admitted = finding.measured_mm.unwrap_or_default();
            assert!((admitted - 10.0).abs() <= 1e-6, "{admitted}");
            // The compared bound is the weld record's leg, named as such.
            assert_eq!(finding.required_mm, Some(14.0));
            // The measurement carries its mesh bound and the effective
            // comparison tolerance like every measurement-bearing family.
            assert_eq!(finding.mesh_bound_mm, Some(0.0));
            assert_eq!(finding.delta_mm, Some(0.25));
            assert_eq!(finding.elements, vec![id(3), id(1), id(2), id(4)]);
            assert!(
                finding.basis.contains("contract 4.4"),
                "{:?}",
                finding.basis
            );
            let names = finding
                .bounds
                .iter()
                .map(|bound| (bound.name, bound.source))
                .collect::<Vec<_>>();
            assert!(names.contains(&("weld_leg", "weld record")), "{names:?}");
            assert!(names.contains(&("admitted_leg", "measured")), "{names:?}");
            assert!(finding.message.contains("requirement stays 10 mm"));
            assert!(findings[0].corner_point_mm.is_some());
        }

        #[test]
        fn a_duplicate_quarter_turn_part_is_a_clash() {
            // #741 on rotated geometry: the evaluated quarter turn is the same
            // mesh twice, so every face is coincident.
            let out = clash_verdict(&column(), &column(), CONTACT_TOLERANCE_MM).unwrap();
            assert_eq!(out.state, ClashState::Penetrated);
            assert!(out.measured_mm.unwrap_or_default() > 1.0, "{out:?}");
        }

        #[test]
        fn the_clearance_finding_never_carries_the_size_fix_and_names_the_conflict() {
            // #723: the clearance finding is marked as such (no fix: a larger
            // cut and a smaller weld both repair it), names the weld in its
            // basis, and says when no accepted cut fits the weld leg at all.
            let rib = rib_record(10.0);
            let along_the_joint = vec![
                [COLUMN_FACE_X, 10.0, PLATE_TOP_Z],
                [COLUMN_FACE_X, 40.0, PLATE_TOP_Z],
            ];
            let findings = run_with_welds(
                requirement(10.0, 1.0),
                &rib,
                vec![weld_between(14.0, Some(along_the_joint.clone()))],
                &[1, 2, 3],
            );
            assert_eq!(findings.len(), 1, "{findings:?}");
            assert!(findings[0].clearance);
            let finding = &findings[0].finding;
            assert!(
                finding.basis.contains(&id(4).to_string()),
                "{}",
                finding.basis
            );
            assert!(finding.message.contains("conflict"), "{}", finding.message);
            assert!(finding.message.contains("requirement stays 10 mm"));
            // A 10.8 mm leg fits a cut the band still accepts: no conflict.
            let findings = run_with_welds(
                requirement(10.0, 1.0),
                &rib,
                vec![weld_between(10.8, Some(along_the_joint))],
                &[1, 2, 3],
            );
            assert_eq!(findings.len(), 1, "{findings:?}");
            assert!(findings[0].clearance);
            let message = &findings[0].finding.message;
            assert!(!message.contains("conflict"), "{message}");
            assert!(message.contains("a larger cut inside the recorded acceptance band"));
            // The size finding of the uncut corner is not a clearance finding.
            let findings = run_with_welds(
                requirement(10.0, 1.0),
                &rib_record(0.0),
                Vec::new(),
                &[1, 2, 3],
            );
            assert_eq!(findings.len(), 1, "{findings:?}");
            assert!(!findings[0].clearance);
        }

        #[test]
        fn a_weld_elsewhere_between_the_neighbours_produces_no_clearance_finding() {
            // The weld is validated on the column-to-base joint away from the
            // corner: its path runs along the joint 10 mm past the rib and
            // provably misses the derived corner, so it does not count and
            // the clearance stays quiet (contract 4.4).
            let rib = rib_record(10.0);
            let away = vec![
                [COLUMN_FACE_X, 20.0, PLATE_TOP_Z],
                [COLUMN_FACE_X, 40.0, PLATE_TOP_Z],
            ];
            let findings = run_with_welds(
                requirement(10.0, 1.0),
                &rib,
                vec![weld_between(14.0, Some(away))],
                &[1, 2, 3],
            );
            assert!(findings.is_empty(), "{findings:?}");
        }

        #[test]
        fn a_pathless_weld_between_the_neighbours_stays_unknown_for_the_clearance() {
            // The pathless A06 weld is never locally attached, so the model
            // cannot establish whether it passes the corner: the clearance
            // stays unknown (attachment_unvalidated), the clearance only.
            let rib = rib_record(10.0);
            let findings = run_with_welds(
                requirement(10.0, 1.0),
                &rib,
                vec![weld_between(6.0, None)],
                &[1, 2, 3],
            );
            assert_eq!(findings.len(), 1, "{findings:?}");
            let finding = &findings[0].finding;
            assert_eq!(finding.status, "unknown");
            assert_eq!(finding.missing_inputs, vec!["attachment_unvalidated"]);
            assert_eq!(finding.elements, vec![id(3), id(1), id(2), id(4)]);
        }

        #[test]
        fn a_leg_at_the_admitted_band_edge_stays_unknown() {
            // The acceptance row: weld leg 10.5 against the admitted 10 mm —
            // the near-threshold band (admitted + contact tolerance 0.5,
            // delta 0.25) straddles, so the clearance stays unknown without
            // a guess.
            let rib = rib_record(10.0);
            let along_the_joint = vec![
                [COLUMN_FACE_X, 10.0, PLATE_TOP_Z],
                [COLUMN_FACE_X, 40.0, PLATE_TOP_Z],
            ];
            let findings = run_with_welds(
                requirement(10.0, 1.0),
                &rib,
                vec![weld_between(10.5, Some(along_the_joint))],
                &[1, 2, 3],
            );
            assert_eq!(findings.len(), 1, "{findings:?}");
            let finding = &findings[0].finding;
            assert_eq!(finding.rule, "CORNER_NOT_CUT");
            assert_eq!(finding.status, "unknown");
            assert_eq!(finding.missing_inputs, vec!["indeterminate"]);
            let admitted = finding.measured_mm.unwrap_or_default();
            assert!((admitted - 10.0).abs() <= 1e-6, "{admitted}");
            assert_eq!(finding.required_mm, Some(10.5));
        }

        #[test]
        fn a_rounding_and_a_chamfer_with_equal_setbacks_differ_in_the_clearance() {
            // A chamfer of 10 mm admits a 10 mm leg; a rounding of radius
            // 10 mm with the same setback admits R·(2 − √2) ≈ 5.86 mm. The
            // 8 mm weld fits the chamfer and does not fit the rounding —
            // whose cut also fails the size band, so both findings are
            // reported and the clearance one names the weld record.
            let chamfer = rib_record(10.0);
            let rounding = rib_with_corners(vec![PlateCorner {
                vertex: 0,
                treatment: CornerTreatment::Round { radius_mm: 10.0 },
            }]);
            let along_the_joint = vec![
                [COLUMN_FACE_X, 10.0, PLATE_TOP_Z],
                [COLUMN_FACE_X, 40.0, PLATE_TOP_Z],
            ];
            let findings = run_with_welds(
                requirement(10.0, 1.0),
                &chamfer,
                vec![weld_between(8.0, Some(along_the_joint.clone()))],
                &[1, 2, 3],
            );
            assert!(findings.is_empty(), "the chamfer fits the 8 mm weld");
            let findings = run_with_welds(
                requirement(10.0, 1.0),
                &rounding,
                vec![weld_between(8.0, Some(along_the_joint))],
                &[1, 2, 3],
            );
            assert_eq!(findings.len(), 2, "the rounding does not: {findings:?}");
            let clearance = findings
                .iter()
                .find(|f| f.finding.basis.contains("contract 4.4"))
                .expect("the clearance finding is missing");
            assert_eq!(clearance.finding.status, "fail");
            assert!(
                clearance
                    .finding
                    .bounds
                    .iter()
                    .any(|bound| bound.name == "weld_leg" && bound.source == "weld record"),
                "{:?}",
                clearance.finding.bounds
            );
            assert!(
                findings
                    .iter()
                    .all(|f| f.finding.basis.contains("contract 4.4")
                        || (f.finding.status == "fail"
                            && f.finding.basis.contains("contract 7.1"))),
                "the other finding is the size fail: {findings:?}"
            );
        }

        #[test]
        fn a_path_at_the_corner_band_edge_stays_undecidable() {
            // The path runs at the corner's tolerance band edge: whether it
            // passes the derived corner straddles, so the clearance stays
            // unknown without a guess.
            let rib = rib_record(10.0);
            let at_the_edge = vec![
                [COLUMN_FACE_X, 10.5, PLATE_TOP_Z],
                [COLUMN_FACE_X, 40.0, PLATE_TOP_Z],
            ];
            let findings = run_with_welds(
                requirement(10.0, 1.0),
                &rib,
                vec![weld_between(6.0, Some(at_the_edge))],
                &[1, 2, 3],
            );
            assert_eq!(findings.len(), 1, "{findings:?}");
            assert_eq!(findings[0].finding.status, "unknown");
            assert_eq!(findings[0].finding.missing_inputs, vec!["indeterminate"]);
        }
    }

    /// The adversarial geometry of contract section 7.1: bodies with holes,
    /// plane cuts and cavities, through the model's own expressible forms —
    /// an A04 through-hole polygon on a plate, an A03 plane-cut record, and a
    /// hollow (RHS) section member. A red verdict here would be a finding,
    /// not something to fix silently.
    mod adversarial {
        use super::*;
        use crate::engine::member::{
            AuthoredMember, MemberBody, ProfileInfo, evaluate_member, member_quantities,
        };
        use crate::model::command::{CutRecord, KeepSide};
        use beamcraft_model::id::OpId;

        /// A 1000x500x10 plate with a 100x100 square through-hole in its
        /// centre (element-local x 450..550, y 200..300).
        fn holed_plate_at(n: u8, x: f64, y: f64, z: f64) -> ElementBody {
            plate_body(AuthoredPlate {
                id: id(n),
                name: format!("Holed plate {n}"),
                class: 0,
                contour_points_mm: vec![
                    [0.0, 0.0, 0.0],
                    [1000.0, 0.0, 0.0],
                    [1000.0, 500.0, 0.0],
                    [0.0, 500.0, 0.0],
                ],
                thickness_mm: 10.0,
                thickness_side: None,
                corners: Vec::new(),
                placement: Placement {
                    translation_mm: [x, y, z],
                    ..Placement::IDENTITY
                },
                material: None,
                holes_mm: vec![vec![
                    [450.0, 200.0],
                    [450.0, 300.0],
                    [550.0, 300.0],
                    [550.0, 200.0],
                ]],
                hole_records: Vec::new(),
                cuts: Vec::new(),
                bends: Vec::new(),
            })
            .unwrap()
        }

        /// A 90x90 tab standing upright (thickness 10 across world Y,
        /// 90 mm across X and Z) through the holed plate's hole, with
        /// `x` placing it inside the hole, at the hole wall, or on the rim.
        fn tab_through_the_hole(x: f64) -> ElementBody {
            plate_body(AuthoredPlate {
                id: id(2),
                name: "Tab".into(),
                class: 0,
                contour_points_mm: vec![
                    [0.0, 0.0, 0.0],
                    [90.0, 0.0, 0.0],
                    [90.0, 90.0, 0.0],
                    [0.0, 90.0, 0.0],
                ],
                thickness_mm: 10.0,
                thickness_side: None,
                corners: Vec::new(),
                // A quarter turn about X maps local (x, y, z) to world
                // (x, -z, y): local x spans world X, local y spans world Z
                // (y + translation), and the thickness spans world Y below
                // the translation.
                placement: Placement {
                    translation_mm: [x, 255.0, -45.0],
                    orientation_xyzw: [
                        std::f64::consts::FRAC_1_SQRT_2,
                        0.0,
                        0.0,
                        std::f64::consts::FRAC_1_SQRT_2,
                    ],
                },
                material: None,
                holes_mm: Vec::new(),
                hole_records: Vec::new(),
                cuts: Vec::new(),
                bends: Vec::new(),
            })
            .unwrap()
        }

        #[test]
        fn a_tab_through_the_hole_is_clear_and_the_same_tab_on_the_rim_penetrates() {
            let plate = holed_plate_at(1, 0.0, 0.0, 0.0);
            // Through the hole itself: 5 mm of clearance to the hole walls —
            // the hole is real missing material, not a hidden clash.
            let tab = tab_through_the_hole(455.0);
            let out = clash_verdict(&plate, &tab, CONTACT_TOLERANCE_MM).unwrap();
            assert_eq!(out.state, ClashState::Clear);
            assert_eq!(out.contact, "separated");
            // The same tab with its face exactly on the hole wall (x = 450):
            // exact face contact — the hole wall and the tab face are one
            // plane with opposite normals, so the pair is clear face contact
            // (#721: the cap-diagonal graze no longer reads as a crossing).
            let tab = tab_through_the_hole(450.0);
            let out = clash_verdict(&plate, &tab, CONTACT_TOLERANCE_MM).unwrap();
            assert_eq!(out.state, ClashState::Clear);
            assert_eq!(out.contact, "face_contact");
            // Shifted into the rim: 45 mm of overlap with the plate material
            // beyond the hole edge x = 550 — a proven penetration.
            let tab = tab_through_the_hole(505.0);
            let out = clash_verdict(&plate, &tab, CONTACT_TOLERANCE_MM).unwrap();
            assert_eq!(out.state, ClashState::Penetrated);
            assert_eq!(out.contact, "penetrating");
            assert!(out.measured_mm.is_some());
        }

        /// A plate with one A03 plane cut keeping x <= `keep_x` (the A' part
        /// of a 1000 mm plate; the uncut plate spans x 0..1000).
        fn cut_plate(keep_x: f64) -> ElementBody {
            plate_body(AuthoredPlate {
                id: id(1),
                name: "Cut plate".into(),
                class: 0,
                contour_points_mm: vec![
                    [0.0, 0.0, 0.0],
                    [1000.0, 0.0, 0.0],
                    [1000.0, 500.0, 0.0],
                    [0.0, 500.0, 0.0],
                ],
                thickness_mm: 10.0,
                thickness_side: None,
                corners: Vec::new(),
                placement: Placement::IDENTITY,
                material: None,
                holes_mm: Vec::new(),
                hole_records: Vec::new(),
                cuts: vec![CutRecord {
                    operation_id: OpId::parse("019a7bf0-0000-7000-8000-0000000000c7").unwrap(),
                    ordinal: 0,
                    point_mm: [keep_x, 0.0, 0.0],
                    normal: [1.0, 0.0, 0.0],
                    keep: KeepSide::Negative,
                }],
                bends: Vec::new(),
            })
            .unwrap()
        }

        #[test]
        fn a_cut_moves_a_pair_across_the_near_threshold_band() {
            // A plate overlapping its neighbour by 5 mm is a proven
            // penetration; the plane cut that takes the neighbour's slab back
            // to the whole plate's exact top face turns the pair into an
            // exact face contact — the cut, not a placement change, moved the
            // pair across the near-threshold band (contract 7.1).
            let whole = plate_at(1, 0.0, 0.0, 0.0).unwrap();
            let overlapping = plate_at(2, 0.0, 0.0, 5.0).unwrap();
            let out = clash_verdict(&whole, &overlapping, CONTACT_TOLERANCE_MM).unwrap();
            assert_eq!(out.state, ClashState::Penetrated);
            assert_eq!(out.measured_mm, Some(5.0));
            // The same neighbour slab (z 5..15) after a cut keeping z <= 10:
            // its lower face lands exactly on the whole plate's upper face.
            let cut_back = plate_body(AuthoredPlate {
                id: id(2),
                name: "Cut slab".into(),
                class: 0,
                contour_points_mm: vec![
                    [0.0, 0.0, 0.0],
                    [1000.0, 0.0, 0.0],
                    [1000.0, 500.0, 0.0],
                    [0.0, 500.0, 0.0],
                ],
                thickness_mm: 10.0,
                thickness_side: None,
                corners: Vec::new(),
                placement: Placement {
                    translation_mm: [0.0, 0.0, 5.0],
                    ..Placement::IDENTITY
                },
                material: None,
                holes_mm: Vec::new(),
                hole_records: Vec::new(),
                cuts: vec![CutRecord {
                    operation_id: OpId::parse("019a7bf0-0000-7000-8000-0000000000c8").unwrap(),
                    ordinal: 0,
                    // Element-local: the slab spans local z 0..10 (the world
                    // translation lifts it to z 5..15), so keeping local
                    // z >= 5 leaves the world slab z 10..15, its lower face
                    // exactly on the whole plate's upper face z = 10.
                    point_mm: [0.0, 0.0, 5.0],
                    normal: [0.0, 0.0, 1.0],
                    keep: KeepSide::Positive,
                }],
                bends: Vec::new(),
            })
            .unwrap();
            let out = clash_verdict(&whole, &cut_back, CONTACT_TOLERANCE_MM).unwrap();
            assert_eq!(out.state, ClashState::Clear);
            assert_eq!(out.contact, "face_contact");
        }

        #[test]
        fn a_cut_creates_a_near_touching_neighbour() {
            // A neighbour slab resting half inside the whole plate's height
            // overlaps it — a proven clash. After the plate is cut back to
            // x = 104 the same neighbour at x = 104.4 sits 0.4 mm from the
            // new cut face: its uncertainty interval straddles the tolerance
            // band edge, so the verdict becomes indeterminate instead of
            // silently clear (contract 7.1). The neighbour stands at z = 5 so
            // neither body's horizontal faces are coplanar with the other's.
            let neighbour = plate_at(2, 104.4, 0.0, 5.0).unwrap();
            let whole = plate_at(1, 0.0, 0.0, 0.0).unwrap();
            // The uncut pair overlaps (the connectivity proximity reads an
            // interpenetration as touching); the cut pair sits 0.4 mm from
            // the new face, and the near-threshold rule refuses to call that
            // clear or clashing.
            let out = proximity_verdict(&whole, &neighbour, CONTACT_TOLERANCE_MM).unwrap();
            assert_eq!(out, Proximity::Touching);
            let cut = cut_plate(104.0);
            let out = proximity_verdict(&cut, &neighbour, CONTACT_TOLERANCE_MM).unwrap();
            match out {
                Proximity::Indeterminate { gap_mm } => {
                    assert!((gap_mm - 0.4).abs() <= 1e-9, "{gap_mm}");
                }
                other => panic!("expected a near-touch, got {other:?}"),
            }
        }

        /// A hollow (RHS 120x120x10, corner radius 12) beam member along
        /// world X: the cavity is the model's own expressible form (the A07.1
        /// parametric family extrudes the inner contour).
        fn rhs_member(n: u8) -> ElementBody {
            let shape = crate::model::section::SectionShape::Parametric(
                crate::model::section::ParametricSection::new(
                    crate::model::section::SectionFamily::Rhs,
                    vec![120.0, 120.0, 10.0, 12.0],
                )
                .unwrap(),
            );
            let authored = AuthoredMember {
                curved: false,
                id: id(n),
                kind: crate::model::command::MemberKind::Beam,
                name: "RHS beam".into(),
                class: 0,
                axis_points_mm: vec![[0.0, 0.0, 0.0], [1000.0, 0.0, 0.0]],
                follow: None,
                profile: beamcraft_model::digest::Sha256::digest(b"rhs-profile").unwrap(),
                roll_rad: None,
                section_placement: None,
                placement: Placement::IDENTITY,
                material: None,
                cuts: Vec::new(),
                hole_records: Vec::new(),
            };
            let body: MemberBody = evaluate_member(
                &authored,
                ProfileInfo::Section {
                    shape,
                    declared_area_mm2: None,
                },
            )
            .unwrap();
            let quantities = member_quantities(&body, None).unwrap();
            let record = crate::engine::committed::MemberRecord {
                authored,
                body,
                quantities,
            };
            ElementBody::of_member(&record).unwrap()
        }

        /// A flat 80x80x10 plate turned upright the same way as the tab: with
        /// the quarter turn about X its 80 mm width spans world Z (z in
        /// [z, z + 80]) and its thickness spans world Y below the translation
        /// (y in [-5, 5]). `z` shifts it against the -z cavity wall.
        fn insert_plate(z: f64) -> ElementBody {
            plate_body(AuthoredPlate {
                id: id(2),
                name: "Insert".into(),
                class: 0,
                contour_points_mm: vec![
                    [0.0, 0.0, 0.0],
                    [80.0, 0.0, 0.0],
                    [80.0, 80.0, 0.0],
                    [0.0, 80.0, 0.0],
                ],
                thickness_mm: 10.0,
                thickness_side: None,
                corners: Vec::new(),
                placement: Placement {
                    translation_mm: [450.0, 5.0, z],
                    orientation_xyzw: [
                        std::f64::consts::FRAC_1_SQRT_2,
                        0.0,
                        0.0,
                        std::f64::consts::FRAC_1_SQRT_2,
                    ],
                },
                material: None,
                holes_mm: Vec::new(),
                hole_records: Vec::new(),
                cuts: Vec::new(),
                bends: Vec::new(),
            })
            .unwrap()
        }

        #[test]
        fn an_insert_in_the_rhs_cavity_clears_penetrates_and_measures_a_through_run() {
            // The cavity is real: the same insert floats determinately clear
            // 10 mm from the inner cavity wall and penetrates the wall
            // material by 5 mm with a measured depth. Pushed all the way
            // through the wall, the crossing's overlap touches no mesh
            // vertex of either body: the edge traversal samples (#740) read
            // the crossing's own edges through the wall, and the measured
            // depth is the mid-wall distance to the wall's faces — red,
            // never the indeterminate guess-refusal the sampled ceiling of
            // contract 7.1 shipped before #722.
            let beam = rhs_member(1);
            let floating = insert_plate(-40.0);
            let out = clash_verdict(&beam, &floating, CONTACT_TOLERANCE_MM).unwrap();
            assert_eq!(out.state, ClashState::Clear);
            assert_eq!(out.contact, "separated");
            let crossing = insert_plate(-55.0);
            let out = clash_verdict(&beam, &crossing, CONTACT_TOLERANCE_MM).unwrap();
            assert_eq!(out.state, ClashState::Penetrated);
            assert_eq!(out.contact, "penetrating");
            assert!(out.measured_mm.is_some());
            let pushed_through = insert_plate(-60.0);
            let out = clash_verdict(&beam, &pushed_through, CONTACT_TOLERANCE_MM).unwrap();
            // Pushed all the way through the wall, the crossing's overlap
            // touches no mesh vertex of either body — the vertex and probe
            // samples stay empty, and the traversal samples read the
            // crossing's own edges through the wall (#722, #740), so the
            // pair is red with the wall's thickness.
            assert_eq!(out.state, ClashState::Penetrated);
            let measured = out.measured_mm.unwrap_or_default();
            assert!((measured - 10.0).abs() <= 1e-6, "{measured}");
        }

        /// A 1000x500x20 base plate at z 0..20: the shared plate of #722's
        /// placement matrix.
        fn base_plate(n: u8) -> ElementBody {
            plate_body(AuthoredPlate {
                id: id(n),
                name: "Base plate".into(),
                class: 0,
                contour_points_mm: vec![
                    [0.0, 0.0, 0.0],
                    [1000.0, 0.0, 0.0],
                    [1000.0, 500.0, 0.0],
                    [0.0, 500.0, 0.0],
                ],
                thickness_mm: 20.0,
                thickness_side: None,
                corners: Vec::new(),
                placement: Placement::IDENTITY,
                material: None,
                holes_mm: Vec::new(),
                hole_records: Vec::new(),
                cuts: Vec::new(),
                bends: Vec::new(),
            })
            .unwrap()
        }

        /// A column member standing on a vertical axis at (`x`, `y`) from
        /// `z0` to `z1`, with the given first-scope or A07.1 profile.
        fn column_body(n: u8, info: ProfileInfo, x: f64, y: f64, z0: f64, z1: f64) -> ElementBody {
            let authored = AuthoredMember {
                curved: false,
                id: id(n),
                kind: crate::model::command::MemberKind::Column,
                name: format!("Column {n}"),
                class: 0,
                axis_points_mm: vec![[x, y, z0], [x, y, z1]],
                follow: None,
                profile: beamcraft_model::digest::Sha256::digest(b"column-profile").unwrap(),
                roll_rad: None,
                section_placement: None,
                placement: Placement::IDENTITY,
                material: None,
                cuts: Vec::new(),
                hole_records: Vec::new(),
            };
            let body: MemberBody = evaluate_member(&authored, info).unwrap();
            let quantities = member_quantities(&body, None).unwrap();
            let record = MemberRecord {
                authored,
                body,
                quantities,
            };
            ElementBody::of_member(&record).unwrap()
        }

        /// The first-scope centred rectangle the CLI defines with
        /// `--width`/`--height`.
        fn rectangle_info(w: f64, h: f64) -> ProfileInfo {
            ProfileInfo::Rectangle {
                width_mm: w,
                height_mm: h,
                declared_area_mm2: None,
            }
        }

        /// The A07.1 doubly symmetric I section with sharp roots.
        fn i_shape_info(h: f64, b: f64, tw: f64, tf: f64) -> ProfileInfo {
            ProfileInfo::Section {
                shape: crate::model::section::SectionShape::Parametric(
                    crate::model::section::ParametricSection::new(
                        crate::model::section::SectionFamily::IShape,
                        vec![h, b, tw, tf, 0.0],
                    )
                    .unwrap(),
                ),
                declared_area_mm2: None,
            }
        }

        #[test]
        fn a_column_through_its_base_plate_is_red_with_the_measured_depth() {
            // #722's placement matrix: a column straight through its base
            // plate proves a surface crossing, but no vertex of either body
            // lies inside the other and no face probe lands in the overlap,
            // so the sampled depth stayed empty and every placement but one
            // reported `unknown` with exit 0 (the exception read 37 mm of
            // probe luck). The edge traversal samples (#740) read the
            // crossing where the column's surface edges run through the
            // plate, and every placement is red with a measured depth.
            let plate = base_plate(1);
            // Per-placement measured depth: the traversal sample reads the
            // column's surface edges through the plate (mid-slab 10 mm), and
            // the two placements that additionally carry a vertex/probe
            // inside the other body keep the documented probe-luck depths.
            for (x, y, w, h, z0, expected) in [
                (500.0, 250.0, 200.0, 200.0, -50.0, 10.0),
                (200.0, 400.0, 100.0, 100.0, -50.0, 10.0),
                (700.0, 300.0, 100.0, 100.0, -50.0, 10.0),
                (300.0, 300.0, 60.0, 60.0, -50.0, 10.0),
                (437.0, 181.0, 200.0, 200.0, -50.0, 37.0),
                (500.0, 250.0, 200.0, 200.0, 0.0, 20.0),
            ] {
                let column = column_body(2, rectangle_info(w, h), x, y, z0, 1000.0);
                let out = clash_verdict(&plate, &column, CONTACT_TOLERANCE_MM).unwrap();
                assert_eq!(out.state, ClashState::Penetrated, "{x} {y} {z0}");
                assert_eq!(out.contact, "penetrating");
                let measured = out.measured_mm.unwrap_or_default();
                assert!(
                    (measured - expected).abs() <= 1e-6,
                    "{x} {y} {z0}: {measured}"
                );
            }
        }

        #[test]
        fn an_i_shape_through_the_plate_measures_the_same_depth() {
            // The I profile's flange and web edges cross the plate caps the
            // same way: the traversal samples read the mid-slab depth; the
            // flush-bottom start additionally reads a plate face probe.
            let plate = base_plate(1);
            for (x, y, z0, expected) in [
                (500.0, 250.0, -50.0, 10.0),
                (437.0, 181.0, -50.0, 10.0),
                (500.0, 250.0, 0.0, 20.0),
            ] {
                let column =
                    column_body(2, i_shape_info(200.0, 200.0, 9.0, 15.0), x, y, z0, 1000.0);
                let out = clash_verdict(&plate, &column, CONTACT_TOLERANCE_MM).unwrap();
                assert_eq!(out.state, ClashState::Penetrated, "{x} {y} {z0}");
                assert_eq!(out.contact, "penetrating");
                let measured = out.measured_mm.unwrap_or_default();
                assert!(
                    (measured - expected).abs() <= 1e-6,
                    "{x} {y} {z0}: {measured}"
                );
            }
        }
    }
}
