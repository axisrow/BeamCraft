// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! R01: the wrapper of a foreign, read-only USD source (F04 section 1, F10 `reference`).
//!
//! A reference has two parts that never mix. The **foreign content** is a USD layer that this
//! project did not author: it is stored as immutable bytes under its SHA-256, composed below the
//! wrapper and never written. The **wrapper** is an owned element: its label, its rigid
//! placement and, when the owner asks for it, an explicit units and up-axis interpretation of
//! the source. Everything here is pure data and admission; reading the file, composing the stage
//! and writing the layers belong to the layer engine, the engine and the host.
//!
//! The units and up-axis correction is an operation on the wrapper ([`UnitsInterpretation`]):
//! the source layer keeps declaring whatever it declares and is never edited to match the
//! project. A source that declares nothing is recorded as declaring nothing, not guessed.
//!
//! A network asset path is refused by [`check_source_path`] on the text alone, before a
//! resolver, a file system or a socket is consulted.

use crate::command::{CommandError, Placement, label_is_valid};
use crate::digest::{ContentDigest, Sha256};
use crate::error::ErrorCode;
use crate::id::ElementId;
use beamcraft_geom::canonical_f64_bytes;

/// Largest foreign layer, in bytes: 32 MiB, matching the host blob cap and the U01 model-profile
/// per-layer budget. The layer engine enforces its stage budgets independently.
pub const MAX_SOURCE_BYTES: u64 = 32 * 1024 * 1024;
/// Directory prefix of the virtual name of every stored foreign source layer
/// (`external/<sha256>.usda`); the layer engine admits only layers under it as displayed sources.
pub const EXTERNAL_LAYER_PREFIX: &str = "external/";
/// Smallest `metersPerUnit` a source or an interpretation may state (nanometres).
pub const MIN_METERS_PER_UNIT: f64 = 1e-9;
/// Largest `metersPerUnit` a source or an interpretation may state (1000 km).
pub const MAX_METERS_PER_UNIT: f64 = 1e6;
/// The project unit: one stage unit is this many metres (millimetres, F04 section 3).
pub const PROJECT_METERS_PER_UNIT: f64 = 0.001;
/// Longest source path text accepted by [`check_source_path`], in bytes.
pub const MAX_SOURCE_PATH_BYTES: usize = 4096;
/// Longest root prim name of a source, in bytes.
pub const MAX_DEFAULT_PRIM_BYTES: usize = 128;
/// Most path segments of a foreign prim path.
pub const MAX_FOREIGN_PATH_SEGMENTS: usize = 16;
/// Longest foreign prim path, in bytes.
pub const MAX_FOREIGN_PATH_BYTES: usize = 1024;
/// Largest source version number.
pub const MAX_SOURCE_VERSION: u32 = 1_000_000;

fn error(code: ErrorCode, field: &'static str, message: &'static str) -> CommandError {
    CommandError::new(code, field, message)
}

/// The axis a source calls up. USD allows exactly two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum UpAxis {
    /// Y is up: the wrapper rotates the source about X by +90 degrees to the project's Z up.
    Y,
    /// Z is up: the source already agrees with the project.
    Z,
}

impl UpAxis {
    /// The USD token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Y => "Y",
            Self::Z => "Z",
        }
    }

    /// Parses `Y` or `Z`; any other text (including `X`, which USD does not allow) is `None`.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "Y" => Some(Self::Y),
            "Z" => Some(Self::Z),
            _ => None,
        }
    }
}

/// How the wrapper reads the source's own units and up axis: the explicit correction of R01.
///
/// The interpretation is a statement by the owner of the wrapper ("read this source as metres,
/// Y up"). It is stored with the wrapper and the digest of the source it was made for; it does
/// not change the source and is not inferred from it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UnitsInterpretation {
    /// Metres per source unit, finite and within [`MIN_METERS_PER_UNIT`], [`MAX_METERS_PER_UNIT`].
    pub meters_per_unit: f64,
    /// The source's up axis.
    pub up_axis: UpAxis,
}

