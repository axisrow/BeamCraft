// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! P11 canonical definition links and bounded materialization for model readers.
use super::*;
use openusd::sdf;

/// Persistent shared-definition relationship name.
pub const RELATIONSHIP: &str = "bim:instanceOf";

/// The command-side cap on part instances of one stage. It mirrors the model's stored cap
/// (`steel::model::instance::MAX_PART_INSTANCES`), which the steel layer reports its links
/// against, so a batch and its loaded stage fan out to the same bound.
const MAX_PART_INSTANCES: usize = 256;

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
fn owned_part(p: &Prim) -> Option<&'static str> {
    if p.sources.is_empty()
        || p.sources.iter().any(|s| !s.owned)
        || text(p, "bim:schemaVersion") != Some("0.1.0")
        || text(p, "bim:geometryMode") != Some("parametric")
    {
        return None;
    }
    disciplines::registered()
        .into_iter()
        .find_map(|discipline| discipline.part_kind(text(p, "bim:elementKind")?))
}
/// Whether a property belongs to the shared shape/operation definition.
/// Whether a property belongs to the shared shape/operation definition of a part instance
/// (P11); answered by the registered disciplines.
pub fn shared_property(name: &str) -> bool {
    disciplines::registered()
        .iter()
        .any(|discipline| discipline.shared_property(name))
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
            is_part_kind(k) || matches!(k, "assembly" | "connection" | "level" | "grid" | "helper")
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

/// Checks a creation's reference to an existing canonical owned part; the seam the instance
/// shape hook validates its definition through.
pub fn validate_source<'a>(
    id: &str,
    kind: &str,
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

/// Caps the part instances of a stage at [`MAX_PART_INSTANCES`]; the platform half of the cap
/// the steel layer reports its links against. The instance identities themselves are a
/// discipline concern and stay with the links.
pub(super) fn cap(prims: &[Prim]) -> Result<(), Diagnostic> {
    let mut count = 0usize;
    for p in prims {
        if definition(p, prims)?.is_some() {
            if count >= MAX_PART_INSTANCES {
                return Err(fail(ErrorCode::LimitExceeded, "too many part instances"));
            }
            count += 1;
        }
    }
    Ok(())
}

/// Clones a bounded reader projection with shared inputs resolved onto each instance. This is
/// transient evaluation state; the authored USD stores the definition once and retains links.
pub fn resolve(prims: Vec<Prim>) -> Result<Vec<Prim>, Diagnostic> {
    cap(&prims)?;
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

/// Authors the `bim:instanceOf` relationship of one instance towards its canonical part; the
/// seam the instance shape hook writes through.
pub fn author(
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
