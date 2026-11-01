// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! F04 section 7 command arguments and their model-level admission.
//!
//! P01 implements `CreatePlate` only. This module holds what is true of the arguments
//! regardless of geometry: identity, label, finiteness, caps and the rigid placement. Contour
//! and thickness *geometry* (floor, winding, supported shapes) is evaluated by the engine with
//! the kernel. No JSON, envelope or canonical-byte form lives here yet: F04 sections 3 and 6
//! (`canonical`) remain planned, and `CreatePlate::content_fingerprint` is a
//! deterministic digest of the typed arguments, not RFC 8785 canonical bytes.

pub use crate::construction::CreateConstructionObject;
use crate::digest::{ContentDigest, Sha256};
use crate::error::ErrorCode;
pub use crate::grid::CreateGrid;
pub use crate::grid_update::UpdateGrid;
use crate::id::ElementId;
use crate::limits::{
    COORDINATE_CAP_MM, MAX_LABEL_BYTES, MIN_LABEL_BYTES, ORIENTATION_NORM_TOLERANCE,
};
use beamcraft_geom::canonical_f64_bytes;
use core::fmt;

/// The only contract version this implementation accepts.
pub const CONTRACT_VERSION: &str = "0.1.0";

/// A typed rejection: a stable F04 code, a fixed field token and a fixed safe message. It never
/// carries caller text, paths or credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandError {
    /// F10 rule for domain admission failures, absent for other errors.
    pub rule_id: Option<&'static str>,
    /// Stable F04 code.
    pub code: ErrorCode,
    /// Fixed schema token of the offending field, for example `contour_points_mm`.
    pub field: &'static str,
    /// Fixed ASCII explanation.
    pub message: &'static str,
}

impl CommandError {
    /// A rejection with the given code, field and message.
    pub const fn new(code: ErrorCode, field: &'static str, message: &'static str) -> Self {
        Self {
            code,
            field,
            message,
            rule_id: None,
        }
    }
    /// Retains the exact F10 rule through engine and CLI admission.
    pub const fn with_rule(mut self, rule_id: &'static str) -> Self {
        self.rule_id = Some(rule_id);
        self
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {} ({})", self.code, self.message, self.field)
    }
}

impl core::error::Error for CommandError {}

/// A rigid pose (F04 section 3): `world = R(q) * local + translation_mm`, quaternion in
/// `(x, y, z, w)` order. The components are authored as submitted; the accepted quaternion is
/// normalized only where it is used.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// Translation in millimetres, each component at most 1,000,000,000 in magnitude.
    pub translation_mm: [f64; 3],
    /// Quaternion components `(x, y, z, w)`, each in `[-1, 1]`, squared norm within 1e-6 of one.
    pub orientation_xyzw: [f64; 4],
}

impl Placement {
    /// No translation and the identity rotation.
    pub const IDENTITY: Self = Self {
        translation_mm: [0.0; 3],
        orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
    };

