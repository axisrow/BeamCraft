// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! W01 saved views: the camera prim of a views layer, with its provenance. A saved view is
//! display state, not model data, so a view prim carries no `bim:id`, enters no element index
//! and links to nothing: saving or restoring views never moves model keys or numbering. The
//! camera prim is a standard USD `Camera` under `/World/Views`, so an ordinary USD reader sees
//! a placed camera, and every BeamCraft-specific fact rides on `bim:view:*` attributes.
//!
//! This module is the closed profile of the views layer:
//! [`VIEWS_SCHEMA_DESCRIPTOR`](crate::authored::views::VIEWS_SCHEMA_DESCRIPTOR) fixes the
//! field/type mappings and bounds, [`bytes()`](crate::authored::views::bytes) produces its
//! canonical layer text and [`read_layer()`](crate::authored::views::read_layer) reads the
//! camera prims and the empty-folder record back. The layer's mapping identity is its own
//! [`views_schema_identity()`](crate::authored::views::views_schema_identity), separate from
//! the element profile's `schema_identity`.

use super::*;
use beamcraft_model::id::Uuid7;
use openusd::sdf;

/// Canonical UTF-8 schema descriptor of the views layer. Its exact bytes commit the mapping and
/// item profile. No generated schema, plugin, fallback values or external schema resources are
/// required.
pub const VIEWS_SCHEMA_DESCRIPTOR: &str = concat!(
    "BeamCraft/W01/codeless-views/1\nmodel=0.1.0;backend=openusd=0.8.0;features=none;save=usda-owned-v1\n",
    "root:metersPerUnit=0.001;upAxis=Z;defaultPrim=World;customLayerData:bimViewsSchemaDigest=sha256\n",
    "camera=/World/Views/VIE_<uuid7-32>:Camera;displayName=saved view name;no bim:id,kind,relationships or Geometry;\n",
    "xformOp:translate=double3;xformOp:orient=quatd;xformOpOrder=token[](translate,orient);projection=token(orthographic|perspective)\n",
    "bim:view:id=string(uuid7);bim:view:spanMm=double(0,2e7];bim:view:depthMm=double[](exactly 2,(0,2e7] each);\n",
    "bim:view:renderMode=token(<=64 bytes);bim:view:grids=bool;bim:view:hiddenKinds=token[](<=16);bim:view:folder=string(<=512 bytes);\n",
    "bim:view:favorite=bool;bim:view:default=bool;bim:view:workAreaMm=double3[](exactly 2 entries,min then max,optional,finite,abs<=1e9);\n",
    "bim:view:rotationRad=double[](exactly 2,finite);bim:view:colorRgb=double3(optional,0..=1 each);bim:view:opacity=double(0..=1)\n",
    "bim:view:folders=string[](on the views scope,exactly the folders no view uses,sorted unique,each at most 512 bytes,\n",
    "at most 8 nonempty segments without controls,outer spaces or dot names,at most 512 entries,optional)\n",
    "provenance(W01)=bim:view:author=string(1..=128,no controls,trimmed);bim:view:savedAtMs=uint64(unix milliseconds);\n",
    "bim:view:comment=string(<=512,no controls);bim:view:modelRevision=string(uuid7);all four or none\n",
    "a saved view is display state,not model data: no element identity,no model keys,no numbering\n",
);

/// Exact mapping identity of the views layer, separate from the element profile's pin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewsSchemaIdentity {
    /// Frozen F04 version.
    pub model: &'static str,
    /// SHA-256 of [`VIEWS_SCHEMA_DESCRIPTOR`].
    pub digest: ContentDigest,
}

/// Computes the admitted views mapping identity without consulting files or environment.
pub fn views_schema_identity() -> Result<ViewsSchemaIdentity, Diagnostic> {
    Ok(ViewsSchemaIdentity {
        model: "0.1.0",
        digest: Sha256::digest(VIEWS_SCHEMA_DESCRIPTOR.as_bytes()).map_err(|_| limit())?,
    })
}

/// Key of the views mapping digest in the views layer's `customLayerData`.
pub const VIEWS_SCHEMA_DIGEST_KEY: &str = "bimViewsSchemaDigest";

