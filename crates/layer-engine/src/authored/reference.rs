// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! R01 reference wrappers in the codeless U02 mapping.
//!
//! A wrapper is an owned element of kind `reference` with a fixed three-level shape:
//!
//! ```text
//! /World/References/REF_<id>                    owned: label, rigid placement, source pin (group)
//!   /Correction                          owned: optional units / up-axis interpretation (group)
//!     /Source                            typeless def: the reference arc and nothing else
//!       ...                              foreign: composed from the pinned layer, never written
//! ```
//!
//! The arc sits on `Source` and no owned layer may author anything else at or below it: not a
//! property, not a child, not an `over`, not a metadata field. That is the whole of the
//! read-only rule for foreign content, and it is checked on the layer specs themselves
//! ([`layers`]) as well as on the composed provenance ([`element`]), so a hand-edited layer
//! cannot slip an override in. The placement and the correction live on the two owned levels
//! above `Source`, so moving or re-reading the units of a source never touches a foreign prim.

use super::edit::{attr, attribute, container, data, field, path, pose, prim};
use super::validate::{self, label, optional, owned_pose, required, text};
use super::*;
use openusd::sdf::{self, AbstractData};

pub use beamcraft_model::reference::{SourceFormat, SourcePin, UnitsInterpretation, UpAxis};

/// Name of the owned units level below the wrapper.
pub(super) const CORRECTION: &str = "Correction";
/// Name of the prim that carries the reference arc.
pub(super) const SOURCE: &str = "Source";

const SOURCE_DIGEST: &str = "bim:reference:sourceDigest";
const SOURCE_BYTES: &str = "bim:reference:sourceBytes";
const SOURCE_FORMAT: &str = "bim:reference:sourceFormat";
const SOURCE_VERSION: &str = "bim:reference:sourceVersion";
const SOURCE_LABEL: &str = "bim:reference:sourceLabel";
const SOURCE_DEFAULT_PRIM: &str = "bim:reference:sourceDefaultPrim";
const DECLARED_UNITS: &str = "bim:reference:declaredMetersPerUnit";
const DECLARED_UP_AXIS: &str = "bim:reference:declaredUpAxis";
const CORRECTION_UNITS: &str = "bim:reference:correctionMetersPerUnit";
const CORRECTION_UP_AXIS: &str = "bim:reference:correctionUpAxis";
const CORRECTION_DIGEST: &str = "bim:reference:correctionSourceDigest";
const OP_ROTATE: &str = "xformOp:rotateX:bimUpAxis";
const OP_SCALE: &str = "xformOp:scale:bimUnits";

/// A complete new wrapper around a foreign source, after host authorization. The source layer
/// must already be a non-owned entry of the manifest with exactly the pinned digest and size.
#[derive(Debug, Clone, PartialEq)]
pub struct Reference {
    /// Full UUIDv7 of the wrapper, never a caller-controlled USD path.
    pub id: String,
    /// Explicit user label.
    pub name: String,
    /// Rigid placement of the wrapper, stored apart from the foreign content.
    pub placement: Placement,
    /// The snapshot of the foreign source (digest, size, format, default prim, version, label,
    /// declared units). The wrapper starts with no units correction.
    pub source: SourcePin,
}

fn refused(error: beamcraft_model::command::CommandError) -> Diagnostic {
    Diagnostic {
        code: error.code,
        message: "reference value refused by the R01 admission rules",
        rule_id: error.rule_id,
    }
}

fn foreign_write() -> Diagnostic {
    fail(
        ErrorCode::ForeignReadOnly,
        "owned layers cannot write foreign content",
    )
}

/// Admission of a new wrapper, before any layer is touched.
pub(super) fn check_new(r: &Reference) -> Result<(), Diagnostic> {
    label(&r.name)?;
    validate::pose(&r.placement)?;
    r.source.validate().map_err(refused)
}

/// Admission of a units interpretation.
pub(super) fn check_units(i: &Option<UnitsInterpretation>) -> Result<(), Diagnostic> {
    i.as_ref().map_or(Ok(()), |i| i.validate().map_err(refused))
}

