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
    pub stage: usd::Stage,
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

pub(crate) fn open(request: OpenRequest, control: &Control) -> Result<Candidate, Diagnostic> {
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
        let (data, dependencies) = load_asset(name, asset, b, control, &mut counters)?;
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
    if !stage.composition_errors().is_empty() {
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
                attr.get::<sdf::Value>()
                    .map_err(|_| corrupt())?
                    .map(|v| typed_depth(&v, b, 0, scalar_budget(&property)))
                    .transpose()?
            };
            if let Some(v) = &value {
                value_size(v, &mut output_bytes, b.snapshot_bytes)?;
            }
            let interpolation = match attr
                .get_metadata::<sdf::Value>("interpolation")
                .map_err(|_| corrupt())?
            {
                Some(sdf::Value::Token(s)) => Some(s.to_string()),
                Some(_) => return Err(unsupported()),
                None => None,
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
        if let Some(id) = &element_id {
            if leaf != format!("E_{}", id.replace('-', "")) {
                return Err(fail(
                    ErrorCode::InvalidId,
                    "element path and identity differ",
                ));
            }
        } else if leaf.starts_with("E_") {
            return Err(fail(
                ErrorCode::InvalidId,
                "element path lacks full identity",
            ));
        }
        let display_name = match prim
            .get_metadata::<sdf::Value>("displayName")
            .map_err(|_| corrupt())?
        {
            Some(sdf::Value::String(s)) => Some(s),
            Some(_) => return Err(unsupported()),
            None => None,
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
        stage,
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
) -> Result<(Box<dyn AbstractData>, Vec<String>), Diagnostic> {
    control.check()?;
    let data: Box<dyn AbstractData> = if name.ends_with(".usda") {
        preflight::text(&asset.bytes, b, control)?;
        Box::new(
            openusd::usda::parse(std::str::from_utf8(&asset.bytes).map_err(|_| corrupt())?)
                .map_err(|_| corrupt())?,
        )
    } else {
        preflight::binary(&asset.bytes, b, &mut counters.decoded)?;
        let file = openusd::usdc::CrateFile::open(asset.bytes.clone()).map_err(|_| corrupt())?;
        file.validate().map_err(|_| corrupt())?;
        if file.tokens.iter().any(|s| s.len() > 1024) {
            return Err(limit());
        }
        preflight::binary_values(&file, &asset.bytes, b, &mut counters.decoded)?;
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
                > b.depth
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
                | "expressionVariables" => return Err(unsupported()),
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
                        if *offset != sdf::LayerOffset::IDENTITY {
                            return Err(unsupported());
                        }
                    }
                }
                "subLayers" => {
                    let sdf::Value::StringVec(names) = value.as_ref() else {
                        return Err(corrupt());
                    };
                    for target in names {
                        dependencies.push(preflight::anchored(target, name)?);
                    }
                }
                "references" => {
                    let sdf::Value::ReferenceListOp(list) = value.as_ref() else {
                        return Err(corrupt());
                    };
                    // No deleted/reordered references or internal arcs in this read profile.
                    if !list.deleted_items.is_empty() || !list.ordered_items.is_empty() {
                        return Err(unsupported());
                    }
                    for reference in list
                        .explicit_items
                        .iter()
                        .chain(&list.added_items)
                        .chain(&list.prepended_items)
                        .chain(&list.appended_items)
                    {
                        if reference.layer_offset != sdf::LayerOffset::IDENTITY
                            || !reference.custom_data.is_empty()
                            || reference.prim_path.is_empty()
                        {
                            return Err(unsupported());
                        }
                        dependencies.push(preflight::anchored(&reference.asset_path, name)?);
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
                    validate_as(value.as_ref(), b, 0, max)?
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
            if v.len() > 1024 {
                return Err(limit());
            }
            Ok(Value::Text(v.clone()))
        }
        V::Token(v) => {
            if v.len() > 1024 {
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
            if v.len() > b.array_items || v.iter().any(|s| s.len() > 1024) {
                return Err(limit());
            }
            array(v.iter().map(|s| Value::Text(s.clone())).collect())
        }
        V::TokenVec(v) => {
            if v.len() > b.array_items || v.iter().any(|s| s.len() > 1024) {
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

/// Admits a foreign layer under the same byte, field and budget rules as every stage asset and
/// reports what it declares. Nothing is composed and nothing is resolved: a layer that names
/// another asset (a sublayer or an external reference) is `DEPENDENCY_UNAVAILABLE`, a URL or an
/// escaping asset identifier is `PERMISSION_DENIED`, and an arc outside the static profile is
/// `UNSUPPORTED_PROPERTY`, all before any stage exists. The layer's own units are reported, not
/// required: only the project's root layer must be millimetres and Z up.
pub(crate) fn inspect_foreign(
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
    let (data, dependencies) = load_asset(name, &asset, b, control, &mut Counters::default())?;
    if !dependencies.is_empty() {
        return Err(fail(
            ErrorCode::DependencyUnavailable,
            "foreign layer names other assets; only a single self-contained layer is supported",
        ));
    }
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
