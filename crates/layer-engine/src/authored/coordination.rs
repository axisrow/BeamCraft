// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! U22 bounded, explicit USD coordination records (F10 0.2, F12 sections 2 to 4): the exact
//! serialization of section, zone, authored-provenance, project-metadata and opening-decision
//! records, the 0.2 schema pin and the bounded parsers that refuse unknown, oversized and
//! malformed records with F04 codes. This module defines no store upgrade, no negotiation
//! advertisement and no scope authorization (U18, U19 and U20 define those); 0.1 stores keep
//! the 0.1 pin and contracts, and a layer carrying these records is only interpreted here when
//! its root pin names the 0.2 mapping digest.

use super::edit;
use super::validate::{optional, required, text};
use super::*;
use beamcraft_model::coordination::{
    AuthoredScope, DecisionReason, DecisionVerdict, MAX_DECISIONS, MAX_SECTION_MODULES,
    MAX_SECTIONS, MAX_ZONES, OpeningDecision, ProjectMetadata, SectionRecord, VOCABULARY_VERSION,
    ZoneRecord,
};
use beamcraft_model::id::Uuid7;
use beamcraft_model::limits::MAX_LABEL_BYTES;
use openusd::sdf::{self, AbstractData};
use std::collections::{BTreeMap, BTreeSet};

/// The coordination scope of the authored layer (F12 section 2: authored project metadata).
pub const COORDINATION_PATH: &str = "/World/Coordination";
/// The section registry scope.
pub const SECTIONS_PATH: &str = "/World/Coordination/Sections";
/// The zone registry scope.
pub const ZONES_PATH: &str = "/World/Coordination/Zones";
/// The opening-decision evidence scope (F10 0.2 section 3).
pub const DECISIONS_PATH: &str = "/World/Coordination/Decisions";

/// The closed custom attributes of a section record, in authoring order.
pub const SECTION_FIELDS: &[(&str, &str)] = &[
    ("bim:schemaVersion", "string"),
    ("bim:section:id", "string"),
    ("bim:section:code", "string"),
    ("bim:section:label", "string"),
    ("bim:section:modules", "string[]"),
];
/// The closed custom attributes of a zone record, in authoring order.
pub const ZONE_FIELDS: &[(&str, &str)] = &[
    ("bim:schemaVersion", "string"),
    ("bim:zone:id", "string"),
    ("bim:zone:code", "string"),
    ("bim:zone:label", "string"),
];
/// The closed required custom attributes of a decision record, in authoring order.
pub const DECISION_FIELDS: &[(&str, &str)] = &[
    ("bim:schemaVersion", "string"),
    ("bim:decision:id", "string"),
    ("bim:decision:proposal", "string"),
    ("bim:decision:proposalDigest", "string"),
    ("bim:decision:hostRevision", "string"),
    ("bim:decision:verdict", "token"),
];
/// The one optional decision field; present exactly on a rejection.
pub const DECISION_OPTIONAL_FIELDS: &[(&str, &str)] = &[("bim:decision:reason", "token")];
/// The closed authored-provenance attributes of an element; both optional, the zone only
/// within a section.
pub const SCOPE_FIELDS: &[(&str, &str)] = &[("bim:sectionId", "string"), ("bim:zoneId", "string")];
/// The root `customLayerData` keys of the project metadata record.
pub const METADATA_KEYS: [&str; 3] = ["bimProjectCode", "bimProjectName", "bimProjectOrganization"];

