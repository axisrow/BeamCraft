// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! P11 canonical definition links and bounded materialization for model readers.
use super::*;
use beamcraft_model::{
    id::ElementId,
    instance::{InstanceLink, MAX_PART_INSTANCES, PartKind},
};
use openusd::sdf;

/// Persistent shared-definition relationship name.
pub const RELATIONSHIP: &str = "bim:instanceOf";

fn text<'a>(p: &'a Prim, name: &str) -> Option<&'a str> {
    match p
        .properties
        .iter()
        .find(|v| v.name == name)?
        .value
        .as_ref()?
    {
        Value::Text(s) => Some(s),
        _ => None,
    }
}
fn owned_part(p: &Prim) -> Option<PartKind> {
    if p.sources.is_empty()
        || p.sources.iter().any(|s| !s.owned)
        || text(p, "bim:schemaVersion") != Some("0.1.0")
        || text(p, "bim:geometryMode") != Some("parametric")
    {
        return None;
    }
    PartKind::parse(text(p, "bim:elementKind")?)
}
/// Whether a property belongs to the shared shape/operation definition.
pub fn shared_property(name: &str) -> bool {
    matches!(
        name,
        "steel:plate:contourPointsMm"
            | "steel:plate:thicknessMm"
            | "steel:plate:thicknessSide"
            | "steel:plate:cornerVertex"
            | "steel:plate:cornerKind"
            | "steel:plate:cornerD1Mm"
            | "steel:plate:cornerD2Mm"
            | "steel:plate:cornerRadiusMm"
            | "steel:plate:holePointsMm"
            | "steel:plate:holeVertexCounts"
            | "steel:hole:pointsMm"
            | "steel:hole:vertexCounts"
            | "steel:hole:operationIds"
            | "steel:hole:ordinals"
            | "steel:hole:kinds"
            | "steel:member:pathType"
            | "steel:member:rollRad"
            | "steel:member:planePosition"
            | "steel:member:planeOffsetMm"
            | "steel:member:depthPosition"
            | "steel:member:depthOffsetMm"
            | "steel:member:startOffsetMm"
            | "steel:member:endOffsetMm"
            | "bim:axisPointsMm"
            | "steel:cut:operationId"
            | "steel:cut:ordinal"
            | "steel:cut:pointMm"
            | "steel:cut:normal"
            | "steel:cut:keep"
            | "steel:cut:nextOrdinal"
            | "steel:cut:retiredOperationIds"
            | "steel:hole:nextOrdinal"
            | "steel:hole:retiredOperationIds"
    )
}
/// Whether a relationship belongs to the shared definition.
pub fn shared_relationship(name: &str) -> bool {
    matches!(name, "bim:profile" | "bim:material")
}

/// Canonical definition prim for an instance, validating ownership, kind and the flat graph.
/// A plain part returns `None`. Shadow opinions on shared fields are always refused.
pub fn definition<'a>(p: &Prim, prims: &'a [Prim]) -> Result<Option<&'a Prim>, Diagnostic> {
    // Foreign and future vocabulary stays inert; only this owned schema is interpreted.
    if p.sources.is_empty()
        || p.sources.iter().any(|s| !s.owned)
        || text(p, "bim:schemaVersion") != Some("0.1.0")
        || text(p, "bim:geometryMode") != Some("parametric")
        || !text(p, "bim:elementKind").is_some_and(|k| {
            PartKind::parse(k).is_some()
                || matches!(k, "assembly" | "connection" | "level" | "grid" | "helper")
        })
    {
        return Ok(None);
    }
    let Some(link) = p.relationships.iter().find(|r| r.name == RELATIONSHIP) else {
        return Ok(None);
    };
    let kind = owned_part(p).ok_or_else(invalid)?;
    let [target] = link.targets.as_slice() else {
        return Err(invalid());
    };
    if target == &p.path {
        return Err(invalid());
    }
    let source = prims.iter().find(|s| s.path == *target).ok_or_else(|| {
        fail(
            ErrorCode::DanglingRelationship,
            "instance definition is absent",
        )
    })?;
    if owned_part(source) != Some(kind)
        || source.relationships.iter().any(|r| r.name == RELATIONSHIP)
    {
        return Err(invalid());
    }
    if p.properties.iter().any(|v| shared_property(&v.name))
        || p.relationships.iter().any(|r| shared_relationship(&r.name))
    {
        return Err(fail(
            ErrorCode::UnsupportedProperty,
            "instance shadows its shared definition",
        ));
    }
    Ok(Some(source))
}