/// The source layer a new wrapper pins must be in the manifest, foreign, and exactly the pinned
/// bytes: the authoring layer can then name it by a relative asset path.
pub(super) fn check_source_asset(
    request: &OpenRequest,
    targets: &EditTargets,
    pin: &SourcePin,
) -> Result<(), Diagnostic> {
    if targets.authored.contains('/') {
        return Err(unsupported());
    }
    let name = pin.layer_name();
    if request.owned_layers.contains(&name) {
        return Err(foreign_write());
    }
    let asset = request
        .assets
        .iter()
        .find(|a| a.name == name)
        .ok_or_else(|| {
            fail(
                ErrorCode::DependencyUnavailable,
                "the source layer is not in the manifest",
            )
        })?;
    if asset.digest != pin.digest || asset.bytes.len() as u64 != pin.bytes {
        return Err(fail(
            ErrorCode::IntegrityError,
            "the source layer differs from its pin",
        ));
    }
    Ok(())
}

/// Authors a new wrapper: the element, its pin, the owned units level and the arc.
pub(super) fn attach(data: &mut sdf::Data, r: &Reference, actor: &str) -> Result<(), Diagnostic> {
    let p = element_path(&r.id, ElementCategory::Reference)?;
    container(data, "/World", "Xform", Some("assembly"))?;
    container(data, "/World/References", "Scope", Some("group"))?;
    prim(data, &p, Some("Xform"), true)?;
    field(data, &p, "kind", sdf::Value::token("group"))?;
    field(data, &p, "displayName", sdf::Value::String(r.name.clone()))?;
    let pin = &r.source;
    let mut fields = vec![
        ("bim:id", "string", sdf::Value::String(r.id.clone())),
        ("bim:name", "string", sdf::Value::String(r.name.clone())),
        ("bim:elementKind", "token", sdf::Value::token("reference")),
        ("bim:geometryMode", "token", sdf::Value::token("reference")),
        (
            "bim:schemaVersion",
            "string",
            sdf::Value::String("0.1.0".into()),
        ),
        ("bim:createdBy", "string", sdf::Value::String(actor.into())),
        (
            SOURCE_DIGEST,
            "string",
            sdf::Value::String(pin.digest.to_string()),
        ),
        (
            SOURCE_BYTES,
            "int",
            sdf::Value::Int(i32::try_from(pin.bytes).map_err(|_| limit())?),
        ),
        (
            SOURCE_FORMAT,
            "token",
            sdf::Value::token(pin.format.as_str()),
        ),
        (
            SOURCE_VERSION,
            "int",
            sdf::Value::Int(i32::try_from(pin.version).map_err(|_| limit())?),
        ),
        (
            SOURCE_LABEL,
            "string",
            sdf::Value::String(pin.label.clone()),
        ),
        (
            SOURCE_DEFAULT_PRIM,
            "string",
            sdf::Value::String(pin.default_prim.clone()),
        ),
    ];
    if let Some(units) = pin.declared_meters_per_unit {
        fields.push((DECLARED_UNITS, "double", sdf::Value::Double(units)));
    }
    if let Some(axis) = pin.declared_up_axis {
        fields.push((DECLARED_UP_AXIS, "token", sdf::Value::token(axis.as_str())));
    }
    for (name, ty, value) in fields {
        attr(data, &p, name, ty, value)?;
    }
    pose(data, &p, &r.placement)?;
    let c = format!("{p}/{CORRECTION}");
    prim(data, &c, Some("Xform"), true)?;
    field(data, &c, "kind", sdf::Value::token("group"))?;
    attribute(
        data,
        &c,
        "xformOpOrder",
        "token[]",
        Opinion::Value(sdf::Value::token_vec(Vec::<&str>::new())),
        false,
    )?;
    // The arc holder has no type, kind, property or child of its own: whatever the foreign root
    // prim is, it is that, unchanged.
    let s = format!("{c}/{SOURCE}");
    prim(data, &s, None, true)?;
    field(
        data,
        &s,
        "references",
        sdf::Value::ReferenceListOp(sdf::ListOp::prepended([sdf::Reference {
            asset_path: pin.layer_name(),
            prim_path: path(&format!("/{}", pin.default_prim))?,
            layer_offset: sdf::LayerOffset::IDENTITY,
            custom_data: Default::default(),
        }])),
    )
}

/// The operations an interpretation authors on the units level, outermost first: the rotation
/// (if the source is Y up) and then the scale (if it is not millimetres). The scale applies to
/// the source first, so the rotation acts on millimetres.
fn expected_ops(i: &UnitsInterpretation) -> Vec<(&'static str, &'static str, sdf::Value)> {
    let mut ops = Vec::new();
    if let Some(degrees) = i.rotate_x_degrees() {
        ops.push((OP_ROTATE, "double", sdf::Value::Double(degrees)));
    }
    let scale = i.scale();
    if scale != 1.0 {
        ops.push((OP_SCALE, "double3", sdf::Value::vec3d(scale, scale, scale)));
    }
    ops
}