/// The 0.2 addition lines of the canonical mapping descriptor. The complete 0.2 mapping text
/// is [`super::SCHEMA_DESCRIPTOR`] (the frozen 0.1 mapping) followed by exactly these bytes,
/// and its SHA-256 is the only digest a layer may pin to have its records interpreted.
pub const SCHEMA_DESCRIPTOR_V02_ADDITIONS: &str = concat!(
    "coordination=BeamCraft/U02/coordination/2;model=0.2.0;vocabulary=0.2.0;records=section|zone|decision;metadata=customLayerData\n",
    "root:customLayerData:bimSchemaDigest=sha256(this mapping);bimProjectCode=string(1..256);bimProjectName=string(1..256);bimProjectOrganization=string(1..256,optional)\n",
    "section=/World/Coordination/Sections/SEC_<uuid7-32>;type=Scope;kind=group;closed field set only\n",
    "bim:schemaVersion=string(0.2.0);bim:section:id=string(uuid7,=path);bim:section:code=string(1..32,[A-Za-z0-9][A-Za-z0-9_-]*);bim:section:label=string(1..256);bim:section:modules=string[](0..16 unique namespace tokens)\n",
    "zone=/World/Coordination/Zones/ZN_<uuid7-32>;type=Scope;kind=group;closed field set only\n",
    "bim:schemaVersion=string(0.2.0);bim:zone:id=string(uuid7,=path);bim:zone:code=string(1..32);bim:zone:label=string(1..256)\n",
    "decision=/World/Coordination/Decisions/DEC_<uuid7-32>;type=Scope;kind=group;closed field set only;immutable evidence\n",
    "bim:schemaVersion=string(0.2.0);bim:decision:id=string(uuid7,=path);bim:decision:proposal=string(uuid7);bim:decision:proposalDigest=string(sha256);bim:decision:hostRevision=string(uuid7);",
    "bim:decision:verdict=token(accepted|rejected);bim:decision:reason=token(position|size|structural|other,rejected exactly)\n",
    "scope=element bim:sectionId=string(uuid7,optional);bim:zoneId=string(uuid7,optional,section required);unknown provenance grants no write permission\n",
    "budgets=sections<=1024;zones<=1024;decisions<=4096;codes 1..32;labels 1..256;modules<=16;ids and codes unique per registry;codes and names are labels, never authority\n",
    "unknown, oversized or malformed record in a coordination scope is refused with an F04 code;no store upgrade, negotiation or scope authorization is defined on this line (U18/U19/U20);0.1 stores keep the 0.1 pin and contracts\n",
);

/// The complete canonical 0.2 mapping descriptor: the frozen 0.1 text plus the 0.2 additions.
pub fn schema_descriptor_v02() -> String {
    format!("{SCHEMA_DESCRIPTOR}\n{SCHEMA_DESCRIPTOR_V02_ADDITIONS}")
}

/// The exact mapping identity of the 0.2 line, separate from the 0.1 [`schema_identity`].
pub fn schema_identity_v02() -> Result<SchemaIdentity, Diagnostic> {
    Ok(SchemaIdentity {
        model: VOCABULARY_VERSION,
        digest: Sha256::digest(schema_descriptor_v02().as_bytes()).map_err(|_| limit())?,
    })
}

/// Only the 0.2 mapping digest is admitted on the 0.2 line; the 0.1 digest stays 0.1.
pub fn pin_accepted_v02(pin: &str) -> Result<bool, Diagnostic> {
    Ok(pin == schema_identity_v02()?.digest.to_string())
}

/// Whether the root metadata pins exactly the 0.2 mapping digest. A layer that does not is
/// never interpreted on this line (PC-VER-001): records on it are unknown content.
pub fn pinned_v02(metadata: &BTreeMap<String, Value>) -> Result<bool, Diagnostic> {
    let Some(Value::Dictionary(data)) = metadata.get("customLayerData") else {
        return Ok(false);
    };
    match data.get("bimSchemaDigest") {
        None => Ok(false),
        Some(Value::Text(pin)) => pin_accepted_v02(pin),
        Some(_) => Err(fail(
            ErrorCode::UnsupportedVersion,
            "layer schema digest is not a text",
        )),
    }
}

/// The refusal for content whose store does not pin the 0.2 mapping (DV-VER-001).
fn unpinned() -> Diagnostic {
    fail(
        ErrorCode::UnsupportedVersion,
        "layer is not pinned to the 0.2 coordination mapping",
    )
}

fn record_path(scope: &str, prefix: &str, id: &str) -> Result<String, Diagnostic> {
    let id = Uuid7::parse(id).map_err(|_| fail(ErrorCode::InvalidId, "invalid record identity"))?;
    Ok(format!("{scope}/{prefix}_{}", id.hex32()))
}

