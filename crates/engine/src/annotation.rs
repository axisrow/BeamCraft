// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D06 annotation admission and binding resolution against a pinned view snapshot.
//!
//! Admission (F04): a fresh identity, bounded anchors that parse as qualified FeatureIds, and
//! the project cap. Resolution (V04/F06): every stored anchor is looked up in the current
//! snapshot exactly as stored, never re-numbered, re-matched or otherwise repaired. A feature
//! that disappeared leaves the annotation unresolved; a resolvable annotation whose workspace
//! generation differs from its placement pin is stale, because a positional role in an address
//! can name a different feature after an edit. Nothing here ever rewrites an anchor.

use crate::footprint::Footprint;
use crate::query::feature::{FeatureGeometry, FeatureId, lookup_feature};
use crate::query::snapshot::ViewSnapshot;
use beamcraft_geom::Point3;
use beamcraft_model::annotation::{
    Annotation, AnnotationPayload, CreateAnnotation, MAX_ANNOTATIONS,
};
use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::ElementId;
use std::collections::BTreeSet;

/// Decides annotation creation without mutating project state. The caller supplies every
/// existing element identity, including annotations already in the project. Publication still
/// checks the base revision.
pub fn create(
    command: &CreateAnnotation,
    annotations: &[Annotation],
    elements: &BTreeSet<ElementId>,
) -> Result<Footprint, CommandError> {
    command.validate_arguments()?;
    let error = |code, field, message| CommandError::new(code, field, message);
    for anchor in command.annotation.payload.anchors() {
        // The full F06 grammar is admitted here, at the engine edge; the model layer only
        // checks the wire bound.
        if FeatureId::parse(anchor.as_str()).is_err() {
            return Err(error(
                ErrorCode::InvalidArgument,
                "feature",
                "anchor is not a valid feature id",
            ));
        }
    }
    // The pins' values come from the caller's verified snapshot; a pin that does not match
    // its element simply makes the annotation stale at the first resolution.
    let mut ids = elements.clone();
    for annotation in annotations {
        ids.insert(annotation.id);
    }
    if !ids.insert(command.annotation.id) {
        return Err(error(
            ErrorCode::DuplicateId,
            "new_id",
            "annotation identity is already in use",
        ));
    }
    if annotations.len() >= MAX_ANNOTATIONS {
        return Err(error(
            ErrorCode::LimitExceeded,
            "annotation",
            "project exceeds the annotation cap",
        ));
    }
    Footprint::for_create(command.annotation.id, None)
}

/// Why an annotation is unresolved. A malformed stored address is a corruption report: the
/// admission path refuses such text, so finding one in a verified store is integrity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnresolvedReason {
    /// The stored anchor is not a well-formed FeatureId.
    MalformedAddress,
    /// The addressed element is not in the snapshot.
    ElementMissing,
    /// The element exists but the addressed feature does not.
    FeatureMissing,
}

impl UnresolvedReason {
    /// The lowercase wire token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MalformedAddress => "malformed-address",
            Self::ElementMissing => "element-missing",
            Self::FeatureMissing => "feature-missing",
        }
    }
}

/// What a resolvable annotation shows, in world millimetres.
#[derive(Debug, Clone, PartialEq)]
pub struct Bound {
    /// The world-space anchor point of each anchor, in stored anchor order.
    pub anchors_mm: Vec<[f64; 3]>,
    /// The measured distance for a dimension.
    pub value_mm: Option<f64>,
    /// The attribute text of a mark, when the anchored element carries that attribute.
    pub text: Option<String>,
}

/// The binding state of one annotation against one snapshot.
#[derive(Debug, Clone, PartialEq)]
pub enum Resolution {
    /// Every anchor resolves and each addressed body still matches its placement digest.
    Current(Bound),
    /// Every anchor resolves, but an addressed body changed since placement: a positional role
    /// may now name a different feature. Reported, never silently presented as the placed
    /// binding.
    Stale(Bound),
    /// At least one anchor does not resolve; the annotation shows nothing.
    Unresolved(UnresolvedReason),
}

