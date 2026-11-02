// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

use super::disciplines::is_part_kind;
use super::*;

pub(super) fn schema(pin: SchemaIdentity) -> Result<(), Diagnostic> {
    if pin != schema_identity()? {
        return Err(fail(
            ErrorCode::UnsupportedVersion,
            "authored schema identity mismatch",
        ));
    }
    Ok(())
}
pub(super) fn input_bounds(input: &Prepare, b: Budgets) -> Result<(), Diagnostic> {
    let mut bytes = 0usize;
    for change in &input.changes {
        // A schema change names no element: its bounded definition or name is the whole payload.
        match change {
            Change::AttrDefine { def } => {
                def.validate().map_err(|_| invalid())?;
                let size = def.name.len()
                    + def.unit.as_ref().map_or(0, |s| s.len())
                    + def.set.as_ref().map_or(0, |s| s.len())
                    + def.options.iter().map(|o| o.len()).sum::<usize>();
                preflight::charge(&mut bytes, size, 1024 * 1024)?;
                continue;
            }
            Change::AttrUndefine { name } => {
                schema_path_name(name)?;
                preflight::charge(&mut bytes, name.len(), 1024 * 1024)?;
                continue;
            }
            _ => {}
        }
        if !uuid7(change.key().0) {
            return Err(fail(ErrorCode::InvalidId, "invalid full element identity")
                .with_rule("DV-BASE-001"));
        }
        preflight::charge(&mut bytes, 256, 1024 * 1024)?;
        let mut count = |n: usize, size: usize| -> Result<(), Diagnostic> {
            if n > b.array_items {
                return Err(limit());
            }
            preflight::charge(
                &mut bytes,
                n.checked_mul(size).ok_or_else(limit)?,
                1024 * 1024,
            )
        };
        match change {
            Change::Create(e) => {
                if e.name.len() > 256 {
                    return Err(limit().with_rule("DV-BASE-003"));
                }
                if let Shape::Discipline(hook) = &e.shape {
                    hook.0.charge(&mut count)?;
                }
            }
            Change::Label { name, .. } if name.len() > 256 => {
                return Err(limit().with_rule("DV-BASE-003"));
            }
            Change::AttrSet { name, value, .. } => {
                // The value's own bounds are model admission; charge its worst stored size.
                let size = name.len()
                    + 8
                    + match value {
                        Some(v) => v.as_text().map_or(8, str::len),
                        None => 0,
                    };
                preflight::charge(&mut bytes, size, 1024 * 1024)?;
            }
            Change::Mark { mark: m, .. } => mark(m)?,
            Change::AttachReference(r) if r.name.len() > 256 => {
                return Err(limit().with_rule("DV-BASE-003"));
            }
            Change::Discipline(hook) => hook.0.charge(&mut count)?,
            Change::Mesh(m) => {
                count(m.points_mm.len().checked_mul(3).ok_or_else(limit)?, 4)?;
                count(m.face_counts.len(), 4)?;
                count(m.indices.len(), 4)?;
            }
            _ => (),
        }
    }
    Ok(())
}
/// Refuses the first non-finite value; the seam's finiteness gate.
pub fn finite(values: impl IntoIterator<Item = f64>) -> Result<(), Diagnostic> {
    if values.into_iter().any(|n| !n.is_finite()) {
        return Err(
            fail(ErrorCode::NonFiniteNumber, "non-finite authored value").with_rule("DV-ATTR-003"),
        );
    }
    Ok(())
}
/// Caps every value's magnitude; the seam's magnitude gate.
pub fn cap(values: impl IntoIterator<Item = f64>, max: f64) -> Result<(), Diagnostic> {
    if values.into_iter().any(|n| n.abs() > max) {
        return Err(limit().with_rule("DV-LIM-001"));
    }
    Ok(())
}
pub(super) fn label(s: &str) -> Result<(), Diagnostic> {
    if s.len() > 256 {
        return Err(limit().with_rule("DV-BASE-003"));
    }
    if s.is_empty() || s.chars().any(label_char_forbidden) {
        return Err(invalid().with_rule("DV-BASE-003"));
    }
    Ok(())
}
/// A user-attribute name, checked on the candidate and at every write (I04).
pub(super) fn schema_path_name(name: &str) -> Result<&str, Diagnostic> {
    if beamcraft_model::attribute::attr_name_is_valid(name) {
        Ok(name)
    } else {
        Err(invalid())
    }
}
pub(super) fn pose(p: &Placement) -> Result<(), Diagnostic> {
    cap(p.translation_mm, 1e9)?;
    cap(p.orientation_xyzw, 1.0)?;
    let [x, y, z, w] = p.orientation_xyzw;
    let norm = ((x * x + y * y) + z * z) + w * w;
    if (norm - 1.0).abs() > 1e-6 {
        return Err(invalid());
    }
    Ok(())
}
/// Admits one definition. The platform's own material runs its spec; a discipline's definition
/// is answered by its own hook, so the platform never matches on a discipline's kind.
pub fn definition(d: &Definition) -> Result<(), Diagnostic> {
    let refused = |e: beamcraft_model::command::CommandError| Diagnostic {
        code: e.code,
        message: "definition value refused by the F04 admission rules",
        rule_id: e.rule_id,
    };
    match d {
        Definition::Material {
            density_kg_m3,
            grade,
            standard,
            display_name,
        } => beamcraft_model::definition::MaterialSpec {
            density_kg_m3: *density_kg_m3,
            grade: grade.clone(),
            standard: standard.clone(),
            display_name: display_name.clone(),
        }
        .validate()
        .map_err(refused),
        Definition::Discipline(hook) => hook.0.validate(),
    }
}
/// Reads the logical definition of a definition prim: the platform's own material kind directly,
/// any other kind through the registered disciplines. A kind no discipline owns has no reader,
/// so the prim is refused. A stored layer is untrusted input: the definition is re-admitted,
/// never trusted.
fn read_definition(p: &Prim, kind: &str) -> Result<Definition, Diagnostic> {
    if kind == "material" {
        return Ok(Definition::Material {
            density_kg_m3: optional_number(p, "bim:densityKgM3")?,
            grade: optional_text(p, "bim:materialGrade")?,
            standard: optional_text(p, "bim:materialStandard")?,
            display_name: p.display_name.clone(),
        });
    }
    for discipline in disciplines::registered() {
        if let Some(definition) = discipline.read_definition(p, kind)? {
            return Ok(definition);
        }
    }
    Err(unsupported())
}
/// An optional property of an exact USD type. A property declared with another type, blocked or
/// without a value is invalid; only an absent one is unknown.
pub fn optional<'a>(prim: &'a Prim, name: &str, ty: &str) -> Result<Option<&'a Value>, Diagnostic> {
    match prim.properties.iter().find(|p| p.name == name) {
        None => Ok(None),
        Some(p) if p.type_name == ty => match &p.value {
            Some(v) if !matches!(v, Value::Block) => Ok(Some(v)),
            _ => Err(invalid()),
        },
        Some(_) => Err(invalid()),
    }
}
/// An optional `double` property; the seam's optional-number read.
pub fn optional_number(prim: &Prim, name: &str) -> Result<Option<f64>, Diagnostic> {
    match optional(prim, name, "double")? {
        None => Ok(None),
        Some(Value::Number(n)) => Ok(Some(*n)),
        Some(_) => Err(invalid()),
    }
}
/// An optional `string` property; the seam's optional-text read.
pub fn optional_text(prim: &Prim, name: &str) -> Result<Option<String>, Diagnostic> {
    match optional(prim, name, "string")? {
        None => Ok(None),
        Some(Value::Text(s)) => Ok(Some(s.clone())),
        Some(_) => Err(invalid()),
    }
}
/// The user-attribute schema scope (I04): every owned definition prim must be one bounded
/// segment below the scope and a complete, well-formed definition; foreign or unknown content
/// stays inert and is never interpreted.
pub(super) fn attribute_schema(prims: &[Prim]) -> Result<(), Diagnostic> {
    let scope = format!("{}/", super::ATTR_SCHEMA_PATH);
    for p in prims.iter().filter(|p| p.path.starts_with(&scope)) {
        if p.sources.iter().any(|s| !s.owned) {
            continue;
        }
        let name = p.path.strip_prefix(&scope).ok_or_else(invalid)?;
        schema_path_name(name)?;
        if name.contains('/') {
            return Err(unsupported());
        }
        let ty = beamcraft_model::attribute::AttrType::parse(text(p, "bim:attr:type", "token")?)
            .ok_or_else(unsupported)?;
        let options = match optional(p, "bim:attr:options", "string[]")? {
            None => Vec::new(),
            Some(Value::Array(items)) => {
                let mut options = Vec::with_capacity(items.len());
                for item in items {
                    match item {
                        Value::Text(option) => options.push(option.clone()),
                        _ => return Err(invalid()),
                    }
                }
                options
            }
            Some(_) => return Err(invalid()),
        };
        let def = beamcraft_model::attribute::AttrDef {
            name: name.to_owned(),
            ty,
            unit: optional_text(p, "bim:attr:unit")?,
            options,
            set: optional_text(p, "bim:attr:set")?,
        };
        def.validate().map_err(|_| invalid())?;
    }
    Ok(())
}
pub(super) fn owned(prims: &[Prim], path: &str, descendants: bool) -> Result<(), Diagnostic> {
    let prefix = format!("{path}/");
    // Ancestors and descendants matter: defining a child/over cannot bypass foreign provenance.
    for prim in prims.iter().filter(|p| {
        p.path == path
            || path.starts_with(&format!("{}/", p.path))
            || (descendants && p.path.starts_with(&prefix))
    }) {
        if prim.sources.is_empty() || prim.sources.iter().any(|s| !s.owned) {
            return Err(fail(
                ErrorCode::ForeignReadOnly,
                "foreign composition is read-only",
            ));
        }
    }
    Ok(())
}
/// A required property of an exact USD type; the seam's typed property read.
pub fn required<'a>(prim: &'a Prim, name: &str, ty: &str) -> Result<&'a Value, Diagnostic> {
    let p = prim
        .properties
        .iter()
        .find(|p| p.name == name)
        .ok_or_else(|| invalid().with_rule("DV-ATTR-001"))?;
    if p.type_name != ty {
        return Err(invalid().with_rule("DV-ATTR-002"));
    }
    p.value
        .as_ref()
        .filter(|v| !matches!(v, Value::Block))
        .ok_or_else(|| invalid().with_rule("DV-ATTR-001"))
}
/// A required property read as exact text; the seam's token/string read.
pub fn text<'a>(prim: &'a Prim, name: &str, ty: &str) -> Result<&'a str, Diagnostic> {
    match required(prim, name, ty)? {
        Value::Text(s) => Ok(s),
        _ => Err(invalid().with_rule("DV-ATTR-002")),
    }
}
/// A required `double` property; the seam's number read.
pub fn number(prim: &Prim, name: &str) -> Result<f64, Diagnostic> {
    match required(prim, name, "double")? {
        Value::Number(n) => Ok(*n),
        _ => Err(invalid()),
    }
}
/// A required `double3[]` property as point triples; the seam's point-array read.
pub fn points(prim: &Prim, name: &str) -> Result<Vec<[f64; 3]>, Diagnostic> {
    let Value::Array(values) = required(prim, name, "double3[]")? else {
        return Err(invalid());
    };
    values
        .iter()
        .map(|v| {
            let Value::Array(items) = v else {
                return Err(invalid());
            };
            match items.as_slice() {
                [Value::Number(x), Value::Number(y), Value::Number(z)] => Ok([*x, *y, *z]),
                _ => Err(invalid()),
            }
        })
        .collect()
}
pub(super) fn vec3s(value: &Value) -> Result<Vec<[f32; 3]>, Diagnostic> {
    let Value::Array(values) = value else {
        return Err(invalid());
    };
    values
        .iter()
        .map(|v| {
            let Value::Array(items) = v else {
                return Err(invalid());
            };
            match items.as_slice() {
                [Value::Number(x), Value::Number(y), Value::Number(z)] => {
                    Ok([*x as f32, *y as f32, *z as f32])
                }
                _ => Err(invalid()),
            }
        })
        .collect()
}
/// Finds one composed relationship, if authored; the seam's relationship read.
pub fn relationship<'a>(prim: &'a Prim, name: &str) -> Option<&'a Relationship> {
    prim.relationships.iter().find(|r| r.name == name)
}
/// Verifies a stored definition prim at a digest: its kind, canonical bytes and pinned digest.
/// The seam a discipline hook resolves and re-checks pinned definitions through.
pub fn resource<'a>(
    prims: &'a [Prim],
    digest: ContentDigest,
    kinds: &[&str],
    category: DefinitionCategory,
) -> Result<&'a Prim, Diagnostic> {
    let path = definition_path(digest, category);
    let p = prims
        .iter()
        .find(|p| p.path == path && p.active)
        .ok_or_else(|| {
            fail(
                ErrorCode::DanglingRelationship,
                "definition target unavailable",
            )
        })?;
    let kind = text(p, "bim:definitionKind", "token")?;
    if text(p, "bim:contentDigest", "string")? != digest.to_string() || !kinds.contains(&kind) {
        return Err(corrupt());
    }
    if text(p, "bim:schemaVersion", "string")? != "0.1.0" || text(p, "bim:units", "token")? != "mm"
    {
        return Err(corrupt());
    }
    let definition = read_definition(p, kind)?;
    if Resource::new(definition)?.digest != digest {
        return Err(corrupt());
    }
    Ok(p)
}
/// Resolves a definition relationship of a prim to its stored, verified definition prim; the
/// seam a discipline hook follows `bim:profile` and `bim:material` links with.
pub fn link<'a>(
    p: &Prim,
    prims: &'a [Prim],
    name: &str,
    kinds: &[&str],
    required: bool,
) -> Result<Option<&'a Prim>, Diagnostic> {
    let targets = relationship(p, name)
        .map(|r| r.targets.as_slice())
        .unwrap_or_default();
    match targets {
        [] if !required => Ok(None),
        [target] => {
            let Some((category, digest)) = DefinitionCategory::parse_path(target) else {
                return Err(invalid());
            };
            if target != &definition_path(digest, category)
                || category
                    != if name == "bim:material" {
                        DefinitionCategory::Material
                    } else {
                        DefinitionCategory::Profile
                    }
            {
                return Err(invalid());
            }
            Ok(Some(resource(prims, digest, kinds, category)?))
        }
        _ => Err(fail(
            ErrorCode::DanglingRelationship,
            "missing or ambiguous definition target",
        )),
    }
}

