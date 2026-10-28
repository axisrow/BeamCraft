// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

use crate::{
    preflight::{self, charge},
    *,
};
use beamcraft_model::{digest::Sha256, error::ErrorCode};
use openusd::{
    ar,
    sdf::{self, AbstractData},
    usd,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
    time::SystemTime,
};

pub(crate) struct Candidate {
    pub identity: Identity,
    pub prims: Vec<Prim>,
    pub budgets: Budgets,
    pub meters_per_unit: f64,
    pub up_axis: String,
    pub metadata: BTreeMap<String, Value>,
    /// Every supplied layer and whether the host selected it as owned.
    pub layers: BTreeMap<String, bool>,
}

/// An entirely virtual byte resolver: no default resolver delegation, file or URL opens.
struct Resolver {
    assets: BTreeMap<String, Arc<[u8]>>,
    identity: String,
}
struct MemoryAsset(io::Cursor<Arc<[u8]>>);
impl io::Read for MemoryAsset {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        io::Read::read(&mut self.0, out)
    }
}
impl io::Seek for MemoryAsset {
    fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
        io::Seek::seek(&mut self.0, pos)
    }
}
impl ar::Asset for MemoryAsset {
    fn size(&self) -> io::Result<u64> {
        Ok(self.0.get_ref().len() as u64)
    }
    fn into_buffer(self: Box<Self>) -> io::Result<ar::AssetBuffer> {
        Ok(self.0.into_inner().into())
    }
}
impl ar::Resolver for Resolver {
    fn create_identifier(&self, asset: &str, anchor: Option<&ar::ResolvedPath>) -> String {
        let parent = anchor.and_then(|p| p.to_str()).unwrap_or("");
        // Failing identifiers cannot collide with any allowed manifest name.
        preflight::anchored(asset, parent).unwrap_or_else(|_| "!denied".to_owned())
    }
    fn resolve(&self, asset: &str) -> Option<ar::ResolvedPath> {
        self.assets
            .contains_key(asset)
            .then(|| ar::ResolvedPath::new(asset))
    }
    fn resolve_for_new_asset(&self, _: &str) -> Option<ar::ResolvedPath> {
        None
    }
    fn open_asset(&self, resolved: &ar::ResolvedPath) -> io::Result<Box<dyn ar::Asset>> {
        let bytes = resolved
            .to_str()
            .and_then(|s| self.assets.get(s))
            .ok_or_else(|| io::Error::from(io::ErrorKind::PermissionDenied))?;
        Ok(Box::new(MemoryAsset(io::Cursor::new(bytes.clone()))))
    }
    fn get_modification_timestamp(&self, _: &str, _: &ar::ResolvedPath) -> Option<SystemTime> {
        None
    }
    fn identity(&self) -> String {
        self.identity.clone()
    }
}

/// Stack reserved for the upstream parser and composer. The text parser recurses once per escape
/// sequence in a string literal, so a long escaped string (a baked preview, a serialised blob)
/// needs far more than a default thread stack, in unoptimised builds above all. Reserving address
/// space costs nothing until it is used, and the string ceiling bounds the recursion.
const PARSER_STACK_BYTES: usize = 256 * 1024 * 1024;

/// Runs one parse-and-compose call on a short-lived thread with [`PARSER_STACK_BYTES`] of stack.
fn on_parser_stack<T: Send>(
    work: impl FnOnce() -> Result<T, Diagnostic> + Send,
) -> Result<T, Diagnostic> {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(PARSER_STACK_BYTES)
            .spawn_scoped(scope, work)
            .map_err(|_| {
                fail(
                    ErrorCode::BackendUnavailable,
                    "parser thread could not be started",
                )
            })?
            .join()
            .map_err(|_| {
                fail(
                    ErrorCode::BackendUnavailable,
                    "backend unwound while preparing read view",
                )
            })?
    })
}

pub(crate) fn open(request: OpenRequest, control: &Control) -> Result<Candidate, Diagnostic> {
    on_parser_stack(|| open_on_stack(request, control))
}