/// Writes (or clears) the explicit units interpretation on the owned units level of wrapper
/// `id`, recorded with the digest of the source it was made for.
pub(super) fn units(
    data: &mut sdf::Data,
    id: &str,
    interpretation: &Option<UnitsInterpretation>,
    source_digest: ContentDigest,
) -> Result<(), Diagnostic> {
    let c = format!(
        "{}/{CORRECTION}",
        element_path(id, ElementCategory::Reference)?
    );
    for (name, ty) in [
        (CORRECTION_UNITS, "double"),
        (CORRECTION_UP_AXIS, "token"),
        (CORRECTION_DIGEST, "string"),
        (OP_ROTATE, "double"),
        (OP_SCALE, "double3"),
    ] {
        attribute(data, &c, name, ty, Opinion::Remove, true)?;
    }
    let mut order = Vec::new();
    if let Some(i) = interpretation {
        attr(
            data,
            &c,
            CORRECTION_UNITS,
            "double",
            sdf::Value::Double(i.meters_per_unit),
        )?;
        attr(
            data,
            &c,
            CORRECTION_UP_AXIS,
            "token",
            sdf::Value::token(i.up_axis.as_str()),
        )?;
        attr(
            data,
            &c,
            CORRECTION_DIGEST,
            "string",
            sdf::Value::String(source_digest.to_string()),
        )?;
        for (name, ty, value) in expected_ops(i) {
            attribute(data, &c, name, ty, Opinion::Value(value), false)?;
            order.push(name);
        }
    }
    attribute(
        data,
        &c,
        "xformOpOrder",
        "token[]",
        Opinion::Value(sdf::Value::token_vec(order)),
        false,
    )
}

/// Reads the pin of a wrapper back from its attributes and re-admits it.
pub(super) fn read_pin(p: &Prim) -> Result<SourcePin, Diagnostic> {
    let int = |name: &str| -> Result<i64, Diagnostic> {
        match required(p, name, "int")? {
            Value::Int(n) => Ok(*n),
            _ => Err(invalid()),
        }
    };
    let format = SourceFormat::parse(text(p, SOURCE_FORMAT, "token")?).ok_or_else(invalid)?;
    let declared_up_axis = match optional(p, DECLARED_UP_AXIS, "token")? {
        None => None,
        Some(Value::Text(s)) => Some(UpAxis::parse(s).ok_or_else(invalid)?),
        Some(_) => return Err(invalid()),
    };
    let declared_meters_per_unit = match optional(p, DECLARED_UNITS, "double")? {
        None => None,
        Some(Value::Number(n)) => Some(*n),
        Some(_) => return Err(invalid()),
    };
    let pin = SourcePin {
        digest: text(p, SOURCE_DIGEST, "string")?
            .parse::<ContentDigest>()
            .map_err(|_| invalid())?,
        bytes: u64::try_from(int(SOURCE_BYTES)?).map_err(|_| invalid())?,
        format,
        default_prim: text(p, SOURCE_DEFAULT_PRIM, "string")?.to_owned(),
        version: u32::try_from(int(SOURCE_VERSION)?).map_err(|_| invalid())?,
        label: text(p, SOURCE_LABEL, "string")?.to_owned(),
        declared_meters_per_unit,
        declared_up_axis,
    };
    pin.validate().map_err(refused)?;
    Ok(pin)
}

/// The explicit units interpretation stored on the units level, or `None`. Either all three
/// records exist or none; the recorded source digest must be the pinned one.
pub(super) fn read_units(
    correction: &Prim,
    pin: &SourcePin,
) -> Result<Option<UnitsInterpretation>, Diagnostic> {
    let units = optional(correction, CORRECTION_UNITS, "double")?;
    let axis = optional(correction, CORRECTION_UP_AXIS, "token")?;
    let digest = optional(correction, CORRECTION_DIGEST, "string")?;
    let interpretation = match (units, axis, digest) {
        (None, None, None) => None,
        (Some(Value::Number(units)), Some(Value::Text(axis)), Some(Value::Text(digest))) => {
            if digest != &pin.digest.to_string() {
                return Err(fail(
                    ErrorCode::IntegrityError,
                    "the units interpretation was made for another source version",
                ));
            }
            Some(UnitsInterpretation {
                meters_per_unit: *units,
                up_axis: UpAxis::parse(axis).ok_or_else(invalid)?,
            })
        }
        _ => return Err(invalid()),
    };
    if let Some(i) = &interpretation {
        i.validate().map_err(refused)?;
    }
    Ok(interpretation)
}