/// The resolution of one annotation, in stored order.
#[derive(Debug, Clone, PartialEq)]
pub struct AnnotationStatus {
    /// The annotation's identity.
    pub id: ElementId,
    /// The binding state.
    pub resolution: Resolution,
}

/// Resolves every annotation against `snapshot`, in stored order.
///
/// `attribute_text` supplies a mark's text from the anchored element's user attributes (I04);
/// a `None` keeps the mark bound but without text.
pub fn resolve(
    snapshot: &ViewSnapshot,
    annotations: &[Annotation],
    attribute_text: &dyn Fn(&ElementId, &str) -> Option<String>,
) -> Vec<AnnotationStatus> {
    annotations
        .iter()
        .map(|annotation| AnnotationStatus {
            id: annotation.id,
            resolution: resolve_one(snapshot, annotation, attribute_text),
        })
        .collect()
}

fn resolve_one(
    snapshot: &ViewSnapshot,
    annotation: &Annotation,
    attribute_text: &dyn Fn(&ElementId, &str) -> Option<String>,
) -> Resolution {
    let anchors: Vec<_> = annotation.payload.anchors().collect();
    if anchors.len() != annotation.pins.len() {
        return Resolution::Unresolved(UnresolvedReason::MalformedAddress);
    }
    let mut points = Vec::new();
    let mut moved = false;
    for (anchor, pin) in anchors.iter().zip(&annotation.pins) {
        match anchor_point(snapshot, anchor.as_str()) {
            Ok(point) => points.push(point),
            Err(reason) => return Resolution::Unresolved(reason),
        }
        let owner = match FeatureId::parse(anchor.as_str()) {
            Ok(id) => id.element(),
            Err(_) => return Resolution::Unresolved(UnresolvedReason::MalformedAddress),
        };
        match snapshot.element(&owner) {
            Some(entry) => moved |= entry.full.mesh_digest != *pin,
            // anchor_point already answered ElementMissing for a missing element.
            None => return Resolution::Unresolved(UnresolvedReason::ElementMissing),
        }
    }
    let bound = Bound {
        value_mm: match &annotation.payload {
            AnnotationPayload::Dimension { .. } => {
                let (a, b) = match points.as_slice() {
                    [a, b] => (*a, *b),
                    _ => return Resolution::Unresolved(UnresolvedReason::MalformedAddress),
                };
                let (a, b) = (Point3::new(a[0], a[1], a[2]), Point3::new(b[0], b[1], b[2]));
                Some(a.distance_to(b))
            }
            _ => None,
        },
        text: match &annotation.payload {
            AnnotationPayload::Mark {
                anchor, attribute, ..
            } => {
                let owner = match FeatureId::parse(anchor.as_str()) {
                    Ok(id) => id.element(),
                    Err(_) => return Resolution::Unresolved(UnresolvedReason::MalformedAddress),
                };
                attribute_text(&owner, attribute)
            }
            _ => None,
        },
        anchors_mm: points,
    };
    if moved {
        Resolution::Stale(bound)
    } else {
        Resolution::Current(bound)
    }
}