/// Authors the root project metadata record (F12 section 2 labels). Existing dictionary
/// entries are kept; only the three metadata keys are written.
pub fn author_metadata(data: &mut sdf::Data, value: &ProjectMetadata) -> Result<(), Diagnostic> {
    value.validate().map_err(|e| fail(e.code, e.message))?;
    let root = sdf::Path::abs_root();
    let mut dictionary = match data
        .try_field(&root, "customLayerData")
        .map_err(|_| corrupt())?
    {
        Some(v) => match v.as_ref() {
            sdf::Value::Dictionary(v) => v.clone(),
            _ => return Err(corrupt()),
        },
        None => Default::default(),
    };
    dictionary.insert(
        METADATA_KEYS[0].into(),
        sdf::Value::String(value.code.clone()),
    );
    dictionary.insert(
        METADATA_KEYS[1].into(),
        sdf::Value::String(value.name.clone()),
    );
    match &value.organization {
        Some(organization) => {
            dictionary.insert(
                METADATA_KEYS[2].into(),
                sdf::Value::String(organization.clone()),
            );
        }
        None => {
            dictionary.remove(METADATA_KEYS[2]);
        }
    }
    edit::field(
        data,
        "/",
        "customLayerData",
        sdf::Value::Dictionary(dictionary),
    )
}

fn record_prim(data: &mut sdf::Data, p: &str) -> Result<(), Diagnostic> {
    edit::container(data, COORDINATION_PATH, "Scope", Some("group"))?;
    let scope = p
        .rsplit_once('/')
        .map(|(parent, _)| parent)
        .ok_or_else(corrupt)?;
    edit::container(data, scope, "Scope", Some("group"))?;
    edit::container(data, p, "Scope", Some("group"))
}

/// Authors or replaces one section record prim. The record is admitted first; a refused
/// record leaves all bytes unchanged.
pub fn author_section(data: &mut sdf::Data, record: &SectionRecord) -> Result<(), Diagnostic> {
    record.validate().map_err(|e| fail(e.code, e.message))?;
    let p = record_path(SECTIONS_PATH, "SEC", &record.id)?;
    record_prim(data, &p)?;
    let values: Vec<String> = record.modules.clone();
    edit::attr(
        data,
        &p,
        "bim:schemaVersion",
        "string",
        sdf::Value::String(VOCABULARY_VERSION.into()),
    )?;
    edit::attr(
        data,
        &p,
        "bim:section:id",
        "string",
        sdf::Value::String(record.id.clone()),
    )?;
    edit::attr(
        data,
        &p,
        "bim:section:code",
        "string",
        sdf::Value::String(record.code.clone()),
    )?;
    edit::attr(
        data,
        &p,
        "bim:section:label",
        "string",
        sdf::Value::String(record.label.clone()),
    )?;
    edit::attr(
        data,
        &p,
        "bim:section:modules",
        "string[]",
        sdf::Value::StringVec(values),
    )
}

/// Authors or replaces one zone record prim.
pub fn author_zone(data: &mut sdf::Data, record: &ZoneRecord) -> Result<(), Diagnostic> {
    record.validate().map_err(|e| fail(e.code, e.message))?;
    let p = record_path(ZONES_PATH, "ZN", &record.id)?;
    record_prim(data, &p)?;
    edit::attr(
        data,
        &p,
        "bim:schemaVersion",
        "string",
        sdf::Value::String(VOCABULARY_VERSION.into()),
    )?;
    edit::attr(
        data,
        &p,
        "bim:zone:id",
        "string",
        sdf::Value::String(record.id.clone()),
    )?;
    edit::attr(
        data,
        &p,
        "bim:zone:code",
        "string",
        sdf::Value::String(record.code.clone()),
    )?;
    edit::attr(
        data,
        &p,
        "bim:zone:label",
        "string",
        sdf::Value::String(record.label.clone()),
    )
}

/// Authors or replaces one immutable decision record (F10 0.2 section 3).
pub fn author_decision(data: &mut sdf::Data, decision: &OpeningDecision) -> Result<(), Diagnostic> {
    decision.validate().map_err(|e| fail(e.code, e.message))?;
    let p = record_path(DECISIONS_PATH, "DEC", &decision.id)?;
    record_prim(data, &p)?;
    for (name, value) in [
        (
            "bim:schemaVersion",
            sdf::Value::String(VOCABULARY_VERSION.into()),
        ),
        ("bim:decision:id", sdf::Value::String(decision.id.clone())),
        (
            "bim:decision:proposal",
            sdf::Value::String(decision.proposal.clone()),
        ),
        (
            "bim:decision:proposalDigest",
            sdf::Value::String(decision.proposal_digest.to_string()),
        ),
        (
            "bim:decision:hostRevision",
            sdf::Value::String(decision.host_revision.clone()),
        ),
        (
            "bim:decision:verdict",
            sdf::Value::token(decision.verdict.as_str()),
        ),
    ] {
        let (_, ty) = DECISION_FIELDS
            .iter()
            .find(|(field, _)| *field == name)
            .ok_or_else(corrupt)?;
        edit::attr(data, &p, name, ty, value)?;
    }
    match decision.reason {
        Some(reason) => edit::attr(
            data,
            &p,
            "bim:decision:reason",
            "token",
            sdf::Value::token(reason.as_str()),
        ),
        // An accepted decision never carries a refusal reason; a replaced record loses it.
        None => edit::attribute(
            data,
            &p,
            "bim:decision:reason",
            "token",
            Opinion::Remove,
            true,
        ),
    }
}