fn open_on_stack(request: OpenRequest, control: &Control) -> Result<Candidate, Diagnostic> {
    let b = request.budgets;
    b.validate()?;
    if request.backend != BACKEND {
        return Err(fail(
            ErrorCode::UnsupportedVersion,
            "backend pin or snapshot schema mismatch",
        ));
    }
    if !uuid7(&request.identity.project) || !uuid7(&request.identity.workspace) {
        return Err(fail(
            ErrorCode::InvalidId,
            "invalid project or workspace identity",
        ));
    }
    if request.manifest_digest()? != request.manifest {
        return Err(corrupt());
    }
    let mut assets = BTreeMap::new();
    let mut bytes_total = 0usize;
    for asset in &request.assets {
        control.check()?;
        if !preflight::name(&asset.name) {
            return Err(fail(
                ErrorCode::PermissionDenied,
                "asset identifier is not contained",
            ));
        }
        if asset.bytes.len() > b.layer_bytes {
            return Err(limit());
        }
        charge(&mut bytes_total, asset.bytes.len(), b.total_bytes)?;
        if Sha256::digest(&asset.bytes).map_err(|_| limit())? != asset.digest {
            return Err(corrupt());
        }
        if assets.insert(asset.name.clone(), asset).is_some() {
            return Err(corrupt());
        }
    }
    // Verify ALL integrity and ownership declarations before the first parser call.
    if !assets.contains_key(&request.root)
        || request.owned_layers.iter().any(|s| !assets.contains_key(s))
    {
        return Err(corrupt());
    }
    let mut parsed = BTreeMap::new();
    let mut graph = BTreeMap::new();
    let mut counters = Counters::default();
    for (name, asset) in &assets {
        let foreign = displayed(name, &request.owned_layers);
        let (data, mut dependencies) = load_asset(name, asset, b, control, &mut counters, foreign)?;
        if foreign {
            // An arc to a layer that was not supplied is inert: nothing is ever resolved.
            dependencies.retain(|dependency| assets.contains_key(dependency));
        }
        graph.insert((*name).clone(), dependencies);
        parsed.insert((*name).clone(), data);
    }
    let mut work = 0usize;
    for name in graph.keys() {
        walk(name, &graph, &mut BTreeSet::new(), b, &mut work, control)?;
    }
    let root = parsed.get(&request.root).ok_or_else(corrupt)?;
    let root_path = sdf::Path::abs_root();
    let meters_per_unit = match root
        .get_field(&root_path, "metersPerUnit")
        .map_err(|_| corrupt())?
        .as_ref()
    {
        sdf::Value::Double(n) => *n,
        _ => return Err(corrupt()),
    };
    let up_axis = match root
        .get_field(&root_path, "upAxis")
        .map_err(|_| corrupt())?
        .as_ref()
    {
        sdf::Value::Token(s) => s.to_string(),
        _ => return Err(corrupt()),
    };
    if meters_per_unit != 0.001 || up_axis != "Z" {
        return Err(fail(
            ErrorCode::InvalidArgument,
            "stage units must be millimetres and Z up",
        ));
    }
    let mut metadata = BTreeMap::new();
    let mut metadata_bytes = 0usize;
    for key in root.list_fields(&root_path).ok_or_else(corrupt)? {
        if matches!(
            key.as_str(),
            "primChildren" | "subLayers" | "subLayerOffsets"
        ) {
            continue;
        }
        let value = typed(
            root.get_field(&root_path, &key)
                .map_err(|_| corrupt())?
                .as_ref(),
            b,
        )?;
        charge(
            &mut metadata_bytes,
            key.len().saturating_add(64),
            b.snapshot_bytes,
        )?;
        value_size(&value, &mut metadata_bytes, b.snapshot_bytes)?;
        metadata.insert(key, value);
    }
    let resolver = Resolver {
        identity: request.manifest.to_string(),
        assets: assets
            .iter()
            .map(|(n, a)| (n.clone(), Arc::from(a.bytes.as_slice())))
            .collect(),
    };
    control.check()?;
    let stage = usd::Stage::builder()
        .resolver(resolver)
        .open(&request.root)
        .map_err(|_| {
            fail(
                ErrorCode::BackendUnavailable,
                "backend could not compose read view",
            )
        })?;
    let mut paths = Vec::new();
    let mut excess = false;
    stage
        .traverse(usd::PrimPredicate::ALL, |p| {
            if paths.len() < b.prims {
                paths.push(p.to_string());
            } else {
                excess = true;
            }
        })
        .map_err(|_| corrupt())?;
    if excess {
        return Err(limit());
    }
    if stage
        .composition_errors()
        .iter()
        .any(|error| !tolerated_composition_error(error, &request.owned_layers))
    {
        return Err(fail(
            ErrorCode::DependencyUnavailable,
            "composition dependency is unavailable",
        ));
    }
    paths.sort();
    let mut prims = Vec::new();
    let mut output_bytes = metadata_bytes;
    let mut output_values = 0usize;
    for path in paths {
        control.check()?;
        let prim = stage.prim(path.as_str()).map_err(|_| corrupt())?;
        let mut sources = Vec::new();
        for site in prim.prim_stack().map_err(|_| corrupt())? {
            let asset = assets.get(&site.layer).ok_or_else(|| {
                fail(
                    ErrorCode::DependencyUnavailable,
                    "unverified composition provenance",
                )
            })?;
            let source_path = site.path.to_string();
            charge(
                &mut output_bytes,
                site.layer
                    .len()
                    .saturating_add(source_path.len())
                    .saturating_add(128),
                b.snapshot_bytes,
            )?;
            sources.push(Provenance {
                layer: site.layer.clone(),
                path: source_path,
                digest: asset.digest,
                owned: request.owned_layers.contains(&site.layer),
            });
        }
        if sources.is_empty() {
            return Err(fail(
                ErrorCode::DependencyUnavailable,
                "missing composition provenance",
            ));
        }
        // The arc holder of a reference wrapper has an owned site too, but never a property of its own.
        let foreign_prim = sources
            .iter()
            .any(|source| !source.owned && is_external(&source.layer));
        let mut properties = Vec::new();
        let mut relationships = Vec::new();
        for property in prim.property_names().map_err(|_| corrupt())? {
            control.check()?;
            charge(&mut output_values, 1, b.values)?;
            let full = format!("{path}.{property}");
            let attr = stage.attribute(full.as_str()).map_err(|_| corrupt())?;
            let ty = attr
                .get_metadata::<sdf::Value>("typeName")
                .map_err(|_| corrupt())?
                .and_then(|v| match v {
                    sdf::Value::Token(s) => Some(s.to_string()),
                    _ => None,
                });
            let Some(type_name) = ty else {
                let relationship = stage.relationship(full.as_str()).map_err(|_| corrupt())?;
                if !relationship.is_defined().map_err(|_| corrupt())? {
                    if foreign_prim {
                        continue;
                    }
                    return Err(unsupported());
                }
                let targets = relationship.targets().map_err(|_| corrupt())?;
                if targets.len() > b.array_items {
                    return Err(limit());
                }
                let targets = targets
                    .into_iter()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>();
                charge(
                    &mut output_bytes,
                    property.len().saturating_add(128),
                    b.snapshot_bytes,
                )?;
                for target in &targets {
                    if target.len() > 1024 {
                        return Err(limit());
                    }
                    charge(
                        &mut output_bytes,
                        target.len().saturating_add(32),
                        b.snapshot_bytes,
                    )?;
                }
                relationships.push(Relationship {
                    name: property.to_string(),
                    targets,
                });
                continue;
            };
            let value = if attr
                .resolve_info()
                .map_err(|_| corrupt())?
                .value_is_blocked()
            {
                Some(Value::Block)
            } else {
                match attr
                    .get::<sdf::Value>()
                    .map_err(|_| corrupt())?
                    .map(|v| typed_depth(&v, b, 0, scalar_budget(&property)))
                    .transpose()
                {
                    Ok(value) => value,
                    // A displayed source may hold values the snapshot cannot represent; the
                    // property is left out of it, never the whole source refused.
                    Err(error) if foreign_prim && droppable(&error) => {
                        continue;
                    }
                    Err(error) => return Err(error),
                }
            };
            if let Some(v) = &value {
                value_size(v, &mut output_bytes, b.snapshot_bytes)?;
            }
            let interpolation = match attr
                .get_metadata::<sdf::Value>("interpolation")
                .map_err(|_| corrupt())?
            {
                Some(sdf::Value::Token(s)) => Some(s.to_string()),
                Some(_) if !foreign_prim => return Err(unsupported()),
                _ => None,
            };
            charge(
                &mut output_bytes,
                property
                    .len()
                    .saturating_add(type_name.len())
                    .saturating_add(interpolation.as_ref().map_or(0, String::len))
                    .saturating_add(128),
                b.snapshot_bytes,
            )?;
            properties.push(Property {
                name: property.to_string(),
                type_name,
                value,
                interpolation,
            });
        }
        properties.sort_by(|a, b| a.name.cmp(&b.name));
        relationships.sort_by(|a, b| a.name.cmp(&b.name));
        let element_id = properties
            .iter()
            .find(|p| p.name == "bim:id")
            .map(|p| match &p.value {
                Some(Value::Text(s)) if uuid7(s) => Ok(s.clone()),
                _ => Err(fail(ErrorCode::InvalidId, "invalid full element identity")),
            })
            .transpose()?;
        let leaf = path.rsplit('/').next().ok_or_else(corrupt)?;
        let reserved_leaf =
            beamcraft_model::prim::ElementCategory::ALL
                .into_iter()
                .any(|category| {
                    category
                        .prefix()
                        .rsplit_once('/')
                        .is_some_and(|(_, code)| leaf.starts_with(code))
                });
        if let Some(id) = &element_id {
            if crate::index::ElementId::from_path(&path).ok()
                != crate::index::ElementId::parse(id).ok()
                || properties
                    .iter()
                    .find(|p| p.name == "bim:elementKind")
                    .and_then(|p| match &p.value {
                        Some(Value::Text(kind)) => {
                            beamcraft_model::prim::ElementCategory::from_kind(kind)
                        }
                        _ => None,
                    })
                    .is_some_and(|category| !path.starts_with(category.prefix()))
            {
                return Err(fail(
                    ErrorCode::InvalidId,
                    "element path and identity differ",
                ));
            }
        } else if reserved_leaf || leaf.starts_with("E_") {
            return Err(fail(
                ErrorCode::InvalidId,
                "element path lacks full identity",
            ));
        }
        let definition_leaf = [
            beamcraft_model::prim::DefinitionCategory::Material,
            beamcraft_model::prim::DefinitionCategory::Profile,
        ]
        .into_iter()
        .any(|category| {
            path.strip_prefix(category.prefix())
                .is_some_and(|rest| !rest.contains('/'))
        });
        if path.starts_with("/World/Definitions/D_")
            || (definition_leaf
                && beamcraft_model::prim::DefinitionCategory::parse_path(&path).is_none())
        {
            return Err(fail(
                ErrorCode::InvalidId,
                "definition path lacks its category and full SHA-256",
            ));
        }
        let display_name = match prim
            .get_metadata::<sdf::Value>("displayName")
            .map_err(|_| corrupt())?
        {
            Some(sdf::Value::String(s)) => Some(s),
            Some(_) if !foreign_prim => return Err(unsupported()),
            _ => None,
        };
        if let Some(name) = &display_name {
            charge(
                &mut output_bytes,
                name.len().saturating_add(32),
                b.snapshot_bytes,
            )?;
        }
        let kind = prim.kind().map_err(|_| corrupt())?.map(|k| k.to_string());
        if let Some(kind) = &kind {
            charge(
                &mut output_bytes,
                kind.len().saturating_add(32),
                b.snapshot_bytes,
            )?;
        }
        charge(
            &mut output_bytes,
            path.len().saturating_add(256),
            b.snapshot_bytes,
        )?;
        prims.push(Prim {
            path,
            element_id,
            type_name: prim
                .type_name()
                .map_err(|_| corrupt())?
                .map(|s| s.to_string()),
            active: prim.is_active().map_err(|_| corrupt())?,
            kind,
            display_name,
            properties,
            relationships,
            sources,
        });
    }
    control.check()?;
    Ok(Candidate {
        identity: request.identity,
        prims,
        budgets: b,
        meters_per_unit,
        up_axis,
        metadata,
        layers: assets
            .keys()
            .map(|n| (n.clone(), request.owned_layers.contains(n)))
            .collect(),
    })
}

