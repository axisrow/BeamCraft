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
    /// `edge_or_point_touch`, or `penetrating`.
    pub contact: &'static str,
    /// The measured penetration depth in millimetres, when one was measured.
    pub measured_mm: Option<f64>,
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
    if measured - delta > lo && measured + delta < hi {
        ClashState::Clear
    } else if measured - delta > hi || measured + delta < lo {
        ClashState::Penetrated
    } else {
        ClashState::Indeterminate
    }
}

/// The declared kind of one joint-intent connection (contract section 5).
/// Bolted kinds are refused at admission in v0.1 and arrive with the
/// fastener slices, so no variant exists for them here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionKind {
    /// The parts must be joined by a weld element.
    Weld,
    /// The parts must be in physical contact within the product tolerance.
    Contact,
}

impl ConnectionKind {
    /// The stable token of the kind.
    pub const fn token(self) -> &'static str {
        match self {
            Self::Weld => "weld",
            Self::Contact => "contact",
        }
    }
}

/// One declared connection of the joint intent (contract section 5): two
/// distinct part elements and the declared kind.
#[derive(Debug, Clone, PartialEq)]
pub struct DeclaredConnection {
    /// One side of the declared connection.
    pub a: ElementId,
    /// The other side, distinct from `a`.
    pub b: ElementId,
    /// What the owner declared must join the parts.
    pub kind: ConnectionKind,
}

/// The joint intent as plain rule input (contract section 5): the elements
/// that form the joint and the declared connections. v0.1 has no recorded
/// intent yet — the recorded-inputs persistence of a later slice carries it
/// into the store; until then a caller without intent passes `None` and the
/// connectivity rules are not applicable for any element (contract section
/// 5: an established absence, never an unknown).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct JointIntent {
    /// The element ids that form the joint.
    pub scope: Vec<ElementId>,
    /// The declared connections between scoped elements.
    pub connections: Vec<DeclaredConnection>,
}

/// The recorded gap requirement for one declared weld connection (contract
/// section 4.1): the permitted local gap range with the tolerance on the
/// comparison, and the owner as the only v0.1 basis — no normative
/// permitted-gap rule is supported (contract section 3.3). `permitted_min_mm`
/// must not exceed `permitted_max_mm`.
#[derive(Debug, Clone, PartialEq)]
pub struct GapRequirement {
    /// One side of the declared weld connection the requirement judges.
    pub a: ElementId,
    /// The other side, distinct from `a`.
    pub b: ElementId,
    /// The permitted minimum local gap in millimetres.
    pub permitted_min_mm: f64,
    /// The permitted maximum local gap in millimetres.
    pub permitted_max_mm: f64,
    /// The recorded tolerance on the comparison (contract section 7.3).
    pub tolerance_mm: f64,
}

/// The weld facts the connectivity rules consume: which two parts a weld
/// element connects, and the authored path when the weld carries one (the
/// A06 record). Existence and path are plain inputs; the recorded-inputs
/// slice reads them from the store.
#[derive(Debug, Clone, PartialEq)]
pub struct WeldFact {
    /// The main part of the weld.
    pub main: ElementId,
    /// The part welded to the main part.
    pub secondary: ElementId,
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
    /// The recorded tolerance that widens this bound's band edge.
    pub tolerance_mm: f64,
    /// The basis of the bound (the recorded requirement's source).
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
    /// Every path point lies on the declared contact within the product
    /// tolerance, measured against both evaluated bodies.
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
/// — within the product tolerance of both surfaces, the mesh bounds absorbed
/// by the near-threshold rule. A weld with no (or an empty) path has no
/// placed solid at the joint and is never locally attached.
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
        for body in [a, b] {
            let measured = contact::point_gap(*point, &body.mesh)?;
            let delta = body.mesh_bound_mm.max(tolerance_mm / 2.0);
            if measured + delta < tolerance_mm {
                continue;
            }
            if measured - delta > tolerance_mm {
                return Ok(Attachment::NotAttached);
            }
            verdict = Attachment::Indeterminate;
        }
    }
    Ok(verdict)
}

/// The measured local gap of an attached weld, in millimetres: the smallest
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
            Some(current) if current <= gap => current,
            _ => gap,
        });
    }
    Ok(best)
}

