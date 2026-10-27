// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! Independent W01 camera layer, with opaque bounded personal records for later view settings.
use super::*;
use beamcraft_model::{
    id::{ElementId, RevisionId, Uuid7, WorkspaceId},
    saved_view::{Camera, MAX_VIEWS, Projection, SavedView},
};
use openusd::sdf::{self, AbstractData};
/// A personal workspace's cameras and auxiliary view records. No model opinions are authored.
#[derive(Debug, Clone)]
pub struct Document {
    /// Owning workspace.
    pub workspace: WorkspaceId,
    /// Independently named cameras.
    pub views: Vec<SavedView>,
    /// Opaque personal records, retained byte-for-byte as UTF-8 strings.
    pub records: BTreeMap<String, String>,
}
const MAX_BYTES: usize = 4 * 1024 * 1024;
fn chunks(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && i.is_multiple_of(128) {
            out.push(current);
            current = String::new();
        }
        current.push(c);
    }
    out.push(current);
    out
}
fn path_for(id: ElementId) -> String {
    format!("/Views/V_{}", id.to_string().replace('-', ""))
}
fn attr(data: &mut sdf::Data, p: &str, n: &str, ty: &str, v: sdf::Value) -> Result<(), Diagnostic> {
    edit::attr(data, p, n, ty, v)
}
fn text(data: &mut sdf::Data, p: &str, n: &str, s: &str) -> Result<(), Diagnostic> {
    attr(data, p, n, "string", sdf::Value::String(s.to_owned()))
}
/// Serialize typed Camera prims with explicit pose and provenance, in a separate USD layer.
pub fn encode(doc: &Document) -> Result<Vec<u8>, Diagnostic> {
    if doc.views.len() > MAX_VIEWS || doc.records.len() > 256 {
        return Err(limit());
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut data = sdf::Data::new();
    edit::prim(&mut data, "/", None, false)?;
    edit::field(&mut data, "/", "metersPerUnit", sdf::Value::Double(0.001))?;
    edit::field(&mut data, "/", "upAxis", sdf::Value::token("Z"))?;
    edit::container(&mut data, "/Views", "Scope", None)?;
    attr(
        &mut data,
        "/Views",
        "bim:personalVersion",
        "int",
        sdf::Value::Int(1),
    )?;
    text(
        &mut data,
        "/Views",
        "bim:workspaceId",
        &doc.workspace.to_string(),
    )?;
    for v in &doc.views {
        v.validate().map_err(|e| fail(e.code, e.message))?;
        if v.workspace != doc.workspace || !seen.insert(v.id) {
            return Err(invalid());
        }
        let p = path_for(v.id);
        edit::prim(&mut data, &p, Some("Camera"), true)?;
        edit::pose(
            &mut data,
            &p,
            &Placement {
                translation_mm: v.camera.placement.translation_mm,
                orientation_xyzw: v.camera.placement.orientation_xyzw,
            },
        )?;
        for (n, s) in [
            ("bim:viewId", v.id.to_string()),
            ("bim:name", v.name.clone()),
            ("bim:author", v.author.to_string()),
            ("bim:sourceRevision", v.source_revision.to_string()),
            ("bim:sourceManifest", v.source_manifest.to_string()),
        ] {
            text(&mut data, &p, n, &s)?;
        }
        attr(
            &mut data,
            &p,
            "projection",
            "token",
            sdf::Value::token(v.camera.projection.token()),
        )?;
        for (n, x) in [
            ("bim:widthMm", v.camera.width_mm),
            ("bim:aspect", v.camera.aspect),
            ("bim:fovRad", v.camera.fov_rad),
        ] {
            attr(&mut data, &p, n, "double", sdf::Value::Double(x))?;
        }
        // USD camera aperture uses tenths of the stage length unit; the stage unit here is mm.
        attr(
            &mut data,
            &p,
            "horizontalAperture",
            "float",
            sdf::Value::Float((v.camera.width_mm * 10.) as f32),
        )?;
        attr(
            &mut data,
            &p,
            "verticalAperture",
            "float",
            sdf::Value::Float((v.camera.width_mm / v.camera.aspect * 10.) as f32),
        )?;
        let focal = v.camera.width_mm * 10. / (2. * (v.camera.fov_rad / 2.).tan());
        attr(
            &mut data,
            &p,
            "focalLength",
            "float",
            sdf::Value::Float(focal as f32),
        )?;
        attr(
            &mut data,
            &p,
            "bim:savedMs",
            "uint64",
            sdf::Value::Uint64(v.saved_ms),
        )?;
        attr(
            &mut data,
            &p,
            "bim:comment",
            "string[]",
            sdf::Value::StringVec(chunks(&v.comment)),
        )?;
        attr(
            &mut data,
            &p,
            "bim:workPlane",
            "double[]",
            sdf::Value::DoubleVec(v.work_plane.into_iter().flatten().collect()),
        )?;
    }
    edit::container(&mut data, "/Records", "Scope", None)?;
    for (key, value) in &doc.records {
        if key.is_empty()
            || key.len() > 80
            || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            || value.len() > 65536
        {
            return Err(limit());
        }
        let p = format!("/Records/R_{key}");
        edit::prim(&mut data, &p, Some("Scope"), true)?;
        attr(
            &mut data,
            &p,
            "bim:payload",
            "string[]",
            sdf::Value::StringVec(chunks(value)),
        )?;
    }
    edit::serialize(&data, MAX_BYTES)
}
fn value(data: &dyn AbstractData, p: &str, name: &str, ty: &str) -> Result<Value, Diagnostic> {
    let path = edit::path(&format!("{p}.{name}"))?;
    let type_name = data
        .try_field(&path, "typeName")
        .map_err(|_| corrupt())?
        .ok_or_else(corrupt)?;
    if !matches!(type_name.as_ref(),sdf::Value::Token(t) if t.as_str()==ty) {
        return Err(corrupt());
    }
    let v = data
        .try_field(&path, "default")
        .map_err(|_| corrupt())?
        .ok_or_else(corrupt)?;
    crate::backend::typed(v.as_ref(), Budgets::default())
}
fn string(data: &dyn AbstractData, p: &str, n: &str) -> Result<String, Diagnostic> {
    match value(data, p, n, "string")? {
        Value::Text(s) => Ok(s),
        _ => Err(corrupt()),
    }
}
fn number(data: &dyn AbstractData, p: &str, n: &str) -> Result<f64, Diagnostic> {
    match value(data, p, n, "double")? {
        Value::Number(v) => Ok(v),
        _ => Err(corrupt()),
    }
}
fn array<const N: usize>(
    data: &dyn AbstractData,
    p: &str,
    n: &str,
    ty: &str,
) -> Result<[f64; N], Diagnostic> {
    let Value::Array(v) = value(data, p, n, ty)? else {
        return Err(corrupt());
    };
    v.iter()
        .map(|v| {
            if let Value::Number(n) = v {
                Ok(*n)
            } else {
                Err(corrupt())
            }
        })
        .collect::<Result<Vec<_>, _>>()?
        .try_into()
        .map_err(|_| corrupt())
}
fn joined(data: &dyn AbstractData, p: &str, n: &str) -> Result<String, Diagnostic> {
    let Value::Array(values) = value(data, p, n, "string[]")? else {
        return Err(corrupt());
    };
    if values.len() > 1024 {
        return Err(limit());
    }
    let mut out = String::new();
    for v in values {
        let Value::Text(v) = v else {
            return Err(corrupt());
        };
        if out.len().saturating_add(v.len()) > 65536 {
            return Err(limit());
        }
        out.push_str(&v);
    }
    Ok(out)
}
/// Parse one contained, preflighted static document. Composition arcs never resolve externally.
pub fn decode(
    bytes: &[u8],
    workspace: WorkspaceId,
    control: &Control,
) -> Result<Document, Diagnostic> {
    if bytes.len() > MAX_BYTES {
        return Err(limit());
    }
    let budgets = Budgets {
        layer_bytes: MAX_BYTES,
        total_bytes: MAX_BYTES,
        prims: 512,
        values: 131072,
        array_items: 8192,
        ..Budgets::default()
    };
    let asset = Asset {
        name: "views.usda".into(),
        digest: beamcraft_model::digest::Sha256::digest(bytes).map_err(|_| limit())?,
        bytes: bytes.to_vec(),
    };
    let (data, dependencies) = crate::backend::load_asset(
        "views.usda",
        &asset,
        budgets,
        control,
        &mut crate::backend::Counters::default(),
    )?;
    if !dependencies.is_empty() {
        return Err(unsupported());
    }
    // Reject fields this personal format cannot preserve; never silently erase foreign opinions.
    let camera_fields = [
        "bim:viewId",
        "bim:name",
        "bim:author",
        "bim:sourceRevision",
        "bim:sourceManifest",
        "projection",
        "bim:widthMm",
        "bim:aspect",
        "bim:fovRad",
        "horizontalAperture",
        "verticalAperture",
        "focalLength",
        "bim:savedMs",
        "bim:comment",
        "bim:workPlane",
        "xformOp:translate",
        "xformOp:orient",
        "xformOpOrder",
    ];
    for path in data.spec_paths() {
        let p = path.to_string();
        match data.spec_type(&path) {
            Some(sdf::SpecType::Attribute) => {
                let (parent, name) = p.rsplit_once('.').ok_or_else(corrupt)?;
                let allowed = if parent == "/Views" {
                    matches!(name, "bim:personalVersion" | "bim:workspaceId")
                } else if parent.starts_with("/Views/V_") {
                    camera_fields.contains(&name)
                } else if parent.starts_with("/Records/R_") {
                    name == "bim:payload"
                } else {
                    false
                };
                if !allowed {
                    return Err(unsupported());
                }
                if data
                    .list_fields(&path)
                    .ok_or_else(corrupt)?
                    .iter()
                    .any(|key| {
                        !matches!(
                            key.as_str(),
                            "custom" | "typeName" | "variability" | "default"
                        )
                    })
                {
                    return Err(unsupported());
                }
            }
            Some(sdf::SpecType::Prim) => {
                let expected = if p.starts_with("/Views/V_") {
                    "Camera"
                } else {
                    "Scope"
                };
                if !matches!(data.try_field(&path,"typeName").map_err(|_|corrupt())?.as_deref(),Some(sdf::Value::Token(t)) if t.as_str()==expected)
                {
                    return Err(unsupported());
                }
                if data
                    .list_fields(&path)
                    .ok_or_else(corrupt)?
                    .iter()
                    .any(|key| {
                        !matches!(
                            key.as_str(),
                            "specifier" | "typeName" | "primChildren" | "propertyChildren"
                        )
                    })
                {
                    return Err(unsupported());
                }
            }
            Some(sdf::SpecType::PseudoRoot) => {
                if data
                    .list_fields(&path)
                    .ok_or_else(corrupt)?
                    .iter()
                    .any(|key| !matches!(key.as_str(), "primChildren" | "metersPerUnit" | "upAxis"))
                {
                    return Err(unsupported());
                }
            }
            _ => return Err(unsupported()),
        }
    }

    for (key, expected) in [
        ("metersPerUnit", Value::Number(0.001)),
        ("upAxis", Value::Text("Z".into())),
    ] {
        let actual = data
            .try_field(&sdf::Path::abs_root(), key)
            .map_err(|_| corrupt())?
            .ok_or_else(corrupt)?;
        if crate::backend::typed(actual.as_ref(), budgets)? != expected {
            return Err(unsupported());
        }
    }
    if value(data.as_ref(), "/Views", "bim:personalVersion", "int")? != Value::Int(1) {
        return Err(fail(
            ErrorCode::UnsupportedVersion,
            "unsupported personal views format",
        ));
    }
    if string(data.as_ref(), "/Views", "bim:workspaceId")? != workspace.to_string() {
        return Err(invalid());
    }
    let mut doc = Document {
        workspace,
        views: Vec::new(),
        records: BTreeMap::new(),
    };
    for path in data.spec_paths() {
        if data.spec_type(&path) != Some(sdf::SpecType::Prim) {
            continue;
        }
        let p = path.to_string();
        if p == "/Views" || p == "/Records" {
            continue;
        }
        if let Some(key) = p.strip_prefix("/Records/R_") {
            if key.contains('/')
                || doc
                    .records
                    .insert(key.to_owned(), joined(data.as_ref(), &p, "bim:payload")?)
                    .is_some()
            {
                return Err(invalid());
            }
            continue;
        }
        if !p.starts_with("/Views/V_") {
            return Err(unsupported());
        }
        if doc.views.len() >= MAX_VIEWS {
            return Err(limit());
        }
        let field = |n| string(data.as_ref(), &p, n);
        let [qw, qx, qy, qz] = array::<4>(data.as_ref(), &p, "xformOp:orient", "quatd")?;
        let [ox, oy, oz, xx, xy, xz, yx, yy, yz] =
            array::<9>(data.as_ref(), &p, "bim:workPlane", "double[]")?;
        let Value::Text(projection) = value(data.as_ref(), &p, "projection", "token")? else {
            return Err(corrupt());
        };
        let saved_ms = match value(data.as_ref(), &p, "bim:savedMs", "uint64")? {
            Value::Uint(v) => v,
            _ => return Err(corrupt()),
        };
        let v = SavedView {
            id: ElementId::parse(&field("bim:viewId")?).map_err(|_| invalid())?,
            name: field("bim:name")?,
            author: Uuid7::parse(&field("bim:author")?).map_err(|_| invalid())?,
            saved_ms,
            comment: joined(data.as_ref(), &p, "bim:comment")?,
            workspace,
            source_revision: RevisionId::parse(&field("bim:sourceRevision")?)
                .map_err(|_| invalid())?,
            source_manifest: field("bim:sourceManifest")?
                .parse()
                .map_err(|_| invalid())?,
            camera: Camera {
                placement: beamcraft_model::command::Placement {
                    translation_mm: array::<3>(data.as_ref(), &p, "xformOp:translate", "double3")?,
                    orientation_xyzw: [qx, qy, qz, qw],
                },
                projection: Projection::parse(&projection).ok_or_else(invalid)?,
                width_mm: number(data.as_ref(), &p, "bim:widthMm")?,
                aspect: number(data.as_ref(), &p, "bim:aspect")?,
                fov_rad: number(data.as_ref(), &p, "bim:fovRad")?,
            },
            work_plane: [[ox, oy, oz], [xx, xy, xz], [yx, yy, yz]],
        };
        v.validate().map_err(|e| fail(e.code, e.message))?;
        if p != path_for(v.id) || doc.views.iter().any(|old| old.id == v.id) {
            return Err(invalid());
        }
        doc.views.push(v);
    }
    doc.views.sort_by_key(|v| v.id);
    Ok(doc)
}