impl UnitsInterpretation {
    /// Admits the values: finite (`NON_FINITE_NUMBER`), positive (`INVALID_ARGUMENT`) and within
    /// the caps (`LIMIT_EXCEEDED`).
    pub fn validate(&self) -> Result<(), CommandError> {
        check_meters_per_unit(self.meters_per_unit, "units.meters_per_unit")
    }

    /// The uniform scale that turns source units into project millimetres:
    /// `meters_per_unit * 1000`. The same expression is evaluated when a stored interpretation is
    /// validated, so the stored scale is exactly reproducible.
    pub fn scale(&self) -> f64 {
        self.meters_per_unit * 1000.0
    }

    /// The rotation about X, in degrees (the USD `rotateX` unit), that makes the source's up axis
    /// the project's Z axis: +90 for Y up, none for Z up.
    pub fn rotate_x_degrees(&self) -> Option<f64> {
        match self.up_axis {
            UpAxis::Y => Some(90.0),
            UpAxis::Z => None,
        }
    }

    /// Whether the interpretation changes nothing: millimetres and Z up.
    pub fn is_identity(&self) -> bool {
        self.scale() == 1.0 && self.up_axis == UpAxis::Z
    }
}

fn check_meters_per_unit(value: f64, field: &'static str) -> Result<(), CommandError> {
    if !value.is_finite() {
        return Err(error(
            ErrorCode::NonFiniteNumber,
            field,
            "value is not finite",
        ));
    }
    if value <= 0.0 {
        return Err(error(
            ErrorCode::InvalidArgument,
            field,
            "metres per unit must be greater than zero",
        ));
    }
    if !(MIN_METERS_PER_UNIT..=MAX_METERS_PER_UNIT).contains(&value) {
        return Err(error(
            ErrorCode::LimitExceeded,
            field,
            "metres per unit is outside 1e-9 to 1e6",
        ));
    }
    Ok(())
}

/// The storage format of a foreign layer, decided from its leading bytes and never from a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SourceFormat {
    /// USDA text.
    Usda,
    /// USDC crate (the restricted binary profile of the layer engine).
    Usdc,
}

impl SourceFormat {
    /// The stable token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Usda => "usda",
            Self::Usdc => "usdc",
        }
    }

    /// Parses the stable token.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "usda" => Some(Self::Usda),
            "usdc" => Some(Self::Usdc),
            _ => None,
        }
    }
}

/// What the wrapper pins about its source: the snapshot of the foreign bytes. Two attachments of
/// the same bytes have the same digest and share one stored layer.
#[derive(Debug, Clone, PartialEq)]
pub struct SourcePin {
    /// Full SHA-256 of the exact source bytes.
    pub digest: ContentDigest,
    /// Length of the source in bytes, 1 to [`MAX_SOURCE_BYTES`].
    pub bytes: u64,
    /// Storage format.
    pub format: SourceFormat,
    /// Name of the source's `defaultPrim` (a root prim, without the slash). A source without a
    /// default prim cannot be attached: nothing is guessed.
    pub default_prim: String,
    /// Version of the source this wrapper shows, starting at 1. Only an explicit source update
    /// (R03) changes it; it is a counter of snapshots, not a clock.
    pub version: u32,
    /// Display label of the source, normally its file name. It is not a path.
    pub label: String,
    /// `metersPerUnit` the source declares, if it declares one.
    pub declared_meters_per_unit: Option<f64>,
    /// `upAxis` the source declares, if it declares one.
    pub declared_up_axis: Option<UpAxis>,
}

impl SourcePin {
    /// The virtual name of the stored source layer inside a revision: `external/<sha256>.usda`
    /// (or `.usdc`). It is derived from the digest, so it is stable and cannot collide with an
    /// owned layer.
    pub fn layer_name(&self) -> String {
        format!(
            "{EXTERNAL_LAYER_PREFIX}{}.{}",
            self.digest,
            self.format.as_str()
        )
    }

