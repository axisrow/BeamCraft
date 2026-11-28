// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Task-level Q03 checks against independently approved model requirements.

use crate::check::{self, CheckReport};
use beamcraft_host::revision::Revision;
use beamcraft_json::{Json, Limits};
use beamcraft_layer_engine::{Prim, Value};
use beamcraft_model::digest::{ContentDigest, Sha256};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, ProjectId};
use beamcraft_model::prim::ElementCategory;
use beamcraft_model::record::{
    CornerCutRequirement, CutPurpose, Record, RecordPayload, UpperBound,
};
use session::Refusal;
use std::collections::BTreeSet;
use steel::engine::check::RecordedInputs;

type Run<T> = Result<T, Refusal>;

/// Largest accepted task specification in bytes.
pub const MAX_SPEC_BYTES: usize = 65_536;
/// Largest number of independent expectations per task.
pub const MAX_REQUIREMENTS: usize = 128;
/// Wire version of an approved task specification.
pub const VERSION: u64 = 1;

fn invalid(message: &'static str) -> Refusal {
    Refusal::new(ErrorCode::InvalidArgument, message)
}

fn invalid_requirement() -> Refusal {
    invalid("the task specification contains an invalid requirement")
}

/// Versioned task specification whose bytes match the task owner's expected digest.
pub struct TaskSpec {
    digest: ContentDigest,
    project: ProjectId,
    requirements: Vec<Requirement>,
}

enum Requirement {
    Element {
        id: ElementId,
        kind: String,
    },
    AbsentElement(ElementId),
    Attribute {
        path: String,
        name: String,
        expected: Json,
    },
    Relationship {
        path: String,
        name: String,
        targets: Vec<String>,
    },
    Layer {
        name: String,
        digest: ContentDigest,
    },
    Corner(CornerCutRequirement),
    RequiredCheck {
        target: String,
        check: String,
    },
}

/// A task-level finding about an expected fact or authored record.
pub struct TaskFinding {
    /// Stable diagnostic code.
    pub code: &'static str,
    /// A contradicted requirement is `fail`; an unavailable check is `unknown`.
    pub status: &'static str,
    /// Index into the approved requirements array, when a requirement was checked.
    pub requirement: Option<usize>,
    /// Name, path or element id of the expected fact.
    pub target: String,
    /// Expected value from the approved task.
    pub expected: Json,
    /// Value observed in the verified model.
    pub actual: Json,
    /// What the caller needs to address.
    pub message: &'static str,
}

/// Model checks and task findings for one verified revision.
pub struct TaskCheck {
    /// Geometric Q03 checks with the owner's independent corner requirements applied.
    pub model: CheckReport,
    /// Digest of the approved specification.
    pub digest: ContentDigest,
    /// Number of requirements in the approved specification.
    pub requirement_count: usize,
    /// Task model mismatches. The geometry findings keep their existing rule codes.
    pub findings: Vec<TaskFinding>,
}

impl TaskSpec {
    /// Decodes bytes bound to an expected digest supplied outside the model store.
    pub fn decode(bytes: &[u8], expected: ContentDigest) -> Run<Self> {
        if bytes.len() > MAX_SPEC_BYTES {
            return Err(invalid("the task specification exceeds its byte budget"));
        }
        let digest =
            Sha256::digest(bytes).map_err(|_| invalid("the task digest could not be computed"))?;
        if digest != expected {
            return Err(Refusal::new(
                ErrorCode::IntegrityError,
                "the task specification does not match the approved digest",
            ));
        }
        let invalid_spec = || {
            invalid("expected a version 1 task specification with a project id and requirements")
        };
        let value = beamcraft_json::decode(bytes, &Limits::DEFAULT).map_err(|_| invalid_spec())?;
        let template = value.get("template").ok();
        let mut keys = vec!["version", "project_id", "requirements"];
        if template.is_some() {
            keys.push("template");
        }
        value.closed(&keys).map_err(|_| invalid_spec())?;
        if value.get("version").and_then(Json::unsigned) != Ok(VERSION) {
            return Err(invalid_spec());
        }
        let project = ProjectId::parse(field(&value, "project_id")?).map_err(|_| invalid_spec())?;
        let rows = value
            .get("requirements")
            .and_then(Json::array)
            .map_err(|_| invalid_spec())?;
        if rows.len() > MAX_REQUIREMENTS {
            return Err(invalid(
                "the task specification must contain 1 to 128 requirements",
            ));
        }
        let mut requirements = rows
            .iter()
            .map(Requirement::decode)
            .collect::<Run<Vec<_>>>()?;
        if let Some(template) = template {
            requirements.extend(column_base_requirements(template)?);
        }
        if requirements.is_empty() || requirements.len() > MAX_REQUIREMENTS {
            return Err(invalid(
                "the task specification must contain 1 to 128 expanded requirements",
            ));
        }
        let mut corners = BTreeSet::new();
        for requirement in &requirements {
            if let Requirement::Corner(corner) = requirement
                && !corners.insert(corner_key(corner))
            {
                return Err(invalid(
                    "the task specification repeats a corner requirement",
                ));
            }
        }
        Ok(Self {
            digest,
            project,
            requirements,
        })
    }