/// The rigid placement an owned element carries: translation, orientation and the exact
/// `[translate, orient]` operation order. Shared by plates, members and reference wrappers.
pub(super) fn owned_pose(p: &Prim) -> Result<(), Diagnostic> {
    let translation = match required(p, "xformOp:translate", "double3")? {
        Value::Array(v) => match v.as_slice() {
            [Value::Number(x), Value::Number(y), Value::Number(z)] => [*x, *y, *z],
            _ => return Err(invalid()),
        },
        _ => return Err(invalid()),
    };
    let orientation = match required(p, "xformOp:orient", "quatd")? {
        Value::Array(v) => match v.as_slice() {
            [
                Value::Number(w),
                Value::Number(x),
                Value::Number(y),
                Value::Number(z),
            ] => [*x, *y, *z, *w],
            _ => return Err(invalid()),
        },
        _ => return Err(invalid()),
    };
    pose(&Placement {
        translation_mm: translation,
        orientation_xyzw: orientation,
    })?;
    if required(p, "xformOpOrder", "token[]")?
        != &Value::Array(vec![
            Value::Text("xformOp:translate".into()),
            Value::Text("xformOp:orient".into()),
        ])
    {
        return Err(unsupported());
    }
    Ok(())
}

pub(super) fn snapshot(
    prims: &[Prim],
    metadata: &BTreeMap<String, Value>,
) -> Result<Validation, Diagnostic> {
    reference::stray_holders(prims)?;
    for (path, kind) in [
        ("/World", "assembly"),
        ("/World/Steel", "group"),
        ("/World/Steel/Parts", "group"),
        ("/World/Steel/Assemblies", "group"),
        ("/World/Steel/Connections", "group"),
        ("/World/Levels", "group"),
        ("/World/Grids", "group"),
        ("/World/References", "group"),
    ] {
        if let Some(p) = prims
            .iter()
            .find(|p| p.path == path && p.sources.iter().all(|s| s.owned))
            && p.kind.as_deref().is_some_and(|k| k != kind)
        {
            return Err(invalid());
        }
    }
    if let Some(Value::Dictionary(data)) = metadata.get("customLayerData")
        && let Some(pin) = data.get("bimSchemaDigest")
        && !matches!(pin, Value::Text(s) if pin_accepted(s)?)
    {
        return Err(fail(
            ErrorCode::UnsupportedVersion,
            "layer schema digest mismatch",
        ));
    }
    project_record(metadata)?;
    // The F12 descriptor is optional; when present it is validated as a whole, and its project
    // identity can never disagree with the root layer's own project record.
    let descriptor = super::descriptor::descriptor_of(metadata)?;
    if let Some(descriptor) = descriptor
        && let Some(record) = project_record(metadata)?
        && descriptor.project.to_string() != record
    {
        return Err(fail(
            ErrorCode::IntegrityError,
            "the project descriptor names another project",
        ));
    }
    let mut operation_ids = std::collections::BTreeSet::new();
    attribute_schema(prims)?;
    for p in prims.iter().filter(|p| p.element_id.is_some()) {
        let id = p.element_id.as_deref().ok_or_else(invalid)?;
        // Foreign content is preserved inertly, not interpreted as engineering input.
        if p.sources.iter().any(|s| !s.owned) {
            continue;
        }
        // Loaded newer schemas, unknown kinds and geometry modes stay inert/read-only.
        // The mutation guard independently refuses editing them.
        if text(p, "bim:schemaVersion", "string")? != "0.1.0" {
            continue;
        }
        crate::is_deactivated(p)?;
        if text(p, "bim:geometryMode", "token")? == "reference" {
            if text(p, "bim:elementKind", "token")? != "reference" {
                return Err(unsupported());
            }
            // A foreign wrapper is no storey member: a level record on it is outside D2.
            if relationship(p, "bim:level").is_some_and(|r| !r.targets.is_empty()) {
                return Err(fail(
                    ErrorCode::InvalidArgument,
                    "only a plate, beam or column is assigned to a level",
                )
                .with_rule("DV-REL-003"));
            }
            reference::element(p, prims)?;
            continue;
        }
        if text(p, "bim:geometryMode", "token")? != "parametric" {
            continue;
        }
        let element_kind = text(p, "bim:elementKind", "token")?;
        if !(is_part_kind(element_kind)
            || matches!(element_kind, "assembly" | "connection" | "level" | "grid"))
        {
            continue;
        }
        if p.path
            != element_path(
                id,
                ElementCategory::from_kind(text(p, "bim:elementKind", "token")?)
                    .ok_or_else(invalid)?,
            )?
        {
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
        let kind = text(p, "bim:elementKind", "token")?;
        let model_kind = if kind == "assembly" {
            "assembly"
        } else {
            "component"
        };
        if p.kind.as_deref() != Some(model_kind) || p.display_name.as_deref() != Some(name) {
            return Err(invalid());
        }
        if p.type_name.as_deref() != Some("Xform") {
            return Err(unsupported());
        }
        if !is_part_kind(kind) && p.properties.iter().any(|p| p.name == "steel:class") {
            return Err(unsupported());
        }
        if is_part_kind(kind)
            && let Some(prop) = p.properties.iter().find(|a| a.name == "steel:class")
            && (prop.type_name != "int"
                || !matches!(prop.value, Some(Value::Int(v)) if i32::try_from(v).is_ok()))
        {
            return Err(invalid());
        }
        if instance::definition(p, prims)?.is_none() {
            if is_part_kind(kind) {
                disciplines::validate_part(kind, p, prims, &mut operation_ids)?;
            }
            match kind {
                k if is_part_kind(k) => {}
                // An assembly has no shape inputs and no material of its own; a connection is a weld
                // (A06) or a bolt group (A05) with its own properties and no material either.
                "grid" => {
                    grid::read(p)?;
                    if relationship(p, "bim:material").is_some_and(|r| !r.targets.is_empty()) {
                        return Err(invalid());
                    }
                }
                "assembly" | "connection" | "level" => {
                    if relationship(p, "bim:material").is_some_and(|r| !r.targets.is_empty()) {
                        return Err(invalid());
                    }
                    if kind == "connection" {
                        disciplines::validate_connection(p, prims)?;
                    }
                    if kind == "level" {
                        elevation(number(p, "bim:elevationMm")?)?;
                    }
                }
                _ => continue, // Unknown vocabulary remains inert; edits below refuse it.
            }
            if let Some(material) = link(p, prims, "bim:material", &["material"], false)? {
                definition(&read_definition(material, "material")?)?;
            }
        }
        owned_pose(p)?;
        // The placement of an assembly, a weld and a level is the identity in this slice.
        if matches!(kind, "assembly" | "connection" | "level")
            && (required(p, "xformOp:translate", "double3")?
                != &Value::Array(vec![Value::Number(0.0); 3])
                || required(p, "xformOp:orient", "quatd")?
                    != &Value::Array(vec![
                        Value::Number(1.0),
                        Value::Number(0.0),
                        Value::Number(0.0),
                        Value::Number(0.0),
                    ]))
        {
            return Err(unsupported());
        }
    }
    grid::read_grids(prims)?;
    instance::cap(prims)?;
    part_of(prims)?;
    main_part(prims)?;
    disciplines::validate_connections(prims)?;
    level_links(prims)?;
    marks(prims)?;
    for p in prims
        .iter()
        .filter(|p| p.type_name.as_deref() == Some("Mesh") && p.sources.iter().all(|s| s.owned))
    {
        let Some((parent, "Geometry")) = p.path.rsplit_once('/') else {
            continue;
        };
        let Some(element) = prims
            .iter()
            .find(|p| p.path == parent && p.element_id.is_some())
        else {
            continue;
        };
        // An assembly, a level or a weld without a path has no body: a Geometry mesh under one
        // is refused, authored or loaded. A grid carries only derived guide meshes (T01.7);
        // only the A15 weld with an authored path carries a derived solid.
        let kind = supported_kind(element);
        let bodyless = matches!(kind, Some("assembly" | "level"))
            || (kind == Some("connection")
                && !element
                    .properties
                    .iter()
                    .any(|v| v.name == "steel:weld:pathMm"));
        if bodyless {
            return Err(unsupported());
        }
        let id = element.element_id.clone().ok_or_else(invalid)?;
        let key = text(p, "bim:shapeKey", "string")?
            .parse::<ContentDigest>()
            .map_err(|_| invalid())?;
        let Value::Array(values) = required(p, "points", "point3f[]")? else {
            return Err(invalid());
        };
        let points = values
            .iter()
            .map(|v| {
                let Value::Array(v) = v else {
                    return Err(invalid());
                };
                match v.as_slice() {
                    [Value::Number(x), Value::Number(y), Value::Number(z)] => {
                        Ok([*x as f32, *y as f32, *z as f32])
                    }
                    _ => Err(invalid()),
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        let integers = |name| -> Result<Vec<i32>, Diagnostic> {
            let Value::Array(values) = required(p, name, "int[]")? else {
                return Err(invalid());
            };
            values
                .iter()
                .map(|v| match v {
                    Value::Int(n) => i32::try_from(*n).map_err(|_| invalid()),
                    _ => Err(invalid()),
                })
                .collect()
        };
        let result = Mesh {
            id,
            shape_key: key,
            points_mm: points,
            face_counts: integers("faceVertexCounts")?,
            indices: integers("faceVertexIndices")?,
            quantities: None,
        };
        mesh(&result, Budgets::default())?;
        if p.kind.as_deref() != Some("subcomponent") {
            return Err(invalid());
        }
        let expected = derive::extent(&result.points_mm).ok_or_else(invalid)?;
        let stored = vec3s(required(p, "extent", "float3[]")?)?;
        if stored.len() != 2 || stored.iter().zip(expected).any(|(s, e)| *s != e) {
            return Err(invalid());
        }
        let expected =
            derive::face_normals(&result.points_mm, &result.face_counts, &result.indices)
                .ok_or_else(invalid)?;
        let stored = vec3s(required(p, "normals", "normal3f[]")?)?;
        // Per-face data read as per-vertex data (the USD default) would mis-shade every reader.
        let interpolation = p
            .properties
            .iter()
            .find(|v| v.name == "normals")
            .and_then(|v| v.interpolation.as_deref());
        if interpolation != Some("uniform") {
            return Err(invalid());
        }
        if stored.len() != expected.len()
            || stored.iter().zip(&expected).any(|(s, e)| {
                s.iter()
                    .zip(e)
                    .any(|(s, e)| (s - e).abs() > derive::NORMAL_TOLERANCE)
            })
        {
            return Err(invalid());
        }
        if text(p, "subdivisionScheme", "token")? != "none"
            || text(p, "orientation", "token")? != "rightHanded"
        {
            return Err(unsupported());
        }
    }
    Ok(Validation {
        profile: "u02-structural-v1",
        rules: &[
            "U02-V01:U01-integrity-budgets-units-provenance-finite",
            "U02-V02:schema-pin-and-supported-field-types",
            "U02-V03:full-id-path-and-required-inputs",
            "U02-V04:definition-target-kind-and-digest-label",
            "U02-V05:scalar-caps-dimensions-scaled-minima-pose",
            "U02-V06:standard-mesh-buffer-structure",
            "A01-V01:part-of-kinds-cardinality-self-link-and-cycles",
            "A16-V01:main-part-on-an-assembly-one-per-assembly-direct-part",
            "A06-V01:weld-properties-and-connects-two-distinct-parts",
            "A05-V01:bolt-properties-and-connects-two-distinct-parts",
            "D2-V01:level-elevation-and-level-kinds-cardinality-and-dangling",
            "T01-V01:grid-arrays-expression-frame-and-project-axis-identities",
        ],
        prims: prims.len(),
    })
}

/// The kind token of an owned, supported 0.1.0 element prim, or `None` for content that stays inert.
pub fn supported_kind(p: &Prim) -> Option<&str> {
    if p.element_id.is_none() || p.sources.is_empty() || p.sources.iter().any(|s| !s.owned) {
        return None;
    }
    let token = |name: &str| match required(p, name, "token").ok()? {
        Value::Text(s) => Some(s.as_str()),
        _ => None,
    };
    let version = match required(p, "bim:schemaVersion", "string").ok()? {
        Value::Text(s) => s.as_str(),
        _ => return None,
    };
    if version != "0.1.0" || token("bim:geometryMode")? != "parametric" {
        return None;
    }
    token("bim:elementKind")
        .filter(|k| is_part_kind(k) || matches!(*k, "assembly" | "connection" | "level" | "grid"))
}

/// The values of an A09 mark: a series of at most 16 letters, digits, `_` and `-`, a number in
/// `0..=i32::MAX` and a non-empty printable-ASCII approval record of at most 512 bytes.
fn mark_values(series: &str, number: i64, basis: &str) -> Result<(), Diagnostic> {
    if series.chars().count() > MARK_MAX_SERIES
        || !series
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        || !(0..=i64::from(i32::MAX)).contains(&number)
        || basis.is_empty()
        || basis.len() > MARK_MAX_BASIS
        || !basis.bytes().all(|b| (0x20..=0x7e).contains(&b))
    {
        return Err(invalid());
    }
    Ok(())
}

/// Checks a mark about to be authored.
pub(super) fn mark(m: &Mark) -> Result<(), Diagnostic> {
    mark_values(&m.series, i64::from(m.number), &m.basis)
}

/// A09 marks on loaded or authored elements: only a plate, beam, column or assembly carries one,
/// and it carries all three properties with their exact types, or none. A partial or malformed
/// record is refused, never read as "unnumbered".
fn marks(prims: &[Prim]) -> Result<(), Diagnostic> {
    for p in prims {
        let Some(kind) = supported_kind(p) else {
            continue;
        };
        let series = optional(p, MARK_SERIES, "string")?;
        let number = optional(p, MARK_NUMBER, "int")?;
        let basis = optional(p, MARK_BASIS, "string")?;
        match (series, number, basis) {
            (None, None, None) => {}
            (Some(Value::Text(series)), Some(number), Some(Value::Text(basis))) => {
                if !(is_part_kind(kind) || matches!(kind, "assembly")) {
                    return Err(unsupported());
                }
                let number = match number {
                    Value::Int(n) => *n,
                    Value::Uint(n) => i64::try_from(*n).map_err(|_| invalid())?,
                    _ => return Err(invalid()),
                };
                mark_values(series, number, basis)?;
            }
            _ => return Err(invalid()),
        }
    }
    Ok(())
}

/// The elevation of a loaded or authored level: finite and within the coordinate cap.
fn elevation(mm: f64) -> Result<(), Diagnostic> {
    finite([mm])?;
    cap([mm], 1e9)
}

/// D2 `level` records (F10 relationships, domain vocabulary section 5.1 pattern): only a plate,
/// beam or column carries a `bim:level` record, with at most one target (`DV-REL-004`,
/// `DEPENDENCY_CONFLICT`), which must be an active owned level (`DV-REL-001` dangling,
/// `DV-REL-003` kind). A level itself carries no `bim:level`, `bim:partOf`, `bim:connects` or
/// `bim:profile` record.
fn level_links(prims: &[Prim]) -> Result<(), Diagnostic> {
    let elements: BTreeMap<&str, (&Prim, &str)> = prims
        .iter()
        .filter_map(|p| supported_kind(p).map(|k| (p.path.as_str(), (p, k))))
        .collect();
    for &(p, kind) in elements.values() {
        if kind == "level" {
            for name in ["bim:level", "bim:partOf", "bim:profile"] {
                if relationship(p, name).is_some_and(|r| !r.targets.is_empty()) {
                    return Err(fail(
                        ErrorCode::InvalidArgument,
                        "a level carries no element or definition links",
                    )
                    .with_rule("DV-REL-003"));
                }
            }
        }
        let Some(r) = relationship(p, "bim:level") else {
            continue;
        };
        let target = match r.targets.as_slice() {
            [] => continue,
            [target] => target.as_str(),
            _ => {
                return Err(fail(
                    ErrorCode::DependencyConflict,
                    "an element is assigned to at most one level",
                )
                .with_rule("DV-REL-004"));
            }
        };
        if !is_part_kind(kind) {
            return Err(fail(
                ErrorCode::InvalidArgument,
                "only a plate, beam or column is assigned to a level",
            )
            .with_rule("DV-REL-003"));
        }
        match elements.get(target) {
            Some((t, "level")) if t.active => {}
            Some((t, _)) if !t.active => {
                return Err(fail(
                    ErrorCode::DanglingRelationship,
                    "level target is not active",
                )
                .with_rule("DV-REL-001"));
            }
            Some(_) => {
                return Err(
                    fail(ErrorCode::InvalidArgument, "level target is not a level")
                        .with_rule("DV-REL-003"),
                );
            }
            None if prims
                .iter()
                .any(|p| p.path == target && p.element_id.is_some()) =>
            {
                return Err(fail(
                    ErrorCode::InvalidArgument,
                    "level target is not a supported owned level",
                )
                .with_rule("DV-REL-003"));
            }
            None => {
                return Err(
                    fail(ErrorCode::DanglingRelationship, "level target unavailable")
                        .with_rule("DV-REL-001"),
                );
            }
        }
    }
    Ok(())
}

/// A01 `part-of` records (domain vocabulary section 5.1): a supported element has at most one
/// target (`DV-REL-004`, `DEPENDENCY_CONFLICT`), never itself (`DV-REL-006`), which must be an
/// active owned assembly (`DV-REL-001` dangling, `DV-REL-003` kind), and the records form no cycle
/// of any length (`DV-REL-005`, `DEPENDENCY_CONFLICT`). The walk is linear in the element count.
fn part_of(prims: &[Prim]) -> Result<(), Diagnostic> {
    let elements: BTreeMap<&str, (&Prim, &str)> = prims
        .iter()
        .filter_map(|p| supported_kind(p).map(|k| (p.path.as_str(), (p, k))))
        .collect();
    let mut parent: BTreeMap<&str, &str> = BTreeMap::new();
    for (&path, &(p, _)) in &elements {
        let Some(r) = relationship(p, "bim:partOf") else {
            continue;
        };
        let target = match r.targets.as_slice() {
            [] => continue,
            [target] => target.as_str(),
            _ => {
                return Err(fail(
                    ErrorCode::DependencyConflict,
                    "an element is a part of at most one assembly",
                )
                .with_rule("DV-REL-004"));
            }
        };
        if target == path {
            return Err(fail(
                ErrorCode::InvalidArgument,
                "an element cannot be a part of itself",
            )
            .with_rule("DV-REL-006"));
        }
        match elements.get(target) {
            Some((t, "assembly")) if t.active => {}
            Some((t, _)) if !t.active => {
                return Err(fail(
                    ErrorCode::DanglingRelationship,
                    "part-of target is not active",
                )
                .with_rule("DV-REL-001"));
            }
            Some(_) => {
                return Err(fail(
                    ErrorCode::InvalidArgument,
                    "part-of target is not an assembly",
                )
                .with_rule("DV-REL-003"));
            }
            None if prims
                .iter()
                .any(|p| p.path == target && p.element_id.is_some()) =>
            {
                return Err(fail(
                    ErrorCode::InvalidArgument,
                    "part-of target is not a supported owned assembly",
                )
                .with_rule("DV-REL-003"));
            }
            None => {
                return Err(fail(
                    ErrorCode::DanglingRelationship,
                    "part-of target unavailable",
                )
                .with_rule("DV-REL-001"));
            }
        }
        parent.insert(path, target);
    }
    // 0 unvisited, 1 on the current chain, 2 proven acyclic.
    let mut state: BTreeMap<&str, u8> = BTreeMap::new();
    for &start in parent.keys() {
        let mut chain = Vec::new();
        let mut at = Some(start);
        while let Some(node) = at {
            match state.get(node).copied().unwrap_or(0) {
                2 => break,
                1 => {
                    return Err(fail(
                        ErrorCode::DependencyConflict,
                        "part-of records form a cycle",
                    )
                    .with_rule("DV-REL-005"));
                }
                _ => {
                    state.insert(node, 1);
                    chain.push(node);
                    at = parent.get(node).copied();
                }
            }
        }
        for node in chain {
            state.insert(node, 2);
        }
    }
    Ok(())
}

/// A16.1 `mainPart` records: only an assembly carries `bim:mainPart`, with at most one target
/// (`DV-REL-004`), which must be an active owned element whose own `part-of` record names this
/// assembly — a direct part, whatever its supported kind (`DV-REL-001` dangling, `DV-REL-003`
/// kind). A record whose part is not a member is refused, never read as "no main part".
fn main_part(prims: &[Prim]) -> Result<(), Diagnostic> {
    let elements: BTreeMap<&str, (&Prim, &str)> = prims
        .iter()
        .filter_map(|p| supported_kind(p).map(|k| (p.path.as_str(), (p, k))))
        .collect();
    for &(p, kind) in elements.values() {
        let Some(r) = relationship(p, "bim:mainPart") else {
            continue;
        };
        let target = match r.targets.as_slice() {
            [] => continue,
            [target] => target.as_str(),
            _ => {
                return Err(fail(
                    ErrorCode::DependencyConflict,
                    "an assembly has at most one main part",
                )
                .with_rule("DV-REL-004"));
            }
        };
        if kind != "assembly" {
            return Err(fail(
                ErrorCode::InvalidArgument,
                "only an assembly carries a main-part record",
            )
            .with_rule("DV-REL-003"));
        }
        match elements.get(target) {
            // The main part must be a direct member: its own part-of record names this assembly.
            Some((t, _)) if t.active && part_of_target(t, p.path.as_str()) => {}
            Some((t, _)) if !t.active => {
                return Err(fail(
                    ErrorCode::DanglingRelationship,
                    "main-part target is not active",
                )
                .with_rule("DV-REL-001"));
            }
            Some(_) => {
                return Err(fail(
                    ErrorCode::InvalidArgument,
                    "main-part target is not a direct part of the assembly",
                )
                .with_rule("DV-REL-003"));
            }
            None if prims
                .iter()
                .any(|p| p.path == target && p.element_id.is_some()) =>
            {
                return Err(fail(
                    ErrorCode::InvalidArgument,
                    "main-part target is not a supported owned part",
                )
                .with_rule("DV-REL-003"));
            }
            None => {
                return Err(fail(
                    ErrorCode::DanglingRelationship,
                    "main-part target unavailable",
                )
                .with_rule("DV-REL-001"));
            }
        }
    }
    Ok(())
}

/// Whether the loaded `part-of` record of `part` names exactly `assembly` (an explicit empty
/// record is not a membership).
fn part_of_target(part: &Prim, assembly: &str) -> bool {
    relationship(part, "bim:partOf")
        .is_some_and(|r| matches!(r.targets.as_slice(), [target] if target.as_str() == assembly))
}

/// The `part-of` chain from `start` upwards: `start`, its assembly, that one's assembly, and so on.
/// The batch's own `PartOf` changes override the composed records. The walk stops at a repeated
/// element (a cycle, refused later by the candidate check) and is bounded by the prim count.
fn chain(prims: &[Prim], changes: &[Change], start: &str) -> Vec<String> {
    let id_of_path: BTreeMap<&str, &str> = prims
        .iter()
        .filter_map(|p| Some((p.path.as_str(), p.element_id.as_deref()?)))
        .collect();
    let mut parent: BTreeMap<String, Option<String>> = BTreeMap::new();
    for p in prims {
        let (Some(id), Some(r)) = (p.element_id.as_deref(), relationship(p, "bim:partOf")) else {
            continue;
        };
        let target = match r.targets.as_slice() {
            [target] => id_of_path.get(target.as_str()).map(|t| (*t).to_owned()),
            _ => None,
        };
        parent.insert(id.to_owned(), target);
    }
    for change in changes {
        if let Change::PartOf { id, assembly } = change {
            parent.insert(id.clone(), assembly.clone());
        }
    }
    let mut out: Vec<String> = Vec::new();
    let mut at = Some(start.to_owned());
    while let Some(node) = at {
        if out.contains(&node) || out.len() > prims.len().saturating_add(changes.len()) {
            break;
        }
        at = parent.get(&node).cloned().flatten();
        out.push(node);
    }
    out
}

/// The U19 scope admission of one change (`DV-SCOPE-002..004`): an edit must name an element
/// whose strongest authored provenance lies inside the authorized scopes, unknown provenance
/// gives no write permission, and common-data layers answer only to authorized shared commands.
/// An element this batch creates is admitted: it is authored in the scope's write layer.
pub(super) fn in_scope(
    change: &Change,
    existing: Option<&Prim>,
    scope: &EditScope,
) -> Result<(), Diagnostic> {
    if matches!(
        change,
        Change::AttrDefine { .. } | Change::AttrUndefine { .. }
    ) {
        return if scope.shared_authorized {
            Ok(())
        } else {
            Err(fail(
                ErrorCode::PermissionDenied,
                "the user-attribute schema is shared data",
            )
            .with_rule("DV-SCOPE-004"))
        };
    }
    if matches!(change, Change::Create(_) | Change::AttachReference(_)) {
        return Ok(());
    }
    // An element created in the same batch has no provenance yet; the kind checks below still
    // refuse a change that names no such create.
    let Some(prim) = existing else {
        return Ok(());
    };
    let layer = prim
        .sources
        .first()
        .map(|s| s.layer.as_str())
        .ok_or_else(|| {
            fail(
                ErrorCode::ForeignReadOnly,
                "unknown provenance gives no write permission",
            )
            .with_rule("DV-SCOPE-002")
        })?;
    if scope.allowed.contains(layer) {
        return Ok(());
    }
    if scope.shared.contains(layer) {
        return if scope.shared_authorized {
            Ok(())
        } else {
            Err(fail(
                ErrorCode::PermissionDenied,
                "grids and levels are shared data",
            )
            .with_rule("DV-SCOPE-004"))
        };
    }
    Err(fail(
        ErrorCode::ForeignReadOnly,
        "the element belongs to another section or zone",
    )
    .with_rule("DV-SCOPE-002"))
}

pub(super) fn changes(
    input: &Prepare,
    scope: Option<&EditScope>,
    prims: &[Prim],
    b: Budgets,
) -> Result<(BTreeSet<Footprint>, BTreeSet<Footprint>), Diagnostic> {
    let mut reads = BTreeSet::new();
    let mut writes = BTreeSet::new();
    for change in &input.changes {
        // A schema change targets no element: it reads and writes the schema as a whole, so two
        // schema edits conflict and neither conflicts with an element edit (I04). The candidate
        // snapshot re-validates the whole scope.
        if matches!(
            change,
            Change::AttrDefine { .. } | Change::AttrUndefine { .. }
        ) {
            if let Some(scope) = scope {
                in_scope(change, None, scope)?;
            }
            writes.insert(Footprint {
                identity: super::ATTR_SCHEMA_IDENTITY.to_owned(),
                group: Group::Lifecycle,
            });
            continue;
        }
        let (id, _) = change.key();
        let path = change_path(id, input, prims)?;
        let existing = prims.iter().find(|p| p.path == path);
        // A wrapper is owned although its `Source` descendant is foreign: only the wrapper's own
        // levels are checked here, and the loaded shape (`reference::element`) refuses every
        // owned opinion below `Source`. Every other element must be wholly owned.
        let wrapper = existing
            .is_some_and(|p| matches!(text(p, "bim:geometryMode", "token"), Ok("reference")));
        owned(prims, &path, !wrapper)?;
        if let Some(scope) = scope {
            in_scope(change, existing, scope)?;
        }
        if matches!(change, Change::Delete { .. })
            && input.changes.iter().filter(|c| c.key().0 == id).count() > 1
        {
            return Err(fail(
                ErrorCode::MalformedCommand,
                "a delete cannot be combined with other changes of the element",
            ));
        }
        match change {
            Change::Create(_) | Change::AttachReference(_) if existing.is_some() => {
                return Err(
                    fail(ErrorCode::DuplicateId, "element already exists").with_rule("DV-BASE-002")
                );
            }
            Change::Create(_) | Change::AttachReference(_) => (),
            // A mesh or part-of record of an element created in the same batch; the candidate
            // snapshot re-validates it as a whole.
            Change::Mesh(_)
            | Change::PartClass { .. }
            | Change::PartOf { .. }
            | Change::Level { .. }
                if existing.is_none()
                    && input
                        .changes
                        .iter()
                        .any(|c| matches!(c,Change::Create(e) if e.id == id)) => {}
            _ => {
                let p = existing.ok_or_else(|| fail(ErrorCode::NotFound, "element not found"))?;
                if p.element_id.as_deref() != Some(id) {
                    return Err(fail(ErrorCode::InvalidId, "element identity mismatch"));
                }
                if text(p, "bim:schemaVersion", "string")? != "0.1.0" {
                    return Err(fail(
                        ErrorCode::UnsupportedVersion,
                        "element schema version mismatch",
                    ));
                }
                let kind = text(p, "bim:elementKind", "token")?;
                let mode = text(p, "bim:geometryMode", "token")?;
                if mode != if wrapper { "reference" } else { "parametric" }
                    || (wrapper && kind != "reference")
                {
                    return Err(unsupported());
                }
                let valid = match change {
                    Change::PartClass { .. } => is_part_kind(kind),
                    Change::Grid { .. } => kind == "grid",
                    Change::Discipline(hook) => hook.0.targets_kind(kind),
                    Change::Mesh(m) if kind == "grid" => m.quantities.is_none(),
                    Change::ReferenceUnits { .. } => wrapper,
                    // A rename and a move are the only edits a wrapper takes as an element; an
                    // assembly can be renamed, deleted and be a part, never moved (identity pose).
                    Change::Placement { .. } => {
                        is_part_kind(kind) || matches!(kind, "reference" | "grid")
                    }
                    Change::Label { .. } => {
                        is_part_kind(kind)
                            || matches!(
                                kind,
                                "reference" | "assembly" | "connection" | "level" | "grid"
                            )
                    }
                    Change::Delete { .. } => {
                        is_part_kind(kind)
                            || matches!(
                                kind,
                                "assembly"
                                    | "connection"
                                    | "level"
                                    | "grid"
                                    | "construction"
                                    | "annotation"
                            )
                    }
                    Change::PartOf { .. } => is_part_kind(kind) || matches!(kind, "assembly"),
                    // The main-part record lives on the assembly; only it is a source.
                    Change::MainPart { .. } => kind == "assembly",
                    Change::Level { .. } => is_part_kind(kind),
                    Change::Mark { .. } => is_part_kind(kind) || matches!(kind, "assembly"),
                    // A user attribute is carried by any element kind, including a wrapper.
                    Change::AttrSet { .. } => true,
                    _ => is_part_kind(kind),
                };
                if !valid {
                    return Err(unsupported());
                }
            }
        }
        let (r, w): (&[Group], &[Group]) = match change {
            Change::Create(_) => (
                &[Group::Lifecycle],
                &[
                    Group::Lifecycle,
                    Group::Label,
                    Group::ShapeInputs,
                    Group::Placement,
                    Group::Material,
                    Group::Relationship,
                    Group::Derived,
                ],
            ),
            Change::AttachReference(_) => (
                &[Group::Lifecycle],
                &[
                    Group::Lifecycle,
                    Group::Label,
                    Group::ShapeInputs,
                    Group::Placement,
                ],
            ),
            // A mark is stored with the label's group: it conflicts with a rename or another
            // mark of the element and with nothing derived.
            Change::Mark { .. } => (&[Group::Lifecycle, Group::Label], &[Group::Label]),
            // A value lives with its element and conflicts only with another change of the
            // element's attributes; no derived result depends on a user attribute.
            Change::AttrSet { .. } => {
                (&[Group::Lifecycle, Group::Attributes], &[Group::Attributes])
            }
            Change::PartClass { .. } | Change::Label { .. } => {
                (&[Group::Lifecycle, Group::Label], &[Group::Label])
            }
            Change::Grid { .. } => (
                &[Group::Lifecycle, Group::ShapeInputs, Group::Placement],
                &[Group::ShapeInputs],
            ),
            // The complete authored inputs and the `connects` record are replaced as a whole
            // (A14.3); a connection derives no body, so no derived group is written.
            Change::Discipline(hook) => hook.0.groups(),
            Change::Material { .. } => (
                &[Group::Lifecycle, Group::Derived],
                &[Group::Material, Group::Relationship, Group::Derived],
            ),
            Change::Placement { .. } => (
                &[Group::Lifecycle, Group::ShapeInputs, Group::Placement],
                &[Group::Placement, Group::Derived],
            ),
            Change::Mesh(_) => (&[Group::Lifecycle, Group::ShapeInputs], &[Group::Derived]),
            Change::PartOf { .. } | Change::MainPart { .. } | Change::Level { .. } => (
                &[Group::Lifecycle, Group::Relationship],
                &[Group::Relationship],
            ),
            Change::Delete { .. } => (
                &[Group::Lifecycle, Group::Relationship],
                &[
                    Group::Lifecycle,
                    Group::Label,
                    Group::ShapeInputs,
                    Group::Placement,
                    Group::Material,
                    Group::Relationship,
                    Group::Derived,
                ],
            ),
            _ => (
                &[Group::Lifecycle, Group::ShapeInputs],
                &[Group::ShapeInputs, Group::Derived],
            ),
        };
        for &group in r {
            reads.insert(Footprint {
                identity: id.into(),
                group,
            });
        }
        for &group in w {
            writes.insert(Footprint {
                identity: id.into(),
                group,
            });
        }
        if let Change::PartOf {
            assembly: Some(target),
            ..
        } = change
        {
            if !uuid7(target) {
                return Err(fail(ErrorCode::InvalidId, "invalid full element identity")
                    .with_rule("DV-BASE-001"));
            }
            // The target's existence and kind and its whole chain decide acceptance (cycle check),
            // so every assembly on the chain is read, with this batch's own records applied.
            for ancestor in chain(prims, &input.changes, target) {
                for group in [Group::Lifecycle, Group::Relationship] {
                    reads.insert(Footprint {
                        identity: ancestor.clone(),
                        group,
                    });
                }
            }
        }
        if let Change::Level {
            level: Some(target),
            ..
        } = change
        {
            if !uuid7(target) {
                return Err(fail(ErrorCode::InvalidId, "invalid full element identity")
                    .with_rule("DV-BASE-001"));
            }
            // The level's existence and kind decide acceptance; it is only read.
            reads.insert(Footprint {
                identity: target.clone(),
                group: Group::Lifecycle,
            });
        }
        if let Change::MainPart { part, .. } = change {
            if !uuid7(part) {
                return Err(fail(ErrorCode::InvalidId, "invalid full element identity")
                    .with_rule("DV-BASE-001"));
            }
            // The part's existence and its membership decide acceptance: only a direct part of
            // the assembly can be its main part, so its relationships are read too.
            for group in [Group::Lifecycle, Group::Relationship] {
                reads.insert(Footprint {
                    identity: part.clone(),
                    group,
                });
            }
        }
        if !matches!(
            change,
            Change::PartClass { .. }
                | Change::Label { .. }
                | Change::Create(_)
                | Change::Delete { .. }
                | Change::PartOf { .. }
                | Change::MainPart { .. }
                | Change::Level { .. }
                | Change::Mark { .. }
        ) && let Some(p) = existing
        {
            for target in p
                .relationships
                .iter()
                .filter(|r| matches!(r.name.as_str(), "bim:profile" | "bim:material"))
                .flat_map(|r| &r.targets)
            {
                let (_, digest) = DefinitionCategory::parse_path(target).ok_or_else(invalid)?;
                reads.insert(Footprint {
                    identity: digest.to_string(),
                    group: Group::Relationship,
                });
            }
        }
        if matches!(change, Change::Mesh(_)) {
            let mesh_path = format!("{path}/Geometry");
            owned(prims, &mesh_path, true)?;
        }
    }
    // Whole-batch finiteness precedes numeric caps/ranges, independent of insertion order.
    for change in &input.changes {
        finite_change(change)?;
    }
    for resource in &input.resources {
        resource.bytes()?;
        reads.insert(Footprint {
            identity: resource.digest.to_string(),
            group: Group::Relationship,
        });
        // A definition that is not in the stage yet is created; an existing one is immutable and
        // must already be exactly this content, active and owned (kind, fields, types, label and
        // the recomputed digest), or the batch is refused instead of treating it as a no-op.
        if prims.iter().any(|p| {
            p.path == definition_path(resource.digest, definition_category(&resource.definition))
        }) {
            let (kind, category) = match &resource.definition {
                Definition::Material { .. } => ("material", DefinitionCategory::Material),
                Definition::Discipline(hook) => (hook.0.kind_token(), hook.0.category()),
            };
            self::resource(prims, resource.digest, &[kind], category)?;
        } else {
            writes.insert(Footprint {
                identity: resource.digest.to_string(),
                group: Group::Relationship,
            });
        }
    }
    if reads.len().saturating_add(writes.len()) > 4096 {
        return Err(limit());
    }
    for change in &input.changes {
        match change {
            Change::Create(e) => {
                label(&e.name)?;
                pose(&e.placement)?;
                match &e.shape {
                    Shape::Assembly => {
                        if e.material.is_some()
                            || e.placement.translation_mm != [0.0; 3]
                            || e.placement.orientation_xyzw != [0.0, 0.0, 0.0, 1.0]
                        {
                            return Err(unsupported());
                        }
                    }
                    Shape::Discipline(hook) => hook.0.validate(&mut hook::ValidateCx {
                        budgets: b,
                        element: e,
                        input,
                        prims,
                        reads: &mut reads,
                    })?,
                    Shape::Grid(g) => {
                        grid::validate_record(g)?;
                        if e.id != g.id.to_string()
                            || e.name != g.name
                            || e.material.is_some()
                            || e.placement != grid::placement(g)
                        {
                            return Err(invalid());
                        }
                    }
                    Shape::Level { elevation_mm } => {
                        if e.material.is_some()
                            || e.placement.translation_mm != [0.0; 3]
                            || e.placement.orientation_xyzw != [0.0, 0.0, 0.0, 1.0]
                        {
                            return Err(unsupported());
                        }
                        elevation(*elevation_mm)?;
                    }
                    Shape::Construction(o) => {
                        construction::validate_record(o)?;
                        if e.id != o.id.to_string()
                            || e.name != o.name
                            || e.material.is_some()
                            || e.placement.translation_mm != [0.0; 3]
                            || e.placement.orientation_xyzw != [0.0, 0.0, 0.0, 1.0]
                        {
                            return Err(invalid());
                        }
                    }
                    Shape::Annotation(a) => {
                        annotation::validate_record(a)?;
                        if e.id != a.id.to_string()
                            || e.name != a.name
                            || e.material.is_some()
                            || e.placement.translation_mm != [0.0; 3]
                            || e.placement.orientation_xyzw != [0.0, 0.0, 0.0, 1.0]
                        {
                            return Err(invalid());
                        }
                    }
                }
                if let Some(d) = e.material {
                    reads.insert(Footprint {
                        identity: d.to_string(),
                        group: Group::Relationship,
                    });
                }
            }
            Change::Grid { id, grid: g } => {
                grid::validate_record(g)?;
                let existing = prims
                    .iter()
                    .find(|p| p.element_id.as_deref() == Some(id))
                    .ok_or_else(invalid)?;
                let before = grid::read(existing)?;
                let mode = |f: &beamcraft_model::grid::GridFamily| {
                    matches!(f, beamcraft_model::grid::GridFamily::Regular(_))
                };
                if g.id != before.id
                    || g.name != before.name
                    || g.translation_mm != before.translation_mm
                    || g.orientation_zw != before.orientation_zw
                    || mode(&g.u) != mode(&before.u)
                    || mode(&g.v) != mode(&before.v)
                {
                    return Err(unsupported());
                }
            }
            Change::PartClass { .. } => {}
            Change::Discipline(hook) => hook.0.validate(&mut hook::ChangeValidateCx {
                budgets: b,
                input,
                prims,
                reads: &mut reads,
            })?,
            Change::Label { name, .. } => label(name)?,
            Change::AttachReference(r) => reference::check_new(r)?,
            Change::ReferenceUnits { interpretation, .. } => {
                reference::check_units(interpretation)?
            }
            Change::Placement { placement, .. } => pose(placement)?,
            Change::Material {
                opinion: MaterialOpinion::Target(d),
                ..
            } => {
                reads.insert(Footprint {
                    identity: d.to_string(),
                    group: Group::Relationship,
                });
            }
            Change::Mesh(m) => {
                if grid::guide_target(input, prims, &m.id) && m.quantities.is_some() {
                    return Err(unsupported());
                }
                mesh(m, b)?;
                if let Some(q) = &m.quantities {
                    quantities(q)?;
                }
            }
            _ => (),
        }
    }
    if reads.len().saturating_add(writes.len()) > 4096 {
        return Err(limit());
    }
    Ok((reads, writes))
}
/// Bounds a point array by count and total values against the batch budgets; the seam's
/// point-budget gate.
pub fn count_points(points: &[[f64; 3]], b: Budgets) -> Result<(), Diagnostic> {
    if points.len() > 1024 || points.len().saturating_mul(3) > b.array_items {
        return Err(limit());
    }
    if points.len() < 3 {
        return Err(invalid());
    }
    Ok(())
}
fn finite_change(c: &Change) -> Result<(), Diagnostic> {
    match c {
        Change::Create(e) => {
            finite(
                e.placement
                    .translation_mm
                    .into_iter()
                    .chain(e.placement.orientation_xyzw),
            )?;
            match &e.shape {
                Shape::Assembly => Ok(()),
                Shape::Discipline(hook) => hook.0.check_finite(),
                Shape::Level { elevation_mm } => finite([*elevation_mm]),
                Shape::Grid(g) => grid::validate_record(g),
                Shape::Construction(o) => construction::validate_record(o),
                Shape::Annotation(a) => annotation::validate_record(a),
            }
        }
        Change::Grid { grid: g, .. } => grid::validate_record(g),
        Change::AttachReference(r) => finite(
            r.placement
                .translation_mm
                .into_iter()
                .chain(r.placement.orientation_xyzw)
                .chain(r.source.declared_meters_per_unit),
        ),
        Change::ReferenceUnits { interpretation, .. } => {
            finite(interpretation.iter().map(|i| i.meters_per_unit))
        }
        Change::Discipline(hook) => hook.0.check_finite(),
        Change::Placement { placement, .. } => finite(
            placement
                .translation_mm
                .into_iter()
                .chain(placement.orientation_xyzw),
        ),
        Change::Mesh(m) => finite(
            m.points_mm.iter().flatten().map(|n| f64::from(*n)).chain(
                m.quantities
                    .iter()
                    .flat_map(|q| [q.volume_mm3].into_iter().chain(q.mass_kg)),
            ),
        ),
        _ => Ok(()),
    }
}
fn quantities(q: &StoredQuantities) -> Result<(), Diagnostic> {
    let status = |s: &str| matches!(s, "current" | "stale" | "unknown");
    if !status(&q.volume_status)
        || !status(&q.mass_status)
        || q.algorithm.len() > 128
        || q.algorithm.is_empty()
        || q.volume_mm3 < 0.0
        || q.mass_kg.is_some_and(|kg| kg < 0.0)
        // A mass value exists exactly when its status is current.
        || q.mass_kg.is_some() != (q.mass_status == "current")
    {
        return Err(invalid());
    }
    cap([q.volume_mm3], 1e30)?;
    cap(q.mass_kg, 1e30)
}
fn mesh(m: &Mesh, b: Budgets) -> Result<(), Diagnostic> {
    if m.points_mm.len().saturating_mul(3) > b.array_items
        || m.face_counts.len() > b.array_items
        || m.indices.len() > b.array_items
    {
        return Err(limit());
    }
    cap(m.points_mm.iter().flatten().map(|n| f64::from(*n)), 1e9)?;
    if m.points_mm.is_empty() || m.face_counts.is_empty() {
        return Err(invalid());
    }
    let mut count = 0usize;
    for &n in &m.face_counts {
        if n < 3 {
            return Err(invalid());
        }
        count = count
            .checked_add(usize::try_from(n).map_err(|_| invalid())?)
            .ok_or_else(limit)?;
    }
    if count != m.indices.len()
        || m.indices
            .iter()
            .any(|&n| usize::try_from(n).map_or(true, |i| i >= m.points_mm.len()))
    {
        return Err(invalid());
    }
    Ok(())
}
