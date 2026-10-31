// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The F12 project descriptor: contract requirements, the declared frame and the
//! coordination connection pins around the F10 0.2 registry shapes
//! (`docs/contracts/f12-project-coordination-v0.1.md`, sections 2-4).
//!
//! The section and zone registries hold the vocabulary 0.2 records of
//! [`crate::coordination`] — the same typed shapes and budgets the coordination line
//! serializes — and the project metadata is [`crate::coordination::ProjectMetadata`]. This
//! module adds only what a 0.2 record set does not carry: the exact contract, vocabulary and
//! schema identifiers that interpret the revision, the declared F04 frame, and the
//! connection pins. Codes and labels are labels and never authority (PC-ID-001); a
//! connection pin records the exact source revision, its manifest digest and the digest of
//! every pinned source layer, so a coordination store retains verified local copies of the
//! dependency closure (PC-REV-001, PC-KEEP-001). Nothing here reads files, clocks or
//! environment; the caller supplies the exact bytes and identities it already verified.

use crate::command::CommandError;
use crate::coordination::{
    MAX_SECTIONS, MAX_ZONES, ProjectMetadata, SectionRecord, VOCABULARY_VERSION,
};
use crate::error::ErrorCode;
use crate::id::{ConnectionId, ProjectId, RevisionId, SectionId, WorkspaceId, ZoneId};

/// Most coordination connections of one project: with three pinned source layers each, the
/// stage layer budget (16, F12 section 9) holds the own layers plus four whole-project
/// closures.
pub const MAX_CONNECTIONS: usize = 4;
/// Most pinned source layers recorded for one connection.
pub const MAX_PINNED_LAYERS: usize = 16;
/// Most bytes of a human label, name or organization.
pub const MAX_LABEL_BYTES: usize = 256;
/// The only declared coordinate frame of a conforming project (F12 PC-FRAME-001).
pub const FRAME_F04: &str = "mm-rad-right-z-up";
/// The contract this descriptor implements; recorded in every descriptor.
pub const CONTRACT_ID: &str = "f12-project-coordination-v0.1";

/// Exact contract, vocabulary and model-schema identifiers needed to interpret the revision
/// (F12 section 2). Identifiers are opaque bounded text; the schema entry retains the digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractRequirements {
    /// The project contract this revision follows.
    pub contract: String,
    /// The domain vocabulary line this revision follows.
    pub vocabulary: String,
    /// The model-schema identity, as the full schema digest.
    pub schema: String,
}

/// The source scope a connection participates in (F12 PC-COORD-003). It controls display and
/// participation, never permission, and cannot discard dependencies the selection needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeSelection {
    /// The whole source project.
    WholeProject,
    /// One section of the source project.
    Section(SectionId),
    /// One zone of the source project.
    Zone(ZoneId),
}

/// The declared rigid transform of a connection into its parent frame (F12 PC-FRAME-001):
/// a finite translation in millimetres and a unit quaternion `xyzw` rotation. `None` is an
/// explicit unknown alignment, never an identity placement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConnectionPlacement {
    /// Translation in millimetres.
    pub translation_mm: [f64; 3],
    /// Unit rotation quaternion, `xyzw` order (the stored layer orientation order).
    pub orientation_xyzw: [f64; 4],
}

/// The authored coordinate cap of a connection translation, matching geom's authored cap.
pub const PLACEMENT_CAP_MM: f64 = 1e9;
/// Largest deviation of the quaternion norm from one that is still accepted as rigid.
pub const UNIT_QUATERNION_TOLERANCE: f64 = 1e-6;

impl ConnectionPlacement {
    /// The exact identity placement.
    pub const IDENTITY: Self = Self {
        translation_mm: [0.0; 3],
        orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
    };

    /// Validates finiteness, the authored translation cap and the unit-quaternion norm
    /// (PC-FRAME-001). Scale, shear and non-unit rotations are outside the admitted scope.
    pub fn validate(&self) -> Result<(), CommandError> {
        for value in self.translation_mm {
            if !value.is_finite() {
                return Err(CommandError::new(
                    ErrorCode::NonFiniteNumber,
                    "translation",
                    "connection translation must be finite",
                )
                .with_rule("PC-FRAME-001"));
            }
            if value.abs() > PLACEMENT_CAP_MM {
                return Err(CommandError::new(
                    ErrorCode::InvalidArgument,
                    "translation",
                    "connection translation exceeds the authored coordinate cap",
                )
                .with_rule("PC-FRAME-001"));
            }
        }
        let norm_sq = self.orientation_xyzw.iter().map(|v| v * v).sum::<f64>();
        if !norm_sq.is_finite() {
            return Err(CommandError::new(
                ErrorCode::NonFiniteNumber,
                "orientation",
                "connection orientation must be finite",
            )
            .with_rule("PC-FRAME-001"));
        }
        if (norm_sq.sqrt() - 1.0).abs() > UNIT_QUATERNION_TOLERANCE {
            return Err(CommandError::new(
                ErrorCode::UnsupportedGeometry,
                "orientation",
                "connection orientation must be a unit quaternion (rigid transform)",
            )
            .with_rule("PC-FRAME-001"));
        }
        Ok(())
    }