fn check_correction(c: &Prim, pin: &SourcePin) -> Result<(), Diagnostic> {
    if c.sources.is_empty()
        || c.sources.iter().any(|s| !s.owned)
        || c.type_name.as_deref() != Some("Xform")
        || c.kind.as_deref() != Some("group")
    {
        return Err(invalid());
    }
    let interpretation = read_units(c, pin)?;
    let expected = interpretation
        .as_ref()
        .map(expected_ops)
        .unwrap_or_default();
    // Every xform operation present must be one the interpretation produces, with exactly the
    // value it produces: a hand-edited scale without the recorded interpretation is refused.
    for property in c
        .properties
        .iter()
        .filter(|v| v.name.starts_with("xformOp:"))
    {
        let Some((_, ty, value)) = expected.iter().find(|(name, ..)| *name == property.name) else {
            return Err(invalid());
        };
        let found = required(c, &property.name, ty)?;
        let want = match value {
            sdf::Value::Double(n) => Value::Number(*n),
            sdf::Value::Vec3d(v) => Value::Array(vec![
                Value::Number(v.x),
                Value::Number(v.y),
                Value::Number(v.z),
            ]),
            _ => return Err(invalid()),
        };
        if *found != want {
            return Err(invalid());
        }
    }
    let order: Vec<Value> = expected
        .iter()
        .map(|(name, ..)| Value::Text((*name).into()))
        .collect();
    if expected
        .iter()
        .any(|(name, ..)| !c.properties.iter().any(|v| v.name == *name))
        || required(c, "xformOpOrder", "token[]")? != &Value::Array(order)
    {
        return Err(invalid());
    }
    Ok(())
}

fn check_source(source: &Prim, pin: &SourcePin, prims: &[Prim]) -> Result<(), Diagnostic> {
    let (owned, foreign): (Vec<&Provenance>, Vec<&Provenance>) =
        source.sources.iter().partition(|s| s.owned);
    if owned.len() != 1 {
        return Err(foreign_write());
    }
    let [site] = foreign.as_slice() else {
        return Err(if foreign.is_empty() {
            fail(
                ErrorCode::DependencyUnavailable,
                "the reference does not compose a source",
            )
        } else {
            unsupported()
        });
    };
    if site.layer != pin.layer_name()
        || site.digest != pin.digest
        || site.path != format!("/{}", pin.default_prim)
    {
        return Err(fail(
            ErrorCode::IntegrityError,
            "the reference does not compose its pinned source",
        ));
    }
    let inside = format!("{}/", source.path);
    if prims
        .iter()
        .filter(|q| q.path.starts_with(&inside))
        .any(|q| q.sources.is_empty() || q.sources.iter().any(|s| s.owned))
    {
        return Err(foreign_write());
    }
    Ok(())
}

/// Validates one loaded wrapper: identity, label, placement, pin, the exact three-level shape,
/// the units level and the composed source. Anything else under the wrapper, and any owned
/// opinion below `Source`, is refused.
pub(super) fn element(p: &Prim, prims: &[Prim]) -> Result<(), Diagnostic> {
    let id = p.element_id.as_deref().ok_or_else(invalid)?;
    if p.path != element_path(id, ElementCategory::Reference)? {
        return Err(fail(
            ErrorCode::InvalidId,
            "invalid canonical owned element path",
        ));
    }
    if !uuid7(text(p, "bim:createdBy", "string")?) {
        return Err(fail(ErrorCode::InvalidId, "invalid actor identity"));
    }
    let name = text(p, "bim:name", "string")?;
    label(name)?;
    if p.kind.as_deref() != Some("group") || p.display_name.as_deref() != Some(name) {
        return Err(invalid());
    }
    if p.type_name.as_deref() != Some("Xform") {
        return Err(unsupported());
    }
    owned_pose(p)?;
    let pin = read_pin(p)?;
    let correction_path = format!("{}/{CORRECTION}", p.path);
    let source_path = format!("{correction_path}/{SOURCE}");
    let below_source = format!("{source_path}/");
    let inside = format!("{}/", p.path);
    for q in prims.iter().filter(|q| q.path.starts_with(&inside)) {
        if q.path != correction_path && q.path != source_path && !q.path.starts_with(&below_source)
        {
            return Err(invalid());
        }
    }
    let find = |wanted: &str| prims.iter().find(|q| q.path == wanted).ok_or_else(invalid);
    check_correction(find(&correction_path)?, &pin)?;
    check_source(find(&source_path)?, &pin, prims)
}

