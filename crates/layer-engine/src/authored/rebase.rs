// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! C03.9: isolated, closed-schema transfer of independent authored opinions.
//!
//! This is a materializer, not an independence proof. The engine must check every intervening
//! command, including reads and reverted edits, before calling it. Host authority, pinned history,
//! kernel re-evaluation, publication and recovery remain the caller's responsibilities.

use super::{EditTargets, edit, schema_identity};
use crate::*;
use beamcraft_model::changeset::{Access, Dependency, DependencyGroup as G, DependencyTarget as T};
use beamcraft_model::id::ElementId;
use beamcraft_model::limits::MAX_FOOTPRINT_ENTRIES;
use beamcraft_model::prim::DefinitionCategory;
use openusd::sdf::{self, AbstractData};
use std::collections::BTreeMap;

type Writes = BTreeSet<(T, G)>;

fn conflict() -> Diagnostic {
    fail(
        ErrorCode::DependencyConflict,
        "candidate overlaps an accepted opinion",
    )
}
fn unsupported_delta() -> Diagnostic {
    fail(
        ErrorCode::UnsupportedProperty,
        "rebase delta is outside the closed authored schema",
    )
}

fn scaffold(path: &str) -> bool {
    looks(path)
        || matches!(
            path,
            "/" | "/World"
                | "/World/Steel"
                | "/World/Steel/Parts"
                | "/World/Steel/Assemblies"
                | "/World/Steel/Connections"
                | "/World/Levels"
                | "/World/Definitions"
                | "/World/Definitions/Materials"
                | "/World/Definitions/Profiles"
                | "/World/Definitions/Geometry"
        )
}

/// The fixed U08 appearance scope: engine-authored look content, never a user opinion.
fn looks(path: &str) -> bool {
    path == "/World/Looks" || path.starts_with("/World/Looks/")
}

/// The immutable geometry asset (U11) a spec path belongs to, by its content-addressed label.
fn geometry_asset(path: &str) -> Option<ContentDigest> {
    path.strip_prefix(super::edit::GEOMETRY_ASSETS)
        .and_then(|rest| rest.strip_prefix("/GEO_"))
        .filter(|label| label.len() == 64)
        .and_then(|label| label.parse().ok())
}

fn property_group(name: &str) -> Option<G> {
    Some(match name {
        "bim:id" | "bim:elementKind" | "bim:geometryMode" | "bim:schemaVersion"
        | "bim:createdBy" | "bim:revision" => G::Lifecycle,
        // The U08 appearance travels with the class: one presentation write, one group.
        "bim:name" | "steel:class" | "primvars:displayColor" | "material:binding" => G::Label,
        "steel:numbering:series" | "steel:numbering:number" | "steel:numbering:basis" => G::Mark,
        "xformOp:translate" | "xformOp:orient" | "xformOpOrder" => G::Placement,
        "bim:material" => G::MaterialRef,
        "bim:partOf" | "bim:level" | "bim:connects" | "bim:instanceOf" => G::Relationship,
        "bim:elevationMm" => G::ShapeInputs,
        _ => return super::disciplines::property_group(name),
    })
}

fn definition_property(name: &str) -> bool {
    matches!(
        name,
        "bim:contentDigest"
            | "bim:schemaVersion"
            | "bim:units"
            | "bim:definitionKind"
            | "bim:densityKgM3"
            | "bim:materialGrade"
            | "bim:materialStandard"
    ) || super::disciplines::registered()
        .iter()
        .any(|discipline| discipline.definition_property(name))
}

fn geometry_property(name: &str) -> bool {
    matches!(
        name,
        "points"
            | "purpose"
            | "faceVertexCounts"
            | "faceVertexIndices"
            | "subdivisionScheme"
            | "orientation"
            | "extent"
            | "normals"
            | "bim:shapeKey"
            | "bim:volumeMm3"
            | "bim:volumeStatus"
            | "bim:massKg"
            | "bim:massStatus"
            | "bim:shapeSignature"
            | "bim:massSignature"
            | "bim:quantityAlgorithm"
    )
}