    /// Admits the pin: size, default prim name, version, label and declared units.
    pub fn validate(&self) -> Result<(), CommandError> {
        if self.bytes == 0 || self.bytes > MAX_SOURCE_BYTES {
            return Err(error(
                ErrorCode::LimitExceeded,
                "source.bytes",
                "source layer is empty or larger than 32 MiB",
            ));
        }
        if !is_prim_name(&self.default_prim) {
            return Err(error(
                ErrorCode::UnsupportedProperty,
                "source.default_prim",
                "default prim is not a plain ASCII prim name",
            ));
        }
        if self.version == 0 || self.version > MAX_SOURCE_VERSION {
            return Err(error(
                ErrorCode::InvalidArgument,
                "source.version",
                "version is outside 1 to 1000000",
            ));
        }
        if !label_is_valid(&self.label) {
            return Err(error(
                ErrorCode::InvalidArgument,
                "source.label",
                "label must be 1 to 256 bytes without control characters",
            ));
        }
        if let Some(declared) = self.declared_meters_per_unit {
            check_meters_per_unit(declared, "source.declared_meters_per_unit")?;
        }
        Ok(())
    }
}

/// A plain ASCII USD prim name: a letter or underscore, then letters, digits and underscores.
pub fn is_prim_name(text: &str) -> bool {
    let mut bytes = text.bytes();
    text.len() <= MAX_DEFAULT_PRIM_BYTES
        && bytes
            .next()
            .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Refuses an asset path that is not a plain local file path, on the text alone.
///
/// This runs before any file system, resolver or network access. A path is refused with
/// `PERMISSION_DENIED` when it
///
/// * starts with a URL scheme of two or more characters (`http:`, `https:`, `ftp:`, `file:`,
///   `s3:`, `omniverse:`, `anon:` and so on; a single letter followed by a colon is a Windows
///   drive and is accepted), or
/// * starts with two slashes or two backslashes (a network share or a device namespace such as
///   `\\server\share` and `\\?\...`).
///
/// A relative file name whose first segment contains a colon must be written with a leading
/// `./`. An empty, over-long or control-character path is `INVALID_ARGUMENT` (`LIMIT_EXCEEDED`
/// for length). Passing this check does not prove that the file exists or may be read.
pub fn check_source_path(text: &str) -> Result<(), CommandError> {
    if text.is_empty() || text.chars().any(|c| c <= '\u{1f}' || c == '\u{7f}') {
        return Err(error(
            ErrorCode::InvalidArgument,
            "source_path",
            "path is empty or has control characters",
        ));
    }
    if text.len() > MAX_SOURCE_PATH_BYTES {
        return Err(error(
            ErrorCode::LimitExceeded,
            "source_path",
            "path is longer than 4096 bytes",
        ));
    }
    let trimmed = text.trim_start();
    if trimmed.starts_with("//") || trimmed.starts_with("\\\\") {
        return Err(error(
            ErrorCode::PermissionDenied,
            "source_path",
            "network and device paths are never opened",
        ));
    }
    if has_scheme(trimmed) {
        return Err(error(
            ErrorCode::PermissionDenied,
            "source_path",
            "URL asset paths are never resolved",
        ));
    }
    Ok(())
}

fn has_scheme(text: &str) -> bool {
    let mut length = 0usize;
    for (index, c) in text.char_indices() {
        if c == ':' {
            // One character before the colon is a Windows drive letter, not a scheme.
            return index >= 2;
        }
        let allowed = if index == 0 {
            c.is_ascii_alphabetic()
        } else {
            c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')
        };
        if !allowed {
            return false;
        }
        length += 1;
        if length > 64 {
            return false;
        }
    }
    false
}

fn digest_error() -> CommandError {
    error(ErrorCode::LimitExceeded, "content", "content is too large")
}

fn push_f64(bytes: &mut Vec<u8>, value: f64, field: &'static str) -> Result<(), CommandError> {
    bytes.extend_from_slice(
        &canonical_f64_bytes(value)
            .map_err(|_| error(ErrorCode::NonFiniteNumber, field, "value cannot be encoded"))?,
    );
    Ok(())
}

fn push_text(bytes: &mut Vec<u8>, text: &str) {
    bytes.extend_from_slice(&(text.len() as u64).to_le_bytes());
    bytes.extend_from_slice(text.as_bytes());
}

fn push_placement(bytes: &mut Vec<u8>, placement: &Placement) -> Result<(), CommandError> {
    for v in placement
        .translation_mm
        .into_iter()
        .chain(placement.orientation_xyzw)
    {
        push_f64(bytes, v, "placement")?;
    }
    Ok(())
}

/// R01 `AttachReference`: a new wrapper around a foreign source, with an explicit identity and a
/// rigid placement. The wrapper starts with no units correction.
#[derive(Debug, Clone, PartialEq)]
pub struct AttachReference {
    /// Identity of the wrapper (an element of kind `reference`).
    pub new_id: ElementId,
    /// Label of the wrapper, 1 to 256 bytes without control characters. Not an identity.
    pub name: String,
    /// Rigid placement of the wrapper, stored apart from the foreign content.
    pub placement: Placement,
    /// The snapshot of the source being attached.
    pub source: SourcePin,
}

impl AttachReference {
    /// Admits the arguments: label, placement (finiteness, cap, quaternion) and the source pin.
    pub fn validate_arguments(&self) -> Result<(), CommandError> {
        if !label_is_valid(&self.name) {
            return Err(error(
                ErrorCode::InvalidArgument,
                "name",
                "label must be 1 to 256 bytes without control characters",
            ));
        }
        self.placement.validate()?;
        self.source.validate()
    }

    /// SHA-256 of a versioned, length-prefixed encoding of the validated request. The file name
    /// and path of the original are not part of it; the label and the source digest are.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        self.validate_arguments()?;
        let mut bytes = Vec::with_capacity(256);
        bytes.extend_from_slice(b"BeamCraft/R01/AttachReference/v1\0");
        bytes.extend_from_slice(self.new_id.as_bytes());
        push_text(&mut bytes, &self.name);
        push_placement(&mut bytes, &self.placement)?;
        bytes.extend_from_slice(self.source.digest.as_bytes());
        bytes.extend_from_slice(&self.source.bytes.to_le_bytes());
        push_text(&mut bytes, self.source.format.as_str());
        push_text(&mut bytes, &self.source.default_prim);
        bytes.extend_from_slice(&self.source.version.to_le_bytes());
        push_text(&mut bytes, &self.source.label);
        match self.source.declared_meters_per_unit {
            Some(v) => {
                bytes.push(1);
                push_f64(&mut bytes, v, "source.declared_meters_per_unit")?;
            }
            None => bytes.push(0),
        }
        match self.source.declared_up_axis {
            Some(axis) => {
                bytes.push(1);
                push_text(&mut bytes, axis.as_str());
            }
            None => bytes.push(0),
        }
        Sha256::digest(&bytes).map_err(|_| digest_error())
    }
}