/// The folder of the camera prims in the views layer.
pub const VIEWS_FOLDER: &str = "/World/Views";

/// The most saved views one layer admits; mirrors the engine's view-book cap.
pub const MAX_VIEWS: usize = 256;

/// The visible-height and visible-depth caps in millimetres; mirrors the engine's caps.
pub const MAX_SPAN_MM: f64 = 2e7;

/// The camera projection of a saved view; the standard USD camera projections.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraProjection {
    /// Everything at the view plane renders at one scale.
    Orthographic,
    /// Lines of sight pass through one eye point.
    Perspective,
}

impl CameraProjection {
    /// The USD `projection` token of this projection.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Orthographic => "orthographic",
            Self::Perspective => "perspective",
        }
    }
    /// Parses the USD `projection` token; anything else is unsupported.
    pub fn parse(text: &str) -> Result<Self, Diagnostic> {
        match text {
            "orthographic" => Ok(Self::Orthographic),
            "perspective" => Ok(Self::Perspective),
            _ => Err(unsupported()),
        }
    }
}

/// The W01 provenance of a saved view: who saved the camera, when, why and against which model
/// revision. Display state only: recording or restoring it never touches the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewStamp {
    /// Who saved the view, 1 to 128 characters without controls or outer spaces.
    pub author: String,
    /// Unix time of the save, in milliseconds.
    pub created_at_ms: u64,
    /// Free-text comment, at most 512 characters without controls; may be empty.
    pub comment: String,
    /// The model revision the view was captured against.
    pub model_revision: Uuid7,
}

impl ViewStamp {
    /// Admits a provenance record; the author is trimmed, the comment is kept as given.
    pub fn new(
        author: &str,
        comment: &str,
        model_revision: Uuid7,
        created_at_ms: u64,
    ) -> Result<Self, Diagnostic> {
        let out = Self {
            author: author.trim().to_owned(),
            created_at_ms,
            comment: comment.to_owned(),
            model_revision,
        };
        out.validate()?;
        Ok(out)
    }
    /// Admits the record: a bounded, printable author and comment.
    pub fn validate(&self) -> Result<(), Diagnostic> {
        let bounded = |text: &str, max: usize| {
            text.chars().count() <= max && !text.chars().any(char::is_control)
        };
        if !bounded(&self.author, 128) || self.author.is_empty() || !bounded(&self.comment, 512) {
            return Err(invalid());
        }
        Ok(())
    }
}

/// One saved view: the camera prim of the views layer with its display record. The fields are
/// the views profile's own bounded forms; mapping between them and the engine's `ModelView` is
/// the caller's duty, so this crate stays free of engine vocabulary.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedView {
    /// Persistent UUIDv7 identity, also the `VIE_` prim name suffix in hex form.
    pub id: Uuid7,
    /// User-visible name, 1 to 128 characters without controls or outer spaces.
    pub name: String,
    /// Camera placement in world millimetres: the view plane origin and orientation. The plane's
    /// local +Z points towards the viewer, so the camera looks along its local -Z, the standard
    /// USD camera convention.
    pub placement: Placement,
    /// Camera projection.
    pub projection: CameraProjection,
    /// Visible height at the view plane, in `(0, 2e7]` millimetres.
    pub span_mm: f64,
    /// Visible distance `[above, below]` the plane in millimetres, each in `(0, 2e7]`; `None`
    /// shows everything in front of and behind the plane.
    pub depth_mm: Option<[f64; 2]>,
    /// Display mode token, at most 64 bytes; an unknown token stays a token, never a refusal.
    pub render_mode: String,
    /// Whether grid guides are displayed.
    pub grids_visible: bool,
    /// Hidden element kinds, at most 16 tokens.
    pub hidden_kinds: Vec<String>,
    /// Browser folder path, `/`-separated; empty is the root; at most 512 bytes.
    pub folder: String,
    /// Listed in quick access.
    pub favorite: bool,
    /// Whether this view is the book's default view; at most one view of a book carries the
    /// mark, which the engine's book enforces, not this profile.
    pub default: bool,
    /// Optional world-space box the view shows, `[min, max]` in millimetres.
    pub work_area_mm: Option<[[f64; 3]; 2]>,
    /// Camera rotations about the view's local Z, then X, in radians.
    pub rotation_rad: [f64; 2],
    /// Optional display colour override, linear RGB in `0..=1`.
    pub color_rgb: Option<[f64; 3]>,
    /// Display opacity in `0..=1`.
    pub opacity: f64,
    /// The saved view's provenance (W01), present exactly for a provenance-bearing capture.
    pub stamp: Option<ViewStamp>,
}