/// The connectivity rules over the recorded inputs (contract sections 2.1
/// and 5). With no joint intent the rules are not applicable for any element
/// and no finding is produced — the report says so in coverage. Connections
/// with a target outside the checked scope are not checked, like any element
/// outside it; a scoped connection whose target is not an evaluated part of
/// the checked revision reports `unknown` (`target_unresolved`) and the
/// missing inputs named, never a guessed verdict (contract section 4.3).
pub fn connectivity_findings(
    inputs: &RecordedInputs,
    bodies: &[ElementBody],
    scope: &[ElementId],
    tolerance_mm: f64,
) -> Vec<ConnectivityFinding> {
    let Some(intent) = &inputs.intent else {
        return Vec::new();
    };
    let mut findings = Vec::new();
    for connection in &intent.connections {
        if scope.binary_search(&connection.a).is_err()
            || scope.binary_search(&connection.b).is_err()
        {
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
                    missing_inputs: vec!["target_unresolved"],
                });
                continue;
            }
        };
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
                missing_inputs: vec!["attachment_unvalidated"],
            });
            continue;
        }
        let gap = match weld_gap_mm(weld, body_a, body_b) {
            Ok(Some(gap)) => gap,
            // A weld without a usable path was reported not attached above.
            Ok(None) => continue,
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
                    missing_inputs: vec!["unresolved contact predicates"],
                });
                continue;
            }
        };
        let delta = body_a
            .mesh_bound_mm
            .max(body_b.mesh_bound_mm)
            .max(tolerance_mm / 2.0);
        match range_threshold(
            gap,
            requirement.permitted_min_mm,
            requirement.permitted_max_mm,
            requirement.tolerance_mm,
            delta,
        ) {
            // A permissible weld gap passes its applicable gap rule.
            ClashState::Clear => {}
            ClashState::Indeterminate => findings.push(ConnectivityFinding {
                rule: "WELD_WITHOUT_CONTACT",
                status: "unknown",
                elements: vec![connection.a, connection.b],
                message: format!(
                    "the measured local gap {} mm sits at the permitted-range band edge",
                    gap,
                ),
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
                missing_inputs: vec!["indeterminate"],
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
                    missing_inputs: Vec::new(),
                });
            }
        }
    }
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
            // The deepest contained point of either side: the vertex and face
            // probes measure both the containment case and the aligned-prism
            // case whose vertices all lie on shared walls. A crossing whose
            // overlap touches no sample stays unmeasured, never guessed.
            let measured = penetration_depth(&a.mesh, &b.mesh)?
                .into_iter()
                .chain(penetration_depth(&b.mesh, &a.mesh)?)
                .fold(None, |best: Option<f64>, depth| {
                    Some(match best {
                        Some(x) if x >= depth => x,
                        _ => depth,
                    })
                });
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
        delta_mm: delta,
        tolerance_mm,
    })
}

fn penetration_depth(inside: &TriMesh, shell: &TriMesh) -> Result<Option<f64>, ContactError> {
    contact::penetration_depth(inside, shell, CONTACT_LIMITS)
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
            main: id(1),
            secondary: id(2),
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
        let findings =
            connectivity_findings(&inputs, &[a, b], &[id(1), id(2)], CONTACT_TOLERANCE_MM);
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
        let findings =
            connectivity_findings(&inputs, &[a, b], &[id(1), id(2)], CONTACT_TOLERANCE_MM);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule, "WELD_WITHOUT_CONTACT");
        assert_eq!(findings[0].missing_inputs, vec!["attachment_unvalidated"]);
    }

    #[test]
    fn a_permitted_gap_passes_and_an_excessive_gap_fails_with_named_bounds() {
        // The owner requires contact at the weld: the permitted range is
        // [0, 0] with no tolerance, so any measured opening outside the
        // uncertainty band fails.
        let gap_requirement = GapRequirement {
            a: id(1),
            b: id(2),
            permitted_min_mm: 0.0,
            permitted_max_mm: 0.0,
            tolerance_mm: 0.0,
        };
        // A 0.3 mm air slab with the weld path mid-air: attached, and the
        // measured 0.3 mm lies strictly above the permitted maximum edge.
        let a = plate_at(1, 0.0, 0.0, 0.0).unwrap();
        let b = plate_at(2, 0.0, 0.0, 10.3).unwrap();
        let mut inputs = intent(vec![weld_pair()]);
        inputs.welds = vec![weld_at(Some(vec![[100.0, 100.0, 10.15]]))];
        inputs.gaps = vec![gap_requirement.clone()];
        let findings =
            connectivity_findings(&inputs, &[a, b], &[id(1), id(2)], CONTACT_TOLERANCE_MM);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule, "WELD_WITHOUT_CONTACT");
        assert_eq!(findings[0].status, "fail");
        let measured = findings[0].measured_mm.unwrap_or_default();
        assert!((measured - 0.3).abs() <= 1e-6, "{measured}");
        assert_eq!(findings[0].bounds.len(), 2);
        assert_eq!(findings[0].bounds[0].name, "permitted_min");
        assert_eq!(findings[0].bounds[1].name, "permitted_max");
        // The same geometry inside a wider permitted range passes: no finding.
        inputs.gaps = vec![GapRequirement {
            permitted_max_mm: 1.0,
            tolerance_mm: 0.05,
            ..gap_requirement
        }];
        let a = plate_at(1, 0.0, 0.0, 0.0).unwrap();
        let b = plate_at(2, 0.0, 0.0, 10.3).unwrap();
        let findings =
            connectivity_findings(&inputs, &[a, b], &[id(1), id(2)], CONTACT_TOLERANCE_MM);
        assert!(findings.is_empty());
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
        let findings =
            connectivity_findings(&inputs, &[a, b], &[id(1), id(2)], CONTACT_TOLERANCE_MM);
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
}
