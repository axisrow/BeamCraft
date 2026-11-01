// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The steel definitions of the authored layer (#388 slice): the rectangle, section and
//! fastener definitions behind the discipline definition hooks, and the readers of the
//! stored definition prims, re-admitted because a committed layer is untrusted input.
//! Each hook body is the arm it replaced in the platform's `edit.rs` and `validate.rs`,
//! moved unchanged, so the bytes written and the refusals returned are the same.

use super::write::{
    optional_number as write_optional_number, optional_text as write_optional_text,
    section_properties as write_section_properties,
};
use crate::model::fastener::{FastenerKind, HeadKind, HoleMode, WasherSpec};
use crate::model::section::{
    CustomSection, MAX_CUSTOM_CAVITIES, MAX_CUSTOM_POINTS, ParametricSection, SectionFamily,
};
use crate::model::{definition::SectionProperties, fastener::FastenerSpec, section::SectionShape};
use beamcraft_layer_engine::authored::seam::author::{
    attr as author_attr, points as author_points,
};
use beamcraft_layer_engine::authored::seam::read::{
    number, optional_number, optional_text, points as read_points, required, text,
};
use beamcraft_layer_engine::authored::{Definition, DefinitionHook, HookedDefinition, invalid};
use beamcraft_layer_engine::{Diagnostic, Prim, Value, limit};
use beamcraft_model::limits::MAX_FASTENER_LENGTHS;
use beamcraft_model::prim::DefinitionCategory;
use openusd::sdf;
use std::any::Any;

/// Every definition kind a member's `bim:profile` may target.
pub(super) const PROFILE_KINDS: [&str; 11] = [
    "rectangle",
    "i_shape",
    "channel",
    "angle",
    "rhs",
    "chs",
    "lipped_c",
    "plain_c",
    "lipped_z",
    "omega",
    "custom",
];

fn section_properties(p: &Prim) -> Result<SectionProperties, Diagnostic> {
    Ok(SectionProperties {
        designation: optional_text(p, "steel:profile:designation")?,
        standard: optional_text(p, "steel:profile:standard")?,
        area_mm2: optional_number(p, "steel:profile:areaMm2")?,
        mass_per_meter_kg: optional_number(p, "steel:profile:massPerMeterKg")?,
        surface_per_meter_m2: optional_number(p, "steel:profile:surfacePerMeterM2")?,
        display_name: p.display_name.clone(),
    })
}
/// The authored outline of a custom profile: the outer contour and the optional cavities (both
/// cavity attributes or neither, counts adding up to the points), all at z = 0 and capped.
fn custom_section(p: &Prim) -> Result<CustomSection, Diagnostic> {
    let flat = |points: Vec<[f64; 3]>| -> Result<Vec<[f64; 2]>, Diagnostic> {
        if points.len() > MAX_CUSTOM_POINTS {
            return Err(limit());
        }
        if points.iter().any(|q| q[2] != 0.0) {
            return Err(invalid());
        }
        Ok(points.into_iter().map(|[x, y, _]| [x, y]).collect())
    };
    let outer = flat(read_points(p, "steel:profile:outerPointsMm")?)?;
    let present = |name: &str| p.properties.iter().any(|q| q.name == name);
    let (has_points, has_counts) = (
        present("steel:profile:cavityPointsMm"),
        present("steel:profile:cavityVertexCounts"),
    );
    if has_points != has_counts {
        return Err(invalid());
    }
    let mut cavities = Vec::new();
    if has_points {
        let all = flat(read_points(p, "steel:profile:cavityPointsMm")?)?;
        let Value::Array(counts) = required(p, "steel:profile:cavityVertexCounts", "int[]")? else {
            return Err(invalid());
        };
        if counts.len() > MAX_CUSTOM_CAVITIES {
            return Err(limit());
        }
        let mut at = 0usize;
        for count in counts {
            let n = match count {
                Value::Int(n) => usize::try_from(*n).map_err(|_| invalid())?,
                _ => return Err(invalid()),
            };
            if n < 3 {
                return Err(invalid());
            }
            let end = at.checked_add(n).ok_or_else(invalid)?;
            cavities.push(all.get(at..end).ok_or_else(invalid)?.to_vec());
            at = end;
        }
        if at != all.len() {
            return Err(invalid());
        }
    }
    Ok(CustomSection { outer, cavities })
}
/// The logical definition stored in a definition prim of this discipline, or `None` when the
/// `bim:definitionKind` is not one it owns. A stored layer is untrusted input: the definition
/// is re-admitted, never trusted.
pub(super) fn read_definition(p: &Prim, kind: &str) -> Result<Option<Definition>, Diagnostic> {
    if let Some(family) = SectionFamily::from_token(kind) {
        let values = family
            .parameters()
            .iter()
            .map(|name| number(p, &format!("steel:profile:{name}Mm")))
            .collect::<Result<Vec<_>, _>>()?;
        let parametric = ParametricSection::new(family, values).map_err(|_| invalid())?;
        return Ok(Some(section(
            SectionShape::Parametric(parametric),
            section_properties(p)?,
        )));
    }
    match kind {
        "rectangle" => Ok(Some(rectangle(
            number(p, "steel:profile:widthMm")?,
            number(p, "steel:profile:heightMm")?,
            section_properties(p)?,
        ))),
        "custom" => Ok(Some(section(
            SectionShape::Custom(custom_section(p)?),
            section_properties(p)?,
        ))),
        "material" => Ok(Some(Definition::Material {
            density_kg_m3: optional_number(p, "bim:densityKgM3")?,
            grade: optional_text(p, "bim:materialGrade")?,
            standard: optional_text(p, "bim:materialStandard")?,
            display_name: p.display_name.clone(),
        })),
        "fastener" => Ok(Some(fastener(read_fastener(p)?))),
        _ => Ok(None),
    }
}