    /// Compares task facts and evaluates approved corner cuts in the same verified revision.
    /// Geometry is measured even when the corresponding model record is missing or altered.
    pub fn check_revision(
        &self,
        revision: &Revision,
        mut inputs: RecordedInputs,
        element: Option<ElementId>,
        offset: usize,
        limit: usize,
    ) -> Run<TaskCheck> {
        let content = revision.manifest().content();
        let same_project = self.project.to_string() == content.project().as_str();
        let mut findings = Vec::new();

        let inspection = crate::plate::inspect_revision_observing(revision, |prims| {
            if !same_project {
                findings.push(TaskFinding {
                    code: "TASK_PROJECT_MISMATCH",
                    status: "fail",
                    requirement: None,
                    target: "project_id".into(),
                    expected: Json::text(self.project.to_string()),
                    actual: Json::text(content.project().as_str()),
                    message: "the approved specification belongs to another project",
                });
                return;
            }
            for (index, requirement) in self.requirements.iter().enumerate() {
                let finding = match requirement {
                    Requirement::Layer { name, digest } => {
                        let actual = content
                            .entries()
                            .iter()
                            .find(|entry| entry.name() == name)
                            .map(|entry| entry.digest());
                        (actual != Some(*digest)).then(|| {
                            mismatch(
                                index,
                                name.clone(),
                                Json::text(digest.to_string()),
                                actual.map_or(Json::Null, |d| Json::text(d.to_string())),
                            )
                        })
                    }
                    Requirement::Corner(corner) => {
                        for id in std::iter::once(corner.target).chain(corner.neighbours) {
                            if element_prim(prims, &id).is_none() {
                                findings.push(mismatch(
                                    index,
                                    id.to_string(),
                                    Json::text("active element"),
                                    Json::Null,
                                ));
                            }
                        }
                        let finding = compare_record(index, corner, &inputs.corner_cuts);
                        inputs
                            .corner_cuts
                            .retain(|record| corner_key(record) != corner_key(corner));
                        inputs.corner_cuts.push(corner.clone());
                        finding
                    }
                    Requirement::RequiredCheck { target, check } => Some(TaskFinding {
                        code: "TASK_CHECK_UNSUPPORTED",
                        status: "unknown",
                        requirement: Some(index),
                        target: target.clone(),
                        expected: Json::text(check),
                        actual: Json::Null,
                        message: "BeamCraft has no evaluator for the requested check",
                    }),
                    _ => requirement.compare_prims(index, prims),
                };
                findings.extend(finding);
            }
        })?;
        let mut model = check::check_model(&inspection, &inputs, element, offset, limit)?;
        if findings.iter().any(|finding| finding.status == "fail") {
            model.status = "red";
        } else if model.status != "red" && (!findings.is_empty() || element.is_some()) {
            model.status = "unknown";
        }
        Ok(TaskCheck {
            model,
            digest: self.digest,
            requirement_count: self.requirements.len(),
            findings,
        })
    }
}

fn field<'a>(value: &'a Json, name: &str) -> Run<&'a str> {
    let text = value
        .get(name)
        .and_then(Json::string)
        .map_err(|_| invalid_requirement())?;
    if text.is_empty() || text.len() > 1024 || text.chars().any(char::is_control) {
        return Err(invalid_requirement());
    }
    Ok(text)
}

fn element_id(text: &str) -> Run<ElementId> {
    ElementId::parse(text).map_err(|_| invalid_requirement())
}

fn fields(value: &Json, allowed: &[&str]) -> Run<()> {
    value.closed(allowed).map_err(|_| invalid_requirement())
}

fn path(value: &str) -> Run<String> {
    if value.len() > 1024
        || !value.starts_with('/')
        || value.len() < 2
        || value.ends_with('/')
        || value
            .split('/')
            .skip(1)
            .any(|c| c.is_empty() || c == "." || c == "..")
        || value.chars().any(char::is_control)
    {
        return Err(invalid_requirement());
    }
    Ok(value.into())
}