/// Authors or removes the provenance record of one element (F12 PC-OWN-001). `None` removes
/// both attributes; the unknown-provenance state stores nothing.
pub fn author_scope(
    data: &mut sdf::Data,
    element_path: &str,
    scope: Option<&AuthoredScope>,
) -> Result<(), Diagnostic> {
    match scope {
        None => {
            for (name, ty) in SCOPE_FIELDS {
                edit::attribute(data, element_path, name, ty, Opinion::Remove, true)?;
            }
            Ok(())
        }
        Some(scope) => {
            scope.validate().map_err(|e| fail(e.code, e.message))?;
            edit::attr(
                data,
                element_path,
                "bim:sectionId",
                "string",
                sdf::Value::String(scope.section.clone()),
            )?;
            match &scope.zone {
                Some(zone) => edit::attr(
                    data,
                    element_path,
                    "bim:zoneId",
                    "string",
                    sdf::Value::String(zone.clone()),
                ),
                None => edit::attribute(
                    data,
                    element_path,
                    "bim:zoneId",
                    "string",
                    Opinion::Remove,
                    true,
                ),
            }
        }
    }
}

/// The parsed coordination content of one 0.2-pinned layer.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Coordination {
    /// The project metadata record, when the root layer authors one.
    pub metadata: Option<ProjectMetadata>,
    /// Every admitted section record, ordered by identity.
    pub sections: Vec<SectionRecord>,
    /// Every admitted zone record, ordered by identity.
    pub zones: Vec<ZoneRecord>,
    /// Every admitted decision record, ordered by identity.
    pub decisions: Vec<OpeningDecision>,
}

/// Parses the whole coordination content of a pinned snapshot. The layer must pin the 0.2
/// mapping digest (`UNSUPPORTED_VERSION` otherwise, including the 0.1 digest). Every owned
/// prim inside the coordination scopes must be a complete, well-formed record of its scope;
/// anything else is refused with an F04 code. Foreign (non-owned) prims stay inert.
pub fn read(
    metadata: &BTreeMap<String, Value>,
    prims: &[Prim],
) -> Result<Coordination, Diagnostic> {
    if !pinned_v02(metadata)? {
        return Err(unpinned());
    }
    unknown_scope(prims)?;
    Ok(Coordination {
        metadata: metadata_record(metadata)?,
        sections: registry(prims, SECTIONS_PATH, "SEC", MAX_SECTIONS, section_record)?,
        zones: registry(prims, ZONES_PATH, "ZN", MAX_ZONES, zone_record)?,
        decisions: registry(prims, DECISIONS_PATH, "DEC", MAX_DECISIONS, decision_record)?,
    })
}

/// Refuses an owned prim in any coordination scope other than the three registries; unknown
/// records are never silently carried as coordination content.
fn unknown_scope(prims: &[Prim]) -> Result<(), Diagnostic> {
    let branch = format!("{COORDINATION_PATH}/");
    for p in prims.iter().filter(|p| p.path.starts_with(&branch)) {
        if p.sources.iter().any(|s| !s.owned) {
            continue;
        }
        let rest = &p.path[branch.len()..];
        if !(rest == "Sections"
            || rest == "Zones"
            || rest == "Decisions"
            || rest.starts_with("Sections/")
            || rest.starts_with("Zones/")
            || rest.starts_with("Decisions/"))
        {
            return Err(fail(
                ErrorCode::InvalidArgument,
                "unknown coordination scope",
            ));
        }
    }
    Ok(())
}