/// The logical fastener definition stored in a fastener definition prim (A14.1): every value
/// re-read and re-admitted, because a committed layer is untrusted input.
fn read_fastener(p: &Prim) -> Result<FastenerSpec, Diagnostic> {
    let token = |name: &str| text(p, name, "token").map(str::to_owned);
    let kind = FastenerKind::from_token(&token("steel:fastener:kind")?).ok_or_else(invalid)?;
    let head_kind = HeadKind::from_token(&token("steel:fastener:headKind")?).ok_or_else(invalid)?;
    let nut_across_flats_mm = optional_number(p, "steel:fastener:nutAcrossFlatsMm")?;
    let nut_height_mm = optional_number(p, "steel:fastener:nutHeightMm")?;
    let washer = match (
        optional_number(p, "steel:fastener:washerOuterDiameterMm")?,
        optional_number(p, "steel:fastener:washerInnerDiameterMm")?,
        optional_number(p, "steel:fastener:washerThicknessMm")?,
    ) {
        (None, None, None) => None,
        (outer, inner, thickness) => Some(WasherSpec {
            outer_diameter_mm: outer.ok_or_else(invalid)?,
            inner_diameter_mm: inner.ok_or_else(invalid)?,
            thickness_mm: thickness.ok_or_else(invalid)?,
        }),
    };
    let lengths = match required(p, "steel:fastener:lengthsMm", "double[]")? {
        Value::Array(values) => values
            .iter()
            .map(|v| match v {
                Value::Number(n) => Ok(*n),
                _ => Err(invalid()),
            })
            .collect::<Result<Vec<_>, Diagnostic>>()?,
        _ => return Err(invalid()),
    };
    if lengths.len() > MAX_FASTENER_LENGTHS {
        return Err(limit());
    }
    let hole_mode = HoleMode::from_token(&token("steel:fastener:holeMode")?).ok_or_else(invalid)?;
    Ok(FastenerSpec {
        designation: optional_text(p, "steel:fastener:designation")?,
        standard: optional_text(p, "steel:fastener:standard")?,
        grade: optional_text(p, "steel:fastener:grade")?,
        display_name: p.display_name.clone(),
        kind,
        head_kind,
        nominal_diameter_mm: number(p, "steel:fastener:nominalDiameterMm")?,
        head_across_flats_mm: number(p, "steel:fastener:headAcrossFlatsMm")?,
        head_height_mm: number(p, "steel:fastener:headHeightMm")?,
        nut_across_flats_mm,
        nut_height_mm,
        washer,
        hole_clearance_mm: number(p, "steel:fastener:holeClearanceMm")?,
        lengths_mm: lengths,
        thread_protrusion_mm: number(p, "steel:fastener:threadProtrusionMm")?,
        mass_each_kg: optional_number(p, "steel:fastener:massEachKg")?,
        washer_under_head: match required(p, "steel:fastener:washerUnderHead", "bool")? {
            Value::Bool(v) => *v,
            _ => return Err(invalid()),
        },
        washer_under_nut: match required(p, "steel:fastener:washerUnderNut", "bool")? {
            Value::Bool(v) => *v,
            _ => return Err(invalid()),
        },
        hole_mode,
    })
}