fn column_base_requirements(value: &Json) -> Run<Vec<Requirement>> {
    fields(
        value,
        &[
            "type",
            "footing",
            "pedestal",
            "base_plate",
            "column",
            "rib_a",
            "rib_b",
            "assembly",
            "required_mm",
            "tolerance_mm",
        ],
    )?;
    if field(value, "type")? != "column_base_v1" {
        return Err(invalid(
            "the task specification names an unsupported template",
        ));
    }
    let id = |role| element_id(field(value, role)?);
    let assembly = id("assembly")?;
    let assembly_path = assembly.prim_path(ElementCategory::Assembly);
    let parts = [
        (id("footing")?, "column"),
        (id("pedestal")?, "column"),
        (id("base_plate")?, "plate"),
        (id("column")?, "column"),
        (id("rib_a")?, "plate"),
        (id("rib_b")?, "plate"),
    ];
    let mut identities = BTreeSet::from([assembly]);
    let mut requirements = vec![Requirement::Element {
        id: assembly,
        kind: "assembly".into(),
    }];
    for (id, kind) in parts {
        if !identities.insert(id) {
            return Err(invalid(
                "the column base template repeats an element identity",
            ));
        }
        requirements.push(Requirement::Element {
            id,
            kind: kind.into(),
        });
        requirements.push(Requirement::Relationship {
            path: id.prim_path(ElementCategory::Part),
            name: "bim:partOf".into(),
            targets: vec![assembly_path.clone()],
        });
    }
    for role in ["rib_a", "rib_b"] {
        requirements.push(approved_corner(CornerCutRequirement {
            target: id(role)?,
            neighbours: [id("base_plate")?, id("column")?],
            required_mm: dimension(value, "required_mm")?,
            tolerance_mm: dimension(value, "tolerance_mm")?,
            upper_bound: None,
            purpose: CutPurpose::Clearance,
        })?);
    }
    Ok(requirements)
}

fn approved_corner(corner: CornerCutRequirement) -> Run<Requirement> {
    // Reuse the model's existing bounds, units and identity validation.
    Record {
        id: corner.target,
        name: "Approved task corner".into(),
        payload: RecordPayload::CornerCut(corner.clone()),
    }
    .validate()?;
    Ok(Requirement::Corner(corner))
}

impl Requirement {
    fn decode(value: &Json) -> Run<Self> {
        match field(value, "type")? {
            "element" => {
                fields(value, &["type", "id", "kind"])?;
                let kind = field(value, "kind")?;
                Ok(Self::Element {
                    id: element_id(field(value, "id")?)?,
                    kind: kind.into(),
                })
            }
            "absent_element" => {
                fields(value, &["type", "id"])?;
                Ok(Self::AbsentElement(element_id(field(value, "id")?)?))
            }
            "attribute" => {
                fields(value, &["type", "path", "name", "value"])?;
                Ok(Self::Attribute {
                    path: path(field(value, "path")?)?,
                    name: field(value, "name")?.into(),
                    expected: value
                        .get("value")
                        .map_err(|_| invalid_requirement())?
                        .clone(),
                })
            }
            "relationship" => {
                fields(value, &["type", "path", "name", "targets"])?;
                let targets = value
                    .get("targets")
                    .and_then(Json::array)
                    .map_err(|_| invalid_requirement())?
                    .iter()
                    .map(|v| path(v.string().map_err(|_| invalid_requirement())?))
                    .collect::<Run<Vec<_>>>()?;
                Ok(Self::Relationship {
                    path: path(field(value, "path")?)?,
                    name: field(value, "name")?.into(),
                    targets,
                })
            }
            "layer" => {
                fields(value, &["type", "name", "sha256"])?;
                Ok(Self::Layer {
                    name: field(value, "name")?.into(),
                    digest: field(value, "sha256")?
                        .parse()
                        .map_err(|_| invalid_requirement())?,
                })
            }
            "corner_cut" => {
                let required = [
                    "type",
                    "target",
                    "neighbours",
                    "required_mm",
                    "tolerance_mm",
                    "purpose",
                ];
                let upper_bound = match value.get("upper_bound") {
                    Ok(bound) => {
                        fields(
                            value,
                            &[
                                "type",
                                "target",
                                "neighbours",
                                "required_mm",
                                "tolerance_mm",
                                "purpose",
                                "upper_bound",
                            ],
                        )?;
                        fields(bound, &["max_mm", "tolerance_mm"])?;
                        Some(UpperBound {
                            max_mm: dimension(bound, "max_mm")?,
                            tolerance_mm: dimension(bound, "tolerance_mm")?,
                        })
                    }
                    Err(_) => {
                        fields(value, &required)?;
                        None
                    }
                };
                let values = value
                    .get("neighbours")
                    .and_then(Json::array)
                    .map_err(|_| invalid_requirement())?;
                let [a, b] = values else {
                    return Err(invalid_requirement());
                };
                let id = |v: &Json| element_id(v.string().map_err(|_| invalid_requirement())?);
                let corner = CornerCutRequirement {
                    target: element_id(field(value, "target")?)?,
                    neighbours: [id(a)?, id(b)?],
                    required_mm: dimension(value, "required_mm")?,
                    tolerance_mm: dimension(value, "tolerance_mm")?,
                    upper_bound,
                    purpose: CutPurpose::of_token(field(value, "purpose")?)
                        .ok_or_else(invalid_requirement)?,
                };
                approved_corner(corner)
            }
            "required_check" => {
                fields(value, &["type", "target", "check"])?;
                Ok(Self::RequiredCheck {
                    target: field(value, "target")?.into(),
                    check: field(value, "check")?.into(),
                })
            }
            _ => Err(invalid_requirement()),
        }
    }