// Spec paths and field names are closed here. Inert unknown opinions can remain in any input,
// but an unknown delta cannot be copied under the authority of a known group's write.
fn scope(path: &str, field: &str) -> Result<(T, G), Diagnostic> {
    let (prim, property) = path
        .split_once('.')
        .map_or((path, None), |(p, a)| (p, Some(a)));
    if let Some((_, digest)) = DefinitionCategory::parse_path(prim) {
        if property.is_some_and(|p| !definition_property(p)) {
            return Err(unsupported_delta());
        }
        return Ok((T::Definition(digest), G::Relationship));
    }
    // U11: a shared immutable geometry asset is a derived write of its content identity.
    if let Some(digest) = geometry_asset(prim) {
        if property.is_some_and(|p| !geometry_property(p)) {
            return Err(unsupported_delta());
        }
        return Ok((T::Definition(digest), G::Derived));
    }
    let (element, geometry) = prim
        .strip_suffix("/Geometry")
        .map_or((prim, false), |p| (p, true));
    let indexed = crate::index::ElementId::from_path(element).map_err(|_| unsupported_delta())?;
    // Reference wrappers and their foreign-facing descendants retain the R01-specific writer.
    if element.starts_with("/World/References/") {
        return Err(fail(
            ErrorCode::ForeignReadOnly,
            "reference delta requires its owned-wrapper writer",
        ));
    }
    let id = ElementId::parse(&indexed.to_string()).map_err(|_| corrupt())?;
    let group = if geometry {
        if property.is_some_and(|p| !geometry_property(p)) {
            return Err(unsupported_delta());
        }
        G::Derived
    } else if let Some(property) = property {
        property_group(property).ok_or_else(unsupported_delta)?
    } else if field == "displayName" {
        G::Label
    } else {
        G::Lifecycle
    };
    Ok((T::Element(id), group))
}

fn permitted(writes: &Writes, entry: (T, G)) -> bool {
    writes.contains(&entry)
        // Existing A09 approval declares label writes; do not change that historical evidence.
        || (entry.1 == G::Mark && writes.contains(&(entry.0, G::Label)))
}

fn require(writes: &Writes, path: &str, field: &str) -> Result<(), Diagnostic> {
    let known = if path.contains('.') {
        matches!(
            field,
            "typeName" | "default" | "custom" | "variability" | "targetPaths"
        ) || (field == "interpolation"
            && (path.ends_with("/Geometry.normals") || geometry_asset_path(path)))
    } else {
        // `apiSchemas` is the prim metadata a U08 classed part applies at create; U11: an
        // element's derived prim is an instanceable reference to the shared geometry asset.
        matches!(field, "specifier" | "typeName" | "kind" | "displayName" | "apiSchemas")
            || ((field == "instanceable" || field == "references") && path.ends_with("/Geometry"))
    };
    if !known {
        return Err(unsupported_delta());
    }
    if !permitted(writes, scope(path, field)?) {
        return Err(fail(
            ErrorCode::IntegrityError,
            "source delta exceeds its verified writes",
        ));
    }
    Ok(())
}

/// Whether a property path sits on an immutable geometry asset (U11).
fn geometry_asset_path(path: &str) -> bool {
    path.split_once('.')
        .and_then(|(prim, _)| geometry_asset(prim))
        .is_some()
}

fn fields(data: &sdf::Data, path: &sdf::Path) -> Result<BTreeMap<String, sdf::Value>, Diagnostic> {
    data.list_fields(path)
        .unwrap_or_default()
        .into_iter()
        .map(|key| {
            let value = data
                .get_field(path, &key)
                .map_err(|_| corrupt())?
                .into_owned();
            Ok((key, value))
        })
        .collect()
}

fn names(value: Option<&sdf::Value>) -> Result<Vec<String>, Diagnostic> {
    match value {
        None => Ok(Vec::new()),
        Some(sdf::Value::TokenVec(values)) => Ok(values.iter().map(ToString::to_string).collect()),
        _ => Err(unsupported_delta()),
    }
}

fn child_path(parent: &str, key: &str, child: &str) -> String {
    if key == "propertyChildren" {
        format!("{parent}.{child}")
    } else {
        format!("{}/{child}", parent.trim_end_matches('/'))
    }
}

fn check_accepted(
    base: &OpenRequest,
    accepted: &OpenRequest,
    writes: &Writes,
    control: &Control,
) -> Result<(), Diagnostic> {
    // A complete semantic group is atomic even when two clients changed different properties.
    // This final-state check supplements, and cannot replace, the engine's history comparison.
    for asset in &base.assets {
        if !base.owned_layers.contains(&asset.name) {
            continue;
        }
        let other = accepted
            .assets
            .iter()
            .find(|a| a.name == asset.name)
            .ok_or_else(conflict)?;
        if asset.digest == other.digest {
            continue;
        }
        let (old, new) = (edit::data(asset)?, edit::data(other)?);
        let paths: BTreeSet<_> = old
            .spec_paths()
            .into_iter()
            .chain(new.spec_paths())
            .collect();
        for path in paths {
            control.check()?;
            let (before, after) = (fields(&old, &path)?, fields(&new, &path)?);
            for key in before.keys().chain(after.keys()) {
                if before.get(key) == after.get(key) {
                    continue;
                }
                if key == "propertyChildren" || key == "primChildren" {
                    let a = names(before.get(key))?;
                    let b = names(after.get(key))?;
                    for child in a.iter().chain(&b) {
                        if a.contains(child) != b.contains(child)
                            && scope(&child_path(&path.to_string(), key, child), "default")
                                .is_ok_and(|entry| permitted(writes, entry))
                        {
                            return Err(conflict());
                        }
                    }
                } else if scope(&path.to_string(), key).is_ok_and(|entry| permitted(writes, entry))
                {
                    return Err(conflict());
                }
            }
        }
    }
    Ok(())
}