/// The platform's mapping of a refused definition value onto the layer diagnostic.
fn refused(e: beamcraft_model::command::CommandError) -> Diagnostic {
    Diagnostic {
        code: e.code,
        message: "definition value refused by the F04 admission rules",
        rule_id: e.rule_id,
    }
}

/// The authored rectangle profile behind the discipline definition hook: F06 section 2's only
/// initial profile, with declared designation, standard and section numbers whose absent values
/// stay unknown.
#[derive(Debug, Clone, PartialEq)]
pub struct RectangleProfileDef {
    /// Width in millimetres.
    pub width_mm: f64,
    /// Height in millimetres.
    pub height_mm: f64,
    /// Declared designation, standard and section numbers; absent values stay unknown.
    pub section: SectionProperties,
}

/// The authored section profile behind the discipline definition hook (A07.1): a parametric
/// family or an authored outline.
#[derive(Debug, Clone, PartialEq)]
pub struct SectionProfileDef {
    /// The section shape.
    pub shape: SectionShape,
    /// Declared designation, standard and section numbers; absent values stay unknown.
    pub section: SectionProperties,
}

/// The authored fastener definition behind the discipline definition hook (A14.1, bolt
/// amendment section 2): one size of one standard.
#[derive(Debug, Clone, PartialEq)]
pub struct FastenerDef {
    /// The complete fastener values, admitted by the model crate.
    pub spec: FastenerSpec,
}

/// Builds the platform definition of a rectangle profile; see [`RectangleProfileDef`].
pub fn rectangle(width_mm: f64, height_mm: f64, section: SectionProperties) -> Definition {
    Definition::Discipline(HookedDefinition::new(RectangleProfileDef {
        width_mm,
        height_mm,
        section,
    }))
}

/// Builds the platform definition of a section profile; see [`SectionProfileDef`].
pub fn section(shape: SectionShape, section: SectionProperties) -> Definition {
    Definition::Discipline(HookedDefinition::new(SectionProfileDef { shape, section }))
}

/// Builds the platform definition of a fastener; see [`FastenerDef`].
pub fn fastener(spec: FastenerSpec) -> Definition {
    Definition::Discipline(HookedDefinition::new(FastenerDef { spec }))
}

/// Authors the `bim:definitionKind` token of a definition prim.
fn attr_kind(data: &mut sdf::Data, p: &str, kind: &'static str) -> Result<(), Diagnostic> {
    author_attr(
        data,
        p,
        "bim:definitionKind",
        "token",
        sdf::Value::token(kind),
    )
}

/// Authors one `double3[]` attribute of a definition prim.
fn attr_points(
    data: &mut sdf::Data,
    p: &str,
    name: &str,
    values: &[[f64; 3]],
) -> Result<(), Diagnostic> {
    author_attr(data, p, name, "double3[]", author_points(values))
}

/// The declared section numbers of a profile definition, in storage order. The per-family
/// parameter attributes of a parametric section are dynamic (`steel:profile:<parameter>Mm`)
/// and cannot be listed under fixed names.
fn section_fields() -> Vec<(&'static str, &'static str)> {
    vec![
        ("steel:profile:designation", "string"),
        ("steel:profile:standard", "string"),
        ("steel:profile:areaMm2", "double"),
        ("steel:profile:massPerMeterKg", "double"),
        ("steel:profile:surfacePerMeterM2", "double"),
    ]
}

impl DefinitionHook for RectangleProfileDef {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn same_as(&self, other: &dyn DefinitionHook) -> bool {
        other.as_any().downcast_ref::<Self>() == Some(self)
    }