    /// Checks finiteness, the translation cap and the quaternion rules, in that order.
    ///
    /// Non-finite is `NON_FINITE_NUMBER`; a translation above the cap is `LIMIT_EXCEEDED`; a
    /// quaternion component outside `[-1, 1]` or a squared norm farther than 1e-6 from one is
    /// `INVALID_ARGUMENT`.
    pub fn validate(&self) -> Result<(), CommandError> {
        let finite = |values: &[f64], field: &'static str| {
            if values.iter().all(|v| v.is_finite()) {
                Ok(())
            } else {
                Err(
                    CommandError::new(ErrorCode::NonFiniteNumber, field, "value is not finite")
                        .with_rule("DV-ATTR-003"),
                )
            }
        };
        finite(&self.translation_mm, "placement.translation_mm")?;
        finite(&self.orientation_xyzw, "placement.orientation_xyzw")?;
        if self
            .translation_mm
            .iter()
            .any(|v| v.abs() > COORDINATE_CAP_MM)
        {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "placement.translation_mm",
                "translation component exceeds the coordinate cap",
            )
            .with_rule("DV-LIM-001"));
        }
        Self::validate_orientation(self.orientation_xyzw)
    }

    /// Checks a quaternion alone: non-finite is `NON_FINITE_NUMBER`; a component outside
    /// `[-1, 1]` or a squared norm farther than 1e-6 from one is `INVALID_ARGUMENT`.
    pub fn validate_orientation(xyzw: [f64; 4]) -> Result<(), CommandError> {
        if xyzw.iter().any(|v| !v.is_finite()) {
            return Err(CommandError::new(
                ErrorCode::NonFiniteNumber,
                "placement.orientation_xyzw",
                "value is not finite",
            )
            .with_rule("DV-ATTR-003"));
        }
        if xyzw.iter().any(|v| v.abs() > 1.0) {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "placement.orientation_xyzw",
                "quaternion component is outside [-1, 1]",
            )
            .with_rule("DV-LIM-003"));
        }
        let [x, y, z, w] = xyzw;
        let squared = ((x * x + y * y) + z * z) + w * w;
        if (squared - 1.0).abs() > ORIENTATION_NORM_TOLERANCE {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "placement.orientation_xyzw",
                "quaternion squared norm is not within 1e-6 of one",
            )
            .with_rule("DV-LIM-003"));
        }
        Ok(())
    }

    /// Whether the accepted rotation is the identity: `x = y = z = 0` (so `w` is `+-1` up to the
    /// accepted norm tolerance). `q` and `-q` are the same rotation.
    pub fn is_identity_rotation(&self) -> bool {
        let [x, y, z, _] = self.orientation_xyzw;
        x == 0.0 && y == 0.0 && z == 0.0
    }
}

/// Whether `name` is a valid element label (F04 section 3, `DV-BASE-003`).
pub fn label_is_valid(name: &str) -> bool {
    (MIN_LABEL_BYTES..=MAX_LABEL_BYTES).contains(&name.len())
        && !name.chars().any(|c| c <= '\u{1f}' || c == '\u{7f}')
}

/// F04 section 7 `DeleteElement` (P08): removes one owned element. It is refused while another
/// element still links to it (`DEPENDENCY_CONFLICT`); there is no cascade.
#[derive(Debug, Clone, PartialEq)]
pub struct DeleteElement {
    /// The element to delete.
    pub target_id: ElementId,
}

impl DeleteElement {
    /// SHA-256 of a versioned encoding of the request. Whether the element exists and whether
    /// something links to it are the engine's and the layer engine's decisions.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        let mut bytes = Vec::with_capacity(64);
        bytes.extend_from_slice(b"BeamCraft/P08/DeleteElement/v1\0");
        bytes.extend_from_slice(self.target_id.as_bytes());
        Sha256::digest(&bytes).map_err(|_| {
            CommandError::new(ErrorCode::LimitExceeded, "content", "content is too large")
                .with_rule("DV-LIM-001")
        })
    }
}

/// A01 `CreateAssembly`: a new `assembly` element (domain vocabulary section 2.3), a grouping
/// target of `part-of` with no shape, material or derived body of its own. It carries the common
/// base only; its placement is the identity in this slice.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateAssembly {
    /// Identity of the new assembly.
    pub new_id: ElementId,
    /// Explicit user label.
    pub name: String,
}

impl CreateAssembly {
    /// Admits the request: the label follows the F04 rule (`DV-BASE-003`). Whether the identity
    /// is free (`DUPLICATE_ID`) is the engine's decision.
    pub fn validate_arguments(&self) -> Result<(), CommandError> {
        if !label_is_valid(&self.name) {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "name",
                "label must be 1 to 256 bytes without control characters",
            )
            .with_rule("DV-BASE-003"));
        }
        Ok(())
    }

    /// SHA-256 of a versioned, length-prefixed encoding of the validated request.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        self.validate_arguments()?;
        let mut bytes = Vec::with_capacity(64 + self.name.len());
        bytes.extend_from_slice(b"BeamCraft/A01/CreateAssembly/v1\0");
        bytes.extend_from_slice(self.new_id.as_bytes());
        bytes.extend_from_slice(&(self.name.len() as u64).to_le_bytes());
        bytes.extend_from_slice(self.name.as_bytes());
        Sha256::digest(&bytes).map_err(|_| {
            CommandError::new(ErrorCode::LimitExceeded, "content", "content is too large")
                .with_rule("DV-LIM-001")
        })
    }
}