/// Parses the project metadata record from root `customLayerData`; absent stays `None`.
fn metadata_record(
    metadata: &BTreeMap<String, Value>,
) -> Result<Option<ProjectMetadata>, Diagnostic> {
    let Some(Value::Dictionary(data)) = metadata.get("customLayerData") else {
        return Ok(None);
    };
    let text_entry = |key: &str| -> Result<Option<String>, Diagnostic> {
        match data.get(key) {
            None => Ok(None),
            Some(Value::Text(v)) => {
                if v.len() > MAX_LABEL_BYTES {
                    return Err(limit());
                }
                Ok(Some(v.clone()))
            }
            Some(_) => Err(invalid()),
        }
    };
    let Some(code) = text_entry(METADATA_KEYS[0])? else {
        for key in METADATA_KEYS.iter().skip(1) {
            if data.contains_key(*key) {
                return Err(invalid());
            }
        }
        return Ok(None);
    };
    let Some(name) = text_entry(METADATA_KEYS[1])? else {
        return Err(invalid());
    };
    let organization = text_entry(METADATA_KEYS[2])?;
    let record = ProjectMetadata {
        code,
        name,
        organization,
    };
    record.validate().map_err(|e| fail(e.code, e.message))?;
    Ok(Some(record))
}

/// Walks one registry scope: exactly one owned record prim per child, at most `max` of them,
/// every record complete and well formed, identities and codes unique.
fn registry<T>(
    prims: &[Prim],
    scope: &str,
    prefix: &str,
    max: usize,
    parse: fn(&Prim, &[Prim]) -> Result<T, Diagnostic>,
) -> Result<Vec<T>, Diagnostic> {
    let branch = format!("{scope}/");
    let mut out = Vec::new();
    let mut ids = BTreeSet::new();
    let mut codes = BTreeSet::new();
    for p in prims.iter().filter(|p| p.path.starts_with(&branch)) {
        if p.sources.iter().any(|s| !s.owned) {
            continue;
        }
        if out.len() >= max {
            return Err(limit());
        }
        let name = p.path.strip_prefix(&branch).ok_or_else(invalid)?;
        if name.contains('/') {
            return Err(fail(
                ErrorCode::InvalidArgument,
                "a coordination record is exactly one segment below its registry scope",
            ));
        }
        let record = parse(p, prims)?;
        let id = record_id(p, prefix)?;
        if !ids.insert(id) {
            return Err(fail(
                ErrorCode::DuplicateId,
                "duplicate coordination record identity",
            ));
        }
        if let Some(code) = record_code(prefix, p)?
            && !codes.insert(code)
        {
            return Err(fail(
                ErrorCode::DuplicateId,
                "duplicate coordination record code",
            ));
        }
        out.push(record);
    }
    Ok(out)
}

fn record_id(p: &Prim, prefix: &str) -> Result<String, Diagnostic> {
    let field = match prefix {
        "SEC" => "bim:section:id",
        "ZN" => "bim:zone:id",
        _ => "bim:decision:id",
    };
    let id = text(p, field, "string")?.to_owned();
    let parsed =
        Uuid7::parse(&id).map_err(|_| fail(ErrorCode::InvalidId, "invalid record identity"))?;
    let name = p
        .path
        .rsplit_once('/')
        .map(|(_, name)| name)
        .ok_or_else(invalid)?;
    if name != format!("{prefix}_{}", parsed.hex32()) {
        return Err(fail(
            ErrorCode::InvalidId,
            "coordination record path and identity disagree",
        ));
    }
    Ok(id)
}

/// The registry code of a record; decisions carry no code (PC-ID-001 uniqueness is
/// per registry of coded identities).
fn record_code(prefix: &str, p: &Prim) -> Result<Option<String>, Diagnostic> {
    let field = match prefix {
        "SEC" => "bim:section:code",
        "ZN" => "bim:zone:code",
        _ => return Ok(None),
    };
    Ok(Some(text(p, field, "string")?.to_owned()))
}

fn schema_version(p: &Prim) -> Result<(), Diagnostic> {
    if text(p, "bim:schemaVersion", "string")? != VOCABULARY_VERSION {
        return Err(fail(
            ErrorCode::UnsupportedVersion,
            "coordination record vocabulary version",
        ));
    }
    Ok(())
}