    fn kind_token(&self) -> &'static str {
        "rectangle"
    }

    fn category(&self) -> DefinitionCategory {
        DefinitionCategory::Profile
    }

    fn author(&self, data: &mut sdf::Data, p: &str) -> Result<(), Diagnostic> {
        attr_kind(data, p, "rectangle")?;
        write_optional_number(data, p, "steel:profile:widthMm", Some(self.width_mm))?;
        write_optional_number(data, p, "steel:profile:heightMm", Some(self.height_mm))?;
        write_section_properties(data, p, &self.section)
    }

    fn attributes(&self) -> Vec<(&'static str, &'static str)> {
        let mut out = vec![
            ("bim:definitionKind", "token"),
            ("steel:profile:widthMm", "double"),
            ("steel:profile:heightMm", "double"),
        ];
        out.extend(section_fields());
        out
    }

    fn validate(&self) -> Result<(), Diagnostic> {
        crate::model::definition::RectangleProfileSpec {
            width_mm: self.width_mm,
            height_mm: self.height_mm,
            section: self.section.clone(),
        }
        .validate()
        .map_err(refused)
    }

    fn check_finite(&self) -> Result<(), Diagnostic> {
        Ok(())
    }

    fn display_name(&self) -> Option<&str> {
        self.section.display_name.as_deref()
    }
}

impl DefinitionHook for SectionProfileDef {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn same_as(&self, other: &dyn DefinitionHook) -> bool {
        other.as_any().downcast_ref::<Self>() == Some(self)
    }

    fn kind_token(&self) -> &'static str {
        self.shape.token()
    }

    fn category(&self) -> DefinitionCategory {
        DefinitionCategory::Profile
    }

    fn author(&self, data: &mut sdf::Data, p: &str) -> Result<(), Diagnostic> {
        attr_kind(data, p, self.shape.token())?;
        match &self.shape {
            SectionShape::Parametric(parametric) => {
                let names = parametric.family().parameters();
                for (name, value) in names.iter().zip(parametric.values()) {
                    write_optional_number(
                        data,
                        p,
                        &format!("steel:profile:{name}Mm"),
                        Some(*value),
                    )?;
                }
            }
            SectionShape::Custom(custom) => {
                let flat = |c: &[[f64; 2]]| c.iter().map(|[x, y]| [*x, *y, 0.0]).collect();
                let outer: Vec<[f64; 3]> = flat(&custom.outer);
                attr_points(data, p, "steel:profile:outerPointsMm", &outer)?;
                if !custom.cavities.is_empty() {
                    let cavities: Vec<[f64; 3]> =
                        custom.cavities.iter().flat_map(|c| flat(c)).collect();
                    let counts = custom
                        .cavities
                        .iter()
                        .map(|c| i32::try_from(c.len()).map_err(|_| limit()))
                        .collect::<Result<Vec<_>, _>>()?;
                    attr_points(data, p, "steel:profile:cavityPointsMm", &cavities)?;
                    author_attr(
                        data,
                        p,
                        "steel:profile:cavityVertexCounts",
                        "int[]",
                        sdf::Value::IntVec(counts),
                    )?;
                }
            }
        }
        write_section_properties(data, p, &self.section)
    }

    fn attributes(&self) -> Vec<(&'static str, &'static str)> {
        let mut out = vec![("bim:definitionKind", "token")];
        if let SectionShape::Custom(_) = &self.shape {
            out.push(("steel:profile:outerPointsMm", "double3[]"));
        }
        out.extend(section_fields());
        out
    }

    fn validate(&self) -> Result<(), Diagnostic> {
        crate::model::definition::SectionProfileSpec {
            shape: self.shape.clone(),
            section: self.section.clone(),
        }
        .validate()
        .map_err(refused)
    }

    fn check_finite(&self) -> Result<(), Diagnostic> {
        Ok(())
    }

    fn display_name(&self) -> Option<&str> {
        self.section.display_name.as_deref()
    }
}

impl DefinitionHook for FastenerDef {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn same_as(&self, other: &dyn DefinitionHook) -> bool {
        other.as_any().downcast_ref::<Self>() == Some(self)
    }

