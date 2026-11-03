// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The U08 portable appearance: every classed part carries a display colour that any USD
//! consumer can see, without running BeamCraft. The colour comes from presentation — the
//! part's `steel:class` read through the P15 class palette — never from the engineering
//! material. The authored form is the standard USD one: a `UsdShade` Material per palette
//! entry with a `UsdPreviewSurface` shader, a `material:binding` relationship on the element
//! (with `MaterialBindingAPI` applied), and `primvars:displayColor` as the fallback that
//! viewers without material support still colour by.

use super::validate::text;
use super::*;
use beamcraft_model::class_palette::{DEFAULT_CLASS_PALETTE, class_colour};
use openusd::sdf::{self, AbstractData};

/// The reserved appearance scope of an authored layer.
pub(super) const LOOKS_PATH: &str = "/World/Looks";
/// The api schema that declares the binding relationship of a part (U08).
const BINDING_API: &str = "MaterialBindingAPI";
/// The property that carries the fallback colour of a part (U08).
const DISPLAY_COLOR: &str = "primvars:displayColor";
/// The direct-binding relationship of a part (U08).
pub(super) const BINDING: &str = "material:binding";

/// The palette entry a class reads as: the same wrap-around the engine's `class_colour`
/// applies, so the viewport overlay and the authored layers agree.
pub(super) fn look_index(class: i32) -> usize {
    u64::from(class.unsigned_abs()) as usize % DEFAULT_CLASS_PALETTE.len()
}

/// The look a class binds to.
pub(super) fn look_path(class: i32) -> String {
    format!("{LOOKS_PATH}/PartClass{}", look_index(class))
}

/// One sRGB palette byte triple as the linear RGB a USD material expects.
fn linear(colour: [u8; 3]) -> [f32; 3] {
    let [r, g, b] = colour;
    let channel = |byte: u8| {
        let c = f64::from(byte) / 255.0;
        let linear = if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        };
        linear as f32
    };
    [channel(r), channel(g), channel(b)]
}

/// The fallback colour of a class, as authored.
pub(super) fn display_color(class: i32) -> Result<[f32; 3], Diagnostic> {
    match class_colour(class, &DEFAULT_CLASS_PALETTE) {
        Some(colour) => Ok(linear(colour)),
        // The default palette has an entry for every class; this is unreachable today.
        None => Err(invalid()),
    }
}

/// Authors one uniform attribute spec, the appearance form of the seam's `attribute` helper.
/// `None` authors a valueless output such as `outputs:surface`.
fn uniform(
    data: &mut sdf::Data,
    p: &str,
    name: &str,
    ty: &str,
    value: Option<sdf::Value>,
) -> Result<(), Diagnostic> {
    let full = super::edit::path(&format!("{p}.{name}"))?;
    if !data.has_spec(&full) {
        data.create_spec(full.clone(), sdf::SpecType::Attribute);
        super::edit::child(data, p, "propertyChildren", name, false)?;
    } else if data.spec_type(&full) != Some(sdf::SpecType::Attribute) {
        return Err(unsupported());
    }
    data.set_field(&full, "typeName", sdf::Value::token(ty));
    data.set_field(&full, "custom", sdf::Value::Bool(false));
    data.set_field(
        &full,
        "variability",
        sdf::Value::Variability(sdf::Variability::Uniform),
    );
    match value {
        Some(v) => data.set_field(&full, "default", v),
        None => data.erase_field(&full, "default"),
    }
    Ok(())
}