/// Checks a creation's reference to an existing canonical owned part.
pub(super) fn validate_source<'a>(
    id: &str,
    kind: PartKind,
    prims: &'a [Prim],
) -> Result<&'a Prim, Diagnostic> {
    let path = element_path(id, ElementCategory::Part)?;
    let source = prims
        .iter()
        .find(|p| p.path == path)
        .ok_or_else(|| fail(ErrorCode::NotFound, "instance definition is absent"))?;
    if owned_part(source) != Some(kind)
        || source.relationships.iter().any(|r| r.name == RELATIONSHIP)
    {
        return Err(invalid());
    }
    Ok(source)
}

/// Persistent links of a validated stage. Definitions are canonical parts, never chains.
pub fn links(prims: &[Prim]) -> Result<Vec<InstanceLink>, Diagnostic> {
    let mut out = Vec::new();
    for p in prims {
        if let Some(source) = definition(p, prims)? {
            if out.len() >= MAX_PART_INSTANCES {
                return Err(fail(ErrorCode::LimitExceeded, "too many part instances"));
            }
            let id = |p: &Prim| {
                ElementId::parse(p.element_id.as_deref().ok_or_else(invalid)?)
                    .map_err(|_| invalid())
            };
            out.push(InstanceLink {
                instance: id(p)?,
                definition: id(source)?,
            });
        }
    }
    Ok(out)
}

/// Clones a bounded reader projection with shared inputs resolved onto each instance. This is
/// transient evaluation state; the authored USD stores the definition once and retains links.
pub fn resolve(prims: Vec<Prim>) -> Result<Vec<Prim>, Diagnostic> {
    links(&prims)?;
    // Charge expanded shared values before cloning, including nested point arrays. Unknown
    // properties are never expanded. One million values is a hard cap independent of disk size.
    let mut charge = 0usize;
    let mut bytes = 0usize;
    for p in &prims {
        if let Some(source) = definition(p, &prims)? {
            for property in source
                .properties
                .iter()
                .filter(|v| shared_property(&v.name))
            {
                if let Some(value) = &property.value {
                    let mut stack = vec![(value, 0u8)];
                    while let Some((v, depth)) = stack.pop() {
                        charge = charge.saturating_add(1);
                        bytes = bytes.saturating_add(64);
                        match v {
                            Value::Text(text) => bytes = bytes.saturating_add(text.len()),
                            Value::Dictionary(_) => return Err(invalid()),
                            _ => {}
                        }
                        if charge > 1_000_000 || depth > 16 || bytes > 64 * 1024 * 1024 {
                            return Err(fail(
                                ErrorCode::LimitExceeded,
                                "resolved instance inputs exceed the value budget",
                            ));
                        }
                        if let Value::Array(values) = v {
                            if values.len() > 1_000_000usize.saturating_sub(charge) {
                                return Err(fail(
                                    ErrorCode::LimitExceeded,
                                    "resolved instance array exceeds the value budget",
                                ));
                            }
                            stack.extend(values.iter().map(|v| (v, depth.saturating_add(1))));
                        }
                    }
                }
            }
        }
    }
    let mut out = Vec::with_capacity(prims.len());
    for p in &prims {
        let mut resolved = p.clone();
        if let Some(source) = definition(p, &prims)? {
            resolved.properties.extend(
                source
                    .properties
                    .iter()
                    .filter(|v| shared_property(&v.name))
                    .cloned(),
            );
            resolved.relationships.extend(
                source
                    .relationships
                    .iter()
                    .filter(|r| shared_relationship(&r.name))
                    .cloned(),
            );
        }
        out.push(resolved);
    }
    Ok(out)
}

pub(super) fn author(
    data: &mut sdf::Data,
    path: &str,
    source: &str,
    input: &Prepare,
    prims: &[Prim],
) -> Result<(), Diagnostic> {
    edit::relationship_targets(
        data,
        path,
        RELATIONSHIP,
        Some(vec![edit::path(&target_path(
            source,
            ElementCategory::Part,
            input,
            prims,
        )?)?]),
    )
}