/// The paths of the arc holders of every owned wrapper in the snapshot.
fn source_paths(prims: &[Prim]) -> Vec<String> {
    prims
        .iter()
        .filter(|p| {
            p.element_id.is_some()
                && p.sources.iter().all(|s| s.owned)
                && prim_text(p, "bim:elementKind") == Some("reference")
                && prim_text(p, "bim:geometryMode") == Some("reference")
        })
        .map(|p| format!("{}/{CORRECTION}/{SOURCE}", p.path))
        .collect()
}

fn prim_text<'a>(p: &'a Prim, name: &str) -> Option<&'a str> {
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

/// Reads the specs of every owned layer: at an arc holder only `specifier` and `references` may
/// exist, and nothing may exist below it (no child, no `over`, no property). This is the
/// spec-level half of the read-only rule; it sees opinions that composition would hide behind a
/// stronger foreign one.
pub(super) fn layers(request: &OpenRequest, prims: &[Prim]) -> Result<(), Diagnostic> {
    let holders = source_paths(prims);
    if holders.is_empty() {
        return Ok(());
    }
    for asset in request
        .assets
        .iter()
        .filter(|a| request.owned_layers.contains(&a.name))
    {
        if !asset.name.ends_with(".usda") {
            return Err(unsupported());
        }
        let layer = data(asset)?;
        for spec in layer.spec_paths() {
            let text = spec.to_string();
            for holder in &holders {
                if text == *holder {
                    let fields = layer.list_fields(&spec).ok_or_else(corrupt)?;
                    if fields
                        .iter()
                        .any(|f| !matches!(f.as_str(), "specifier" | "references"))
                    {
                        return Err(foreign_write());
                    }
                    // The holder is a plain `def`: an `over` or a `class` spec carrying the same
                    // arc would be another kind of opinion on the foreign root prim.
                    let specifier = layer.get_field(&spec, "specifier").map_err(|_| corrupt())?;
                    if !matches!(
                        specifier.as_ref(),
                        sdf::Value::Specifier(sdf::Specifier::Def)
                    ) {
                        return Err(foreign_write());
                    }
                } else if text.starts_with(holder.as_str())
                    && matches!(text.as_bytes().get(holder.len()), Some(b'/' | b'.' | b'['))
                {
                    return Err(foreign_write());
                }
            }
        }
    }
    Ok(())
}

/// The source digest recorded on an existing wrapper.
pub(super) fn recorded_digest(p: &Prim) -> Result<ContentDigest, Diagnostic> {
    text(p, SOURCE_DIGEST, "string")?
        .parse::<ContentDigest>()
        .map_err(|_| invalid())
}

/// The fixed attribute names (and USD types) that creating a wrapper authors on the element.
pub(super) const ATTACH_FIELDS: &[(&str, &str)] = &[
    ("bim:id", "string"),
    ("bim:name", "string"),
    ("bim:elementKind", "token"),
    ("bim:geometryMode", "token"),
    ("bim:schemaVersion", "string"),
    ("bim:createdBy", "string"),
    (SOURCE_DIGEST, "string"),
    (SOURCE_BYTES, "int"),
    (SOURCE_FORMAT, "token"),
    (SOURCE_VERSION, "int"),
    (SOURCE_LABEL, "string"),
    (SOURCE_DEFAULT_PRIM, "string"),
    (DECLARED_UNITS, "double"),
    (DECLARED_UP_AXIS, "token"),
];

/// The fixed attribute names (and USD types) that a units change authors on the units level.
pub(super) const UNITS_FIELDS: &[(&str, &str)] = &[
    (CORRECTION_UNITS, "double"),
    (CORRECTION_UP_AXIS, "token"),
    (CORRECTION_DIGEST, "string"),
    (OP_ROTATE, "double"),
    (OP_SCALE, "double3"),
    ("xformOpOrder", "token[]"),
];

