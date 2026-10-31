// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D06 hand-placed annotations: dimensions, marks and leader callouts bound to features.
//!
//! An annotation is an authored element of kind `annotation` with no engineering body. Its
//! anchors are qualified `FeatureId` texts (F06 section 5) stored verbatim: the model layer
//! never rewrites, re-numbers or re-binds an address, so a feature that disappears leaves the
//! annotation unresolved instead of silently pointing somewhere else. With the anchors the
//! annotation stores the full-body mesh digest of each addressed element at placement time;
//! resolution (an engine query) reports a resolvable annotation as stale when any addressed
//! body has changed, because a positional role in an address may then name a different
//! feature. An edit that touches no addressed body never stales anything.
//!
//! A mark's displayed text comes from the anchored element's user attributes (I04): the mark
//! stores the attribute name, and the text is looked up at resolution time, never copied.

use crate::attribute::attr_name_is_valid;
use crate::command::{CommandError, label_is_valid};
use crate::digest::{ContentDigest, Sha256};
use crate::error::ErrorCode;
use crate::id::ElementId;
use crate::limits::COORDINATE_CAP_MM;
use beamcraft_geom::canonical_f64_bytes;

/// Most annotations of one project.
pub const MAX_ANNOTATIONS: usize = 4096;

/// Longest stored qualified `FeatureId` text, the F06 section 5 wire bound.
pub const MAX_FEATURE_REF_BYTES: usize = 64;

/// Longest leader text, the bounded label rule.
pub const MAX_LEADER_TEXT_BYTES: usize = 256;

/// One stored anchor: the qualified `FeatureId` text of the addressed feature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeatureRef(String);

impl FeatureRef {
    /// Wraps an anchor after the model-layer shape check: non-empty bounded ASCII with the
    /// `rule:element:local` separators. The full F06 grammar is the engine's decision.
    pub fn parse(text: &str) -> Result<Self, CommandError> {
        let shape = text.len() <= MAX_FEATURE_REF_BYTES
            && text.is_ascii()
            && text.matches(':').count() >= 2;
        if !shape {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "feature",
                "anchor is not a bounded qualified feature id",
            ));
        }
        Ok(Self(text.to_owned()))
    }

    /// The stored text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What the annotation shows, and what addresses it.
#[derive(Debug, Clone, PartialEq)]
pub enum AnnotationPayload {
    /// A linear dimension between two addressed features.
    Dimension {
        /// First anchor.
        from: FeatureRef,
        /// Second anchor.
        to: FeatureRef,
    },
    /// A mark whose text is the value of one user attribute of the anchored element.
    Mark {
        /// The addressed feature.
        anchor: FeatureRef,
        /// Attribute name without the `bim:attr:` prefix, admitted by the I04 name rule.
        attribute: String,
    },
    /// A leader callout: free text at a hand-placed end, pointing at the addressed feature.
    Leader {
        /// The addressed feature.
        anchor: FeatureRef,
        /// The callout text.
        text: String,
        /// The leader end in world millimetres.
        end_mm: [f64; 3],
    },
}

impl AnnotationPayload {
    /// The stored kind token: `dimension`, `mark` or `leader`.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Dimension { .. } => "dimension",
            Self::Mark { .. } => "mark",
            Self::Leader { .. } => "leader",
        }
    }

    /// Every stored anchor, in address order: two for a dimension, one otherwise.
    pub fn anchors(&self) -> impl Iterator<Item = &FeatureRef> {
        let pair = match self {
            Self::Dimension { from, to } => [Some(from), Some(to), None],
            Self::Mark { anchor, .. } | Self::Leader { anchor, .. } => [Some(anchor), None, None],
        };
        pair.into_iter().flatten()
    }

    /// Admits every anchor, the attribute name, the text and the leader end.
    pub fn validate(&self) -> Result<(), CommandError> {
        let bad = |field: &'static str, message: &'static str| {
            CommandError::new(ErrorCode::InvalidArgument, field, message)
        };
        let coordinate = |field: &'static str, value: f64| {
            if value.is_finite() && value.abs() <= COORDINATE_CAP_MM {
                Ok(())
            } else {
                Err(CommandError::new(
                    ErrorCode::NonFiniteNumber,
                    field,
                    "leader end is not a finite coordinate within the cap",
                )
                .with_rule("DV-ATTR-003"))
            }
        };
        match self {
            Self::Dimension { from, to } => {
                if from == to {
                    return Err(bad("to", "the two anchors must differ"));
                }
                from.validate("from")?;
                to.validate("to")
            }
            Self::Mark { anchor, attribute } => {
                anchor.validate("anchor")?;
                if !attr_name_is_valid(attribute) {
                    return Err(bad("attribute", "not a valid attribute name"));
                }
                Ok(())
            }
            Self::Leader {
                anchor,
                text,
                end_mm,
            } => {
                anchor.validate("anchor")?;
                if !label_is_valid(text) || text.len() > MAX_LEADER_TEXT_BYTES {
                    return Err(bad(
                        "text",
                        "leader text must be 1 to 256 bytes, no controls",
                    ));
                }
                coordinate("end_mm", end_mm[0])?;
                coordinate("end_mm", end_mm[1])?;
                coordinate("end_mm", end_mm[2])
            }
        }
    }
}