/// The world-space anchor point of one stored address: a vertex's position, an edge's midpoint
/// or a face's corner centroid.
fn anchor_point(snapshot: &ViewSnapshot, text: &str) -> Result<[f64; 3], UnresolvedReason> {
    let malformed = UnresolvedReason::MalformedAddress;
    let id = FeatureId::parse(text).map_err(|_| malformed)?;
    if snapshot.element(&id.element()).is_none() {
        return Err(UnresolvedReason::ElementMissing);
    }
    let world = match lookup_feature(snapshot, text) {
        Ok(lookup) => lookup.world,
        Err(_) => return Err(UnresolvedReason::FeatureMissing),
    };
    Ok(match world {
        FeatureGeometry::Vertex { position } => position,
        FeatureGeometry::Edge { ends, .. } => {
            let mut mid = [0.0; 3];
            for end in &ends {
                for (m, v) in mid.iter_mut().zip(end) {
                    *m += *v / 2.0;
                }
            }
            mid
        }
        FeatureGeometry::Face { corners, .. } => {
            let mut centre = [0.0; 3];
            for corner in &corners {
                for (c, v) in centre.iter_mut().zip(corner) {
                    *c += *v / 4.0;
                }
            }
            centre
        }
        FeatureGeometry::Surface { triangles, .. } => {
            let count = triangles.len() * 3;
            if count == 0 {
                return Err(UnresolvedReason::FeatureMissing);
            }
            let mut centre = [0.0; 3];
            for triangle in &triangles {
                for point in triangle {
                    for (c, v) in centre.iter_mut().zip(point) {
                        *c += *v / count as f64;
                    }
                }
            }
            centre
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::Limits;
    use crate::query::snapshot::ViewPin;
    use crate::synthetic::plate_scene;
    use beamcraft_model::annotation::FeatureRef;
    use beamcraft_model::digest::{ContentDigest, Sha256};

    const X0: &str = "fid1:01990000-0000-7000-8000-000000000000:plate:f.x0";
    const X1: &str = "fid1:01990000-0000-7000-8000-000000000000:plate:f.x1";

    fn id(seed: u32) -> ElementId {
        ElementId::parse(&format!("01990000-0000-7000-8000-{seed:012x}")).unwrap()
    }

    fn snapshot(generation: u64) -> ViewSnapshot {
        let pin = ViewPin::new(Sha256::digest(b"d06").unwrap(), generation);
        ViewSnapshot::build(pin, plate_scene(1, false).unwrap(), &Limits::default()).unwrap()
    }

    fn annotation(id: ElementId, from: &str, to: &str, pins: Vec<ContentDigest>) -> Annotation {
        Annotation {
            id,
            name: "D1".to_owned(),
            payload: AnnotationPayload::Dimension {
                from: FeatureRef::parse(from).unwrap(),
                to: FeatureRef::parse(to).unwrap(),
            },
            pins,
        }
    }

    /// The full-body digest of the synthetic plate, as a snapshot reports it.
    fn body_digest(snapshot: &ViewSnapshot) -> ContentDigest {
        match snapshot.element(&id(0)) {
            Some(entry) => entry.full.mesh_digest,
            None => ContentDigest::from_bytes([0; 32]),
        }
    }

    /// A dimension placed against the given body digests (one per anchor).
    fn dimension(to: &str) -> CreateAnnotation {
        placed_dimension(to, body_digest(&snapshot(1)))
    }

    fn placed_dimension(to: &str, pin: ContentDigest) -> CreateAnnotation {
        CreateAnnotation {
            annotation: annotation(id(0xaaa), X0, to, vec![pin; 2]),
        }
    }

    fn one(request: &CreateAnnotation) -> &[Annotation] {
        std::slice::from_ref(&request.annotation)
    }

    #[test]
    fn creation_admits_a_fresh_identity_and_refuses_malformed_and_reused_and_the_cap() {
        let elements = BTreeSet::from([id(0)]);
        assert!(create(&dimension(X1), &[], &elements).is_ok());
        let mut malformed = dimension(X1);
        if let AnnotationPayload::Dimension { to, .. } = &mut malformed.annotation.payload {
            *to =
                FeatureRef::parse("fid1:01990000-0000-7000-8000-000000000000:plate:f.x9").unwrap();
        }
        assert_eq!(
            create(&malformed, &[], &elements).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        let mut unpinned = dimension(X1);
        unpinned.annotation.pins.clear();
        assert_eq!(
            create(&unpinned, &[], &elements).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            create(&dimension(X1), &[], &BTreeSet::from([id(0xaaa)]))
                .unwrap_err()
                .code,
            ErrorCode::DuplicateId
        );
        let pin = body_digest(&snapshot(1));
        let full: Vec<Annotation> = (0..MAX_ANNOTATIONS)
            .map(|i| annotation(id(0x1000 + i as u32), X0, X1, vec![pin; 2]))
            .collect();
        assert_eq!(
            create(&dimension(X1), &full, &BTreeSet::new())
                .unwrap_err()
                .code,
            ErrorCode::LimitExceeded
        );
    }

    #[test]
    fn a_placed_dimension_resolves_current_and_measures_the_anchors() {
        let snapshot = snapshot(1);
        let request = dimension(X1);
        let statuses = resolve(&snapshot, one(&request), &|_, _| None);
        assert_eq!(statuses.len(), 1);
        let Resolution::Current(bound) = &statuses[0].resolution else {
            panic!("expected a current resolution");
        };
        assert_eq!(bound.anchors_mm.len(), 2);
        let value = bound.value_mm.unwrap_or(0.0);
        assert!((value - 65.0).abs() < 1e-9, "width {value}");
    }

    #[test]
    fn a_mark_takes_its_text_from_the_attribute_lookup() {
        let snapshot = snapshot(1);
        let mark = CreateAnnotation {
            annotation: Annotation {
                id: id(0xbbb),
                name: "M1".to_owned(),
                payload: AnnotationPayload::Mark {
                    anchor: FeatureRef::parse(X1).unwrap(),
                    attribute: "PartName".to_owned(),
                },
                pins: vec![body_digest(&snapshot)],
            },
        };
        let statuses = resolve(&snapshot, one(&mark), &|element, name| {
            (*element == id(0) && name == "PartName").then(|| "P/1".to_owned())
        });
        match &statuses[0].resolution {
            Resolution::Current(bound) => assert_eq!(bound.text.as_deref(), Some("P/1")),
            other => panic!("expected a current mark, got {other:?}"),
        }
    }

    #[test]
    fn a_changed_body_is_stale_never_silently_current() {
        // The address resolves against a plate whose body differs from the placement pin:
        // reported stale, because the role may now name a different feature of that body.
        let request = placed_dimension(X1, ContentDigest::from_bytes([7; 32]));
        let statuses = resolve(&snapshot(1), one(&request), &|_, _| None);
        assert!(matches!(statuses[0].resolution, Resolution::Stale(_)));
    }

    #[test]
    fn a_lost_element_and_a_lost_feature_are_unresolved_with_the_reason() {
        let empty = ViewSnapshot::build(
            ViewPin::new(Sha256::digest(b"d06").unwrap(), 1),
            Vec::new(),
            &Limits::default(),
        )
        .unwrap();
        let request = dimension(X1);
        let statuses = resolve(&empty, one(&request), &|_, _| None);
        assert_eq!(
            statuses[0].resolution,
            Resolution::Unresolved(UnresolvedReason::ElementMissing)
        );
        // A well-formed address whose feature the body does not carry: a member role on a
        // plate element is never re-matched to a plate role.
        let member = "fid1:01990000-0000-7000-8000-000000000000:member:f.x1";
        let other = CreateAnnotation {
            annotation: annotation(id(0xccc), X0, member, vec![body_digest(&snapshot(1)); 2]),
        };
        let statuses = resolve(&snapshot(1), one(&other), &|_, _| None);
        assert_eq!(
            statuses[0].resolution,
            Resolution::Unresolved(UnresolvedReason::FeatureMissing)
        );
    }

    #[test]
    fn a_tampered_stored_address_is_unresolved_not_repaired() {
        let snapshot = snapshot(1);
        let mut broken = dimension(X1);
        if let AnnotationPayload::Dimension { from, .. } = &mut broken.annotation.payload {
            // Beyond the wire bound the store admits only engine-checked text; an address that
            // names a foreign element stays unresolved, never re-bound to a local feature.
            *from =
                FeatureRef::parse("fid1:01990000-0000-7000-8000-000000000001:plate:f.x1").unwrap();
        }
        let statuses = resolve(&snapshot, one(&broken), &|_, _| None);
        assert_eq!(
            statuses[0].resolution,
            Resolution::Unresolved(UnresolvedReason::ElementMissing)
        );
    }

    #[test]
    fn a_pin_count_that_disagrees_with_the_anchors_is_malformed() {
        let mut broken = dimension(X1);
        broken.annotation.pins.pop();
        let statuses = resolve(&snapshot(1), one(&broken), &|_, _| None);
        assert_eq!(
            statuses[0].resolution,
            Resolution::Unresolved(UnresolvedReason::MalformedAddress)
        );
    }
}