fn is_external(name: &str) -> bool {
    name.starts_with(beamcraft_model::reference::EXTERNAL_LAYER_PREFIX)
}

/// A layer is a displayed source when the host did not select it as owned and it is stored as a
/// foreign reference source. Every other layer, owned or not, keeps the strict profile.
fn displayed(name: &str, owned: &BTreeSet<String>) -> bool {
    !owned.contains(name) && is_external(name)
}

/// A value of a displayed source that the snapshot cannot represent (an unknown type, a number
/// outside the stage scalar budget, a non-finite number) costs that property, not the source.
fn droppable(error: &Diagnostic) -> bool {
    matches!(
        error.code,
        ErrorCode::UnsupportedProperty | ErrorCode::LimitExceeded | ErrorCode::NonFiniteNumber
    )
}

/// A foreign layer is displayed, not validated: an arc of its own that cannot be composed (a
/// layer that was not supplied, a missing prim) is dropped by the composer and the rest of the
/// prim still composes. An arc authored by an owned layer stays a refusal.
fn tolerated_composition_error(
    error: &openusd::pcp::CompositionDiagnostic,
    owned: &BTreeSet<String>,
) -> bool {
    use openusd::pcp::CompositionDiagnostic as D;
    match error {
        D::UnresolvedLayer { introduced_by, .. }
        | D::MalformedLayer { introduced_by, .. }
        | D::MutedAssetPath { introduced_by, .. }
        | D::UnresolvedPrimPath { introduced_by, .. }
        | D::UnresolvedSublayer { introduced_by, .. } => displayed(introduced_by, owned),
        _ => false,
    }
}