impl SavedView {
    /// A view with the default display properties: orthographic, no depth limit, grids shown,
    /// no work area, no rotation, no colour override, opaque, unstamped.
    pub fn new(
        id: Uuid7,
        name: &str,
        placement: Placement,
        span_mm: f64,
    ) -> Result<Self, Diagnostic> {
        let out = Self {
            id,
            name: name.trim().to_owned(),
            placement,
            projection: CameraProjection::Orthographic,
            span_mm,
            depth_mm: None,
            render_mode: "shaded".to_owned(),
            grids_visible: true,
            hidden_kinds: Vec::new(),
            folder: String::new(),
            favorite: false,
            default: false,
            work_area_mm: None,
            rotation_rad: [0.0; 2],
            color_rgb: None,
            opacity: 1.0,
            stamp: None,
        };
        out.validate()?;
        Ok(out)
    }

    /// Stamps the provenance of a saved view. The camera and display properties stay untouched.
    pub fn stamp(
        &mut self,
        author: &str,
        comment: &str,
        model_revision: Uuid7,
        created_at_ms: u64,
    ) -> Result<(), Diagnostic> {
        self.stamp = Some(ViewStamp::new(
            author,
            comment,
            model_revision,
            created_at_ms,
        )?);
        Ok(())
    }

    /// Admits one view: bounded identity, camera and display record.
    pub fn validate(&self) -> Result<(), Diagnostic> {
        let printable = |text: &str, max: usize| {
            !text.is_empty() && text.len() <= max && !text.chars().any(char::is_control)
        };
        let finite = |values: &[f64]| values.iter().all(|v| v.is_finite());
        if !printable(&self.name, 128) {
            return Err(invalid());
        }
        if !finite(&self.placement.translation_mm) || !finite(&self.placement.orientation_xyzw) {
            return Err(invalid());
        }
        let [qx, qy, qz, qw] = self.placement.orientation_xyzw;
        let norm = (qx * qx + qy * qy + qz * qz + qw * qw).sqrt();
        if !norm.is_finite() || (norm - 1.0).abs() > 1e-6 {
            return Err(invalid());
        }
        if !(self.span_mm > 0.0 && self.span_mm <= MAX_SPAN_MM) {
            return Err(invalid());
        }
        if let Some([above, below]) = self.depth_mm
            && !(above > 0.0 && above <= MAX_SPAN_MM && below > 0.0 && below <= MAX_SPAN_MM)
        {
            return Err(invalid());
        }
        if self.render_mode.is_empty() || self.render_mode.len() > 64 {
            return Err(invalid());
        }
        if self.hidden_kinds.len() > 16
            || self
                .hidden_kinds
                .iter()
                .any(|k| k.is_empty() || k.len() > 64)
        {
            return Err(invalid());
        }
        if self.folder.len() > 512 {
            return Err(invalid());
        }
        if let Some([min, max]) = self.work_area_mm {
            let box_ok = finite(&min)
                && finite(&max)
                && min
                    .iter()
                    .zip(max)
                    .all(|(lo, hi)| lo.abs() <= 1e9 && hi.abs() <= 1e9 && *lo <= hi);
            if !box_ok {
                return Err(invalid());
            }
        }
        if !finite(&self.rotation_rad) || self.rotation_rad.iter().any(|v| v.abs() > 1e6) {
            return Err(invalid());
        }
        if let Some(color) = self.color_rgb
            && (!finite(&color) || color.iter().any(|v| !(0.0..=1.0).contains(v)))
        {
            return Err(invalid());
        }
        if !(0.0..=1.0).contains(&self.opacity) {
            return Err(invalid());
        }
        if let Some(stamp) = &self.stamp {
            stamp.validate()?;
        }
        Ok(())
    }
}

