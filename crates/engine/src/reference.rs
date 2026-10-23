// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! R01 engine API: attach a foreign USD source as a read-only reference, move or re-read the
//! wrapper, and refuse every write aimed at foreign content.
//!
//! The engine decides; it reads no file and composes no stage. The source bytes are admitted
//! earlier (their digest, size, default prim and declared units arrive as a
//! [`SourcePin`]), and the layer engine authors the result. What this module fixes is the policy:
//!
//! * **Two parts.** The wrapper (label, rigid placement, units interpretation) is owned and
//!   editable; the foreign content is never edited. [`write_foreign`] is the one door a write to
//!   the source or to a foreign prim can knock on, and its success type is [`Infallible`]: no
//!   input makes it succeed.
//! * **Separate storage.** Moving a wrapper writes the placement and the derived (world
//!   measurement) group; correcting its units writes the source-input group. Neither group names
//!   the foreign layer, and the footprints say so.
//! * **Explicit correction.** The units and up axis a source declares are recorded as declared.
//!   Reading the source as something else is a command ([`CorrectReferenceUnits`]) on the
//!   wrapper, stored with the source digest it was made for. Without it the source is taken as
//!   stored, in project units and Z up, and [`ReferenceRecord::units_status`] says when that is
//!   doubtful.
//!
//! For a reference the F04 section 8 group `shape_inputs` means the source pin and its units
//! interpretation: what is referenced and how it is read.

use crate::footprint::Footprint;
use beamcraft_model::command::{CommandError, Placement};
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::ElementId;
use beamcraft_model::reference::{
    AttachReference, CorrectReferenceUnits, ForeignTarget, MoveReference, PROJECT_METERS_PER_UNIT,
    SourcePin, UnitsInterpretation, UpAxis, UpdateReferenceSource, WriteForeign,
};
use core::convert::Infallible;

/// A wrapper as the engine sees it, whether just admitted or read back from a committed
/// revision.
#[derive(Debug, Clone, PartialEq)]
pub struct ReferenceRecord {
    /// Identity of the wrapper.
    pub id: ElementId,
    /// Label of the wrapper.
    pub name: String,
    /// Rigid placement of the wrapper, stored apart from the foreign content.
    pub placement: Placement,
    /// The pinned snapshot of the source.
    pub source: SourcePin,
    /// The explicit units interpretation, if one was made.
    pub units: Option<UnitsInterpretation>,
}

/// How the wrapper reads the units and up axis of its source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitsStatus {
    /// No correction, and the source declares the project's own units and Z up (or declares only
    /// the part that agrees).
    Matches,
    /// No correction, and the source declares units or an up axis that differ from the
    /// project's: the geometry is not in project millimetres until the owner decides.
    NeedsCorrection,
    /// No correction, and the source declares nothing. USD's own fallback (0.01 m, Y up) is not
    /// the project's, so nothing is assumed.
    Undeclared,
    /// An explicit correction that agrees with what the source declares.
    Corrected,
    /// An explicit correction that contradicts what the source declares. It is the owner's
    /// statement and is honoured, but it is flagged.
    CorrectedDiffersFromDeclared,
}

impl UnitsStatus {
    /// The stable token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Matches => "matches_project",
            Self::NeedsCorrection => "needs_correction",
            Self::Undeclared => "undeclared",
            Self::Corrected => "corrected",
            Self::CorrectedDiffersFromDeclared => "corrected_differs_from_declared",
        }
    }
}

impl ReferenceRecord {
    /// See [`UnitsStatus`].
    pub fn units_status(&self) -> UnitsStatus {
        let declared = (
            self.source.declared_meters_per_unit,
            self.source.declared_up_axis,
        );
        match self.units {
            Some(i) => {
                let agrees = declared.0.is_none_or(|d| d == i.meters_per_unit)
                    && declared.1.is_none_or(|d| d == i.up_axis);
                if agrees {
                    UnitsStatus::Corrected
                } else {
                    UnitsStatus::CorrectedDiffersFromDeclared
                }
            }
            None => match declared {
                (None, None) => UnitsStatus::Undeclared,
                (units, axis) => {
                    if units.is_none_or(|u| u == PROJECT_METERS_PER_UNIT)
                        && axis.is_none_or(|a| a == UpAxis::Z)
                    {
                        UnitsStatus::Matches
                    } else {
                        UnitsStatus::NeedsCorrection
                    }
                }
            },
        }
    }