impl FeatureRef {
    fn validate(&self, field: &'static str) -> Result<(), CommandError> {
        FeatureRef::parse(self.as_str())
            .map(|_| ())
            .map_err(|mut e| {
                e.field = field;
                e
            })
    }
}

/// A hand-placed annotation: a named element of kind `annotation` bound to features.
#[derive(Debug, Clone, PartialEq)]
pub struct Annotation {
    /// Stable element identity.
    pub id: ElementId,
    /// Label, 1 to 256 UTF-8 bytes without control characters. Not an identity.
    pub name: String,
    /// What the annotation shows and what it addresses.
    pub payload: AnnotationPayload,
    /// The full-body mesh digest of each addressed element at placement time, in anchor
    /// order. Resolution compares them with the current bodies and reports a resolvable
    /// annotation as stale on any difference; one pin per anchor is required.
    pub pins: Vec<ContentDigest>,
}

impl Annotation {
    /// Admits the label and the payload, in that order.
    pub fn validate(&self) -> Result<(), CommandError> {
        if !label_is_valid(&self.name) {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "name",
                "label must be 1 to 256 bytes without control characters",
            )
            .with_rule("DV-BASE-003"));
        }
        if self.pins.len() != self.payload.anchors().count() {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "pins",
                "every anchor carries one placement body digest",
            ));
        }
        self.payload.validate()
    }
}

/// D06 creation request.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateAnnotation {
    /// The complete annotation to create.
    pub annotation: Annotation,
}

impl CreateAnnotation {
    /// Admits all model arguments before consulting project state.
    pub fn validate_arguments(&self) -> Result<(), CommandError> {
        self.annotation.validate()
    }