/// The `bim:view:*` attribute profile: names and their USD types, in authoring order. The
/// provenance attributes close the list; a prim authors all four of them or none.
pub const FIELDS: &[(&str, &str)] = &[
    ("bim:view:id", "string"),
    ("bim:view:spanMm", "double"),
    ("bim:view:depthMm", "double[]"),
    ("bim:view:renderMode", "token"),
    ("bim:view:grids", "bool"),
    ("bim:view:hiddenKinds", "token[]"),
    ("bim:view:folder", "string"),
    ("bim:view:favorite", "bool"),
    ("bim:view:default", "bool"),
    ("bim:view:workAreaMm", "double3[]"),
    ("bim:view:rotationRad", "double[]"),
    ("bim:view:colorRgb", "double3"),
    ("bim:view:opacity", "double"),
    ("bim:view:author", "string"),
    ("bim:view:savedAtMs", "uint64"),
    ("bim:view:comment", "string"),
    ("bim:view:modelRevision", "string"),
];

/// The prim path of one saved view.
fn view_path(id: &Uuid7) -> String {
    format!("{VIEWS_FOLDER}/VIE_{}", id.hex32())
}

/// Admits one folder path: at most 512 bytes and 8 segments, no controls, no outer spaces per
/// segment, no empty or dot segment; the root (empty) folder is not a record.
fn validate_folder(folder: &str) -> Result<(), Diagnostic> {
    if folder.is_empty()
        || folder.len() > 512
        || folder.chars().any(char::is_control)
        || folder.split('/').count() > 8
        || folder
            .split('/')
            .any(|s| s.trim() != s || s.is_empty() || matches!(s, "." | ".."))
    {
        return Err(invalid());
    }
    Ok(())
}

/// Authors the layer header: units, up axis and the pinned views mapping digest.
fn header(data: &mut sdf::Data, schema: &ViewsSchemaIdentity) -> Result<(), Diagnostic> {
    edit::prim(data, "/", None, false)?;
    edit::field(data, "/", "metersPerUnit", sdf::Value::Double(0.001))?;
    edit::field(data, "/", "upAxis", sdf::Value::token("Z"))?;
    edit::field(data, "/", "defaultPrim", sdf::Value::token("World"))?;
    edit::field(
        data,
        "/",
        "customLayerData",
        sdf::Value::Dictionary(
            [(
                VIEWS_SCHEMA_DIGEST_KEY.to_owned(),
                sdf::Value::String(schema.digest.to_string()),
            )]
            .into_iter()
            .collect(),
        ),
    )
}