    /// Maps a point of the source frame into the parent frame: rotate, then translate.
    /// Fails on a non-finite input or result instead of guessing.
    pub fn apply_point(&self, point_mm: [f64; 3]) -> Result<[f64; 3], CommandError> {
        for value in point_mm {
            if !value.is_finite() {
                return Err(CommandError::new(
                    ErrorCode::NonFiniteNumber,
                    "point",
                    "source point must be finite",
                )
                .with_rule("PC-FRAME-001"));
            }
        }
        let rotated = rotate_by_quaternion(self.orientation_xyzw, point_mm)?;
        let mut out = [0.0; 3];
        for (slot, (value, translation)) in
            out.iter_mut().zip(rotated.iter().zip(self.translation_mm))
        {
            let moved = value + translation;
            if !moved.is_finite() {
                return Err(CommandError::new(
                    ErrorCode::NonFiniteNumber,
                    "point",
                    "the transformed coordinate is not finite",
                )
                .with_rule("PC-FRAME-001"));
            }
            *slot = moved;
        }
        Ok(out)
    }

    /// Composes two placements of one chain: `outer` is expressed in the frame `self` maps
    /// into. The documented nesting order (PC-FRAME-001) applies the innermost source
    /// placement first and the outermost coordination placement last, so composing is
    /// `outer * self`: rotate by `self` then by `outer`, then add `self`'s translation and
    /// finally `outer`'s.
    pub fn compose(
        &self,
        outer: &ConnectionPlacement,
    ) -> Result<ConnectionPlacement, CommandError> {
        outer.validate()?;
        self.validate()?;
        let [ox, oy, oz, ow] = outer.orientation_xyzw;
        let [sx, sy, sz, sw] = self.orientation_xyzw;
        // Quaternion product outer * self, as xyzw.
        let w = ow * sw - (ox * sx + oy * sy + oz * sz);
        let xyz = [
            ow * sx + ox * sw + oy * sz - oz * sy,
            ow * sy - ox * sz + oy * sw + oz * sx,
            ow * sz + ox * sy - oy * sx + oz * sw,
        ];
        let rotated = rotate_by_quaternion(outer.orientation_xyzw, self.translation_mm)?;
        let placement = ConnectionPlacement {
            translation_mm: [
                outer.translation_mm[0] + rotated[0],
                outer.translation_mm[1] + rotated[1],
                outer.translation_mm[2] + rotated[2],
            ],
            orientation_xyzw: [xyz[0], xyz[1], xyz[2], w],
        };
        placement.validate()?;
        Ok(placement)
    }
}

/// Rotates one point by a unit quaternion `xyzw` (the caller validated the norm):
/// `out = p + 2w(v x p) + 2 v x (v x p)` with `q = (w, v)`.
fn rotate_by_quaternion(q: [f64; 4], p: [f64; 3]) -> Result<[f64; 3], CommandError> {
    let [x, y, z, w] = q;
    let cross_vp = [
        y * p[2] - z * p[1],
        z * p[0] - x * p[2],
        x * p[1] - y * p[0],
    ];
    let t = [2.0 * cross_vp[0], 2.0 * cross_vp[1], 2.0 * cross_vp[2]];
    let cross_vt = [
        y * t[2] - z * t[1],
        z * t[0] - x * t[2],
        x * t[1] - y * t[0],
    ];
    let mut out = [0.0; 3];
    let terms = p.iter().zip(t.iter()).zip(cross_vt.iter());
    for (slot, ((point, cross_t), cross_p)) in out.iter_mut().zip(terms) {
        let value = point + w * cross_t + cross_p;
        if !value.is_finite() {
            return Err(CommandError::new(
                ErrorCode::NonFiniteNumber,
                "point",
                "rotation produced a non-finite coordinate",
            )
            .with_rule("PC-FRAME-001"));
        }
        *slot = value;
    }
    Ok(out)
}