/// R01 `MoveReference`: replaces the rigid placement of the wrapper. The foreign content, its
/// units interpretation and its stored bytes are not touched.
#[derive(Debug, Clone, PartialEq)]
pub struct MoveReference {
    /// The wrapper to move.
    pub target_id: ElementId,
    /// The new absolute placement.
    pub placement: Placement,
}

impl MoveReference {
    /// Admits the placement.
    pub fn validate_arguments(&self) -> Result<(), CommandError> {
        self.placement.validate()
    }

    /// SHA-256 of a versioned encoding of the validated request.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        self.validate_arguments()?;
        let mut bytes = Vec::with_capacity(96);
        bytes.extend_from_slice(b"BeamCraft/R01/MoveReference/v1\0");
        bytes.extend_from_slice(self.target_id.as_bytes());
        push_placement(&mut bytes, &self.placement)?;
        Sha256::digest(&bytes).map_err(|_| digest_error())
    }
}

/// R01 `CorrectReferenceUnits`: the explicit units and up-axis correction of a wrapper. `None`
/// clears it. It is an operation on the wrapper; the source layer is never edited.
#[derive(Debug, Clone, PartialEq)]
pub struct CorrectReferenceUnits {
    /// The wrapper to correct.
    pub target_id: ElementId,
    /// The interpretation to store, or `None` to remove an earlier one.
    pub interpretation: Option<UnitsInterpretation>,
}