/// Authors one camera prim: a standard placed camera, then the `bim:view:*` record. The layer
/// containers and the views scope already exist (`bytes` authors them before the cameras).
fn author(data: &mut sdf::Data, view: &SavedView) -> Result<(), Diagnostic> {
    let p = view_path(&view.id);
    edit::prim(data, &p, Some("Camera"), true)?;
    edit::field(
        data,
        &p,
        "displayName",
        sdf::Value::String(view.name.clone()),
    )?;
    edit::pose(data, &p, &view.placement)?;
    edit::attribute(
        data,
        &p,
        "projection",
        "token",
        Opinion::Value(sdf::Value::token(view.projection.as_str())),
        false,
    )?;
    let numbers = |values: &[f64]| sdf::Value::DoubleVec(values.to_vec());
    let kinds =
        |values: &[String]| sdf::Value::TokenVec(values.iter().map(|v| v.clone().into()).collect());
    edit::attr(
        data,
        &p,
        "bim:view:id",
        "string",
        sdf::Value::String(view.id.to_string()),
    )?;
    edit::attr(
        data,
        &p,
        "bim:view:spanMm",
        "double",
        sdf::Value::Double(view.span_mm),
    )?;
    if let Some([above, below]) = view.depth_mm {
        edit::attr(
            data,
            &p,
            "bim:view:depthMm",
            "double[]",
            numbers(&[above, below]),
        )?;
    }
    edit::attr(
        data,
        &p,
        "bim:view:renderMode",
        "token",
        sdf::Value::token(view.render_mode.clone()),
    )?;
    edit::attr(
        data,
        &p,
        "bim:view:grids",
        "bool",
        sdf::Value::Bool(view.grids_visible),
    )?;
    edit::attr(
        data,
        &p,
        "bim:view:hiddenKinds",
        "token[]",
        kinds(&view.hidden_kinds),
    )?;
    edit::attr(
        data,
        &p,
        "bim:view:folder",
        "string",
        sdf::Value::String(view.folder.clone()),
    )?;
    edit::attr(
        data,
        &p,
        "bim:view:favorite",
        "bool",
        sdf::Value::Bool(view.favorite),
    )?;
    edit::attr(
        data,
        &p,
        "bim:view:default",
        "bool",
        sdf::Value::Bool(view.default),
    )?;
    if let Some([min, max]) = view.work_area_mm {
        edit::attr(
            data,
            &p,
            "bim:view:workAreaMm",
            "double3[]",
            edit::points(&[min, max]),
        )?;
    }
    edit::attr(
        data,
        &p,
        "bim:view:rotationRad",
        "double[]",
        numbers(&view.rotation_rad),
    )?;
    if let Some(color) = view.color_rgb {
        edit::attr(
            data,
            &p,
            "bim:view:colorRgb",
            "double3",
            sdf::Value::vec3d(color[0], color[1], color[2]),
        )?;
    }
    edit::attr(
        data,
        &p,
        "bim:view:opacity",
        "double",
        sdf::Value::Double(view.opacity),
    )?;
    if let Some(stamp) = &view.stamp {
        edit::attr(
            data,
            &p,
            "bim:view:author",
            "string",
            sdf::Value::String(stamp.author.clone()),
        )?;
        edit::attr(
            data,
            &p,
            "bim:view:savedAtMs",
            "uint64",
            sdf::Value::Uint64(stamp.created_at_ms),
        )?;
        edit::attr(
            data,
            &p,
            "bim:view:comment",
            "string",
            sdf::Value::String(stamp.comment.clone()),
        )?;
        edit::attr(
            data,
            &p,
            "bim:view:modelRevision",
            "string",
            sdf::Value::String(stamp.model_revision.to_string()),
        )?;
    }
    Ok(())
}

/// The whole content of a views layer: the camera prims and the folders no view uses. A folder
/// that at least one view names lives on those views' own `bim:view:folder` attribute; this
/// record keeps the explicitly created empty ones, so a restored book keeps its tree.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ViewsLayer {
    /// The saved views, in any order; the layer text stores them in identity order.
    pub views: Vec<SavedView>,
    /// The folders no view uses, in any order; the layer text stores them sorted and unique.
    pub empty_folders: Vec<String>,
}

/// Produces the canonical views layer text: the header, the views folder and one camera prim per
/// view, in identity order. Duplicate identities, an over-full book or a view that breaks its
/// bounds are refused; the result is deterministic, so its SHA-256 commits the content.
pub fn bytes(layer: &ViewsLayer) -> Result<Vec<u8>, Diagnostic> {
    if layer.views.len() > MAX_VIEWS || layer.empty_folders.len() > 512 {
        return Err(limit());
    }
    for view in &layer.views {
        view.validate()?;
    }
    for folder in &layer.empty_folders {
        validate_folder(folder)?;
    }
    let schema = views_schema_identity()?;
    let mut ordered = layer.views.clone();
    ordered.sort_by_key(|view| view.id);
    if ordered
        .iter()
        .zip(ordered.iter().skip(1))
        .any(|(a, b)| a.id == b.id)
    {
        return Err(fail(ErrorCode::InvalidId, "duplicate saved-view identity"));
    }
    let mut folders = layer.empty_folders.clone();
    folders.sort();
    folders.dedup();
    let mut data = sdf::Data::new();
    header(&mut data, &schema)?;
    edit::container(&mut data, "/World", "Xform", None)?;
    edit::container(&mut data, VIEWS_FOLDER, "Scope", Some("group"))?;
    if !folders.is_empty() {
        edit::attr(
            &mut data,
            VIEWS_FOLDER,
            "bim:view:folders",
            "string[]",
            sdf::Value::StringVec(folders),
        )?;
    }
    for view in &ordered {
        author(&mut data, view)?;
    }
    edit::serialize(&data, Budgets::default().layer_bytes)
}