/// Running totals charged against the stage budgets while layers are loaded.
#[derive(Default)]
pub(crate) struct Counters {
    decoded: usize,
    values: usize,
    prim_count: usize,
}

/// Preflights, parses and scans one layer: the byte-level and field-level admission that every
/// supplied asset passes, whether it is owned or foreign. Returns the parsed spec data and the
/// local assets it depends on (sublayers and explicit external references).
pub(crate) fn load_asset(
    name: &str,
    asset: &Asset,
    b: Budgets,
    control: &Control,
    counters: &mut Counters,
    foreign: bool,
) -> Result<(Box<dyn AbstractData>, Vec<String>), Diagnostic> {
    control.check()?;
    let data: Box<dyn AbstractData> = if name.ends_with(".usda") {
        preflight::text(&asset.bytes, b, control, foreign)?;
        Box::new(
            openusd::usda::parse(std::str::from_utf8(&asset.bytes).map_err(|_| corrupt())?)
                .map_err(|_| corrupt())?,
        )
    } else {
        preflight::binary(&asset.bytes, b, &mut counters.decoded, foreign)?;
        let file = openusd::usdc::CrateFile::open(asset.bytes.clone()).map_err(|_| corrupt())?;
        file.validate().map_err(|_| corrupt())?;
        if file.tokens.iter().any(|s| s.len() > b.text_bytes) {
            return Err(limit());
        }
        preflight::binary_values(&file, &asset.bytes, b, &mut counters.decoded, foreign)?;
        Box::new(openusd::usdc::CrateData::from_file(file, true).map_err(|_| corrupt())?)
    };
    control.check()?;
    let mut dependencies = Vec::new();
    for path in data.spec_paths() {
        control.check()?;
        if path.to_string().len() > 1024
            || path
                .to_string()
                .split('/')
                .filter(|p| !p.is_empty())
                .count()
                > if foreign {
                    preflight::FOREIGN_PRIM_DEPTH
                } else {
                    b.depth
                }
        {
            return Err(limit());
        }
        match data.spec_type(&path) {
            Some(sdf::SpecType::Prim) => charge(&mut counters.prim_count, 1, b.prims)?,
            Some(
                sdf::SpecType::PseudoRoot
                | sdf::SpecType::Attribute
                | sdf::SpecType::Relationship
                | sdf::SpecType::RelationshipTarget,
            ) => (),
            _ if foreign => continue,
            _ => return Err(unsupported()),
        }
        for field in data.list_fields(&path).ok_or_else(corrupt)? {
            charge(&mut counters.values, 1, b.values)?;
            let value = data.get_field(&path, &field).map_err(|_| corrupt())?;
            match field.as_str() {
                "payload"
                | "inheritPaths"
                | "specializes"
                | "variantSelection"
                | "variantSetNames"
                | "variantChildren"
                | "relocates"
                | "instanceable"
                | "clips"
                | "timeSamples"
                | "expressionVariables" => {
                    if !foreign {
                        return Err(unsupported());
                    }
                }
                "subLayerOffsets" => {
                    let sdf::Value::LayerOffsetVec(offsets) = value.as_ref() else {
                        return Err(corrupt());
                    };
                    if offsets.len() > b.array_items {
                        return Err(limit());
                    }
                    for offset in offsets {
                        number(offset.offset, SCALAR_MAX)?;
                        number(offset.scale, SCALAR_MAX)?;
                        if *offset != sdf::LayerOffset::IDENTITY && !foreign {
                            return Err(unsupported());
                        }
                    }
                }
                "subLayers" => {
                    let sdf::Value::StringVec(names) = value.as_ref() else {
                        return Err(corrupt());
                    };
                    for target in names {
                        match preflight::anchored(target, name) {
                            Ok(dependency) => dependencies.push(dependency),
                            Err(_) if foreign => (),
                            Err(error) => return Err(error),
                        }
                    }
                }
                "references" => {
                    let sdf::Value::ReferenceListOp(list) = value.as_ref() else {
                        return Err(corrupt());
                    };
                    // No deleted/reordered references or internal arcs in this read profile.
                    if (!list.deleted_items.is_empty() || !list.ordered_items.is_empty())
                        && !foreign
                    {
                        return Err(unsupported());
                    }
                    for reference in list
                        .explicit_items
                        .iter()
                        .chain(&list.added_items)
                        .chain(&list.prepended_items)
                        .chain(&list.appended_items)
                    {
                        if (reference.layer_offset != sdf::LayerOffset::IDENTITY
                            || !reference.custom_data.is_empty()
                            || reference.prim_path.is_empty())
                            && !foreign
                        {
                            return Err(unsupported());
                        }
                        match preflight::anchored(&reference.asset_path, name) {
                            Ok(dependency) => dependencies.push(dependency),
                            Err(_) if foreign => (),
                            Err(error) => return Err(error),
                        }
                    }
                }
                _ => {
                    // An attribute's value fields follow the attribute's own scalar budget
                    // (the stored quantities have theirs, see `scalar_budget`); every other
                    // spec keeps the generic one.
                    let max = if data.spec_type(&path) == Some(sdf::SpecType::Attribute) {
                        scalar_budget(path.to_string().rsplit('.').next().unwrap_or_default())
                    } else {
                        SCALAR_MAX
                    };
                    match validate_as(value.as_ref(), b, 0, max) {
                        // A displayed source may hold values the snapshot cannot represent;
                        // they are left out of it, not refused.
                        Err(error) if foreign && droppable(&error) => (),
                        other => other?,
                    }
                }
            }
        }
    }
    Ok((data, dependencies))
}