fn merge_names(
    path: &sdf::Path,
    key: &str,
    old: Option<&sdf::Value>,
    source: Option<&sdf::Value>,
    accepted: &mut sdf::Data,
    writes: &Writes,
) -> Result<(), Diagnostic> {
    let old = names(old)?;
    let source = names(source)?;
    let mut current = names(
        accepted
            .try_field(path, key)
            .map_err(|_| corrupt())?
            .as_deref(),
    )?;
    for child in old.iter().chain(&source) {
        if old.contains(child) == source.contains(child) {
            continue;
        }
        let target = child_path(&path.to_string(), key, child);
        if !scaffold(&target) {
            let field = if key == "propertyChildren" {
                "default"
            } else {
                "specifier"
            };
            if !permitted(writes, scope(&target, field)?) {
                return Err(fail(
                    ErrorCode::IntegrityError,
                    "source child delta exceeds its writes",
                ));
            }
        }
        if source.contains(child) {
            if !current.contains(child) {
                current.push(child.clone());
            }
        } else {
            current.retain(|name| name != child);
        }
    }
    accepted.set_field(
        path,
        key,
        sdf::Value::TokenVec(current.into_iter().map(Into::into).collect()),
    );
    Ok(())
}

fn merge_layer(
    old: &sdf::Data,
    source: &sdf::Data,
    accepted: &mut sdf::Data,
    writes: &Writes,
    control: &Control,
) -> Result<bool, Diagnostic> {
    let paths: BTreeSet<_> = old
        .spec_paths()
        .into_iter()
        .chain(source.spec_paths())
        .collect();
    let mut changed = false;
    for path in paths {
        control.check()?;
        let before = fields(old, &path)?;
        let after = fields(source, &path)?;
        if before == after && old.spec_type(&path) == source.spec_type(&path) {
            continue;
        }
        changed = true;
        let path_text = path.to_string();
        // The fixed appearance scope is engine-authored content both sides carry identically:
        // equal content is no conflict, and anything else is a tampered look and refuses.
        // It is never a user opinion, so it is copied wholesale, never permission-merged.
        if looks(&path_text) {
            let accepted_type = accepted.spec_type(&path);
            let same = accepted_type == source.spec_type(&path)
                && accepted.has_spec(&path)
                && fields(accepted, &path)? == after;
            if same {
                continue;
            }
            if accepted_type.is_some() {
                return Err(conflict());
            }
            if let Some(ty) = source.spec_type(&path) {
                accepted.create_spec(path.clone(), ty);
            }
            for (key, value) in &after {
                accepted.set_field(&path, key, value.clone());
            }
            continue;
        }
        let container = scaffold(&path_text);
        let old_type = old.spec_type(&path);
        let source_type = source.spec_type(&path);
        if old_type != source_type {
            if !container {
                // A new property is a group write; a new element/geometry is lifecycle/derived.
                if !permitted(writes, scope(&path_text, "specifier")?) {
                    return Err(fail(
                        ErrorCode::IntegrityError,
                        "source spec delta exceeds its writes",
                    ));
                }
            }
            if old_type.is_some() && source_type.is_some() {
                return Err(unsupported_delta());
            }
            if let Some(ty) = source_type {
                if accepted.has_spec(&path) && !container {
                    return Err(conflict());
                }
                if !accepted.has_spec(&path) {
                    accepted.create_spec(path.clone(), ty);
                }
            } else {
                if container || fields(accepted, &path)? != before {
                    return Err(conflict());
                }
                for key in before.keys() {
                    if key != "propertyChildren" && key != "primChildren" {
                        require(writes, &path_text, key)?;
                    }
                }
                accepted.erase_spec(&path);
                continue;
            }
        }
        let keys: BTreeSet<_> = before.keys().chain(after.keys()).collect();
        for key in keys {
            let (a, b) = (before.get(key), after.get(key));
            if a == b {
                continue;
            }
            if key == "propertyChildren" || key == "primChildren" {
                merge_names(&path, key, a, b, accepted, writes)?;
                continue;
            }
            let current = accepted.try_field(&path, key).map_err(|_| corrupt())?;
            if container {
                if old_type.is_some() || !matches!(key.as_str(), "specifier" | "typeName" | "kind")
                {
                    return Err(unsupported_delta());
                }
                if current.as_deref().is_some_and(|v| Some(v) != b) {
                    return Err(conflict());
                }
            } else {
                require(writes, &path_text, key)?;
                if current.as_deref() != a {
                    return Err(conflict());
                }
            }
            match b {
                Some(value) => accepted.set_field(&path, key, value.clone()),
                None => accepted.erase_field(&path, key),
            }
        }
    }
    Ok(changed)
}