/// One read-only coordination connection of the coordination project (F12 section 5): a stable
/// occurrence identity, the exact pinned source revision and the verified digest of every
/// pinned source layer. The bytes themselves live in the coordination store as external
/// manifest entries, which retained revisions keep (PC-KEEP-001).
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectionRecord {
    /// Stable identity of this occurrence; a repeated placement of one source is a distinct
    /// connection (PC-ID-002).
    pub id: ConnectionId,
    /// Source project identity.
    pub source_project: ProjectId,
    /// Source workspace identity of the pinned revision.
    pub source_workspace: WorkspaceId,
    /// Exact pinned source revision.
    pub source_revision: RevisionId,
    /// Full SHA-256 of the pinned source manifest; the record and the pinned bytes must agree.
    pub manifest: String,
    /// Pinned source layers as `(source layer name, digest)` pairs, sorted by name.
    pub layers: Vec<(String, String)>,
    /// Selected source scope (PC-COORD-003).
    pub scope: ScopeSelection,
    /// Declared rigid transform into the parent frame; absent means unknown alignment.
    pub placement: Option<ConnectionPlacement>,
    /// Human description; never a resolver or identity.
    pub label: String,
}

/// The project descriptor (F12 section 2): authored project metadata, the section/zone
/// registries of the 0.2 vocabulary, the contract requirements and the coordination
/// connections.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectDescriptor {
    /// The project this descriptor belongs to; agrees with the store's project identity.
    pub project: ProjectId,
    /// Human project metadata: labels without authority (PC-ID-001).
    pub metadata: ProjectMetadata,
    /// Contract, vocabulary and schema requirements.
    pub contract: ContractRequirements,
    /// Declared coordinate frame; conforming projects declare [`FRAME_F04`].
    pub frame: String,
    /// Sections by stable identity; codes are unique within the registry.
    pub sections: Vec<SectionRecord>,
    /// Zones by stable identity; codes are unique within the registry.
    pub zones: Vec<crate::coordination::ZoneRecord>,
    /// Coordination connections by stable occurrence identity.
    pub connections: Vec<ConnectionRecord>,
}

/// A human label: 1 to [`MAX_LABEL_BYTES`] UTF-8 bytes without control characters.
pub fn validate_label(label: &str, field: &'static str) -> Result<(), CommandError> {
    if label.is_empty() || label.len() > MAX_LABEL_BYTES {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            field,
            "a label is 1 to 256 bytes",
        ));
    }
    if label.chars().any(char::is_control) {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            field,
            "a label holds no control characters",
        ));
    }
    Ok(())
}

/// A full lowercase SHA-256 digest as text.
pub fn validate_digest(text: &str, field: &'static str) -> Result<(), CommandError> {
    if text.len() != 64
        || !text
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            field,
            "a digest is 64 lowercase hex digits",
        )
        .with_rule("PC-REV-001"));
    }
    Ok(())
}

/// Encodes the pinned layer map of one connection as one bounded text entry: pairs of
/// `name:digest` joined by `;`. Names come from the source manifest's validated virtual names
/// and hold no `;`, `:` or control characters.
pub fn encode_layers(layers: &[(String, String)]) -> Result<String, CommandError> {
    if layers.len() > MAX_PINNED_LAYERS {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "layers",
            "too many pinned layers for one connection",
        )
        .with_rule("PC-REV-001"));
    }
    let mut text = String::new();
    for (name, digest) in layers {
        if name.is_empty()
            || name.len() > 255
            || name
                .bytes()
                .any(|b| matches!(b, b';' | b':') || b.is_ascii_control())
        {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "layers",
                "a source layer name is bounded safe ASCII",
            )
            .with_rule("PC-REV-001"));
        }
        validate_digest(digest, "layers")?;
        if !text.is_empty() {
            text.push(';');
        }
        text.push_str(name);
        text.push(':');
        text.push_str(digest);
    }
    Ok(text)
}

/// Decodes the pinned layer map; the pairs come back sorted by source layer name.
pub fn decode_layers(text: &str) -> Result<Vec<(String, String)>, CommandError> {
    let mut layers = Vec::new();
    for pair in text.split(';') {
        let (name, digest) = pair
            .split_once(':')
            .ok_or_else(|| bad_layers("a layer entry is name:digest"))?;
        if name.is_empty() || name.len() > 255 {
            return Err(bad_layers("a source layer name is bounded safe ASCII"));
        }
        validate_digest(digest, "layers")?;
        layers.push((name.to_owned(), digest.to_owned()));
    }
    if layers.len() > MAX_PINNED_LAYERS {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "layers",
            "too many pinned layers for one connection",
        )
        .with_rule("PC-REV-001"));
    }
    layers.sort();
    Ok(layers)
}