fn walk(
    name: &str,
    graph: &BTreeMap<String, Vec<String>>,
    chain: &mut BTreeSet<String>,
    b: Budgets,
    work: &mut usize,
    control: &Control,
) -> Result<(), Diagnostic> {
    control.check()?;
    if chain.contains(name) {
        return Err(fail(
            ErrorCode::DependencyConflict,
            "cyclic layer dependency",
        ));
    }
    if chain.len() >= b.depth {
        return Err(limit());
    }
    charge(work, 1, b.values)?;
    let edges = graph.get(name).ok_or_else(|| {
        fail(
            ErrorCode::DependencyUnavailable,
            "missing declared local asset",
        )
    })?;
    chain.insert(name.to_owned());
    for edge in edges {
        walk(edge, graph, chain, b, work, control)?;
    }
    chain.remove(name);
    Ok(())
}

fn validate_as(value: &sdf::Value, b: Budgets, depth: usize, max: f64) -> Result<(), Diagnostic> {
    if depth >= b.depth {
        return Err(limit());
    }
    match value {
        sdf::Value::Specifier(_) | sdf::Value::Permission(_) | sdf::Value::Variability(_) => Ok(()),
        sdf::Value::Dictionary(map) => {
            if map.len() > b.array_items {
                return Err(limit());
            }
            for (key, v) in map {
                if key.len() > 1024 {
                    return Err(limit());
                }
                validate_as(v, b, depth + 1, max)?;
            }
            Ok(())
        }
        sdf::Value::AssetPath(_) | sdf::Value::AssetPathVec(_) => Err(unsupported()),
        sdf::Value::PathVec(v) => {
            if v.len() > b.array_items {
                return Err(limit());
            }
            Ok(())
        }
        sdf::Value::PathListOp(list) => {
            let count = list
                .explicit_items
                .len()
                .saturating_add(list.added_items.len())
                .saturating_add(list.prepended_items.len())
                .saturating_add(list.appended_items.len())
                .saturating_add(list.deleted_items.len())
                .saturating_add(list.ordered_items.len());
            if count > b.array_items {
                return Err(limit());
            }
            for path in list
                .explicit_items
                .iter()
                .chain(&list.added_items)
                .chain(&list.prepended_items)
                .chain(&list.appended_items)
                .chain(&list.deleted_items)
                .chain(&list.ordered_items)
            {
                if path.to_string().len() > 1024 {
                    return Err(limit());
                }
            }
            Ok(())
        }
        _ => {
            typed_depth(value, b, depth, max)?;
            Ok(())
        }
    }
}