    /// Maps a point given in the source layer's own coordinates to world millimetres:
    /// the explicit units interpretation (scale, then Y-up to Z-up), then the wrapper's rigid
    /// placement. Without an interpretation the point is taken as stored. This is the transform
    /// that measurements of the source (R02) must use; no measurement is made here.
    ///
    /// Fails with `NON_FINITE_NUMBER` when an input or the result is not finite.
    pub fn source_point_to_world(&self, point: [f64; 3]) -> Result<[f64; 3], CommandError> {
        let bad = || {
            CommandError::new(
                ErrorCode::NonFiniteNumber,
                "point",
                "value is not finite or the result overflows",
            )
        };
        if point.iter().any(|v| !v.is_finite()) {
            return Err(bad());
        }
        let mut v = point;
        if let Some(i) = &self.units {
            let scale = i.scale();
            v = v.map(|c| c * scale);
            if i.up_axis == UpAxis::Y {
                // Rotation about X by +90 degrees: y becomes z, z becomes -y.
                v = [v[0], -v[2], v[1]];
            }
        }
        let [x, y, z, w] = self.placement.orientation_xyzw;
        let norm = (((x * x + y * y) + z * z) + w * w).sqrt();
        if !(norm.is_finite() && norm > 0.0) {
            return Err(bad());
        }
        let (u, w) = ([x / norm, y / norm, z / norm], w / norm);
        let cross = |a: [f64; 3], b: [f64; 3]| {
            [
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ]
        };
        let t = cross(u, v).map(|c| 2.0 * c);
        let uxt = cross(u, t);
        let rotated = [
            v[0] + w * t[0] + uxt[0],
            v[1] + w * t[1] + uxt[1],
            v[2] + w * t[2] + uxt[2],
        ];
        let [tx, ty, tz] = self.placement.translation_mm;
        let world = [rotated[0] + tx, rotated[1] + ty, rotated[2] + tz];
        if world.iter().any(|c| !c.is_finite()) {
            return Err(bad());
        }
        Ok(world)
    }
}

/// An admitted `AttachReference`.
#[derive(Debug, Clone, PartialEq)]
pub struct AttachOutcome {
    /// The wrapper to author.
    pub record: ReferenceRecord,
    /// Declared reads and writes.
    pub footprint: Footprint,
    /// Content fingerprint of the request, for replay decisions.
    pub fingerprint: ContentDigest,
}

/// Admits an `AttachReference`: the arguments (F04 order: label, placement, source pin), then
/// that the identity is unused (`DUPLICATE_ID`). Nothing is applied.
pub fn attach(
    command: &AttachReference,
    existing: &dyn Fn(&ElementId) -> bool,
) -> Result<AttachOutcome, CommandError> {
    command.validate_arguments()?;
    if existing(&command.new_id) {
        return Err(CommandError::new(
            ErrorCode::DuplicateId,
            "new_id",
            "element identity is already owned",
        ));
    }
    Ok(AttachOutcome {
        record: ReferenceRecord {
            id: command.new_id,
            name: command.name.clone(),
            placement: command.placement,
            source: command.source.clone(),
            units: None,
        },
        footprint: Footprint::for_attach(command.new_id)?,
        fingerprint: command.content_fingerprint()?,
    })
}

/// Admitted result of an explicit R03 source replacement.
#[derive(Debug, Clone, PartialEq)]
pub struct UpdateSourceOutcome {
    /// Updated wrapper. The previous explicit units correction is cleared because it was pinned
    /// to the old digest and cannot be silently carried to new bytes.
    pub record: ReferenceRecord,
    /// Source-input replacement and derived geometry invalidation.
    pub footprint: Footprint,
    /// Fingerprint used for exact operation replay.
    pub fingerprint: ContentDigest,
}

/// Admits replacing an existing wrapper's pinned source. The expected digest and monotonic
/// version make this an explicit compare-and-swap; no file-system change is followed implicitly.
pub fn update_source(
    current: &ReferenceRecord,
    command: &UpdateReferenceSource,
) -> Result<UpdateSourceOutcome, CommandError> {
    command.validate_arguments()?;
    same_wrapper(current, command.target_id)?;
    if current.source.digest != command.expected_source_digest {
        return Err(CommandError::new(
            ErrorCode::BaseRevisionMismatch,
            "expected_source_digest",
            "the reference source changed since it was inspected",
        ));
    }
    let version = current.source.version.checked_add(1).ok_or_else(|| {
        CommandError::new(
            ErrorCode::LimitExceeded,
            "source.version",
            "reference source version is exhausted",
        )
    })?;
    if command.source.version != version {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "source.version",
            "updated source version must increment by one",
        ));
    }
    let mut record = current.clone();
    record.source = command.source.clone();
    record.units = None;
    Ok(UpdateSourceOutcome {
        record,
        footprint: Footprint::for_update(current.id, false, false, true, false, None)?,
        fingerprint: command.content_fingerprint()?,
    })
}