/// The digest of the canonical layer bytes; the caller must compare it against an independently
/// stored value, never self-certify.
pub fn layer_digest(bytes: &[u8]) -> Result<ContentDigest, Diagnostic> {
    Sha256::digest(bytes).map_err(|_| limit())
}

fn property<'a>(p: &'a Prim, name: &str, ty: &str) -> Result<&'a Value, Diagnostic> {
    p.properties
        .iter()
        .find(|v| v.name == name && v.type_name == ty)
        .and_then(|v| v.value.as_ref())
        .ok_or_else(invalid)
}

fn text<'a>(p: &'a Prim, name: &str, ty: &str) -> Result<&'a str, Diagnostic> {
    match property(p, name, ty)? {
        Value::Text(s) => Ok(s),
        _ => Err(invalid()),
    }
}

fn boolean(p: &Prim, name: &str) -> Result<bool, Diagnostic> {
    match property(p, name, "bool")? {
        Value::Bool(v) => Ok(*v),
        _ => Err(invalid()),
    }
}

fn number(value: &Value) -> Result<f64, Diagnostic> {
    match value {
        Value::Number(n) if n.is_finite() => Ok(*n),
        _ => Err(invalid()),
    }
}

fn double(p: &Prim, name: &str) -> Result<f64, Diagnostic> {
    number(property(p, name, "double")?)
}

/// A `double[]` of exactly `N` finite numbers.
fn numbers<const N: usize>(p: &Prim, name: &str) -> Result<[f64; N], Diagnostic> {
    let Value::Array(values) = property(p, name, "double[]")? else {
        return Err(invalid());
    };
    let read = values.iter().map(number).collect::<Result<Vec<_>, _>>()?;
    <[f64; N]>::try_from(read).map_err(|_| invalid())
}

/// A `double3` as three finite numbers.
fn triple(p: &Prim, name: &str) -> Result<[f64; 3], Diagnostic> {
    let Value::Array(values) = property(p, name, "double3")? else {
        return Err(invalid());
    };
    let read = values.iter().map(number).collect::<Result<Vec<_>, _>>()?;
    <[f64; 3]>::try_from(read).map_err(|_| invalid())
}

fn tokens(p: &Prim, name: &str) -> Result<Vec<String>, Diagnostic> {
    let Value::Array(values) = property(p, name, "token[]")? else {
        return Err(invalid());
    };
    values
        .iter()
        .map(|t| match t {
            Value::Text(s) => Ok(s.clone()),
            _ => Err(invalid()),
        })
        .collect()
}

/// Reads an optional attribute through its reader; an absent property is `None`.
fn optional<T>(
    p: &Prim,
    name: &str,
    read: impl Fn(&Prim) -> Result<T, Diagnostic>,
) -> Result<Option<T>, Diagnostic> {
    // Presence is by name alone: a property declared with another USD type is invalid, never
    // silently absent (`validate::optional`).
    if p.properties.iter().any(|v| v.name == name) {
        Ok(Some(read(p)?))
    } else {
        Ok(None)
    }
}