    fn kind_token(&self) -> &'static str {
        "fastener"
    }

    fn category(&self) -> DefinitionCategory {
        DefinitionCategory::Fastener
    }

    fn author(&self, data: &mut sdf::Data, p: &str) -> Result<(), Diagnostic> {
        let spec = &self.spec;
        attr_kind(data, p, "fastener")?;
        author_attr(
            data,
            p,
            "steel:fastener:kind",
            "token",
            sdf::Value::token(spec.kind.token()),
        )?;
        author_attr(
            data,
            p,
            "steel:fastener:headKind",
            "token",
            sdf::Value::token(spec.head_kind.token()),
        )?;
        for (name, value) in [
            ("nominalDiameterMm", spec.nominal_diameter_mm),
            ("headAcrossFlatsMm", spec.head_across_flats_mm),
            ("headHeightMm", spec.head_height_mm),
        ] {
            author_attr(
                data,
                p,
                &format!("steel:fastener:{name}"),
                "double",
                sdf::Value::Double(value),
            )?;
        }
        write_optional_number(
            data,
            p,
            "steel:fastener:nutAcrossFlatsMm",
            spec.nut_across_flats_mm,
        )?;
        write_optional_number(data, p, "steel:fastener:nutHeightMm", spec.nut_height_mm)?;
        if let Some(washer) = &spec.washer {
            for (name, value) in [
                ("washerOuterDiameterMm", washer.outer_diameter_mm),
                ("washerInnerDiameterMm", washer.inner_diameter_mm),
                ("washerThicknessMm", washer.thickness_mm),
            ] {
                author_attr(
                    data,
                    p,
                    &format!("steel:fastener:{name}"),
                    "double",
                    sdf::Value::Double(value),
                )?;
            }
        }
        author_attr(
            data,
            p,
            "steel:fastener:holeClearanceMm",
            "double",
            sdf::Value::Double(spec.hole_clearance_mm),
        )?;
        author_attr(
            data,
            p,
            "steel:fastener:lengthsMm",
            "double[]",
            sdf::Value::DoubleVec(spec.lengths_mm.clone()),
        )?;
        author_attr(
            data,
            p,
            "steel:fastener:threadProtrusionMm",
            "double",
            sdf::Value::Double(spec.thread_protrusion_mm),
        )?;
        write_optional_number(data, p, "steel:fastener:massEachKg", spec.mass_each_kg)?;
        author_attr(
            data,
            p,
            "steel:fastener:washerUnderHead",
            "bool",
            sdf::Value::Bool(spec.washer_under_head),
        )?;
        author_attr(
            data,
            p,
            "steel:fastener:washerUnderNut",
            "bool",
            sdf::Value::Bool(spec.washer_under_nut),
        )?;
        author_attr(
            data,
            p,
            "steel:fastener:holeMode",
            "token",
            sdf::Value::token(spec.hole_mode.token()),
        )?;
        write_optional_text(data, p, "steel:fastener:designation", &spec.designation)?;
        write_optional_text(data, p, "steel:fastener:standard", &spec.standard)?;
        write_optional_text(data, p, "steel:fastener:grade", &spec.grade)?;
        Ok(())
    }

    fn attributes(&self) -> Vec<(&'static str, &'static str)> {
        let mut out = vec![
            ("bim:definitionKind", "token"),
            ("steel:fastener:kind", "token"),
            ("steel:fastener:headKind", "token"),
            ("steel:fastener:nominalDiameterMm", "double"),
            ("steel:fastener:headAcrossFlatsMm", "double"),
            ("steel:fastener:headHeightMm", "double"),
            ("steel:fastener:nutAcrossFlatsMm", "double"),
            ("steel:fastener:nutHeightMm", "double"),
            ("steel:fastener:holeClearanceMm", "double"),
            ("steel:fastener:lengthsMm", "double[]"),
            ("steel:fastener:threadProtrusionMm", "double"),
            ("steel:fastener:massEachKg", "double"),
            ("steel:fastener:washerUnderHead", "bool"),
            ("steel:fastener:washerUnderNut", "bool"),
            ("steel:fastener:holeMode", "token"),
            ("steel:fastener:designation", "string"),
            ("steel:fastener:standard", "string"),
            ("steel:fastener:grade", "string"),
        ];
        if self.spec.washer.is_some() {
            out.extend([
                ("steel:fastener:washerOuterDiameterMm", "double"),
                ("steel:fastener:washerInnerDiameterMm", "double"),
                ("steel:fastener:washerThicknessMm", "double"),
            ]);
        }
        out
    }

    fn validate(&self) -> Result<(), Diagnostic> {
        self.spec.validate().map_err(refused)
    }

    fn check_finite(&self) -> Result<(), Diagnostic> {
        Ok(())
    }

    fn display_name(&self) -> Option<&str> {
        self.spec.display_name.as_deref()
    }
}