/// A01 `AttachToAssembly`: authors the one `part-of` record of `part_id` (domain vocabulary
/// section 5.1), so the part becomes a member of `assembly_id`. The record lives with its source,
/// the part; the assembly is only read.
#[derive(Debug, Clone, PartialEq)]
pub struct AttachToAssembly {
    /// The part: a plate, beam, column or (for nesting) another assembly.
    pub part_id: ElementId,
    /// The target assembly.
    pub assembly_id: ElementId,
}

impl AttachToAssembly {
    /// Admits the request: a self-link is `INVALID_ARGUMENT` (`DV-REL-006`). Kinds, cardinality
    /// and cycles (`DV-REL-003` to `DV-REL-005`) need the model and are the engine's decisions.
    pub fn validate_arguments(&self) -> Result<(), CommandError> {
        if self.part_id == self.assembly_id {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "assembly_id",
                "an element cannot be a part of itself",
            )
            .with_rule("DV-REL-006"));
        }
        Ok(())
    }

    /// SHA-256 of a versioned encoding of the validated request.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        self.validate_arguments()?;
        let mut bytes = Vec::with_capacity(80);
        bytes.extend_from_slice(b"BeamCraft/A01/AttachToAssembly/v1\0");
        bytes.extend_from_slice(self.part_id.as_bytes());
        bytes.extend_from_slice(self.assembly_id.as_bytes());
        Sha256::digest(&bytes).map_err(|_| {
            CommandError::new(ErrorCode::LimitExceeded, "content", "content is too large")
                .with_rule("DV-LIM-001")
        })
    }
}

/// A16.1 `SetAssemblyMainPart`: names `part_id` as the assembly's one main part (A16.1). The
/// record is stored on the assembly as its `bim:mainPart` relationship; the part is only read.
#[derive(Debug, Clone, PartialEq)]
pub struct SetAssemblyMainPart {
    /// The assembly whose main part is named.
    pub assembly_id: ElementId,
    /// The part: a direct member of the assembly (plate, beam, column or nested assembly).
    pub part_id: ElementId,
}

impl SetAssemblyMainPart {
    /// SHA-256 of a versioned encoding of the request. That the part exists and is a direct
    /// member of the assembly, and that the main part is unique, are the engine's decisions.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        let mut bytes = Vec::with_capacity(80);
        bytes.extend_from_slice(b"BeamCraft/A16/SetAssemblyMainPart/v1\0");
        bytes.extend_from_slice(self.assembly_id.as_bytes());
        bytes.extend_from_slice(self.part_id.as_bytes());
        Sha256::digest(&bytes).map_err(|_| {
            CommandError::new(ErrorCode::LimitExceeded, "content", "content is too large")
                .with_rule("DV-LIM-001")
        })
    }
}

/// A16 `ExplodeAssembly`: dissolves one assembly level. The assembly element is deleted and its
/// direct parts lose their `part-of` records; geometry, placement and attributes of the parts do
/// not change, and a nested sub-assembly stays an assembly with its own parts (decision of
/// 2026-10-21: explode means dissolve).
#[derive(Debug, Clone, PartialEq)]
pub struct ExplodeAssembly {
    /// The assembly to dissolve.
    pub assembly_id: ElementId,
}

impl ExplodeAssembly {
    /// SHA-256 of a versioned encoding of the request. That the target exists and is an assembly
    /// is the engine's decision.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        let mut bytes = Vec::with_capacity(64);
        bytes.extend_from_slice(b"BeamCraft/A16/ExplodeAssembly/v1\0");
        bytes.extend_from_slice(self.assembly_id.as_bytes());
        Sha256::digest(&bytes).map_err(|_| {
            CommandError::new(ErrorCode::LimitExceeded, "content", "content is too large")
                .with_rule("DV-LIM-001")
        })
    }
}

/// A01 `DetachFromAssembly`: removes the `part-of` record of `part_id`. The former assembly is
/// not written and stays, even when it has no parts left.
#[derive(Debug, Clone, PartialEq)]
pub struct DetachFromAssembly {
    /// The part whose membership ends.
    pub part_id: ElementId,
}