/// An admitted `MoveReference`.
#[derive(Debug, Clone, PartialEq)]
pub struct MoveOutcome {
    /// The wrapper after the move; its source pin and units are the earlier ones.
    pub record: ReferenceRecord,
    /// Declared reads and writes: the placement and the derived (world-measurement) group.
    pub footprint: Footprint,
    /// Content fingerprint of the request.
    pub fingerprint: ContentDigest,
}

fn same_wrapper(record: &ReferenceRecord, id: ElementId) -> Result<(), CommandError> {
    if record.id == id {
        Ok(())
    } else {
        Err(CommandError::new(
            ErrorCode::NotFound,
            "target_id",
            "the reference is not the addressed element",
        ))
    }
}

/// Admits a `MoveReference` against the wrapper as it is now. The source pin, the units
/// interpretation and the foreign content are not part of the result's writes.
pub fn move_reference(
    record: &ReferenceRecord,
    command: &MoveReference,
) -> Result<MoveOutcome, CommandError> {
    same_wrapper(record, command.target_id)?;
    command.validate_arguments()?;
    let mut moved = record.clone();
    moved.placement = command.placement;
    Ok(MoveOutcome {
        record: moved,
        footprint: Footprint::for_update(record.id, false, false, false, true, None)?,
        fingerprint: command.content_fingerprint()?,
    })
}

/// An admitted `CorrectReferenceUnits`.
#[derive(Debug, Clone, PartialEq)]
pub struct CorrectionOutcome {
    /// The wrapper after the correction.
    pub record: ReferenceRecord,
    /// Declared reads and writes: the source-input group and the derived group.
    pub footprint: Footprint,
    /// Content fingerprint of the request.
    pub fingerprint: ContentDigest,
}

/// Admits a `CorrectReferenceUnits` against the wrapper as it is now.
pub fn correct_units(
    record: &ReferenceRecord,
    command: &CorrectReferenceUnits,
) -> Result<CorrectionOutcome, CommandError> {
    same_wrapper(record, command.target_id)?;
    command.validate_arguments()?;
    let mut corrected = record.clone();
    corrected.units = command.interpretation;
    Ok(CorrectionOutcome {
        record: corrected,
        footprint: Footprint::for_update(record.id, false, false, true, false, None)?,
        fingerprint: command.content_fingerprint()?,
    })
}