/// The generic scalar budget of the bounded read, sized for millimetre coordinates; the
/// domain rules cap each kind of value tighter.
const SCALAR_MAX: f64 = 1_000_000_000.0;
/// The budget of the stored-quantity doubles, which the quantity rules validate on their own
/// (`authored::validate::quantities`): a volume above one cubic metre is a legitimate value
/// above 1e9 mm3 and must not read as a stage-budget breach.
const QUANTITY_MAX: f64 = 1e30;

/// The scalar budget of one attribute's numbers. Only the two stored quantities carry the
/// wider budget; coordinates, dimensions and unknown attributes keep the generic one.
fn scalar_budget(property: &str) -> f64 {
    if matches!(property, "bim:volumeMm3" | "bim:massKg") {
        QUANTITY_MAX
    } else {
        SCALAR_MAX
    }
}

/// Converts a value read back from a layer under the scalar budget of the named attribute:
/// the capture of existing opinions must not refuse the quantities it itself stored.
pub(crate) fn typed_attribute(
    value: &sdf::Value,
    b: Budgets,
    property: &str,
) -> Result<Value, Diagnostic> {
    typed_depth(value, b, 0, scalar_budget(property))
}

fn number(n: f64, max: f64) -> Result<Value, Diagnostic> {
    if !n.is_finite() {
        return Err(fail(ErrorCode::NonFiniteNumber, "non-finite numeric value"));
    }
    if n.abs() > max {
        return Err(limit());
    }
    Ok(Value::Number(n))
}
pub(crate) fn typed(value: &sdf::Value, b: Budgets) -> Result<Value, Diagnostic> {
    typed_depth(value, b, 0, SCALAR_MAX)
}
fn typed_depth(
    value: &sdf::Value,
    b: Budgets,
    depth: usize,
    max: f64,
) -> Result<Value, Diagnostic> {
    if depth >= b.depth {
        return Err(limit());
    }
    use sdf::Value as V;
    let array = |values: Vec<Value>| -> Result<Value, Diagnostic> {
        if values.len() > b.array_items {
            return Err(limit());
        }
        Ok(Value::Array(values))
    };
    match value {
        V::Dictionary(map) => {
            if map.len() > b.array_items || map.keys().any(|s| s.len() > 1024) {
                return Err(limit());
            }
            Ok(Value::Dictionary(
                map.iter()
                    .map(|(k, v)| Ok((k.clone(), typed_depth(v, b, depth + 1, max)?)))
                    .collect::<Result<_, Diagnostic>>()?,
            ))
        }
        V::ValueBlock => Ok(Value::Block),
        V::Bool(v) => Ok(Value::Bool(*v)),
        V::Uchar(v) => Ok(Value::Uint((*v).into())),
        V::Int(v) => Ok(Value::Int((*v).into())),
        V::Int64(v) => Ok(Value::Int(*v)),
        V::Uint(v) => Ok(Value::Uint((*v).into())),
        V::Uint64(v) => Ok(Value::Uint(*v)),
        V::Half(v) => number(v.to_f64(), max),
        V::Float(v) => number((*v).into(), max),
        V::Double(v) => number(*v, max),
        V::String(v) => {
            if v.len() > b.text_bytes {
                return Err(limit());
            }
            Ok(Value::Text(v.clone()))
        }
        V::Token(v) => {
            if v.len() > b.text_bytes {
                return Err(limit());
            }
            Ok(Value::Text(v.to_string()))
        }
        V::DoubleVec(v) => {
            if v.len() > b.array_items {
                return Err(limit());
            }
            array(
                v.iter()
                    .map(|n| number(*n, max))
                    .collect::<Result<_, _>>()?,
            )
        }
        V::FloatVec(v) => {
            if v.len() > b.array_items {
                return Err(limit());
            }
            array(
                v.iter()
                    .map(|n| number((*n).into(), max))
                    .collect::<Result<_, _>>()?,
            )
        }
        V::IntVec(v) => {
            if v.len() > b.array_items {
                return Err(limit());
            }
            array(v.iter().map(|n| Value::Int((*n).into())).collect())
        }
        V::StringVec(v) => {
            if v.len() > b.array_items || v.iter().any(|s| s.len() > b.text_bytes) {
                return Err(limit());
            }
            array(v.iter().map(|s| Value::Text(s.clone())).collect())
        }
        V::TokenVec(v) => {
            if v.len() > b.array_items || v.iter().any(|s| s.len() > b.text_bytes) {
                return Err(limit());
            }
            array(v.iter().map(|s| Value::Text(s.to_string())).collect())
        }
        V::Vec3d(v) => array(
            [v.x, v.y, v.z]
                .into_iter()
                .map(|n| number(n, max))
                .collect::<Result<_, _>>()?,
        ),
        V::Quatd(v) => array(
            [v.w, v.x, v.y, v.z]
                .into_iter()
                .map(|n| number(n, max))
                .collect::<Result<_, _>>()?,
        ),
        V::Quatf(v) => array(
            [v.w, v.x, v.y, v.z]
                .into_iter()
                .map(|n| number(n.into(), max))
                .collect::<Result<_, _>>()?,
        ),
        // Row-major, as USD stores it: the translation is the last row.
        V::Matrix4d(m) => array(
            m.0.iter()
                .map(|n| number(*n, max))
                .collect::<Result<_, _>>()?,
        ),
        V::Vec3f(v) => array(
            [v.x, v.y, v.z]
                .into_iter()
                .map(|n| number(n.into(), max))
                .collect::<Result<_, _>>()?,
        ),
        V::Vec3fVec(v) => {
            if v.len().saturating_mul(3) > b.array_items {
                return Err(limit());
            }
            array(
                v.iter()
                    .map(|v| typed_depth(&V::Vec3f(*v), b, 0, max))
                    .collect::<Result<_, _>>()?,
            )
        }
        V::Vec3dVec(v) => {
            if v.len() > b.array_items {
                return Err(limit());
            }
            array(
                v.iter()
                    .map(|v| typed_depth(&V::Vec3d(*v), b, 0, max))
                    .collect::<Result<_, _>>()?,
            )
        }
        _ => Err(unsupported()),
    }
}
fn value_size(v: &Value, bytes: &mut usize, max: usize) -> Result<(), Diagnostic> {
    charge(bytes, 32, max)?;
    match v {
        Value::Text(s) => charge(bytes, s.len(), max)?,
        Value::Array(values) => {
            for value in values {
                value_size(value, bytes, max)?;
            }
        }
        Value::Dictionary(values) => {
            for (key, value) in values {
                charge(bytes, key.len().saturating_add(64), max)?;
                value_size(value, bytes, max)?;
            }
        }
        _ => (),
    }
    Ok(())
}