fn bad_layers(message: &'static str) -> CommandError {
    CommandError::new(ErrorCode::InvalidArgument, "layers", message).with_rule("PC-REV-001")
}

impl ProjectDescriptor {
    /// A new descriptor for a project: default metadata, the declared F04 frame, the contract
    /// requirements this code interprets (the implemented 0.2 vocabulary) and empty
    /// registries. The schema digest is the caller's verified schema identity.
    pub fn new(project: ProjectId, schema: &str) -> Result<Self, CommandError> {
        validate_digest(schema, "schema")?;
        Ok(Self {
            project,
            metadata: ProjectMetadata {
                code: String::new(),
                name: String::new(),
                organization: None,
            },
            contract: ContractRequirements {
                contract: CONTRACT_ID.to_owned(),
                vocabulary: VOCABULARY_VERSION.to_owned(),
                schema: schema.to_owned(),
            },
            frame: FRAME_F04.to_owned(),
            sections: Vec::new(),
            zones: Vec::new(),
            connections: Vec::new(),
        })
    }

    /// Sets or replaces the human metadata (PC-OWN-002: only the explicit shared commands do
    /// this). `None` keeps the current text; the merged record must be complete, so the first
    /// set names the code and the name together (the 0.2 required pair); the project identity
    /// is never changed here.
    pub fn set_metadata(
        &mut self,
        code: Option<&str>,
        name: Option<&str>,
        organization: Option<&str>,
    ) -> Result<(), CommandError> {
        if let Some(code) = code {
            self.metadata.code = code.to_owned();
        }
        if let Some(name) = name {
            self.metadata.name = name.to_owned();
        }
        if let Some(organization) = organization {
            self.metadata.organization = Some(organization.to_owned());
        }
        self.metadata.validate()
    }