/// Authors the fixed look of one palette entry: a Material whose `outputs:surface` connects
/// to a UsdPreviewSurface shader carrying the class colour.
fn author_look(data: &mut sdf::Data, index: usize) -> Result<(), Diagnostic> {
    let mat = format!("{LOOKS_PATH}/PartClass{index}");
    let shader = format!("{mat}/PreviewSurface");
    let colour = match DEFAULT_CLASS_PALETTE.get(index) {
        Some(c) => linear(*c),
        None => return Err(invalid()),
    };
    super::edit::prim(data, &mat, Some("Material"), true)?;
    uniform(data, &mat, "outputs:surface", "token", None)?;
    data.set_field(
        &super::edit::path(&format!("{mat}.outputs:surface"))?,
        "connectionPaths",
        sdf::Value::PathListOp(sdf::ListOp::explicit(vec![super::edit::path(&format!(
            "{shader}.outputs:surface"
        ))?])),
    );
    super::edit::prim(data, &shader, Some("Shader"), true)?;
    uniform(
        data,
        &shader,
        "info:id",
        "token",
        Some(sdf::Value::token("UsdPreviewSurface")),
    )?;
    uniform(
        data,
        &shader,
        "inputs:diffuseColor",
        "color3f",
        Some(sdf::Value::Vec3f(colour.into())),
    )?;
    uniform(data, &shader, "outputs:surface", "token", None)?;
    Ok(())
}

/// Authors the reserved appearance scope with the eight class looks. Idempotent: every
/// batch that touches a part's class re-asserts the fixed bytes.
pub(super) fn author_looks(data: &mut sdf::Data) -> Result<(), Diagnostic> {
    super::edit::container(data, LOOKS_PATH, "Scope", None)?;
    for index in 0..DEFAULT_CLASS_PALETTE.len() {
        author_look(data, index)?;
    }
    Ok(())
}

/// Authors the appearance of one classed part: the binding api, the fallback colour and
/// the direct binding, replaced as a whole on every class edit.
pub(super) fn author_element(data: &mut sdf::Data, p: &str, class: i32) -> Result<(), Diagnostic> {
    let colour = display_color(class)?;
    author_looks(data)?;
    data.set_field(
        &super::edit::path(p)?,
        "apiSchemas",
        sdf::Value::TokenListOp(sdf::TokenListOp::prepended(
            [BINDING_API].map(openusd::tf::Token::from),
        )),
    );
    super::edit::attribute(
        data,
        p,
        DISPLAY_COLOR,
        "color3f[]",
        Opinion::Value(sdf::Value::Vec3fVec(vec![colour.into()])),
        true,
    )?;
    // The primvar is a constant, not a per-sample value.
    data.set_field(
        &super::edit::path(&format!("{p}.{DISPLAY_COLOR}"))?,
        "variability",
        sdf::Value::Variability(sdf::Variability::Uniform),
    );
    super::edit::relationship_targets(
        data,
        p,
        BINDING,
        Some(vec![super::edit::path(&look_path(class))?]),
    )?;
    Ok(())
}

/// The exact appearance of one supported element. A classed part carries the pair its class
/// maps to; every other supported kind carries none.
pub(super) fn validate_element(p: &Prim, classed: bool) -> Result<(), Diagnostic> {
    let class = match p.properties.iter().find(|a| a.name == "steel:class") {
        Some(prop) => match prop.value {
            Some(Value::Int(class)) => i32::try_from(class).map_err(|_| invalid())?,
            _ => return Err(invalid()),
        },
        None if !classed => {
            if p.properties.iter().any(|a| a.name == DISPLAY_COLOR)
                || p.relationships.iter().any(|r| r.name == BINDING)
            {
                return Err(unsupported());
            }
            return Ok(());
        }
        None => return Err(invalid()),
    };
    let expected = display_color(class)?;
    let stored = match p.properties.iter().find(|a| a.name == DISPLAY_COLOR) {
        Some(a) if a.type_name == "color3f[]" => {
            let triple = match a.value.as_ref() {
                Some(Value::Array(items)) => match items.as_slice() {
                    [Value::Array(triple)] => triple.clone(),
                    _ => return Err(invalid()),
                },
                _ => return Err(invalid()),
            };
            let read = triple
                .iter()
                .map(|v| match v {
                    Value::Number(n) => Ok(*n as f32),
                    _ => Err(invalid()),
                })
                .collect::<Result<Vec<f32>, _>>()?;
            match read.as_slice() {
                [r, g, b] => [*r, *g, *b],
                _ => return Err(invalid()),
            }
        }
        _ => return Err(invalid()),
    };
    if stored
        .iter()
        .zip(expected)
        .any(|(s, e)| (s - e).abs() > 1.0 / 255.0)
    {
        return Err(invalid());
    }
    match p
        .relationships
        .iter()
        .find(|r| r.name == BINDING)
        .map(|r| r.targets.as_slice())
    {
        Some([target]) if target == &look_path(class) => {}
        _ => return Err(invalid()),
    }
    Ok(())
}

