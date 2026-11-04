// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! C03.9: isolated, closed-schema transfer of independent authored opinions.
//!
//! This is a materializer, not an independence proof. The engine must check every intervening
//! command, including reads and reverted edits, before calling it. Host authority, pinned history,
//! kernel re-evaluation, publication and recovery remain the caller's responsibilities.

use super::{EditTargets, edit, element_layer_id, schema_identity};
use crate::*;
use beamcraft_model::changeset::{Access, Dependency, DependencyGroup as G, DependencyTarget as T};
use beamcraft_model::id::ElementId;
use beamcraft_model::limits::MAX_FOOTPRINT_ENTRIES;
use beamcraft_model::prim::DefinitionCategory;
use openusd::sdf::{self, AbstractData};
use std::collections::BTreeMap;

type Writes = BTreeSet<(T, G)>;

/// Rebuilds the derived stack's sublayer list from the candidate asset set (U15): the stack is
/// derived state, a sorted set of the referenced asset layer (when the manifest carries it)
/// plus exactly the element files.
fn reconcile_element_stack(
    candidate: &mut OpenRequest,
    targets: &EditTargets,
) -> Result<(), Diagnostic> {
    let mut entries: Vec<String> = candidate
        .assets
        .iter()
        .filter(|a| element_layer_id(&a.name).is_some())
        .map(|a| a.name.clone())
        .collect();
    if candidate
        .assets
        .iter()
        .any(|a| a.name == super::GEOMETRY_ASSET_LAYER)
    {
        entries.push(super::GEOMETRY_ASSET_LAYER.to_owned());
    }
    entries.sort();
    entries.dedup();
    let asset = candidate
        .assets
        .iter_mut()
        .find(|a| a.name == targets.derived)
        .ok_or_else(corrupt)?;
    let mut derived = edit::data(asset)?;
    let root = sdf::Path::abs_root();
    let current = match derived
        .try_field(&root, "subLayers")
        .map_err(|_| corrupt())?
    {
        Some(value) => match value.as_ref() {
            sdf::Value::StringVec(names) => Some(names.clone()),
            _ => {
                return Err(fail(
                    ErrorCode::InvalidArgument,
                    "the derived stack lists non-token entries",
                ));
            }
        },
        None => None,
    };
    let changed = match (&current, entries.is_empty()) {
        (None, true) => false,
        (Some(names), false) => names != &entries,
        _ => true,
    };
    if !changed {
        return Ok(());
    }
    if entries.is_empty() {
        derived.erase_field(&root, "subLayers");
    } else {
        derived.set_field(&root, "subLayers", sdf::Value::StringVec(entries));
    }
    let bytes = edit::serialize(&derived, candidate.budgets.layer_bytes)?;
    *asset = Asset {
        name: asset.name.clone(),
        digest: Sha256::digest(&bytes).map_err(|_| limit())?,
        bytes,
    };
    Ok(())
}

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
        matches!(
            field,
            "specifier" | "typeName" | "kind" | "displayName" | "apiSchemas"
        ) || ((field == "instanceable" || field == "references") && path.ends_with("/Geometry"))
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
        let other = match accepted.assets.iter().find(|a| a.name == asset.name) {
            Some(other) => other,
            None => {
                // The accepted head removed a whole layer. Roots never vanish; an element
                // file is an upstream element delete (U15) and conflicts only where the
                // local write set covers content the deletion removes.
                if element_layer_id(&asset.name).is_none() {
                    return Err(conflict());
                }
                let removed = edit::data(asset)?;
                for path in removed.spec_paths() {
                    control.check()?;
                    let before = fields(&removed, &path)?;
                    for key in before.keys() {
                        if key == "propertyChildren" || key == "primChildren" {
                            for child in names(before.get(key))? {
                                let child_path = child_path(&path.to_string(), key, &child);
                                if scope(&child_path, "default")
                                    .is_ok_and(|entry| permitted(writes, entry))
                                {
                                    return Err(conflict());
                                }
                            }
                        } else if scope(&path.to_string(), key)
                            .is_ok_and(|entry| permitted(writes, entry))
                        {
                            return Err(conflict());
                        }
                    }
                }
                continue;
            }
        };
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
        // The pseudo-root of an edit layer carries engine bookkeeping (units, schema pin, and
        // since U15 the element-file stack): the candidate keeps the accepted side of it, and
        // only the root prim children merge — a workspace may author its first element, and
        // the writer lists prims from that children field, so a skipped root drops them.
        if path.to_string() == "/" {
            let before = names(
                accepted
                    .try_field(&path, "primChildren")
                    .map_err(|_| corrupt())?
                    .as_deref(),
            )?;
            merge_names(
                &path,
                "primChildren",
                old.try_field(&path, "primChildren")
                    .map_err(|_| corrupt())?
                    .as_deref(),
                source
                    .try_field(&path, "primChildren")
                    .map_err(|_| corrupt())?
                    .as_deref(),
                accepted,
                writes,
            )?;
            changed |= before
                != names(
                    accepted
                        .try_field(&path, "primChildren")
                        .map_err(|_| corrupt())?
                        .as_deref(),
                )?;
            continue;
        }
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
    // The layer set of a delta may change by whole element files (U15) and by the referenced
    // geometry-asset layer: any other layer that the base or the source holds alone is outside
    // this API's closed transfer form.
    let transferable =
        |name: &str| element_layer_id(name).is_some() || name == super::GEOMETRY_ASSET_LAYER;
    for asset in &base.assets {
        let in_source = source.assets.iter().any(|a| a.name == asset.name);
        if !in_source && !transferable(&asset.name) {
            return Err(unsupported_delta());
        }
    }
    for asset in &source.assets {
        let in_base = base.assets.iter().any(|a| a.name == asset.name);
        if !in_base && !transferable(&asset.name) {
            return Err(unsupported_delta());
        }
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        check_accepted(base, accepted, &writes, control)?;
        let mut candidate = accepted.clone();
        // Element files the source removed: whole element deletes (U15), dropped from the
        // candidate when the accepted head still carries them.
        let mut dropped: Vec<String> = Vec::new();
        for old in &base.assets {
            let element_file = element_layer_id(&old.name).is_some();
            let source_asset = source.assets.iter().find(|a| a.name == old.name);
            let Some(source_asset) = source_asset else {
                if !element_file {
                    return Err(unsupported_delta());
                }
                dropped.push(old.name.clone());
                continue;
            };
            if source_asset.digest == old.digest {
                continue;
            }
            if old.name != targets.authored
                && old.name != targets.derived
                && !transferable(&old.name)
            {
                return Err(unsupported_delta());
            }
            let target = match candidate.assets.iter_mut().find(|a| a.name == old.name) {
                Some(target) => target,
                // The accepted head removed the element file too: the deletion carries.
                None if element_file => {
                    dropped.retain(|n| n != &old.name);
                    continue;
                }
                None => return Err(unsupported_delta()),
            };
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
        // Layers the source added: locally created element files (U15) and the referenced
        // geometry-asset layer ride with the batch; the authored prim arrives through the
        // authored-layer merge.
        for added in &source.assets {
            if base.assets.iter().any(|a| a.name == added.name) {
                continue;
            }
            if !transferable(&added.name) {
                return Err(unsupported_delta());
            }
            // The accepted head may carry a layer of the same name already (reachable: both
            // branches create their first meshed element and both add the asset layer).
            // An identical copy adds nothing. A diverging element file is the same element
            // authored two ways with no common ancestor: a conflict. A diverging asset layer
            // unions: the bodies are content-addressed, so a body the accepted head lacks is
            // a new definition — it must be named by a definition write, and one outside the
            // verified writes is a conflict between clients, not a silent drop of the other
            // client's body.
            let target = match candidate.assets.iter_mut().find(|a| a.name == added.name) {
                Some(target) if target.digest == added.digest => continue,
                // A diverging element file is the same element authored two ways with no
                // common ancestor: a conflict.
                Some(_) if element_layer_id(&added.name).is_some() => return Err(conflict()),
                Some(target) => {
                    let text = String::from_utf8_lossy(&added.bytes).into_owned();
                    for digest in text.match_indices("GEO_").filter_map(|(at, _)| {
                        text.get(at + 4..at + 68)
                            .filter(|label| label.len() == 64)
                            .and_then(|label| label.parse::<ContentDigest>().ok())
                    }) {
                        if !permitted(&writes, (T::Definition(digest), G::Derived)) {
                            return Err(conflict());
                        }
                    }
                    target
                }
                None => {
                    // A new or rewritten body asset carries the U11 content identity in the
                    // verified writes: every GEO_ body of the added layer must be named by a
                    // definition write.
                    if added.name == super::GEOMETRY_ASSET_LAYER {
                        let text = String::from_utf8_lossy(&added.bytes).into_owned();
                        for digest in text.match_indices("GEO_").filter_map(|(at, _)| {
                            text.get(at + 4..at + 68)
                                .filter(|label| label.len() == 64)
                                .and_then(|label| label.parse::<ContentDigest>().ok())
                        }) {
                            require(
                                &writes,
                                &format!("{}/GEO_{digest}", super::edit::GEOMETRY_ASSETS),
                                "specifier",
                            )?;
                        }
                    }
                    candidate.assets.push(added.clone());
                    candidate.owned_layers.insert(added.name.clone());
                    continue;
                }
            };
            if added.name == super::GEOMETRY_ASSET_LAYER {
                // Union: an empty base makes every body of the added layer a new spec gated
                // by its definition write, and never erases the accepted side's bodies.
                let empty = sdf::Data::new();
                let mut merged = edit::data(target)?;
                if merge_layer(&empty, &edit::data(added)?, &mut merged, &writes, control)? {
                    let bytes = edit::serialize(&merged, accepted.budgets.layer_bytes)?;
                    *target = Asset {
                        name: target.name.clone(),
                        digest: Sha256::digest(&bytes).map_err(|_| limit())?,
                        bytes,
                    };
                }
            }
        }
        candidate.assets.retain(|a| !dropped.contains(&a.name));
        reconcile_element_stack(&mut candidate, targets)?;
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