    fn compare_prims(&self, index: usize, prims: &[Prim]) -> Option<TaskFinding> {
        let by_id = |id: &ElementId| element_prim(prims, id);
        let by_path = |path: &str| prims.iter().find(|prim| prim.active && prim.path == path);
        match self {
            Self::Element { id, kind } => {
                let actual = by_id(id)
                    .and_then(|prim| {
                        prim.properties
                            .iter()
                            .find(|prop| prop.name == "bim:elementKind")
                    })
                    .and_then(|prop| match &prop.value {
                        Some(Value::Text(text)) => Some(text.as_str()),
                        _ => None,
                    });
                (actual != Some(kind)).then(|| {
                    mismatch(
                        index,
                        id.to_string(),
                        Json::text(kind),
                        actual.map_or(Json::Null, Json::text),
                    )
                })
            }
            Self::AbsentElement(id) => by_id(id)
                .map(|prim| mismatch(index, id.to_string(), Json::Null, Json::text(&prim.path))),
            Self::Attribute {
                path,
                name,
                expected,
            } => {
                let actual = by_path(path)
                    .and_then(|prim| prim.properties.iter().find(|p| &p.name == name))
                    .and_then(|prop| prop.value.as_ref());
                let matches = actual.is_some_and(|value| matches_value(expected, value));
                (!matches).then(|| {
                    mismatch(
                        index,
                        format!("{path}.{name}"),
                        expected.clone(),
                        actual.map_or(Json::Null, |value| value_json(value, 0)),
                    )
                })
            }
            Self::Relationship {
                path,
                name,
                targets,
            } => {
                let actual = by_path(path)
                    .and_then(|prim| prim.relationships.iter().find(|r| &r.name == name))
                    .map(|rel| &rel.targets);
                (actual != Some(targets)).then(|| {
                    mismatch(
                        index,
                        format!("{path}.{name}"),
                        texts(targets),
                        actual.map_or(Json::Null, |values| texts(values)),
                    )
                })
            }
            Self::Corner(_) | Self::Layer { .. } | Self::RequiredCheck { .. } => None,
        }
    }
}

fn dimension(value: &Json, name: &str) -> Run<f64> {
    value
        .get(name)
        .and_then(Json::float)
        .map_err(|_| invalid_requirement())
}

fn element_prim<'a>(prims: &'a [Prim], id: &ElementId) -> Option<&'a Prim> {
    let text = id.to_string();
    prims
        .iter()
        .find(|prim| prim.active && prim.element_id.as_deref() == Some(&text))
}

fn mismatch(index: usize, target: String, expected: Json, actual: Json) -> TaskFinding {
    TaskFinding {
        code: "TASK_MODEL_MISMATCH",
        status: "fail",
        requirement: Some(index),
        target,
        expected: preview(expected),
        actual: preview(actual),
        message: "the verified model differs from the approved requirement",
    }
}

fn corner_key(corner: &CornerCutRequirement) -> (ElementId, [ElementId; 2]) {
    let mut neighbours = corner.neighbours;
    neighbours.sort();
    (corner.target, neighbours)
}