/// Applies the source's closed-schema delta over an accepted snapshot, entirely in memory.
/// Every source delta must fit the supplied, nonempty write set (maximum 4096 entries). Unknown
/// changes, overlapping fields and changes outside the two edit targets refuse. Parent child
/// lists merge by membership, retaining independently created accepted elements and definitions.
/// Untouched assets retain exact bytes; foreign assets are never rewritten. All three inputs
/// and the complete output pass structural/index validation. This does not run the kernel or
/// prove independence from command reads/history; callers must do both before a durable commit.
pub fn prepare(
    base: &OpenRequest,
    source: &OpenRequest,
    accepted: &OpenRequest,
    dependencies: &[Dependency],
    targets: &EditTargets,
    control: &Control,
) -> Result<OpenRequest, Diagnostic> {
    control.check()?;
    if dependencies.is_empty() || dependencies.len() > MAX_FOOTPRINT_ENTRIES {
        return Err(limit());
    }
    let writes: Writes = dependencies
        .iter()
        .filter(|d| d.access == Access::Write)
        .map(|d| (d.target, d.group))
        .collect();
    if writes.is_empty() {
        return Err(unsupported_delta());
    }
    let schema = schema_identity()?;
    for request in [base, source, accepted] {
        if request.identity.project != base.identity.project || request.root != base.root {
            return Err(fail(
                ErrorCode::InvalidArgument,
                "rebase snapshot context differs",
            ));
        }
        edit::targets(request, targets)?;
        let mut reader = Reader::default();
        let (opened, _) = reader.open_validated(request.clone(), schema, control)?;
        let index = reader.element_index(&opened.context, control)?;
        reader.verify_index(&index, control)?;
        for &(target, _) in &writes {
            let path: Result<&str, _> = match target {
                T::Element(id) => index
                    .lookup(&id.to_string())
                    .map(|entry| entry.path.as_str()),
                T::Definition(digest) => index.definition(&digest).map(|entry| entry.path.as_str()),
                // The schema scope is authored content like any element folder; a rebase of a
                // delta under it is refused earlier as an unsupported property.
                T::AttrSchema => Ok(super::ATTR_SCHEMA_PATH),
            };
            match path {
                Ok(path) => {
                    super::validate::owned(&reader.pinned(&opened.context)?.prims, path, true)?
                }
                Err(error) if error.code == ErrorCode::NotFound => {}
                Err(error) => return Err(error),
            }
        }
    }
    if base.owned_layers != source.owned_layers || base.assets.len() != source.assets.len() {
        return Err(unsupported_delta());
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        check_accepted(base, accepted, &writes, control)?;
        let mut candidate = accepted.clone();
        for old in &base.assets {
            let source_asset = source
                .assets
                .iter()
                .find(|a| a.name == old.name)
                .ok_or_else(unsupported_delta)?;
            if source_asset.digest == old.digest {
                continue;
            }
            if old.name != targets.authored && old.name != targets.derived {
                return Err(unsupported_delta());
            }
            let target = candidate
                .assets
                .iter_mut()
                .find(|a| a.name == old.name)
                .ok_or_else(unsupported_delta)?;
            let mut merged = edit::data(target)?;
            if merge_layer(
                &edit::data(old)?,
                &edit::data(source_asset)?,
                &mut merged,
                &writes,
                control,
            )? {
                let bytes = edit::serialize(&merged, accepted.budgets.layer_bytes)?;
                *target = Asset {
                    name: target.name.clone(),
                    digest: Sha256::digest(&bytes).map_err(|_| limit())?,
                    bytes,
                };
            }
        }
        candidate.manifest = candidate.manifest_digest()?;
        let mut reader = Reader::default();
        let (opened, _) = reader.open_validated(candidate.clone(), schema, control)?;
        let index = reader.element_index(&opened.context, control)?;
        reader.verify_index(&index, control)?;
        Ok(candidate)
    }))
    .map_err(|_| {
        fail(
            ErrorCode::BackendUnavailable,
            "backend unwound while materializing rebase",
        )
    })??;
    control.check()?;
    Ok(result)
}