/// One prim of the foreign content, as composed below a wrapper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignPrim {
    /// Composed path in the project stage.
    pub path: String,
    /// The same prim's path in the source layer's own namespace (`/DefaultPrim/Child`).
    pub source_path: String,
    /// Composed prim type, as the foreign layer states it.
    pub type_name: Option<String>,
}

/// A loaded wrapper, read back from a pinned snapshot after the same validation that publishing
/// applies. It carries the owned wrapper data and the foreign prims it composes, never a handle
/// that could write.
#[derive(Debug, Clone, PartialEq)]
pub struct ReferenceView {
    /// Full UUIDv7 of the wrapper.
    pub id: String,
    /// Composed path of the wrapper element.
    pub path: String,
    /// Label of the wrapper.
    pub name: String,
    /// Rigid placement of the wrapper, as authored on the owned level.
    pub placement: Placement,
    /// The pinned snapshot of the source.
    pub source: SourcePin,
    /// The explicit units interpretation, if one was made.
    pub units: Option<UnitsInterpretation>,
    /// Composed path of the arc holder: the root of the foreign content.
    pub source_root: String,
    /// Every foreign prim below (and including) the arc holder, in path order.
    pub foreign: Vec<ForeignPrim>,
}

fn numbers<const N: usize>(p: &Prim, name: &str, ty: &str) -> Result<[f64; N], Diagnostic> {
    let Value::Array(items) = required(p, name, ty)? else {
        return Err(invalid());
    };
    let parsed: Vec<f64> = items
        .iter()
        .map(|v| match v {
            Value::Number(n) => Ok(*n),
            _ => Err(invalid()),
        })
        .collect::<Result<_, _>>()?;
    parsed.try_into().map_err(|_| invalid())
}

/// Reads every wrapper of a complete pinned snapshot, validating each exactly as publishing does.
/// A wrapper that fails validation fails the whole read; nothing is guessed or skipped.
pub fn read_references(prims: &[Prim]) -> Result<Vec<ReferenceView>, Diagnostic> {
    let mut views = Vec::new();
    for p in prims.iter().filter(|p| {
        p.element_id.is_some()
            && p.sources.iter().all(|s| s.owned)
            && prim_text(p, "bim:elementKind") == Some("reference")
            && prim_text(p, "bim:geometryMode") == Some("reference")
    }) {
        element(p, prims)?;
        let pin = read_pin(p)?;
        let id = p.element_id.clone().ok_or_else(invalid)?;
        let [x, y, z] = numbers::<3>(p, "xformOp:translate", "double3")?;
        let [w, qx, qy, qz] = numbers::<4>(p, "xformOp:orient", "quatd")?;
        let correction = format!("{}/{CORRECTION}", p.path);
        let units = prims
            .iter()
            .find(|q| q.path == correction)
            .map(|c| read_units(c, &pin))
            .transpose()?
            .flatten();
        let source_root = format!("{correction}/{SOURCE}");
        let below = format!("{source_root}/");
        let foreign = prims
            .iter()
            .filter(|q| q.path == source_root || q.path.starts_with(&below))
            .map(|q| ForeignPrim {
                path: q.path.clone(),
                source_path: format!(
                    "/{}{}",
                    pin.default_prim,
                    q.path.get(source_root.len()..).unwrap_or_default()
                ),
                type_name: q.type_name.clone(),
            })
            .collect();
        views.push(ReferenceView {
            id,
            path: p.path.clone(),
            name: text(p, "bim:name", "string")?.to_owned(),
            placement: Placement {
                translation_mm: [x, y, z],
                orientation_xyzw: [qx, qy, qz, w],
            },
            source: pin,
            units,
            source_root,
            foreign,
        });
    }
    views.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(views)
}

/// Refuses an arc holder that does not belong to a validated wrapper. A prim named
/// `<element>/Correction/Source` that composes foreign content is only legitimate below an owned
/// element of kind `reference` and geometry mode `reference`; a half-faked wrapper (another
/// geometry mode or kind, partly foreign element, missing parts) would otherwise hide a foreign
/// tree from the wrapper validator and from the read-only scans, which are keyed by the wrapper.
pub(super) fn stray_holders(prims: &[Prim]) -> Result<(), Diagnostic> {
    let holders = source_paths(prims);
    for p in prims {
        if !p.path.ends_with(&format!("/{CORRECTION}/{SOURCE}")) {
            continue;
        }
        if p.sources.iter().any(|s| !s.owned) && !holders.contains(&p.path) {
            return Err(foreign_write());
        }
    }
    Ok(())
}