impl DetachFromAssembly {
    /// SHA-256 of a versioned encoding of the request. Whether the part exists and is a member
    /// of an assembly are the engine's decisions.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        let mut bytes = Vec::with_capacity(64);
        bytes.extend_from_slice(b"BeamCraft/A01/DetachFromAssembly/v1\0");
        bytes.extend_from_slice(self.part_id.as_bytes());
        Sha256::digest(&bytes).map_err(|_| {
            CommandError::new(ErrorCode::LimitExceeded, "content", "content is too large")
                .with_rule("DV-LIM-001")
        })
    }
}

/// D2 `CreateLevel`: a new `level` element (a storey): a name and an elevation in millimetres.
/// A level has no geometry and no placement of its own; plates and members are assigned to it
/// through `UpdateElement` (`ElementPatch::level`), and the elevation moves no element.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateLevel {
    /// Identity of the new level.
    pub new_id: ElementId,
    /// Explicit user label, 1 to 256 UTF-8 bytes without control characters.
    pub name: String,
    /// Elevation in millimetres: finite and at most 1,000,000,000 in magnitude (the coordinate
    /// cap). Negative elevations are allowed (a basement).
    pub elevation_mm: f64,
}

impl CreateLevel {
    /// Admits the request: the label follows the F04 rule (`INVALID_ARGUMENT`), the elevation is
    /// finite (`NON_FINITE_NUMBER`) and within the coordinate cap (`LIMIT_EXCEEDED`). Whether the
    /// identity is free (`DUPLICATE_ID`) is the engine's decision.
    pub fn validate_arguments(&self) -> Result<(), CommandError> {
        if !label_is_valid(&self.name) {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "name",
                "label must be 1 to 256 bytes without control characters",
            )
            .with_rule("DV-BASE-003"));
        }
        if !self.elevation_mm.is_finite() {
            return Err(CommandError::new(
                ErrorCode::NonFiniteNumber,
                "elevation_mm",
                "value is not finite",
            )
            .with_rule("DV-ATTR-003"));
        }
        if self.elevation_mm.abs() > COORDINATE_CAP_MM {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "elevation_mm",
                "elevation exceeds the coordinate cap",
            )
            .with_rule("DV-LIM-001"));
        }
        Ok(())
    }

    /// SHA-256 of a versioned, length-prefixed encoding of the validated request.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        self.validate_arguments()?;
        let mut bytes = Vec::with_capacity(72 + self.name.len());
        bytes.extend_from_slice(b"BeamCraft/D2/CreateLevel/v1\0");
        bytes.extend_from_slice(self.new_id.as_bytes());
        bytes.extend_from_slice(&(self.name.len() as u64).to_le_bytes());
        bytes.extend_from_slice(self.name.as_bytes());
        bytes.extend_from_slice(&canonical_f64_bytes(self.elevation_mm).map_err(|_| {
            CommandError::new(
                ErrorCode::NonFiniteNumber,
                "elevation_mm",
                "value cannot be encoded",
            )
            .with_rule("DV-ATTR-003")
        })?);
        Sha256::digest(&bytes).map_err(|_| {
            CommandError::new(ErrorCode::LimitExceeded, "content", "content is too large")
                .with_rule("DV-LIM-001")
        })
    }
}

/// A08 `ApproveMark`: a person approves the mark of one element, freezing its value. Whether the
/// element exists, carries a mark and is not already approved are the engine's decisions.
#[derive(Debug, Clone, PartialEq)]
pub struct ApproveMark {
    /// The element whose mark is approved.
    pub target_id: ElementId,
}