impl CorrectReferenceUnits {
    /// Admits the interpretation.
    pub fn validate_arguments(&self) -> Result<(), CommandError> {
        match &self.interpretation {
            Some(interpretation) => interpretation.validate(),
            None => Ok(()),
        }
    }

    /// SHA-256 of a versioned encoding of the validated request.
    pub fn content_fingerprint(&self) -> Result<ContentDigest, CommandError> {
        self.validate_arguments()?;
        let mut bytes = Vec::with_capacity(64);
        bytes.extend_from_slice(b"BeamCraft/R01/CorrectReferenceUnits/v1\0");
        bytes.extend_from_slice(self.target_id.as_bytes());
        match &self.interpretation {
            Some(i) => {
                bytes.push(1);
                push_f64(&mut bytes, i.meters_per_unit, "units.meters_per_unit")?;
                push_text(&mut bytes, i.up_axis.as_str());
            }
            None => bytes.push(0),
        }
        Sha256::digest(&bytes).map_err(|_| digest_error())
    }
}

/// What a refused write was aimed at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForeignTarget {
    /// The stored source layer itself (its bytes, its metadata, its spec tree).
    SourceLayer,
    /// A prim of the source, given by its absolute path in the **source layer's** namespace
    /// (`/DefaultPrim/Child`), together with an optional attribute to set. Anything at or below
    /// the source's root prim is foreign, whether or not it exists.
    Prim {
        /// Absolute prim path in the source's namespace.
        path: String,
        /// Attribute the caller tried to set, if any.
        attribute: Option<String>,
    },
}

/// R01 `WriteForeign`: a request to write into foreign content (the source layer or one of its
/// prims, including an override authored in an owned layer). It exists so that the refusal is an
/// observable, testable API result; no admission rule can turn it into a write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteForeign {
    /// The wrapper whose source is addressed.
    pub reference_id: ElementId,
    /// What the caller tried to write.
    pub target: ForeignTarget,
}

impl WriteForeign {
    /// Checks the syntax of the addressed path and attribute (bounded, plain names). A request
    /// that passes is still refused by the engine.
    pub fn validate_arguments(&self) -> Result<(), CommandError> {
        if let ForeignTarget::Prim { path, attribute } = &self.target {
            check_foreign_prim_path(path)?;
            if let Some(name) = attribute
                && !is_attribute_name(name)
            {
                return Err(error(
                    ErrorCode::InvalidArgument,
                    "attribute",
                    "attribute is not a plain USD property name",
                ));
            }
        }
        Ok(())
    }
}