    /// Inserts a section or replaces the section of this identity. The record admits itself
    /// with the 0.2 rules; the code must stay unique among the other sections (PC-ID-001) and
    /// the registry cap is [`MAX_SECTIONS`].
    pub fn put_section(&mut self, record: SectionRecord) -> Result<(), CommandError> {
        record.validate()?;
        if self
            .sections
            .iter()
            .any(|s| s.id != record.id && s.code == record.code)
        {
            return Err(duplicate_code("section code"));
        }
        if let Some(existing) = self.sections.iter_mut().find(|s| s.id == record.id) {
            *existing = record;
            return Ok(());
        }
        if self.sections.len() >= MAX_SECTIONS {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "sections",
                "the section registry is full",
            ));
        }
        self.sections.push(record);
        Ok(())
    }

    /// Inserts a zone or replaces the zone of this identity, with the same uniqueness and cap
    /// rules as sections (PC-ID-001).
    pub fn put_zone(
        &mut self,
        record: crate::coordination::ZoneRecord,
    ) -> Result<(), CommandError> {
        record.validate()?;
        if self
            .zones
            .iter()
            .any(|z| z.id != record.id && z.code == record.code)
        {
            return Err(duplicate_code("zone code"));
        }
        if let Some(existing) = self.zones.iter_mut().find(|z| z.id == record.id) {
            *existing = record;
            return Ok(());
        }
        if self.zones.len() >= MAX_ZONES {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "zones",
                "the zone registry is full",
            ));
        }
        self.zones.push(record);
        Ok(())
    }

    /// Inserts a connection or replaces the pin of this occurrence identity. A new connection
    /// beyond [`MAX_CONNECTIONS`] is refused.
    pub fn put_connection(&mut self, record: ConnectionRecord) -> Result<(), CommandError> {
        self.validate_connection(&record)?;
        if let Some(existing) = self.connections.iter_mut().find(|c| c.id == record.id) {
            *existing = record;
            return Ok(());
        }
        if self.connections.len() >= MAX_CONNECTIONS {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "connections",
                "the connection registry is full",
            )
            .with_rule("PC-COORD-002"));
        }
        self.connections.push(record);
        Ok(())
    }

    /// Removes the connection of this identity. Older retained revisions keep their own pins
    /// and bytes; this changes only the current descriptor (PC-KEEP-001).
    pub fn remove_connection(&mut self, id: ConnectionId) -> Result<(), CommandError> {
        let before = self.connections.len();
        self.connections.retain(|c| c.id != id);
        if self.connections.len() == before {
            return Err(CommandError::new(
                ErrorCode::NotFound,
                "connection",
                "no connection of this identity exists",
            ));
        }
        Ok(())
    }

    /// The section of this code, if the registry holds one.
    pub fn section_by_code(&self, code: &str) -> Option<&SectionRecord> {
        self.sections.iter().find(|s| s.code == code)
    }

    /// The zone of this code, if the registry holds one.
    pub fn zone_by_code(&self, code: &str) -> Option<&crate::coordination::ZoneRecord> {
        self.zones.iter().find(|z| z.code == code)
    }

    /// Resolves a scope selection text against this descriptor: `whole` selects the whole
    /// project, otherwise the text is a section or zone code. An unresolved reference is
    /// refused (PC-ID-001).
    pub fn scope_of(&self, text: &str) -> Result<ScopeSelection, CommandError> {
        if text == "whole" {
            return Ok(ScopeSelection::WholeProject);
        }
        if let Some(section) = self.section_by_code(text) {
            let id = SectionId::parse(&section.id).map_err(|_| bad_registry_id())?;
            return Ok(ScopeSelection::Section(id));
        }
        if let Some(zone) = self.zone_by_code(text) {
            let id = ZoneId::parse(&zone.id).map_err(|_| bad_registry_id())?;
            return Ok(ScopeSelection::Zone(id));
        }
        Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "scope",
            "the scope names no section or zone of the source descriptor",
        )
        .with_rule("PC-ID-001"))
    }

    /// Full validation of every registry and record (the layer validator runs this on every
    /// open): records admit themselves with the 0.2 rules, codes stay unique per registry,
    /// counts stay within caps, digests and identities are well-formed and placements are
    /// rigid. The checks that remain (bytes present, digests match the store) belong to the
    /// coordination reader.
    pub fn validate(&self) -> Result<(), CommandError> {
        if self.sections.len() > MAX_SECTIONS
            || self.zones.len() > MAX_ZONES
            || self.connections.len() > MAX_CONNECTIONS
        {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "descriptor",
                "a registry exceeds its cap",
            ));
        }
        let mut seen = std::collections::BTreeSet::new();
        for section in &self.sections {
            section.validate()?;
            if !seen.insert(("s", section.code.as_str())) {
                return Err(duplicate_code("section code"));
            }
        }
        for zone in &self.zones {
            zone.validate()?;
            if !seen.insert(("z", zone.code.as_str())) {
                return Err(duplicate_code("zone code"));
            }
        }
        let mut connection_ids = std::collections::BTreeSet::new();
        for connection in &self.connections {
            self.validate_connection(connection)?;
            if !connection_ids.insert(connection.id) {
                return Err(CommandError::new(
                    ErrorCode::DuplicateId,
                    "connection",
                    "a connection identity is repeated",
                )
                .with_rule("PC-ID-001"));
            }
        }
        if self.frame != FRAME_F04 {
            return Err(CommandError::new(
                ErrorCode::UnsupportedVersion,
                "frame",
                "the declared frame is outside the admitted F04 line",
            )
            .with_rule("PC-FRAME-001"));
        }
        // An entirely unset metadata record is valid; anything set must be the complete 0.2
        // record (the code and name pair is required, per ProjectMetadata::validate).
        if !(self.metadata.code.is_empty()
            && self.metadata.name.is_empty()
            && self.metadata.organization.is_none())
        {
            self.metadata.validate()?;
        }
        Ok(())
    }

    fn validate_connection(&self, connection: &ConnectionRecord) -> Result<(), CommandError> {
        validate_digest(&connection.manifest, "manifest")?;
        if connection.layers.is_empty() || connection.layers.len() > MAX_PINNED_LAYERS {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "layers",
                "a connection pins at least one and at most 16 source layers",
            )
            .with_rule("PC-REV-001"));
        }
        let mut sorted = connection.layers.clone();
        sorted.sort();
        if sorted != connection.layers {
            return Err(bad_layers("the pinned layers must be sorted by name"));
        }
        for (name, digest) in &connection.layers {
            if name.is_empty()
                || name.len() > 255
                || name
                    .bytes()
                    .any(|b| matches!(b, b';' | b':') || b.is_ascii_control())
            {
                return Err(bad_layers("a source layer name is bounded safe ASCII"));
            }
            validate_digest(digest, "layers")?;
        }
        if let Some(placement) = connection.placement {
            placement.validate()?;
        }
        validate_label(&connection.label, "label")?;
        Ok(())
    }
}

fn duplicate_code(field: &'static str) -> CommandError {
    CommandError::new(
        ErrorCode::DuplicateId,
        field,
        "the code is already registered",
    )
    .with_rule("PC-ID-001")
}