fn compare_record(
    index: usize,
    expected: &CornerCutRequirement,
    records: &[CornerCutRequirement],
) -> Option<TaskFinding> {
    let key = corner_key(expected);
    let mut records = records.iter().filter(|record| corner_key(record) == key);
    let first = records.next();
    let duplicate = records.next().is_some();
    if !duplicate
        && first.is_some_and(|record| {
            record.required_mm == expected.required_mm
                && record.tolerance_mm == expected.tolerance_mm
                && record.purpose == expected.purpose
                && record.upper_bound == expected.upper_bound
        })
    {
        return None;
    }
    let (code, message) = match (first, duplicate) {
        (None, _) => (
            "TASK_CORNER_RECORD_MISSING",
            "the required corner-cut record is missing",
        ),
        (_, true) => (
            "TASK_CORNER_RECORD_MISMATCH",
            "the rib has multiple records for the approved corner",
        ),
        _ => (
            "TASK_CORNER_RECORD_MISMATCH",
            "the corner-cut record differs from the approved requirement",
        ),
    };
    Some(TaskFinding {
        code,
        status: "fail",
        requirement: Some(index),
        target: expected.target.to_string(),
        expected: corner_json(expected),
        actual: first.map_or(Json::Null, corner_json),
        message,
    })
}

fn corner_json(corner: &CornerCutRequirement) -> Json {
    let mm = |v| Json::number(v).unwrap_or(Json::Null);
    Json::object([
        (
            "neighbours",
            Json::Array(
                corner
                    .neighbours
                    .iter()
                    .map(|id| Json::text(id.to_string()))
                    .collect(),
            ),
        ),
        ("required_mm", mm(corner.required_mm)),
        ("tolerance_mm", mm(corner.tolerance_mm)),
        ("purpose", Json::text(corner.purpose.token())),
        (
            "upper_bound",
            corner.upper_bound.as_ref().map_or(Json::Null, |bound| {
                Json::object([
                    ("max_mm", mm(bound.max_mm)),
                    ("tolerance_mm", mm(bound.tolerance_mm)),
                ])
            }),
        ),
    ])
}

fn texts(values: &[String]) -> Json {
    if values.len() > 16 {
        return collection_size(values.len());
    }
    Json::Array(values.iter().map(Json::text).collect())
}

fn collection_size(len: usize) -> Json {
    Json::object([
        ("length", Json::uint(len as u64)),
        ("preview_only", Json::Bool(true)),
    ])
}

fn preview(value: Json) -> Json {
    const LIMITS: Limits = Limits {
        bytes: 2048,
        depth: 5,
        ..Limits::DEFAULT
    };
    if value.encode(&LIMITS).is_ok() {
        return value;
    }
    match value {
        Json::Array(values) => collection_size(values.len()),
        Json::Object(values) => collection_size(values.len()),
        _ => Json::object([("preview_only", Json::Bool(true))]),
    }
}

fn matches_value(expected: &Json, actual: &Value) -> bool {
    match (expected, actual) {
        (Json::Null, Value::Block) => true,
        (Json::Bool(a), Value::Bool(b)) => a == b,
        (Json::String(a), Value::Text(b)) => a == b,
        (Json::Number(a), Value::Int(b)) => a.parse::<i64>() == Ok(*b),
        (_, Value::Uint(b)) => expected.unsigned() == Ok(*b),
        (_, Value::Number(b)) => expected.float() == Ok(*b),
        (Json::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| matches_value(a, b))
        }
        (Json::Object(a), Value::Dictionary(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(key, a)| b.get(key).is_some_and(|b| matches_value(a, b)))
        }
        _ => false,
    }
}

// Keep observed values bounded even if an authored attribute is a large mesh array.
fn value_json(value: &Value, depth: usize) -> Json {
    if depth >= 3 {
        return Json::text("<nested value>");
    }
    match value {
        Value::Block => Json::object([("authored", Json::text("blocked"))]),
        Value::Bool(v) => Json::Bool(*v),
        Value::Int(v) => Json::Number(v.to_string()),
        Value::Uint(v) => Json::uint(*v),
        Value::Number(v) => Json::number(*v).unwrap_or(Json::Null),
        Value::Text(v) if v.len() <= 1024 => Json::text(v),
        Value::Text(v) => Json::object([
            ("bytes", Json::uint(v.len() as u64)),
            ("preview_only", Json::Bool(true)),
        ]),
        Value::Array(v) if v.len() <= 8 => {
            Json::Array(v.iter().map(|v| value_json(v, depth + 1)).collect())
        }
        Value::Dictionary(v) if v.len() <= 8 => Json::Object(
            v.iter()
                .map(|(k, v)| (k.clone(), value_json(v, depth + 1)))
                .collect(),
        ),
        Value::Array(v) => collection_size(v.len()),
        Value::Dictionary(v) => collection_size(v.len()),
    }
}