/// Checks an absolute prim path of a source namespace: `/A/B/C`, plain ASCII names, at most
/// [`MAX_FOREIGN_PATH_SEGMENTS`] segments and [`MAX_FOREIGN_PATH_BYTES`] bytes. No property
/// suffix, variant selection, relative segment or empty segment.
pub fn check_foreign_prim_path(path: &str) -> Result<(), CommandError> {
    if path.len() > MAX_FOREIGN_PATH_BYTES {
        return Err(error(
            ErrorCode::LimitExceeded,
            "path",
            "prim path is longer than 1024 bytes",
        ));
    }
    let Some(rest) = path.strip_prefix('/') else {
        return Err(error(
            ErrorCode::InvalidArgument,
            "path",
            "prim path must be absolute",
        ));
    };
    let segments: Vec<&str> = rest.split('/').collect();
    if segments.len() > MAX_FOREIGN_PATH_SEGMENTS {
        return Err(error(
            ErrorCode::LimitExceeded,
            "path",
            "prim path has more than 16 segments",
        ));
    }
    if !segments.iter().all(|s| is_prim_name(s)) {
        return Err(error(
            ErrorCode::InvalidArgument,
            "path",
            "prim path segments must be plain ASCII prim names",
        ));
    }
    Ok(())
}

fn is_attribute_name(text: &str) -> bool {
    !text.is_empty() && text.len() <= 256 && text.split(':').all(is_prim_name)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn attach() -> AttachReference {
        AttachReference {
            new_id: ElementId::parse("019a7bf0-0000-7000-8000-000000000a01").unwrap(),
            name: "Architect site".into(),
            placement: Placement::IDENTITY,
            source: pin(),
        }
    }

    #[test]
    fn network_and_device_paths_are_refused_on_the_text_alone() {
        for path in [
            "http://example.invalid/model.usd",
            "https://example.invalid/model.usd",
            "HTTPS://EXAMPLE.INVALID/MODEL.USD",
            "ftp://example.invalid/m.usd",
            "file:///C:/m.usd",
            "file://server/share/m.usd",
            "s3://bucket/m.usd",
            "omniverse://server/Projects/m.usd",
            "anon:0x1234:layer.usda",
            "data:text/plain,x",
            "\\\\server\\share\\m.usd",
            "//server/share/m.usd",
            "\\\\?\\C:\\m.usd",
            "  http://padded.invalid/m.usd",
        ] {
            assert_eq!(
                check_source_path(path).unwrap_err().code,
                ErrorCode::PermissionDenied,
                "{path}"
            );
        }
    }

    #[test]
    fn local_paths_pass_the_text_check() {
        for path in [
            "model.usda",
            "./a:b.usda",
            "dir/model.usda",
            "C:\\models\\site.usda",
            "c:/models/site.usda",
            "/home/me/site.usda",
            "..\\up\\site.usda",
            "relative path with spaces/site.usda",
        ] {
            assert!(check_source_path(path).is_ok(), "{path}");
        }
    }

    #[test]
    fn malformed_paths_are_invalid_not_network() {
        assert_eq!(
            check_source_path("").unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            check_source_path("a\u{0}b").unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            check_source_path(&"a".repeat(MAX_SOURCE_PATH_BYTES + 1))
                .unwrap_err()
                .code,
            ErrorCode::LimitExceeded
        );
        assert!(check_source_path(&"a".repeat(MAX_SOURCE_PATH_BYTES)).is_ok());
    }

    #[test]
    fn up_axis_accepts_only_y_and_z() {
        assert_eq!(UpAxis::parse("Y"), Some(UpAxis::Y));
        assert_eq!(UpAxis::parse("Z"), Some(UpAxis::Z));
        for bad in ["X", "y", "z", "", "Y ", "YZ"] {
            assert_eq!(UpAxis::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn units_interpretation_scale_and_rotation() {
        let mm = UnitsInterpretation {
            meters_per_unit: 0.001,
            up_axis: UpAxis::Z,
        };
        assert_eq!(mm.scale(), 1.0);
        assert!(mm.is_identity());
        let metres_y_up = UnitsInterpretation {
            meters_per_unit: 1.0,
            up_axis: UpAxis::Y,
        };
        assert_eq!(metres_y_up.scale(), 1000.0);
        assert_eq!(metres_y_up.rotate_x_degrees(), Some(90.0));
        assert!(!metres_y_up.is_identity());
        let centimetres = UnitsInterpretation {
            meters_per_unit: 0.01,
            up_axis: UpAxis::Z,
        };
        assert_eq!(centimetres.scale(), 10.0);
        assert_eq!(centimetres.rotate_x_degrees(), None);
    }

    #[test]
    fn units_interpretation_is_bounded_in_the_f04_order() {
        let at = |meters_per_unit: f64| UnitsInterpretation {
            meters_per_unit,
            up_axis: UpAxis::Z,
        };
        assert_eq!(
            at(f64::NAN).validate().unwrap_err().code,
            ErrorCode::NonFiniteNumber
        );
        assert_eq!(
            at(f64::INFINITY).validate().unwrap_err().code,
            ErrorCode::NonFiniteNumber
        );
        assert_eq!(
            at(0.0).validate().unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            at(-1.0).validate().unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            at(1e-10).validate().unwrap_err().code,
            ErrorCode::LimitExceeded
        );
        assert_eq!(
            at(1.1e6).validate().unwrap_err().code,
            ErrorCode::LimitExceeded
        );
        assert!(at(MIN_METERS_PER_UNIT).validate().is_ok());
        assert!(at(MAX_METERS_PER_UNIT).validate().is_ok());
    }

    #[test]
    fn layer_name_is_derived_from_the_digest_and_the_format() {
        let mut p = pin();
        let name = p.layer_name();
        assert_eq!(name, format!("external/{}.usda", p.digest));
        p.format = SourceFormat::Usdc;
        assert!(p.layer_name().ends_with(".usdc"));
        assert!(name.len() <= 128);
    }

    #[test]
    fn source_pin_rules() {
        assert!(pin().validate().is_ok());
        let mut p = pin();
        p.bytes = MAX_SOURCE_BYTES;
        assert!(p.validate().is_ok());
        let mut p = pin();
        p.bytes = 0;
        assert_eq!(p.validate().unwrap_err().code, ErrorCode::LimitExceeded);
        let mut p = pin();
        p.bytes = MAX_SOURCE_BYTES + 1;
        assert_eq!(p.validate().unwrap_err().code, ErrorCode::LimitExceeded);
        for bad in ["", "1abc", "a b", "a/b", "caf\u{e9}", &"a".repeat(129)] {
            let mut p = pin();
            p.default_prim = bad.into();
            assert_eq!(
                p.validate().unwrap_err().code,
                ErrorCode::UnsupportedProperty,
                "{bad:?}"
            );
        }
        let mut p = pin();
        p.version = 0;
        assert_eq!(p.validate().unwrap_err().code, ErrorCode::InvalidArgument);
        let mut p = pin();
        p.label = "bad\nlabel".into();
        assert_eq!(p.validate().unwrap_err().code, ErrorCode::InvalidArgument);
        let mut p = pin();
        p.declared_meters_per_unit = Some(0.0);
        assert_eq!(p.validate().unwrap_err().code, ErrorCode::InvalidArgument);
        let mut p = pin();
        p.declared_meters_per_unit = None;
        p.declared_up_axis = None;
        assert!(p.validate().is_ok());
    }

    #[test]
    fn attach_rules_and_fingerprint() {
        assert!(attach().validate_arguments().is_ok());
        let mut a = attach();
        a.name = String::new();
        assert_eq!(
            a.validate_arguments().unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        let mut a = attach();
        a.placement.translation_mm = [f64::NAN, 0.0, 0.0];
        assert_eq!(
            a.validate_arguments().unwrap_err().code,
            ErrorCode::NonFiniteNumber
        );
        let base = attach().content_fingerprint().unwrap();
        assert_eq!(base, attach().content_fingerprint().unwrap());
        let mut other = attach();
        other.placement.translation_mm = [1.0, 0.0, 0.0];
        assert_ne!(base, other.content_fingerprint().unwrap());
        let mut other = attach();
        other.source.digest = Sha256::digest(b"other").unwrap();
        assert_ne!(base, other.content_fingerprint().unwrap());
        let mut other = attach();
        other.source.declared_up_axis = None;
        assert_ne!(base, other.content_fingerprint().unwrap());
        // Signed zero is the same request.
        let mut zero = attach();
        zero.placement.translation_mm = [-0.0, 0.0, 0.0];
        assert_eq!(base, zero.content_fingerprint().unwrap());
    }

    #[test]
    fn move_and_correct_fingerprints_differ_by_content() {
        let id = ElementId::parse("019a7bf0-0000-7000-8000-000000000a01").unwrap();
        let mut moved = MoveReference {
            target_id: id,
            placement: Placement::IDENTITY,
        };
        let a = moved.content_fingerprint().unwrap();
        moved.placement.translation_mm = [0.0, 0.0, 5.0];
        assert_ne!(a, moved.content_fingerprint().unwrap());
        let clear = CorrectReferenceUnits {
            target_id: id,
            interpretation: None,
        };
        let set = CorrectReferenceUnits {
            target_id: id,
            interpretation: Some(UnitsInterpretation {
                meters_per_unit: 1.0,
                up_axis: UpAxis::Y,
            }),
        };
        assert_ne!(
            clear.content_fingerprint().unwrap(),
            set.content_fingerprint().unwrap()
        );
        let bad = CorrectReferenceUnits {
            target_id: id,
            interpretation: Some(UnitsInterpretation {
                meters_per_unit: -1.0,
                up_axis: UpAxis::Y,
            }),
        };
        assert!(bad.content_fingerprint().is_err());
    }

    #[test]
    fn foreign_paths_are_plain_bounded_names() {
        for good in ["/A", "/Site/Building_1/Wall_2", "/_x1"] {
            assert!(check_foreign_prim_path(good).is_ok(), "{good}");
        }
        for bad in [
            "", "A", "/", "//A", "/A/", "/A//B", "/A/../B", "/A.attr", "/A{v=x}", "/A B", "/1A",
        ] {
            assert_eq!(
                check_foreign_prim_path(bad).unwrap_err().code,
                ErrorCode::InvalidArgument,
                "{bad:?}"
            );
        }
        let deep = format!("/{}", vec!["a"; MAX_FOREIGN_PATH_SEGMENTS + 1].join("/"));
        assert_eq!(
            check_foreign_prim_path(&deep).unwrap_err().code,
            ErrorCode::LimitExceeded
        );
        let ok = format!("/{}", vec!["a"; MAX_FOREIGN_PATH_SEGMENTS].join("/"));
        assert!(check_foreign_prim_path(&ok).is_ok());
        assert_eq!(
            check_foreign_prim_path(&format!("/{}", "a".repeat(MAX_FOREIGN_PATH_BYTES)))
                .unwrap_err()
                .code,
            ErrorCode::LimitExceeded
        );
    }

    #[test]
    fn write_foreign_syntax_is_checked_before_it_is_refused() {
        let id = ElementId::parse("019a7bf0-0000-7000-8000-000000000a01").unwrap();
        let ok = WriteForeign {
            reference_id: id,
            target: ForeignTarget::Prim {
                path: "/Site/Wall".into(),
                attribute: Some("visibility".into()),
            },
        };
        assert!(ok.validate_arguments().is_ok());
        let bad_attribute = WriteForeign {
            reference_id: id,
            target: ForeignTarget::Prim {
                path: "/Site/Wall".into(),
                attribute: Some("a b".into()),
            },
        };
        assert_eq!(
            bad_attribute.validate_arguments().unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        let source = WriteForeign {
            reference_id: id,
            target: ForeignTarget::SourceLayer,
        };
        assert!(source.validate_arguments().is_ok());
    }
}