/// A record prim's closed structure: type, kind and the exact field set with types.
fn record_shape(
    p: &Prim,
    fields: &[(&str, &str)],
    optional: &[(&str, &str)],
) -> Result<(), Diagnostic> {
    if p.type_name.as_deref() != Some("Scope") || p.kind.as_deref() != Some("group") {
        return Err(fail(
            ErrorCode::InvalidArgument,
            "a coordination record must be a group Scope prim",
        ));
    }
    let known = fields.iter().chain(optional.iter());
    for property in &p.properties {
        if !known.clone().any(|(name, _)| name == &property.name) {
            return Err(fail(
                ErrorCode::InvalidArgument,
                "unknown field on a coordination record",
            ));
        }
    }
    for (name, ty) in fields {
        required(p, name, ty)?;
    }
    Ok(())
}

fn section_record(p: &Prim, _prims: &[Prim]) -> Result<SectionRecord, Diagnostic> {
    record_shape(p, SECTION_FIELDS, &[])?;
    schema_version(p)?;
    let modules = match optional(p, "bim:section:modules", "string[]")? {
        Some(Value::Array(items)) => {
            let mut modules = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    Value::Text(module) => modules.push(module.clone()),
                    _ => return Err(invalid()),
                }
            }
            modules
        }
        _ => return Err(invalid()),
    };
    if modules.len() > MAX_SECTION_MODULES {
        return Err(limit());
    }
    let record = SectionRecord {
        id: text(p, "bim:section:id", "string")?.to_owned(),
        code: text(p, "bim:section:code", "string")?.to_owned(),
        label: text(p, "bim:section:label", "string")?.to_owned(),
        modules,
    };
    record.validate().map_err(|e| fail(e.code, e.message))?;
    Ok(record)
}

fn zone_record(p: &Prim, _prims: &[Prim]) -> Result<ZoneRecord, Diagnostic> {
    record_shape(p, ZONE_FIELDS, &[])?;
    schema_version(p)?;
    let record = ZoneRecord {
        id: text(p, "bim:zone:id", "string")?.to_owned(),
        code: text(p, "bim:zone:code", "string")?.to_owned(),
        label: text(p, "bim:zone:label", "string")?.to_owned(),
    };
    record.validate().map_err(|e| fail(e.code, e.message))?;
    Ok(record)
}

fn decision_record(p: &Prim, _prims: &[Prim]) -> Result<OpeningDecision, Diagnostic> {
    record_shape(p, DECISION_FIELDS, DECISION_OPTIONAL_FIELDS)?;
    schema_version(p)?;
    let verdict_token = text(p, "bim:decision:verdict", "token")?;
    let verdict = DecisionVerdict::parse(verdict_token).ok_or_else(invalid)?;
    let reason = match optional(p, "bim:decision:reason", "token")? {
        None => None,
        Some(Value::Text(token)) => Some(DecisionReason::parse(token).ok_or_else(invalid)?),
        Some(_) => return Err(invalid()),
    };
    let digest = text(p, "bim:decision:proposalDigest", "string")?
        .parse::<ContentDigest>()
        .map_err(|_| invalid())?;
    let record = OpeningDecision {
        id: text(p, "bim:decision:id", "string")?.to_owned(),
        proposal: text(p, "bim:decision:proposal", "string")?.to_owned(),
        proposal_digest: digest,
        host_revision: text(p, "bim:decision:hostRevision", "string")?.to_owned(),
        verdict,
        reason,
    };
    record.validate().map_err(|e| fail(e.code, e.message))?;
    Ok(record)
}

/// Parses the authored-provenance record of one element (F12 PC-OWN-001): `None` when neither
/// attribute is present, the section scope otherwise. A zone without a section, a wrong type
/// or an unparseable identity is refused; parsing never grants write permission.
pub fn authored_scope(p: &Prim) -> Result<Option<AuthoredScope>, Diagnostic> {
    let section = match optional(p, "bim:sectionId", "string")? {
        None => None,
        Some(Value::Text(id)) => Some(id.clone()),
        Some(_) => return Err(invalid()),
    };
    let zone = match optional(p, "bim:zoneId", "string")? {
        None => None,
        Some(Value::Text(id)) => Some(id.clone()),
        Some(_) => return Err(invalid()),
    };
    match (section, zone) {
        (None, None) => Ok(None),
        (None, Some(_)) => Err(invalid()),
        (Some(section), zone) => {
            let scope = AuthoredScope { section, zone };
            scope.validate().map_err(|e| fail(e.code, e.message))?;
            Ok(Some(scope))
        }
    }
}