/// Decodes and validates one owned camera prim (W01).
pub fn read(p: &Prim) -> Result<SavedView, Diagnostic> {
    let id = Uuid7::parse(text(p, "bim:view:id", "string")?)
        .map_err(|_| fail(ErrorCode::InvalidId, "invalid saved-view identity"))?;
    if p.path != view_path(&id) {
        return Err(invalid());
    }
    let name = p.display_name.clone().ok_or_else(invalid)?;
    let orientation = {
        // USD stores a quaternion real-first; F04 maps it to x,y,z,w order.
        let Value::Array(parts) = property(p, "xformOp:orient", "quatd")? else {
            return Err(invalid());
        };
        let read = parts.iter().map(number).collect::<Result<Vec<_>, _>>()?;
        let [w, x, y, z] = <[f64; 4]>::try_from(read).map_err(|_| invalid())?;
        [x, y, z, w]
    };
    let view = SavedView {
        id,
        name,
        placement: Placement {
            translation_mm: triple(p, "xformOp:translate")?,
            orientation_xyzw: orientation,
        },
        projection: CameraProjection::parse(text(p, "projection", "token")?)?,
        span_mm: double(p, "bim:view:spanMm")?,
        depth_mm: optional(p, "bim:view:depthMm", |p| numbers(p, "bim:view:depthMm"))?,
        render_mode: text(p, "bim:view:renderMode", "token")?.to_owned(),
        grids_visible: boolean(p, "bim:view:grids")?,
        hidden_kinds: tokens(p, "bim:view:hiddenKinds")?,
        folder: text(p, "bim:view:folder", "string")?.to_owned(),
        favorite: boolean(p, "bim:view:favorite")?,
        // The mark predates the layer form, so its absence is the unmarked default, while a
        // present property of another USD type still refuses the prim (`optional`).
        default: optional(p, "bim:view:default", |p| boolean(p, "bim:view:default"))?
            .unwrap_or(false),
        work_area_mm: optional(p, "bim:view:workAreaMm", |p| {
            let Value::Array(entries) = property(p, "bim:view:workAreaMm", "double3[]")? else {
                return Err(invalid());
            };
            let mut corners = Vec::with_capacity(entries.len());
            for entry in entries {
                let Value::Array(parts) = entry else {
                    return Err(invalid());
                };
                let read = parts.iter().map(number).collect::<Result<Vec<_>, _>>()?;
                corners.push(<[f64; 3]>::try_from(read).map_err(|_| invalid())?);
            }
            <[[f64; 3]; 2]>::try_from(corners).map_err(|_| invalid())
        })?,
        rotation_rad: numbers(p, "bim:view:rotationRad")?,
        color_rgb: optional(p, "bim:view:colorRgb", |p| triple(p, "bim:view:colorRgb"))?,
        opacity: double(p, "bim:view:opacity")?,
        stamp: {
            let present = |name: &str| p.properties.iter().any(|v| v.name == name);
            match (
                present("bim:view:author"),
                present("bim:view:savedAtMs"),
                present("bim:view:comment"),
                present("bim:view:modelRevision"),
            ) {
                (false, false, false, false) => None,
                (true, true, true, true) => {
                    let created = match property(p, "bim:view:savedAtMs", "uint64")? {
                        Value::Uint(v) => *v,
                        _ => return Err(invalid()),
                    };
                    let revision = Uuid7::parse(text(p, "bim:view:modelRevision", "string")?)
                        .map_err(|_| fail(ErrorCode::InvalidId, "invalid model revision"))?;
                    Some(ViewStamp::new(
                        text(p, "bim:view:author", "string")?,
                        text(p, "bim:view:comment", "string")?,
                        revision,
                        created,
                    )?)
                }
                _ => return Err(invalid()),
            }
        },
    };
    view.validate()?;
    Ok(view)
}

/// Reads the owned content of a composed snapshot: the camera prims in identity order and the
/// folders no view uses. Foreign cameras are displayed sources and are never read; the
/// project-wide caps bind.
pub fn read_layer(prims: &[Prim]) -> Result<ViewsLayer, Diagnostic> {
    let scope = format!("{VIEWS_FOLDER}/");
    let mut layer = ViewsLayer::default();
    for p in prims
        .iter()
        .filter(|p| !p.sources.is_empty() && p.sources.iter().all(|s| s.owned))
    {
        match p.type_name.as_deref() {
            Some("Camera") if p.path.starts_with(&scope) => {
                if layer.views.len() >= MAX_VIEWS {
                    return Err(limit());
                }
                layer.views.push(read(p)?);
            }
            _ if p.path == VIEWS_FOLDER => {
                // Presence is by name alone: a `bim:view:folders` of another USD type, or one
                // whose parsed value is not an array, refuses the prim, never silently absent
                // (the same rule the optional record attributes follow).
                if p.properties.iter().any(|v| v.name == "bim:view:folders") {
                    let Value::Array(entries) = property(p, "bim:view:folders", "string[]")? else {
                        return Err(invalid());
                    };
                    if entries.len() > 512 {
                        return Err(limit());
                    }
                    for entry in entries {
                        let Value::Text(folder) = entry else {
                            return Err(invalid());
                        };
                        validate_folder(folder)?;
                        layer.empty_folders.push(folder.clone());
                    }
                }
            }
            _ => (),
        }
    }
    layer.views.sort_by_key(|v| v.id);
    Ok(layer)
}