fn bad_registry_id() -> CommandError {
    CommandError::new(
        ErrorCode::InvalidId,
        "scope",
        "the registry identity is malformed",
    )
    .with_rule("PC-ID-001")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordination::ZoneRecord;
    use crate::id::{ElementId, Uuid7};

    const PROJECT: &str = "019a7bf0-0000-7000-8000-00000000c001";
    const OTHER: &str = "019a7bf0-0000-7000-8000-00000000c002";
    const SECTION: &str = "019a7bf0-0000-7000-8000-00000000c101";
    const ZONE: &str = "019a7bf0-0000-7000-8000-00000000c201";
    const CONNECTION: &str = "019a7bf0-0000-7000-8000-00000000c301";
    const SCHEMA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn descriptor() -> ProjectDescriptor {
        ProjectDescriptor::new(ProjectId::parse(PROJECT).unwrap(), SCHEMA).unwrap()
    }

    fn section(code: &str) -> SectionRecord {
        SectionRecord {
            id: SECTION.to_owned(),
            code: code.to_owned(),
            label: code.to_owned(),
            modules: vec!["steel".to_owned()],
        }
    }

    fn zone(code: &str) -> ZoneRecord {
        ZoneRecord {
            id: ZONE.to_owned(),
            code: code.to_owned(),
            label: code.to_owned(),
        }
    }

    fn pin() -> ConnectionRecord {
        ConnectionRecord {
            id: ConnectionId::parse(CONNECTION).unwrap(),
            source_project: ProjectId::parse(OTHER).unwrap(),
            source_workspace: WorkspaceId::parse(OTHER).unwrap(),
            source_revision: RevisionId::parse(OTHER).unwrap(),
            manifest: SCHEMA.to_owned(),
            layers: vec![("authored.usda".to_owned(), SCHEMA.to_owned())],
            scope: ScopeSelection::WholeProject,
            placement: Some(ConnectionPlacement::IDENTITY),
            label: "steel source".to_owned(),
        }
    }

    fn code_of(error: &CommandError) -> ErrorCode {
        error.code
    }

    #[test]
    fn labels_and_digests_are_bounded() {
        assert!(validate_label("Steel detailing", "label").is_ok());
        assert_eq!(
            code_of(&validate_label("", "label").unwrap_err()),
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            code_of(&validate_label("a\nb", "label").unwrap_err()),
            ErrorCode::InvalidArgument
        );
        assert!(validate_digest(SCHEMA, "schema").is_ok());
        assert_eq!(
            code_of(&validate_digest("ABC", "schema").unwrap_err()),
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn metadata_updates_and_keeps_the_project() {
        let mut d = descriptor();
        d.set_metadata(Some("2401-DEMO"), Some("Demo warehouse"), Some("ACME"))
            .unwrap();
        assert_eq!(d.metadata.code, "2401-DEMO");
        assert_eq!(d.project.to_string(), PROJECT);
        d.set_metadata(None, Some("Renamed"), None).unwrap();
        assert_eq!(d.metadata.code, "2401-DEMO");
        assert_eq!(d.metadata.name, "Renamed");
        assert_eq!(d.metadata.organization.as_deref(), Some("ACME"));
        // The project code is a label per the 0.2 record: bounded text, not a registry code.
        d.set_metadata(Some("2401 demo (rev.)"), None, None)
            .unwrap();
        assert_eq!(
            code_of(&d.set_metadata(Some(""), None, None).unwrap_err()),
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn a_rename_keeps_the_identity_and_a_repeated_code_is_refused() {
        let mut d = descriptor();
        d.put_zone(zone("K1")).unwrap();
        let renamed = ZoneRecord {
            code: "K1".to_owned(),
            label: "Building K1".to_owned(),
            ..zone("K1")
        };
        d.put_zone(renamed).unwrap();
        assert_eq!(d.zones.len(), 1);
        assert_eq!(d.zones[0].label, "Building K1");
        let other = ZoneRecord {
            id: "019a7bf0-0000-7000-8000-00000000c202".to_owned(),
            code: "K1".to_owned(),
            label: "K1".to_owned(),
        };
        assert_eq!(
            code_of(&d.put_zone(other).unwrap_err()),
            ErrorCode::DuplicateId
        );
        d.put_zone(ZoneRecord {
            id: ZONE.to_owned(),
            code: "K2".to_owned(),
            label: "K2".to_owned(),
        })
        .unwrap();
        assert_eq!(d.zones[0].code, "K2");
        assert_eq!(d.zones[0].id, ZONE);
    }

    #[test]
    fn sections_and_zones_are_separate_registries() {
        let mut d = descriptor();
        d.put_section(section("KM")).unwrap();
        d.put_zone(zone("KM")).unwrap();
        assert_eq!(d.sections.len(), 1);
        assert_eq!(d.zones.len(), 1);
    }

    #[test]
    fn scope_resolution_resolves_codes_and_refuses_unknown_ones() {
        let mut d = descriptor();
        d.put_section(section("KM")).unwrap();
        d.put_zone(zone("K1")).unwrap();
        assert_eq!(d.scope_of("whole").unwrap(), ScopeSelection::WholeProject);
        assert_eq!(
            d.scope_of("KM").unwrap(),
            ScopeSelection::Section(SectionId::parse(SECTION).unwrap())
        );
        assert_eq!(
            d.scope_of("K1").unwrap(),
            ScopeSelection::Zone(ZoneId::parse(ZONE).unwrap())
        );
        assert_eq!(
            code_of(&d.scope_of("XX").unwrap_err()),
            ErrorCode::InvalidArgument
        );
        assert_eq!(d.scope_of("XX").unwrap_err().rule_id, Some("PC-ID-001"));
    }

    #[test]
    fn registries_have_caps() {
        let mut d = descriptor();
        for i in 0..MAX_ZONES {
            d.put_zone(ZoneRecord {
                id: format!("019a7bf0-0000-7{i:03x}-8000-{i:012x}"),
                code: format!("Z{i}"),
                label: "z".to_owned(),
            })
            .unwrap();
        }
        assert_eq!(
            code_of(&d.put_zone(zone("EXTRA")).unwrap_err()),
            ErrorCode::LimitExceeded
        );
    }

    #[test]
    fn connections_are_replaced_by_identity_and_validated() {
        let mut d = descriptor();
        d.put_connection(pin()).unwrap();
        let mut moved = pin();
        moved.source_revision = RevisionId::parse("019a7bf0-0000-7000-8000-00000000c003").unwrap();
        d.put_connection(moved).unwrap();
        assert_eq!(d.connections.len(), 1);
        let mut duplicate = pin();
        duplicate.id = ConnectionId::parse("019a7bf0-0000-7000-8000-00000000c302").unwrap();
        duplicate.manifest = "no".to_owned();
        assert_eq!(
            code_of(&d.put_connection(duplicate).unwrap_err()),
            ErrorCode::InvalidArgument
        );
        let mut unsorted = pin();
        unsorted.id = ConnectionId::parse("019a7bf0-0000-7000-8000-00000000c303").unwrap();
        unsorted.layers = vec![
            ("b.usda".to_owned(), SCHEMA.to_owned()),
            ("a.usda".to_owned(), SCHEMA.to_owned()),
        ];
        assert_eq!(
            code_of(&d.put_connection(unsorted).unwrap_err()),
            ErrorCode::InvalidArgument
        );
        let mut empty_layers = pin();
        empty_layers.id = ConnectionId::parse("019a7bf0-0000-7000-8000-00000000c304").unwrap();
        empty_layers.layers = Vec::new();
        assert_eq!(
            code_of(&d.put_connection(empty_layers).unwrap_err()),
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn removing_an_unknown_connection_is_refused_and_a_known_one_leaves() {
        let mut d = descriptor();
        d.put_connection(pin()).unwrap();
        let id = d.connections[0].id;
        let missing = ConnectionId::parse("019a7bf0-0000-7000-8000-00000000c305").unwrap();
        assert_eq!(
            code_of(&d.remove_connection(missing).unwrap_err()),
            ErrorCode::NotFound
        );
        d.remove_connection(id).unwrap();
        assert!(d.connections.is_empty());
    }

    #[test]
    fn the_frame_is_checked_and_a_fresh_descriptor_validates() {
        let mut d = descriptor();
        d.validate().unwrap();
        assert_eq!(d.contract.vocabulary, "0.2.0");
        assert_eq!(d.contract.contract, CONTRACT_ID);
        d.frame = "other".to_owned();
        assert_eq!(
            code_of(&d.validate().unwrap_err()),
            ErrorCode::UnsupportedVersion
        );
        assert_eq!(d.validate().unwrap_err().rule_id, Some("PC-FRAME-001"));
        let mut fresh = descriptor();
        assert!(fresh.metadata.code.is_empty());
        fresh.validate().unwrap();
        let bad = SectionRecord {
            label: String::new(),
            ..section("KM")
        };
        assert_eq!(
            code_of(&fresh.put_section(bad).unwrap_err()),
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn identity_placements_apply_and_compose() {
        let identity = ConnectionPlacement::IDENTITY;
        let point = [1000.0, 2000.0, 3000.0];
        assert_eq!(identity.apply_point(point).unwrap(), point);
        let outer = ConnectionPlacement {
            translation_mm: [100.0, 0.0, 0.0],
            orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
        };
        let composed = identity.compose(&outer).unwrap();
        assert_eq!(
            composed.apply_point(point).unwrap(),
            [1100.0, 2000.0, 3000.0]
        );
        // A 90-degree rotation about Z maps +X to +Y; the inner translation rotates with it.
        let sqrt_half = 0.5f64.sqrt();
        let turn = ConnectionPlacement {
            translation_mm: [0.0, 0.0, 0.0],
            orientation_xyzw: [0.0, 0.0, sqrt_half, sqrt_half],
        };
        let nested = identity.compose(&turn).unwrap();
        let got = nested.apply_point([1000.0, 0.0, 5.0]).unwrap();
        assert!((got[0] - 0.0).abs() < 1e-9);
        assert!((got[1] - 1000.0).abs() < 1e-9);
        assert!((got[2] - 5.0).abs() < 1e-9);
        // Nested: two quarter turns compose to a half turn.
        let twice = turn.compose(&turn).unwrap();
        let got = twice.apply_point([1000.0, 0.0, 5.0]).unwrap();
        assert!((got[0] + 1000.0).abs() < 1e-9);
        assert!(got[1].abs() < 1e-9);
    }

    #[test]
    fn non_rigid_or_non_finite_placements_are_refused() {
        let identity = ConnectionPlacement::IDENTITY;
        let mut scaled = ConnectionPlacement::IDENTITY;
        scaled.orientation_xyzw = [0.0, 0.0, 0.0, 2.0];
        assert_eq!(
            code_of(&scaled.validate().unwrap_err()),
            ErrorCode::UnsupportedGeometry
        );
        assert_eq!(scaled.validate().unwrap_err().rule_id, Some("PC-FRAME-001"));
        let mut far = ConnectionPlacement::IDENTITY;
        far.translation_mm = [PLACEMENT_CAP_MM + 1.0, 0.0, 0.0];
        assert_eq!(
            code_of(&far.validate().unwrap_err()),
            ErrorCode::InvalidArgument
        );
        let mut nan = ConnectionPlacement::IDENTITY;
        nan.translation_mm[1] = f64::NAN;
        assert_eq!(
            code_of(&nan.validate().unwrap_err()),
            ErrorCode::NonFiniteNumber
        );
        assert_eq!(
            code_of(&identity.apply_point([f64::INFINITY, 0.0, 0.0]).unwrap_err()),
            ErrorCode::NonFiniteNumber
        );
    }

    #[test]
    fn an_absent_placement_is_explicit_unknown_alignment() {
        let mut d = descriptor();
        let mut unknown = pin();
        unknown.placement = None;
        d.put_connection(unknown).unwrap();
        d.validate().unwrap();
        assert!(d.connections[0].placement.is_none());
    }

    #[test]
    fn layer_maps_round_trip_sorted_and_bounded() {
        let layers = vec![
            ("authored.usda".to_owned(), SCHEMA.to_owned()),
            ("derived.usda".to_owned(), SCHEMA.to_owned()),
            ("root.usda".to_owned(), SCHEMA.to_owned()),
        ];
        let text = encode_layers(&layers).unwrap();
        assert_eq!(decode_layers(&text).unwrap(), layers);
        let unsorted = vec![
            ("b.usda".to_owned(), SCHEMA.to_owned()),
            ("a.usda".to_owned(), SCHEMA.to_owned()),
        ];
        let text = encode_layers(&unsorted).unwrap();
        assert_eq!(
            decode_layers(&text).unwrap(),
            vec![
                ("a.usda".to_owned(), SCHEMA.to_owned()),
                ("b.usda".to_owned(), SCHEMA.to_owned())
            ]
        );
        let too_many = (0..MAX_PINNED_LAYERS + 1)
            .map(|i| (format!("l{i}.usda"), SCHEMA.to_owned()))
            .collect::<Vec<_>>();
        assert_eq!(
            code_of(&encode_layers(&too_many).unwrap_err()),
            ErrorCode::LimitExceeded
        );
        let evil = vec![("a;b.usda".to_owned(), SCHEMA.to_owned())];
        assert!(encode_layers(&evil).is_err());
        let bad_digest = vec![("a.usda".to_owned(), "zz".to_owned())];
        assert!(encode_layers(&bad_digest).is_err());
        assert!(decode_layers("nocolon.usda").is_err());
    }

    #[test]
    fn typed_ids_are_distinct_from_element_ids() {
        let id = ElementId::from_uuid(Uuid7::parse(SECTION).unwrap());
        let section = SectionId::from_uuid(Uuid7::parse(SECTION).unwrap());
        assert_eq!(id.uuid(), section.uuid());
    }
}