    /// Versioned, length-prefixed fingerprint of all authored input, as [`crate::command`]
    /// commands do; it is not the F04 canonical form.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        self.validate_arguments()?;
        let bad = || {
            CommandError::new(
                ErrorCode::NonFiniteNumber,
                "annotation",
                "value cannot be encoded",
            )
            .with_rule("DV-ATTR-003")
        };
        let mut out = b"BeamCraft/D06/CreateAnnotation/v1\0".to_vec();
        out.extend_from_slice(self.annotation.id.as_bytes());
        let part = |out: &mut Vec<u8>, text: &str| {
            out.extend_from_slice(&(text.len() as u64).to_le_bytes());
            out.extend_from_slice(text.as_bytes());
        };
        part(&mut out, &self.annotation.name);
        part(&mut out, self.annotation.payload.as_str());
        let mut scalars: Vec<f64> = Vec::new();
        match &self.annotation.payload {
            AnnotationPayload::Dimension { from, to } => {
                part(&mut out, from.as_str());
                part(&mut out, to.as_str());
            }
            AnnotationPayload::Mark { anchor, attribute } => {
                part(&mut out, anchor.as_str());
                part(&mut out, attribute);
            }
            AnnotationPayload::Leader {
                anchor,
                text,
                end_mm,
            } => {
                part(&mut out, anchor.as_str());
                part(&mut out, text);
                scalars.extend(end_mm);
            }
        }
        for value in scalars {
            out.extend_from_slice(&canonical_f64_bytes(value).map_err(|_| bad())?);
        }
        for pin in &self.annotation.pins {
            out.extend_from_slice(pin.as_bytes());
        }
        Sha256::digest(&out).map_err(|_| {
            CommandError::new(ErrorCode::LimitExceeded, "content", "content is too large")
                .with_rule("DV-LIM-001")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ELEMENT: &str = "01990000-0000-7000-8000-000000000001";
    const FACE: &str = "fid1:01990000-0000-7000-8000-000000000001:plate:f.x1";

    fn pin(seed: u8) -> ContentDigest {
        ContentDigest::from_bytes([seed; 32])
    }

    fn annotation(payload: AnnotationPayload) -> CreateAnnotation {
        let pins = match &payload {
            AnnotationPayload::Dimension { .. } => vec![pin(1), pin(2)],
            _ => vec![pin(1)],
        };
        CreateAnnotation {
            annotation: Annotation {
                id: ElementId::parse(ELEMENT).unwrap(),
                name: "D1".to_owned(),
                payload,
                pins,
            },
        }
    }

    #[test]
    fn a_dimension_with_two_distinct_bounded_anchors_is_admitted() {
        let request = annotation(AnnotationPayload::Dimension {
            from: FeatureRef::parse(FACE).unwrap(),
            to: FeatureRef::parse("fid1:01990000-0000-7000-8000-000000000001:plate:f.x0").unwrap(),
        });
        assert!(request.validate_arguments().is_ok());
        assert!(request.content_fingerprint().is_ok());
    }

    #[test]
    fn anchors_outside_the_wire_bound_are_refused() {
        let long = format!("fid1:{ELEMENT}:plate:f.x{}", "0".repeat(64));
        assert_eq!(
            FeatureRef::parse(&long).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            FeatureRef::parse("fid1:no-separator").unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            FeatureRef::parse("fid1:plaat\u{00e9}:f.x1")
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn a_dimension_needs_two_distinct_anchors_and_a_mark_a_valid_attribute() {
        let same = FeatureRef::parse(FACE).unwrap();
        assert_eq!(
            annotation(AnnotationPayload::Dimension {
                from: same.clone(),
                to: same,
            })
            .validate_arguments()
            .unwrap_err()
            .code,
            ErrorCode::InvalidArgument
        );
        let mut mark = annotation(AnnotationPayload::Mark {
            anchor: FeatureRef::parse(FACE).unwrap(),
            attribute: "PartName".to_owned(),
        });
        assert!(mark.validate_arguments().is_ok());
        mark.annotation.payload = AnnotationPayload::Mark {
            anchor: FeatureRef::parse(FACE).unwrap(),
            attribute: "bad name!".to_owned(),
        };
        assert_eq!(
            mark.validate_arguments().unwrap_err().code,
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn a_leader_needs_bounded_text_and_capped_coordinates() {
        let mut leader = annotation(AnnotationPayload::Leader {
            anchor: FeatureRef::parse(FACE).unwrap(),
            text: "web".to_owned(),
            end_mm: [100.0, 200.0, 300.0],
        });
        assert!(leader.validate_arguments().is_ok());
        leader.annotation.payload = AnnotationPayload::Leader {
            anchor: FeatureRef::parse(FACE).unwrap(),
            text: String::new(),
            end_mm: [100.0, 200.0, 300.0],
        };
        assert_eq!(
            leader.validate_arguments().unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        leader.annotation.payload = AnnotationPayload::Leader {
            anchor: FeatureRef::parse(FACE).unwrap(),
            text: "web".to_owned(),
            end_mm: [f64::NAN, 0.0, 0.0],
        };
        assert_eq!(
            leader.validate_arguments().unwrap_err().code,
            ErrorCode::NonFiniteNumber
        );
    }

    #[test]
    fn the_fingerprint_covers_every_authored_field() {
        let base = annotation(AnnotationPayload::Leader {
            anchor: FeatureRef::parse(FACE).unwrap(),
            text: "web".to_owned(),
            end_mm: [100.0, 200.0, 300.0],
        });
        let mut repinned = base.clone();
        repinned.annotation.pins = vec![pin(9)];
        assert_ne!(
            base.content_fingerprint().unwrap(),
            repinned.content_fingerprint().unwrap()
        );
        let mut renamed = base.clone();
        renamed.annotation.name = "D2".to_owned();
        assert_ne!(
            base.content_fingerprint().unwrap(),
            renamed.content_fingerprint().unwrap()
        );
    }

    #[test]
    fn every_anchor_needs_exactly_one_pin() {
        let dimension = Annotation {
            id: ElementId::parse(ELEMENT).unwrap(),
            name: "D1".to_owned(),
            payload: AnnotationPayload::Dimension {
                from: FeatureRef::parse(FACE).unwrap(),
                to: FeatureRef::parse("fid1:01990000-0000-7000-8000-000000000001:plate:f.x0")
                    .unwrap(),
            },
            pins: vec![pin(1)],
        };
        assert_eq!(
            dimension.validate().unwrap_err().code,
            ErrorCode::InvalidArgument
        );
    }
}