/// The reserved appearance scope of a loaded layer: only the eight fixed class looks,
/// exact in the fields the snapshot can see, and nothing else owned under it.
pub(super) fn validate_looks(prims: &[Prim]) -> Result<(), Diagnostic> {
    let prefix = format!("{LOOKS_PATH}/");
    for p in prims
        .iter()
        .filter(|p| !p.sources.is_empty() && p.sources.iter().all(|s| s.owned))
        .filter(|p| p.path == LOOKS_PATH || p.path.starts_with(&prefix))
    {
        if p.path == LOOKS_PATH {
            if p.type_name.as_deref() != Some("Scope") {
                return Err(unsupported());
            }
            continue;
        }
        let name = p.path.rsplit_once('/').map_or(p.path.as_str(), |(_, n)| n);
        let shader = name == "PreviewSurface";
        // The closed property set of a look: a Material carries only its valueless surface
        // output; the shader adds the UsdPreviewSurface identity and its diffuse colour.
        // Any other property, with or without a value, is outside the reserved scope.
        let allowed: &[&str] = if shader {
            &["info:id", "inputs:diffuseColor", "outputs:surface"]
        } else {
            &["outputs:surface"]
        };
        if p.properties.len() != allowed.len()
            || p.properties
                .iter()
                .any(|a| !allowed.contains(&a.name.as_str()))
        {
            return Err(unsupported());
        }
        // The look's index names the palette entry: the material path for a shader child,
        // the prim's own path otherwise.
        let holder = if shader {
            p.path
                .rsplit_once('/')
                .map(|(parent, _)| parent.to_owned())
                .ok_or_else(invalid)?
        } else {
            p.path.clone()
        };
        let index = match holder
            .rsplit_once('/')
            .and_then(|(_, n)| n.strip_prefix("PartClass"))
        {
            Some(digits) => digits.parse::<usize>().map_err(|_| invalid())?,
            None => return Err(unsupported()),
        };
        let expected = match DEFAULT_CLASS_PALETTE.get(index) {
            Some(colour) => linear(*colour),
            None => return Err(unsupported()),
        };
        let output = p
            .properties
            .iter()
            .find(|a| a.name == "outputs:surface")
            .ok_or_else(invalid)?;
        if output.type_name != "token" {
            return Err(invalid());
        }
        if shader {
            if p.type_name.as_deref() != Some("Shader") {
                return Err(unsupported());
            }
            if text(p, "info:id", "token")? != "UsdPreviewSurface" {
                return Err(invalid());
            }
            // The diffuse colour is declared with an exact value, never valueless.
            let diffuse = match p
                .properties
                .iter()
                .find(|a| a.name == "inputs:diffuseColor")
                .map(|a| a.value.as_ref())
            {
                Some(Some(Value::Array(items))) => match items.as_slice() {
                    [Value::Number(r), Value::Number(g), Value::Number(b)] => {
                        [*r as f32, *g as f32, *b as f32]
                    }
                    _ => return Err(invalid()),
                },
                _ => return Err(invalid()),
            };
            if diffuse
                .iter()
                .zip(expected)
                .any(|(s, e)| (s - e).abs() > 1.0 / 255.0)
            {
                return Err(invalid());
            }
        } else if p.type_name.as_deref() != Some("Material") {
            return Err(unsupported());
        }
        if p.relationships.iter().any(|r| !r.targets.is_empty()) {
            return Err(invalid());
        }
    }
    Ok(())
}