/// The decision on a write to foreign content. It is **always a refusal**: the success type is
/// [`Infallible`].
///
/// * the stored source layer: `FOREIGN_READ_ONLY`;
/// * a prim at or below the source's root prim, existing or not (setting an attribute, adding a
///   child or authoring an override all alter foreign content): `FOREIGN_READ_ONLY`;
/// * a path outside the source's namespace is not part of this reference: `NOT_FOUND`;
/// * a malformed path or attribute: its argument error first (F04 validation order).
///
/// The wrapper itself is not foreign: its label, placement and units are changed with their own
/// commands, never with this one.
pub fn write_foreign(
    record: &ReferenceRecord,
    command: &WriteForeign,
) -> Result<Infallible, CommandError> {
    command.validate_arguments()?;
    same_wrapper(record, command.reference_id)?;
    let refusal = |message: &'static str| {
        Err(CommandError::new(
            ErrorCode::ForeignReadOnly,
            "target",
            message,
        ))
    };
    match &command.target {
        ForeignTarget::SourceLayer => refusal("the source layer is read-only"),
        ForeignTarget::Prim { path, .. } => {
            let root = format!("/{}", record.source.default_prim);
            let inside = path == &root
                || path
                    .strip_prefix(&root)
                    .is_some_and(|rest| rest.starts_with('/'));
            if inside {
                refusal("foreign prims are read-only, including overrides in owned layers")
            } else {
                Err(CommandError::new(
                    ErrorCode::NotFound,
                    "path",
                    "the path is not in this reference's source",
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use beamcraft_model::digest::Sha256;
    use beamcraft_model::reference::{ForeignTarget, SourceFormat};

    fn id(n: u8) -> ElementId {
        ElementId::parse(&format!("019a7bf0-0000-7000-8000-0000000000{n:02x}")).unwrap()
    }

    fn pin() -> SourcePin {
        SourcePin {
            digest: Sha256::digest(b"source").unwrap(),
            bytes: 6,
            format: SourceFormat::Usda,
            default_prim: "Site".into(),
            version: 1,
            label: "site.usda".into(),
            declared_meters_per_unit: Some(1.0),
            declared_up_axis: Some(UpAxis::Y),
        }
    }

    fn attach_command() -> AttachReference {
        AttachReference {
            new_id: id(0xa1),
            name: "Site".into(),
            placement: Placement::IDENTITY,
            source: pin(),
        }
    }

    fn record() -> ReferenceRecord {
        attach(&attach_command(), &|_| false).unwrap().record
    }

    #[test]
    fn attach_admits_a_new_wrapper_and_declares_its_footprint() {
        let outcome = attach(&attach_command(), &|_| false).unwrap();
        assert_eq!(outcome.record.units, None);
        assert_eq!(outcome.record.source, pin());
        let groups: Vec<_> = outcome.footprint.writes().iter().map(|e| e.group).collect();
        use crate::footprint::Group as G;
        for g in [G::Lifecycle, G::Label, G::ShapeInputs, G::Placement] {
            assert!(groups.contains(&g), "{g:?}");
        }
        assert!(!groups.contains(&G::Derived));
        assert_eq!(outcome.footprint.reads().len(), 1);
        assert_eq!(
            outcome.fingerprint,
            attach_command().content_fingerprint().unwrap()
        );
    }

    #[test]
    fn attach_refuses_a_used_identity_and_bad_arguments() {
        assert_eq!(
            attach(&attach_command(), &|i| *i == id(0xa1))
                .unwrap_err()
                .code,
            ErrorCode::DuplicateId
        );
        let mut bad = attach_command();
        bad.name = String::new();
        assert_eq!(
            attach(&bad, &|_| false).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        let mut bad = attach_command();
        bad.source.bytes = 0;
        assert_eq!(
            attach(&bad, &|_| false).unwrap_err().code,
            ErrorCode::LimitExceeded
        );
    }

    #[test]
    fn a_move_writes_the_placement_and_nothing_about_the_source() {
        let moved = move_reference(
            &record(),
            &MoveReference {
                target_id: id(0xa1),
                placement: Placement {
                    translation_mm: [1.0, 2.0, 3.0],
                    orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
                },
            },
        )
        .unwrap();
        assert_eq!(moved.record.placement.translation_mm, [1.0, 2.0, 3.0]);
        assert_eq!(moved.record.source, pin());
        use crate::footprint::Group as G;
        let groups: Vec<_> = moved.footprint.writes().iter().map(|e| e.group).collect();
        assert_eq!(groups, vec![G::Placement, G::Derived]);
        let wrong = move_reference(
            &record(),
            &MoveReference {
                target_id: id(0xa2),
                placement: Placement::IDENTITY,
            },
        );
        assert_eq!(wrong.unwrap_err().code, ErrorCode::NotFound);
        let bad = move_reference(
            &record(),
            &MoveReference {
                target_id: id(0xa1),
                placement: Placement {
                    translation_mm: [f64::NAN, 0.0, 0.0],
                    orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
                },
            },
        );
        assert_eq!(bad.unwrap_err().code, ErrorCode::NonFiniteNumber);
    }

    #[test]
    fn a_correction_writes_the_source_input_group_and_keeps_the_placement() {
        let command = CorrectReferenceUnits {
            target_id: id(0xa1),
            interpretation: Some(UnitsInterpretation {
                meters_per_unit: 1.0,
                up_axis: UpAxis::Y,
            }),
        };
        let outcome = correct_units(&record(), &command).unwrap();
        assert_eq!(outcome.record.units, command.interpretation);
        assert_eq!(outcome.record.placement, Placement::IDENTITY);
        use crate::footprint::Group as G;
        let groups: Vec<_> = outcome.footprint.writes().iter().map(|e| e.group).collect();
        assert_eq!(groups, vec![G::ShapeInputs, G::Derived]);
        let cleared = correct_units(
            &outcome.record,
            &CorrectReferenceUnits {
                target_id: id(0xa1),
                interpretation: None,
            },
        )
        .unwrap();
        assert_eq!(cleared.record.units, None);
    }

    #[test]
    fn units_status_tells_when_the_source_is_not_in_project_units() {
        let mut r = record();
        // Declares metres and Y up: not the project's.
        assert_eq!(r.units_status(), UnitsStatus::NeedsCorrection);
        r.source.declared_meters_per_unit = Some(0.001);
        r.source.declared_up_axis = Some(UpAxis::Z);
        assert_eq!(r.units_status(), UnitsStatus::Matches);
        r.source.declared_meters_per_unit = None;
        assert_eq!(r.units_status(), UnitsStatus::Matches);
        r.source.declared_up_axis = None;
        assert_eq!(r.units_status(), UnitsStatus::Undeclared);
        r.source.declared_meters_per_unit = Some(0.01);
        assert_eq!(r.units_status(), UnitsStatus::NeedsCorrection);
        r.units = Some(UnitsInterpretation {
            meters_per_unit: 0.01,
            up_axis: UpAxis::Z,
        });
        assert_eq!(r.units_status(), UnitsStatus::Corrected);
        r.units = Some(UnitsInterpretation {
            meters_per_unit: 1.0,
            up_axis: UpAxis::Z,
        });
        assert_eq!(r.units_status(), UnitsStatus::CorrectedDiffersFromDeclared);
    }

    #[test]
    fn source_points_map_through_the_units_interpretation_then_the_placement() {
        let mut r = record();
        // Uncorrected: taken as stored.
        assert_eq!(
            r.source_point_to_world([2.0, 1.0, 3.0]).unwrap(),
            [2.0, 1.0, 3.0]
        );
        // Metres, Y up: scale by 1000, then y becomes z and z becomes -y.
        r.units = Some(UnitsInterpretation {
            meters_per_unit: 1.0,
            up_axis: UpAxis::Y,
        });
        assert_eq!(
            r.source_point_to_world([2.0, 1.0, 3.0]).unwrap(),
            [2000.0, -3000.0, 1000.0]
        );
        // Placement: a quarter turn about Z and a translation, applied after the correction.
        let h = core::f64::consts::FRAC_1_SQRT_2;
        r.placement = Placement {
            translation_mm: [10.0, 20.0, 30.0],
            orientation_xyzw: [0.0, 0.0, h, h],
        };
        let world = r.source_point_to_world([2.0, 1.0, 3.0]).unwrap();
        // (2000, -3000, 1000) rotated 90 degrees about Z is (3000, 2000, 1000).
        for (got, want) in world.iter().zip([3010.0, 2020.0, 1030.0]) {
            assert!((got - want).abs() < 1e-9, "{world:?}");
        }
        assert_eq!(
            r.source_point_to_world([f64::NAN, 0.0, 0.0])
                .unwrap_err()
                .code,
            ErrorCode::NonFiniteNumber
        );
    }

    #[test]
    fn every_write_to_foreign_content_is_refused_and_nothing_else_is_possible() {
        let r = record();
        let prim = |path: &str, attribute: Option<&str>| WriteForeign {
            reference_id: id(0xa1),
            target: ForeignTarget::Prim {
                path: path.into(),
                attribute: attribute.map(str::to_owned),
            },
        };
        let code = |command: &WriteForeign| {
            write_foreign(&r, command)
                .map(|n| match n {})
                .unwrap_err()
                .code
        };
        assert_eq!(
            code(&WriteForeign {
                reference_id: id(0xa1),
                target: ForeignTarget::SourceLayer,
            }),
            ErrorCode::ForeignReadOnly
        );
        for path in [
            "/Site",
            "/Site/Block",
            "/Site/Block/Body",
            "/Site/New/Child",
        ] {
            assert_eq!(
                code(&prim(path, None)),
                ErrorCode::ForeignReadOnly,
                "{path}"
            );
            assert_eq!(
                code(&prim(path, Some("visibility"))),
                ErrorCode::ForeignReadOnly,
                "{path}"
            );
        }
        // Not in this source's namespace.
        for path in ["/Other", "/SiteX", "/Sit"] {
            assert_eq!(code(&prim(path, None)), ErrorCode::NotFound, "{path}");
        }
        // Argument errors come first; an unknown wrapper is NOT_FOUND.
        assert_eq!(code(&prim("Site", None)), ErrorCode::InvalidArgument);
        assert_eq!(
            code(&prim("/Site", Some("a b"))),
            ErrorCode::InvalidArgument
        );
        let unknown = WriteForeign {
            reference_id: id(0xa9),
            target: ForeignTarget::SourceLayer,
        };
        assert_eq!(code(&unknown), ErrorCode::NotFound);
    }
}