impl ApproveMark {
    /// SHA-256 of a versioned encoding of the request.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        let mut bytes = Vec::with_capacity(64);
        bytes.extend_from_slice(b"BeamCraft/A08/ApproveMark/v1\0");
        bytes.extend_from_slice(self.target_id.as_bytes());
        Sha256::digest(&bytes).map_err(|_| {
            CommandError::new(ErrorCode::LimitExceeded, "content", "content is too large")
                .with_rule("DV-LIM-001")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u8) -> ElementId {
        ElementId::parse(&format!("019a7bf0-0000-7000-8000-0000000000{n:02x}")).unwrap()
    }

    #[test]
    fn the_delete_fingerprint_names_the_target() {
        let a = DeleteElement { target_id: id(5) };
        let b = DeleteElement { target_id: id(6) };
        assert_eq!(
            a.content_fingerprint().unwrap(),
            a.clone().content_fingerprint().unwrap()
        );
        assert_ne!(
            a.content_fingerprint().unwrap(),
            b.content_fingerprint().unwrap()
        );
    }

    #[test]
    fn assembly_commands_admit_labels_refuse_self_links_and_fingerprint_their_content() {
        let a = ElementId::parse("019a7bf0-0000-7000-8000-000000200001").unwrap();
        let b = ElementId::parse("019a7bf0-0000-7000-8000-000000200002").unwrap();
        let create = CreateAssembly {
            new_id: a,
            name: "Frame A".into(),
        };
        assert!(create.validate_arguments().is_ok());
        let renamed = CreateAssembly {
            name: "Frame B".into(),
            ..create.clone()
        };
        assert_ne!(
            create.content_fingerprint().unwrap(),
            renamed.content_fingerprint().unwrap()
        );
        for bad in ["", "tab\there"] {
            let c = CreateAssembly {
                name: bad.into(),
                ..create.clone()
            };
            assert_eq!(
                c.validate_arguments().unwrap_err().code,
                ErrorCode::InvalidArgument
            );
        }
        let attach = AttachToAssembly {
            part_id: a,
            assembly_id: b,
        };
        assert!(attach.validate_arguments().is_ok());
        let swapped = AttachToAssembly {
            part_id: b,
            assembly_id: a,
        };
        assert_ne!(
            attach.content_fingerprint().unwrap(),
            swapped.content_fingerprint().unwrap()
        );
        let own = AttachToAssembly {
            part_id: a,
            assembly_id: a,
        };
        assert_eq!(
            own.content_fingerprint().unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        let detach = DetachFromAssembly { part_id: a };
        assert_ne!(
            detach.content_fingerprint().unwrap(),
            DetachFromAssembly { part_id: b }
                .content_fingerprint()
                .unwrap()
        );
        let main = SetAssemblyMainPart {
            assembly_id: b,
            part_id: a,
        };
        assert_ne!(
            main.content_fingerprint().unwrap(),
            SetAssemblyMainPart {
                assembly_id: b,
                part_id: b,
            }
            .content_fingerprint()
            .unwrap()
        );
        let explode = ExplodeAssembly { assembly_id: a };
        assert_ne!(
            explode.content_fingerprint().unwrap(),
            ExplodeAssembly { assembly_id: b }
                .content_fingerprint()
                .unwrap()
        );
    }

    #[test]
    fn create_level_admits_a_label_and_a_bounded_finite_elevation() {
        let level = CreateLevel {
            new_id: id(1),
            name: "Level 1".into(),
            elevation_mm: 3000.0,
        };
        assert!(level.validate_arguments().is_ok());
        let code = |l: CreateLevel| l.validate_arguments().unwrap_err().code;
        for elevation in [-1_000_000_000.0, 0.0, 1_000_000_000.0] {
            let l = CreateLevel {
                elevation_mm: elevation,
                ..level.clone()
            };
            assert!(l.validate_arguments().is_ok(), "{elevation}");
        }
        for elevation in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let l = CreateLevel {
                elevation_mm: elevation,
                ..level.clone()
            };
            assert_eq!(code(l), ErrorCode::NonFiniteNumber);
        }
        for elevation in [1_000_000_000.5, -1e300] {
            let l = CreateLevel {
                elevation_mm: elevation,
                ..level.clone()
            };
            assert_eq!(code(l), ErrorCode::LimitExceeded);
        }
        for name in ["", "tab\there"] {
            let l = CreateLevel {
                name: name.into(),
                ..level.clone()
            };
            assert_eq!(code(l), ErrorCode::InvalidArgument);
        }
        let base = level.content_fingerprint().unwrap();
        for changed in [
            CreateLevel {
                new_id: id(2),
                ..level.clone()
            },
            CreateLevel {
                name: "Level 2".into(),
                ..level.clone()
            },
            CreateLevel {
                elevation_mm: 3000.5,
                ..level.clone()
            },
        ] {
            assert_ne!(changed.content_fingerprint().unwrap(), base);
        }
        assert_eq!(level.clone().content_fingerprint().unwrap(), base);
    }
}
