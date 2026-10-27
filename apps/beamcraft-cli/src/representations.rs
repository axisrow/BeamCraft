// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! Personal ordered filter sets with strict versioning and cooperative file CAS publication.
use crate::{attributes::Attributes, filters, plate::PlateError, saved_views::Views};
use beamcraft_engine::representation::{Appearance, StyleRule};
use beamcraft_json::{Json, Limits};
use beamcraft_model::{error::ErrorCode, id::ElementId};
use std::collections::BTreeMap;
fn invalid(message: &'static str) -> PlateError {
    PlateError::new(ErrorCode::InvalidArgument, message)
}
fn key(id: ElementId) -> String {
    format!("representation_set_{}", id.to_string().replace('-', ""))
}
fn binding_key(id: ElementId) -> String {
    format!("representation_binding_{}", id.to_string().replace('-', ""))
}
/// Stable named display set; rules retain saved filter identities and ordering.
#[derive(Debug, Clone)]
pub struct Set {
    /// Stable identity.
    pub id: ElementId,
    /// Readable name.
    pub name: String,
    /// Original P15 class palette fallback.
    pub by_class: bool,
    /// First matching filter wins.
    pub rules: Vec<(ElementId, Appearance)>,
}
/// Decode strict finite appearance channels.
pub fn appearance(j: &Json) -> Result<Appearance, PlateError> {
    let color = j
        .get("color")
        .and_then(Json::array)
        .map_err(|_| invalid("color must have three channels"))?;
    let [r, g, b] = color else {
        return Err(invalid("color must have three channels"));
    };
    let n = |j: &Json| {
        j.float()
            .map_err(|_| invalid("appearance channel must be numeric"))
    };
    let a = Appearance {
        color: [n(r)?, n(g)?, n(b)?],
        transparency: n(j
            .get("transparency")
            .map_err(|_| invalid("transparency is required"))?)?,
    };
    a.validate()?;
    Ok(a)
}
/// Appearance transport JSON.
pub fn appearance_json(a: Appearance) -> Json {
    Json::object([
        (
            "color",
            Json::Array(
                a.color
                    .into_iter()
                    .map(|v| Json::Number(v.to_string()))
                    .collect(),
            ),
        ),
        ("transparency", Json::Number(a.transparency.to_string())),
    ])
}
/// Decode a complete user-supplied set body: by_class and ordered rules.
pub fn decode(id: ElementId, name: String, j: &Json) -> Result<Set, PlateError> {
    j.closed(&["by_class", "rules"])
        .map_err(|_| invalid("representation body needs by_class and rules"))?;
    if name.trim().is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
        return Err(invalid("invalid representation name"));
    }
    let by_class = j
        .get("by_class")
        .and_then(Json::boolean)
        .map_err(|_| invalid("by_class must be boolean"))?;
    let rules = j
        .get("rules")
        .and_then(Json::array)
        .map_err(|_| invalid("rules must be an array"))?;
    if rules.len() > 64 {
        return Err(invalid("at most 64 representation rules"));
    }
    let rules = rules
        .iter()
        .map(|j| {
            j.closed(&["filter", "color", "transparency"])
                .map_err(|_| invalid("unknown representation rule fields"))?;
            let id = ElementId::parse(
                j.get("filter")
                    .and_then(Json::string)
                    .map_err(|_| invalid("filter UUID required"))?,
            )
            .map_err(|_| invalid("invalid filter UUID"))?;
            Ok((id, appearance(j)?))
        })
        .collect::<Result<Vec<_>, PlateError>>()?;
    Ok(Set {
        id,
        name,
        by_class,
        rules,
    })
}
/// Complete inspectable metadata.
pub fn json(s: &Set) -> Json {
    Json::object([
        ("id", Json::text(s.id.to_string())),
        ("name", Json::text(&s.name)),
        ("by_class", Json::Bool(s.by_class)),
        (
            "rules",
            Json::Array(
                s.rules
                    .iter()
                    .map(|(id, a)| {
                        let mut j = appearance_json(*a);
                        if let Json::Object(f) = &mut j {
                            f.insert("filter".into(), Json::text(id.to_string()));
                        }
                        j
                    })
                    .collect(),
            ),
        ),
    ])
}
/// Inspect a saved set, rejecting incompatible or corrupt private records.
pub fn saved(v: &Views, id: ElementId) -> Result<Set, PlateError> {
    let s = v
        .document
        .records
        .get(&key(id))
        .ok_or_else(|| PlateError::new(ErrorCode::NotFound, "representation set not found"))?;
    let j = beamcraft_json::decode(s.as_bytes(), &Limits::DEFAULT)
        .map_err(|_| invalid("invalid saved representation JSON"))?;
    j.closed(&["version", "id", "name", "by_class", "rules"])
        .map_err(|_| invalid("unknown saved representation fields"))?;
    if j.get("version")
        .and_then(Json::unsigned)
        .map_err(|_| invalid("representation version required"))?
        != 1
    {
        return Err(PlateError::new(
            ErrorCode::UnsupportedVersion,
            "unsupported representation version",
        ));
    }
    if j.get("id")
        .and_then(Json::string)
        .map_err(|_| invalid("representation identity required"))?
        != id.to_string()
    {
        return Err(invalid("representation identity mismatch"));
    }
    let name = j
        .get("name")
        .and_then(Json::string)
        .map_err(|_| invalid("representation name required"))?
        .to_owned();
    decode(
        id,
        name,
        &Json::object([
            (
                "by_class",
                j.get("by_class")
                    .map_err(|_| invalid("by_class missing"))?
                    .clone(),
            ),
            (
                "rules",
                j.get("rules")
                    .map_err(|_| invalid("rules missing"))?
                    .clone(),
            ),
        ]),
    )
}
/// List stable IDs in order.
pub fn list(v: &Views) -> Result<Vec<Set>, PlateError> {
    v.document
        .records
        .keys()
        .filter_map(|k| k.strip_prefix("representation_set_"))
        .map(|s| {
            let id =
                beamcraft_layer_engine::authored::attribute::property_id(&format!("user:a_{s}"))
                    .ok_or_else(|| invalid("invalid representation key"))?;
            saved(v, id)
        })
        .collect()
}
/// Resolve saved filters through the one V14 evaluator registry.
pub fn compile(v: &Views, s: &Set, attrs: &Attributes) -> Result<Vec<StyleRule>, PlateError> {
    let data = filters::dataset(attrs)?;
    s.rules
        .iter()
        .map(|(id, a)| {
            a.validate()?;
            let rule = filters::decode_rule(&filters::saved(v, *id)?.rule, attrs)?;
            data.predicate(&rule)?;
            Ok(StyleRule {
                filter: rule,
                appearance: *a,
            })
        })
        .collect()
}
/// Stage one admitted set. Caller writes the private file atomically with CAS.
pub fn save(v: &mut Views, s: &Set, attrs: &Attributes) -> Result<(), PlateError> {
    let normalized = decode(
        s.id,
        s.name.clone(),
        &Json::object([
            ("by_class", Json::Bool(s.by_class)),
            (
                "rules",
                json(s)
                    .get("rules")
                    .map_err(|_| invalid("rules missing"))?
                    .clone(),
            ),
        ]),
    )?;
    compile(v, &normalized, attrs)?;
    let mut j = json(s);
    if let Json::Object(f) = &mut j {
        f.insert("version".into(), Json::uint(1));
    }
    let b = j
        .encode(&Limits::DEFAULT)
        .map_err(|_| invalid("representation exceeds JSON limit"))?;
    if b.len() > 65536 {
        return Err(invalid("representation exceeds private record limit"));
    }
    v.document.records.insert(
        key(s.id),
        String::from_utf8(b).map_err(|_| invalid("representation is not UTF8"))?,
    );
    Ok(())
}
/// Read the optional binding without treating missing sets as a default.
pub fn binding(v: &Views, id: ElementId) -> Result<Option<ElementId>, PlateError> {
    v.document
        .records
        .get(&binding_key(id))
        .map(|s| ElementId::parse(s).map_err(|_| invalid("invalid representation binding")))
        .transpose()
}
/// Stage a view binding.
pub fn bind(v: &mut Views, id: ElementId, set: Option<ElementId>) -> Result<(), PlateError> {
    if !v.document.views.iter().any(|view| view.id == id) {
        return Err(PlateError::new(ErrorCode::NotFound, "view not found"));
    }
    if let Some(s) = set {
        saved(v, s)?;
        v.document.records.insert(binding_key(id), s.to_string());
    } else {
        v.document.records.remove(&binding_key(id));
    }
    Ok(())
}
/// Delete only unused sets.
pub fn delete(v: &mut Views, id: ElementId) -> Result<(), PlateError> {
    saved(v, id)?;
    if v.document
        .records
        .iter()
        .any(|(k, val)| k.starts_with("representation_binding_") && val == &id.to_string())
    {
        return Err(PlateError::new(
            ErrorCode::DependencyConflict,
            "representation set is used by a view",
        ));
    }
    v.document.records.remove(&key(id));
    Ok(())
}
/// Decode an ephemeral per-element overlay (1024 entries). Caller validates snapshot identities.
pub fn overlay(j: &Json) -> Result<BTreeMap<ElementId, Appearance>, PlateError> {
    let a = j.array().map_err(|_| invalid("overlay must be an array"))?;
    if a.len() > 1024 {
        return Err(invalid("overlay exceeds 1024 elements"));
    }
    let mut out = BTreeMap::new();
    for j in a {
        j.closed(&["id", "color", "transparency"])
            .map_err(|_| invalid("unknown overlay fields"))?;
        let id = ElementId::parse(
            j.get("id")
                .and_then(Json::string)
                .map_err(|_| invalid("overlay UUID required"))?,
        )
        .map_err(|_| invalid("invalid overlay UUID"))?;
        if out.insert(id, appearance(j)?).is_some() {
            return Err(invalid("duplicate overlay UUID"));
        }
    }
    Ok(out)
}