/// What a foreign layer states about itself, read without composing it.
#[derive(Debug, Clone, PartialEq)]
pub struct ForeignLayer {
    /// Storage format, decided from the leading bytes.
    pub format: beamcraft_model::reference::SourceFormat,
    /// Name of the `defaultPrim`, if the layer states one.
    pub default_prim: Option<String>,
    /// `metersPerUnit`, if the layer states one.
    pub meters_per_unit: Option<f64>,
    /// `upAxis`, if the layer states one.
    pub up_axis: Option<String>,
    /// Prim specs in the layer.
    pub prim_count: usize,
}

/// Admits a foreign layer under the byte and budget rules of a displayed source and reports what
/// it declares. Nothing is composed and nothing is resolved. The layer is displayed, not
/// validated: encodings, metadata and arcs the profile cannot use (payloads, variants, sublayers,
/// references to other assets, URLs and escaping identifiers) are inert, never refused, because
/// the resolver only ever serves the supplied layers; what such an arc would have contributed is
/// absent from the composed view. The layer's own units are reported, not required: only the
/// project's root layer must be millimetres and Z up.
pub(crate) fn inspect_foreign(
    bytes: &[u8],
    b: Budgets,
    control: &Control,
) -> Result<ForeignLayer, Diagnostic> {
    on_parser_stack(|| inspect_foreign_on_stack(bytes, b, control))
}

