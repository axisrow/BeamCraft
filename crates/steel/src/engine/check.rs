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
        Ok(Self {
            id,
            name,
            kind,
            mesh,
            mesh_bound_mm,
        })
    }

    /// The cheap world-bounds overlap with another body, before the exact
    /// pair decision.
    pub fn bounds_overlap(&self, other: &Self) -> bool {
        let extent = |mesh: &TriMesh| {
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
        };
        let (alo, ahi) = extent(&self.mesh);
        let (blo, bhi) = extent(&other.mesh);
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
}