fn inspect_foreign_on_stack(
    bytes: &[u8],
    b: Budgets,
    control: &Control,
) -> Result<ForeignLayer, Diagnostic> {
    use beamcraft_model::reference::SourceFormat;
    b.validate()?;
    control.check()?;
    if bytes.is_empty() || bytes.len() > b.layer_bytes {
        return Err(if bytes.is_empty() { corrupt() } else { limit() });
    }
    let (format, name) = if bytes.starts_with(b"#usda 1.0") {
        (SourceFormat::Usda, "probe.usda")
    } else if bytes.starts_with(b"PXR-USDC") {
        (SourceFormat::Usdc, "probe.usdc")
    } else if bytes.starts_with(b"PK\x03\x04") {
        return Err(unsupported());
    } else {
        return Err(corrupt());
    };
    let asset = Asset {
        name: name.to_owned(),
        digest: Sha256::digest(bytes).map_err(|_| limit())?,
        bytes: bytes.to_vec(),
    };
    let (data, _) = load_asset(name, &asset, b, control, &mut Counters::default(), true)?;
    let root = sdf::Path::abs_root();
    let field = |key: &str| -> Result<Option<sdf::Value>, Diagnostic> {
        Ok(data
            .try_field(&root, key)
            .map_err(|_| corrupt())?
            .map(|v| v.as_ref().clone()))
    };
    let text_of = |value: Option<sdf::Value>| -> Result<Option<String>, Diagnostic> {
        match value {
            None => Ok(None),
            Some(sdf::Value::Token(s)) => Ok(Some(s.to_string())),
            Some(sdf::Value::String(s)) => Ok(Some(s)),
            Some(_) => Err(unsupported()),
        }
    };
    let meters_per_unit = match field("metersPerUnit")? {
        None => None,
        Some(sdf::Value::Double(n)) => Some(n),
        Some(sdf::Value::Float(n)) => Some(f64::from(n)),
        Some(_) => return Err(unsupported()),
    };
    if let Some(n) = meters_per_unit {
        number(n, SCALAR_MAX)?;
    }
    let default_prim = text_of(field("defaultPrim")?)?;
    let up_axis = text_of(field("upAxis")?)?;
    let mut prim_count = 0usize;
    for path in data.spec_paths() {
        if data.spec_type(&path) == Some(sdf::SpecType::Prim) {
            prim_count = prim_count.saturating_add(1);
        }
    }
    if let Some(prim) = &default_prim {
        let path = sdf::Path::new(&format!("/{prim}")).map_err(|_| corrupt())?;
        let defined = data.spec_type(&path) == Some(sdf::SpecType::Prim)
            && matches!(
                data.try_field(&path, "specifier")
                    .map_err(|_| corrupt())?
                    .as_deref(),
                Some(sdf::Value::Specifier(sdf::Specifier::Def))
            );
        if !defined {
            return Err(fail(
                ErrorCode::NotFound,
                "default prim is not defined in the foreign layer",
            ));
        }
    }
    Ok(ForeignLayer {
        format,
        default_prim,
        meters_per_unit,
        up_axis,
        prim_count,
    })
}
