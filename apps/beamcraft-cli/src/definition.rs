// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! P02 adapter: immutable profile and material definitions as USD assets of a committed revision.
//!
//! A definition is a content-addressed `Scope` prim under `/World/Definitions/Materials/MAT_`
//! or `/World/Definitions/Profiles/PRF_`, followed by its full SHA-256, in the model
//! layer. Its digest is the full SHA-256 of the canonical isolated definition layer that the layer
//! engine writes (`Resource::bytes`), so a digest pins values, units (`mm`) and schema version
//! (`0.1.0`) together. Publishing writes the definition and no element; an element then pins it
//! through a `bim:material` (or, for members, `bim:profile`) relationship. The physical material
//! never carries display appearance: that is a separate reference and signature (F04 section 4).
//!
//! Scope and honesty:
//!
//! * Only the F06 rectangle profile and a material with an optional density exist. There is no
//!   catalogue, no commercial designation, no environment default and no density inferred from a
//!   name: a material without a density stays unknown, and so does the mass of its elements.
//! * No command consumes a profile yet (P03 adds the straight member), so a published profile is
//!   verified and listed but not referenced.
//! * Like `plate`, this runs the layer engine in-process (no LEW-03 containment) and is not a
//!   hostile-parser boundary. Definitions are not a F04 command: the contract lists
//!   contributor/engine-supplied immutable resources, so publication is an adapter operation
//!   whose identity is the content digest and which is therefore idempotent without an `op_id`
//!   replay record.

use crate::plate::{
    Base, PlateError, RevisionSummary, base_of, control, decode, host_uuid, integrity, load_base,
    number, open_base, summarize, text,
};
use crate::plate::{decode_polygon, decode_polygons};
use crate::publication::{PublishRequest, publish_revision};
use beamcraft_engine::command::MaterialInfo;
use beamcraft_engine::footprint::{Entry, Footprint, Group, Target};
use beamcraft_engine::member::{ProfileInfo, declared_section_area_warning};
use beamcraft_host::revision::{CommitHook, CommitOutcome, RevisionStore, Uuid7};
use beamcraft_layer_engine::authored::{
    Author, Definition, EditTargets, Prepare, Resource, schema_identity,
};
use beamcraft_layer_engine::{Prim, Reader, Value};
use beamcraft_model::command::{CommandError, label_is_valid};
use beamcraft_model::definition::{
    DeclaredAreaWarning, MaterialSpec, RectangleProfileSpec, SectionProfileSpec, SectionProperties,
};
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::fastener::{FastenerKind, FastenerSpec, HeadKind, HoleMode, WasherSpec};
use beamcraft_model::id::{OpId, ProjectId, RevisionId, WorkspaceId};
use beamcraft_model::section::{
    CustomSection, MAX_CUSTOM_CAVITIES, ParametricSection, SectionFamily, SectionShape,
};
use std::collections::{BTreeMap, BTreeSet};

/// Path prefix used by the synthetic profile fixtures.
#[cfg(test)]
const DEFINITION_PREFIX: &str = "/World/Definitions/Profiles/PRF_";

/// The logical content of one published definition.
#[derive(Debug, Clone, PartialEq)]
pub enum DefinitionRecord {
    /// A physical material; the density is unknown when `None`.
    Material {
        /// Density in kg/m3, when authored.
        density_kg_m3: Option<f64>,
        /// Declared grade, a label only.
        grade: Option<String>,
        /// Declared standard of the grade, a label only.
        standard: Option<String>,
        /// Readable name, part of the definition content.
        display_name: Option<String>,
    },
    /// The first-scope centred rectangle section.
    RectangleProfile {
        /// Section width in millimetres.
        width_mm: f64,
        /// Section height in millimetres.
        height_mm: f64,
        /// Declared designation, standard and section numbers; absent values stay unknown.
        section: SectionProperties,
    },
    /// An A07.1 section family or custom outline.
    SectionProfile {
        /// The section shape.
        shape: SectionShape,
        /// Declared designation, standard and section numbers; absent values stay unknown.
        section: SectionProperties,
    },
    /// An A14.1 fastener definition (bolt amendment section 2).
    Fastener {
        /// The complete fastener values.
        spec: FastenerSpec,
    },
}

impl DefinitionRecord {
    /// The `bim:definitionKind` token.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Material { .. } => "material",
            Self::RectangleProfile { .. } => "rectangle",
            Self::SectionProfile { shape, .. } => shape.token(),
            Self::Fastener { .. } => "fastener",
        }
    }

    /// Whether this definition is a profile (of any kind).
    pub const fn is_profile(&self) -> bool {
        !matches!(self, Self::Material { .. })
    }

    /// The readable name stored in the definition, if it has one (definitions published before
    /// A07.2 have none).
    pub fn display_name(&self) -> Option<&str> {
        match self {
            Self::Material { display_name, .. } => display_name.as_deref(),
            Self::RectangleProfile { section, .. } | Self::SectionProfile { section, .. } => {
                section.display_name.as_deref()
            }
            Self::Fastener { spec } => spec.display_name.as_deref(),
        }
    }

    /// The names a user may select this definition by: its display name and its catalogue label
    /// (the designation of a profile or fastener, the grade of a material). Exact, case-sensitive.
    pub fn selectable_names(&self) -> Vec<&str> {
        let label = match self {
            Self::Material { grade, .. } => grade.as_deref(),
            Self::RectangleProfile { section, .. } | Self::SectionProfile { section, .. } => {
                section.designation.as_deref()
            }
            Self::Fastener { spec } => spec.designation.as_deref(),
        };
        self.display_name().into_iter().chain(label).collect()
    }

    /// Admits the values with the F04 codes and caps.
    pub fn validate(&self) -> Result<(), CommandError> {
        match self {
            Self::Material {
                density_kg_m3,
                grade,
                standard,
                display_name,
            } => MaterialSpec {
                density_kg_m3: *density_kg_m3,
                grade: grade.clone(),
                standard: standard.clone(),
                display_name: display_name.clone(),
            }
            .validate(),
            Self::RectangleProfile {
                width_mm,
                height_mm,
                section,
            } => RectangleProfileSpec {
                width_mm: *width_mm,
                height_mm: *height_mm,
                section: section.clone(),
            }
            .validate(),
            Self::SectionProfile { shape, section } => SectionProfileSpec {
                shape: shape.clone(),
                section: section.clone(),
            }
            .validate(),
            Self::Fastener { spec } => spec.validate(),
        }
    }

    fn definition(&self) -> Definition {
        match self.clone() {
            Self::Material {
                density_kg_m3,
                grade,
                standard,
                display_name,
            } => Definition::Material {
                density_kg_m3,
                grade,
                standard,
                display_name,
            },
            Self::RectangleProfile {
                width_mm,
                height_mm,
                section,
            } => Definition::RectangleProfile {
                width_mm,
                height_mm,
                section,
            },
            Self::SectionProfile { shape, section } => {
                Definition::SectionProfile { shape, section }
            }
            Self::Fastener { spec } => Definition::Fastener { spec: spec.clone() },
        }
    }

    /// The warning for a section profile whose declared area differs from the generated outline
    /// by more than 1 percent (owner decision D3: catalogue values are authoritative, so this
    /// is reported and never refuses). `None` for every other definition and when it agrees.
    pub fn declared_area_warning(&self) -> Option<DeclaredAreaWarning> {
        match self {
            Self::SectionProfile { shape, section } => {
                declared_section_area_warning(&SectionProfileSpec {
                    shape: shape.clone(),
                    section: section.clone(),
                })
            }
            _ => None,
        }
    }

    /// The pinned resource: the definition and the digest of its canonical USD layer.
    pub fn resource(&self) -> Result<Resource, PlateError> {
        self.validate()?;
        Ok(Resource::new(self.definition())?)
    }
}

/// The USD path of the definition with this digest.
pub fn definition_path(digest: &ContentDigest, kind: &str) -> String {
    let category = match kind {
        "material" => beamcraft_model::prim::DefinitionCategory::Material,
        "fastener" => beamcraft_model::prim::DefinitionCategory::Fastener,
        _ => beamcraft_model::prim::DefinitionCategory::Profile,
    };
    category.path(*digest)
}

/// The digest named by a definition path, or `None` for any other path (including a descendant of
/// a definition and a short or upper-case digest).
pub(crate) fn definition_digest(path: &str) -> Option<ContentDigest> {
    beamcraft_model::prim::DefinitionCategory::parse_path(path).map(|(_, digest)| digest)
}

/// A required property of an exact USD type with a value. A missing property is `None`; a
/// property declared with another type, without a value or blocked is an integrity error, so a
/// value-only match can never accept a field that the layer engine would reject.
pub(crate) fn field<'a>(
    prim: &'a Prim,
    name: &str,
    ty: &str,
) -> Result<Option<&'a Value>, PlateError> {
    let Some(property) = prim.properties.iter().find(|p| p.name == name) else {
        return Ok(None);
    };
    match &property.value {
        Some(value) if property.type_name == ty && !matches!(value, Value::Block) => {
            Ok(Some(value))
        }
        _ => Err(integrity(
            "committed definition field has another type or no value",
        )),
    }
}

fn decode_definition(
    prim: &Prim,
    digest: ContentDigest,
) -> Result<Option<DefinitionRecord>, PlateError> {
    let bad = || integrity("committed definition lacks a required value");
    let required = |name: &str, ty: &str| field(prim, name, ty)?.ok_or_else(bad);
    let kind = required("bim:definitionKind", "token")?;
    let kind = text(kind).ok_or_else(bad)?;
    // A newer kind stays inert: it can be listed by an ordinary reader, never used here.
    let family = SectionFamily::from_token(kind);
    if !matches!(kind, "rectangle" | "material" | "custom" | "fastener") && family.is_none() {
        return Ok(None);
    }
    if !prim.active {
        return Err(integrity("committed definition is inactive"));
    }
    if text(required("bim:units", "token")?) != Some("mm")
        || text(required("bim:schemaVersion", "string")?) != Some("0.1.0")
    {
        return Err(integrity("committed definition has other units or version"));
    }
    let double = |name: &str| number(required(name, "double")?).ok_or_else(bad);
    let optional_double = |name: &str| {
        field(prim, name, "double")?
            .map(|v| number(v).ok_or_else(bad))
            .transpose()
    };
    let optional_text = |name: &str| {
        field(prim, name, "string")?
            .map(|v| text(v).map(str::to_owned).ok_or_else(bad))
            .transpose()
    };
    let section = || -> Result<SectionProperties, PlateError> {
        Ok(SectionProperties {
            designation: optional_text("steel:profile:designation")?,
            standard: optional_text("steel:profile:standard")?,
            area_mm2: optional_double("steel:profile:areaMm2")?,
            mass_per_meter_kg: optional_double("steel:profile:massPerMeterKg")?,
            surface_per_meter_m2: optional_double("steel:profile:surfacePerMeterM2")?,
            display_name: prim.display_name.clone(),
        })
    };
    let record = if let Some(family) = family {
        let values = family
            .parameters()
            .iter()
            .map(|name| double(&format!("steel:profile:{name}Mm")))
            .collect::<Result<Vec<_>, _>>()?;
        DefinitionRecord::SectionProfile {
            shape: SectionShape::Parametric(ParametricSection::new(family, values)?),
            section: section()?,
        }
    } else if kind == "custom" {
        let damaged = "committed custom profile has inconsistent outline attributes";
        DefinitionRecord::SectionProfile {
            shape: SectionShape::Custom(CustomSection {
                outer: decode_polygon(prim, "steel:profile:outerPointsMm", damaged)?,
                cavities: decode_polygons(
                    prim,
                    "steel:profile:cavityPointsMm",
                    "steel:profile:cavityVertexCounts",
                    MAX_CUSTOM_CAVITIES,
                    damaged,
                )?,
            }),
            section: section()?,
        }
    } else if kind == "rectangle" {
        DefinitionRecord::RectangleProfile {
            width_mm: double("steel:profile:widthMm")?,
            height_mm: double("steel:profile:heightMm")?,
            section: section()?,
        }
    } else if kind == "fastener" {
        DefinitionRecord::Fastener {
            spec: decode_fastener(prim)?,
        }
    } else {
        DefinitionRecord::Material {
            // Only an absent property means an unknown density.
            density_kg_m3: optional_double("bim:densityKgM3")?,
            grade: optional_text("bim:materialGrade")?,
            standard: optional_text("bim:materialStandard")?,
            display_name: prim.display_name.clone(),
        }
    };
    record.validate()?;
    // The path, the label and the recomputed canonical bytes must name the same content.
    let pinned = record.resource()?.digest;
    let label = text(required("bim:contentDigest", "string")?);
    if pinned != digest || label != Some(&digest.to_string()) {
        return Err(integrity("definition content does not match its digest"));
    }
    Ok(Some(record))
}

/// The logical fastener definition of a committed fastener definition prim (A14.1). The writer
/// never writes a partial washer, so a washer dimension without the other two is an integrity
/// error; the result is re-admitted by the caller's `record.validate()`.
fn decode_fastener(prim: &Prim) -> Result<FastenerSpec, PlateError> {
    let bad = || integrity("committed fastener definition lacks a required value");
    let double = |name: &str| -> Result<f64, PlateError> {
        number(field(prim, name, "double")?.ok_or_else(bad)?).ok_or_else(bad)
    };
    let optional_double = |name: &str| -> Result<Option<f64>, PlateError> {
        field(prim, name, "double")?
            .map(|v| number(v).ok_or_else(bad))
            .transpose()
    };
    let optional_text = |name: &str| -> Result<Option<String>, PlateError> {
        field(prim, name, "string")?
            .map(|v| text(v).map(str::to_owned).ok_or_else(bad))
            .transpose()
    };
    let token = |name: &str| -> Result<String, PlateError> {
        text(field(prim, name, "token")?.ok_or_else(bad)?)
            .map(str::to_owned)
            .ok_or_else(|| integrity("committed fastener token is not text"))
    };
    let boolean = |name: &str| -> Result<bool, PlateError> {
        match field(prim, name, "bool")? {
            Some(Value::Bool(value)) => Ok(*value),
            _ => Err(bad()),
        }
    };
    let kind = FastenerKind::from_token(&token("steel:fastener:kind")?).ok_or_else(bad)?;
    let head_kind = HeadKind::from_token(&token("steel:fastener:headKind")?).ok_or_else(bad)?;
    let hole_mode = HoleMode::from_token(&token("steel:fastener:holeMode")?).ok_or_else(bad)?;
    let washer = match (
        optional_double("steel:fastener:washerOuterDiameterMm")?,
        optional_double("steel:fastener:washerInnerDiameterMm")?,
        optional_double("steel:fastener:washerThicknessMm")?,
    ) {
        (None, None, None) => None,
        (outer, inner, thickness) => Some(WasherSpec {
            outer_diameter_mm: outer.ok_or_else(bad)?,
            inner_diameter_mm: inner.ok_or_else(bad)?,
            thickness_mm: thickness.ok_or_else(bad)?,
        }),
    };
    let lengths = match field(prim, "steel:fastener:lengthsMm", "double[]")? {
        Some(Value::Array(values)) => values
            .iter()
            .map(|v| number(v).ok_or_else(bad))
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Err(bad()),
    };
    Ok(FastenerSpec {
        designation: optional_text("steel:fastener:designation")?,
        standard: optional_text("steel:fastener:standard")?,
        grade: optional_text("steel:fastener:grade")?,
        display_name: prim.display_name.clone(),
        kind,
        head_kind,
        nominal_diameter_mm: double("steel:fastener:nominalDiameterMm")?,
        head_across_flats_mm: double("steel:fastener:headAcrossFlatsMm")?,
        head_height_mm: double("steel:fastener:headHeightMm")?,
        nut_across_flats_mm: optional_double("steel:fastener:nutAcrossFlatsMm")?,
        nut_height_mm: optional_double("steel:fastener:nutHeightMm")?,
        washer,
        hole_clearance_mm: double("steel:fastener:holeClearanceMm")?,
        lengths_mm: lengths,
        thread_protrusion_mm: double("steel:fastener:threadProtrusionMm")?,
        mass_each_kg: optional_double("steel:fastener:massEachKg")?,
        washer_under_head: boolean("steel:fastener:washerUnderHead")?,
        washer_under_nut: boolean("steel:fastener:washerUnderNut")?,
        hole_mode,
    })
}

/// Every immutable definition of a pinned stage, by full digest. Values are re-admitted and the
/// digest recomputed from them, because a committed layer is untrusted input.
pub(crate) fn decode_definitions(
    prims: &[Prim],
) -> Result<BTreeMap<ContentDigest, DefinitionRecord>, PlateError> {
    let mut definitions = BTreeMap::new();
    for prim in prims {
        let Some(digest) = definition_digest(&prim.path) else {
            continue;
        };
        if let Some(record) = decode_definition(prim, digest)? {
            if prim.path != definition_path(&digest, record.kind()) {
                return Err(integrity("definition path does not match its kind"));
            }
            definitions.insert(digest, record);
        }
    }
    Ok(definitions)
}

/// Resolves a pinned material against the definitions of a stage: absent is `NOT_FOUND`, a
/// profile is `INVALID_ARGUMENT`.
pub(crate) fn material_lookup(
    definitions: &BTreeMap<ContentDigest, DefinitionRecord>,
) -> impl Fn(&ContentDigest) -> Result<MaterialInfo, CommandError> + '_ {
    move |digest| match definitions.get(digest) {
        Some(DefinitionRecord::Material { density_kg_m3, .. }) => Ok(MaterialInfo {
            density_kg_m3: *density_kg_m3,
        }),
        Some(
            DefinitionRecord::RectangleProfile { .. } | DefinitionRecord::SectionProfile { .. },
        ) => Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "material_digest",
            "digest names a profile, not a material",
        )),
        Some(DefinitionRecord::Fastener { .. }) => Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "material_digest",
            "digest names a fastener, not a material",
        )),
        None => Err(CommandError::new(
            ErrorCode::NotFound,
            "material_digest",
            "material definition is not in the model",
        )),
    }
}

/// Resolves a pinned profile against the definitions of a stage: absent is `NOT_FOUND`, a
/// material is `INVALID_ARGUMENT`.
pub(crate) fn profile_lookup(
    definitions: &BTreeMap<ContentDigest, DefinitionRecord>,
) -> impl Fn(&ContentDigest) -> Result<ProfileInfo, CommandError> + '_ {
    move |digest| match definitions.get(digest) {
        Some(DefinitionRecord::RectangleProfile {
            width_mm,
            height_mm,
            section,
        }) => Ok(ProfileInfo::Rectangle {
            width_mm: *width_mm,
            height_mm: *height_mm,
            declared_area_mm2: section.area_mm2,
        }),
        Some(DefinitionRecord::SectionProfile { shape, section }) => Ok(ProfileInfo::Section {
            shape: shape.clone(),
            declared_area_mm2: section.area_mm2,
        }),
        Some(DefinitionRecord::Material { .. }) => Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "profile_digest",
            "digest names a material, not a profile",
        )),
        Some(DefinitionRecord::Fastener { .. }) => Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "profile_digest",
            "digest names a fastener, not a profile",
        )),
        None => Err(CommandError::new(
            ErrorCode::NotFound,
            "profile_digest",
            "profile definition is not in the model",
        )),
    }
}

/// What a readable name selected in the definitions of a stage.
#[derive(Debug, Clone, PartialEq)]
pub enum NameResolution {
    /// Exactly one definition of the wanted kind has the name.
    Found(ContentDigest),
    /// No definition of the wanted kind has the name.
    NotFound,
    /// More than one distinct definition has the name; none is chosen.
    Ambiguous(Vec<(ContentDigest, DefinitionRecord)>),
}

/// Selects a definition by name: a profile (`profiles` true) or a material, whose display name or
/// catalogue label (designation of a profile, grade of a material) equals `name` exactly. The
/// match is case-sensitive; an ambiguous name is reported with every candidate, never guessed.
pub fn resolve_name(
    definitions: &BTreeMap<ContentDigest, DefinitionRecord>,
    name: &str,
    profiles: bool,
) -> NameResolution {
    let mut found: Vec<(ContentDigest, DefinitionRecord)> = definitions
        .iter()
        .filter(|(_, record)| record.is_profile() == profiles)
        .filter(|(_, record)| record.selectable_names().contains(&name))
        .map(|(digest, record)| (*digest, record.clone()))
        .collect();
    match found.len() {
        0 => NameResolution::NotFound,
        1 => found.pop().map_or(NameResolution::NotFound, |(digest, _)| {
            NameResolution::Found(digest)
        }),
        _ => NameResolution::Ambiguous(found),
    }
}

/// The definitions of the head revision, verified and re-admitted like every read; empty for a
/// store without a revision.
pub fn head_definitions(
    store: &RevisionStore,
) -> Result<BTreeMap<ContentDigest, DefinitionRecord>, PlateError> {
    let revision = match store.open() {
        Ok(revision) => revision,
        Err(beamcraft_host::revision::RevisionError::NoRevision) => return Ok(BTreeMap::new()),
        Err(error) => return Err(error.into()),
    };
    let control = control();
    let request = open_base(&base_of(&revision)?)?;
    let mut reader = Reader::default();
    let opened = reader.open(request, &control)?;
    Ok(decode(&reader, &opened, &control)?.definitions)
}

/// Dimensions of a section as they appear in a default display name: the leading parameters of
/// its family, without the thicknesses of an I section or channel and without the radius.
fn default_dimensions(shape: &SectionShape) -> String {
    let SectionShape::Parametric(parametric) = shape else {
        return String::new();
    };
    parametric
        .family()
        .parameters()
        .iter()
        .zip(parametric.values())
        .filter(|(name, _)| !matches!(**name, "r" | "tw" | "tf"))
        .map(|(_, value)| value.to_string())
        .collect::<Vec<_>>()
        .join("x")
}

impl DefinitionRecord {
    /// The display name given to a definition published without `--name`: the designation of a
    /// profile, else its family and dimensions (`I 200x100`, `RECT 200x300`); a material is its
    /// grade and density (`S355 7850 kg/m3`). Only what the definition itself declares is used,
    /// so an unknown density never appears as a number.
    pub fn default_display_name(&self) -> String {
        match self {
            Self::Material {
                density_kg_m3,
                grade,
                ..
            } => {
                let density = density_kg_m3.map(|d| format!("{d} kg/m3"));
                let combined = match (grade, &density) {
                    (Some(grade), Some(density)) => Some(format!("{grade} {density}")),
                    _ => None,
                };
                // The name is a label too: when grade and density together are longer than a
                // label may be, the grade alone (or the density alone) is still a valid name.
                [combined, grade.clone(), density]
                    .into_iter()
                    .flatten()
                    .find(|name| label_is_valid(name))
                    .unwrap_or_else(|| "Material (density unknown)".to_owned())
            }
            Self::RectangleProfile {
                width_mm,
                height_mm,
                section,
            } => section
                .designation
                .clone()
                .unwrap_or_else(|| format!("RECT {width_mm}x{height_mm}")),
            Self::SectionProfile { shape, section } => {
                if let Some(designation) = &section.designation {
                    return designation.clone();
                }
                match shape {
                    SectionShape::Custom(custom) => {
                        format!("Custom {} points", custom.outer.len())
                    }
                    SectionShape::Parametric(parametric) => {
                        let prefix = match parametric.family() {
                            SectionFamily::IShape => "I",
                            SectionFamily::Channel => "C",
                            SectionFamily::Angle => "L",
                            SectionFamily::Rhs => "RHS",
                            SectionFamily::Chs => "CHS",
                            SectionFamily::LippedC => "LC",
                            SectionFamily::PlainC => "PC",
                            SectionFamily::LippedZ => "LZ",
                            SectionFamily::Omega => "OM",
                        };
                        format!("{prefix} {}", default_dimensions(shape))
                    }
                }
            }
            Self::Fastener { spec } => spec
                .designation
                .clone()
                .unwrap_or_else(|| format!("Fastener M{}", spec.nominal_diameter_mm)),
        }
    }

    /// This definition with its display name set to `name`, or to the default name when none is
    /// given. The name is part of the content, so it is part of the digest.
    pub fn named(mut self, name: Option<&str>) -> Self {
        let chosen = name
            .map(str::to_owned)
            .unwrap_or_else(|| self.default_display_name());
        match &mut self {
            Self::Material { display_name, .. } => *display_name = Some(chosen),
            Self::RectangleProfile { section, .. } | Self::SectionProfile { section, .. } => {
                section.display_name = Some(chosen);
            }
            Self::Fastener { spec } => spec.display_name = Some(chosen),
        }
        self
    }
}

/// Inputs of one `define` call.
#[derive(Debug, Clone)]
pub struct DefineRequest {
    /// Identity of the revision this call creates, if it creates one.
    pub revision_id: RevisionId,
    /// Operation recorded for the revision, if one is created.
    pub op_id: OpId,
    /// The publishing actor (trusted context, `bim:createdBy` of elements, not of definitions).
    pub actor: Uuid7,
    /// Project of a store that has no revision yet. Ignored otherwise.
    pub project: ProjectId,
    /// Workspace of a store that has no revision yet. Ignored otherwise.
    pub workspace: WorkspaceId,
    /// The definition to publish.
    pub definition: DefinitionRecord,
}

/// Result of `define`.
#[derive(Debug, Clone, PartialEq)]
pub struct Defined {
    /// True when the head already held this exact definition: nothing was written.
    pub already_defined: bool,
    /// Full digest of the canonical definition layer; what a command pins.
    pub digest: ContentDigest,
    /// The logical content that digest pins.
    pub record: DefinitionRecord,
    /// The revision that holds the definition (the head itself when it already did).
    pub revision: RevisionSummary,
}

/// Publishes one immutable definition as the next revision, or reports that the head already has
/// it. A definition never changes: the same content has the same digest and the same bytes.
///
/// Steps, each failing closed: admit the values (F04 caps); open the head and read its
/// definitions; author the definition over the head's layers with U02 (no element is touched);
/// check that the layer engine wrote exactly the engine's declared footprint; publish the layer
/// blobs and commit through U06/U07 with the verifier of `plate` (U03 index, structural check,
/// digest and kernel re-evaluation of every plate).
pub fn define(
    store: &RevisionStore,
    request: &DefineRequest,
    hook: &mut dyn CommitHook,
) -> Result<Defined, PlateError> {
    let control = control();
    let resource = request.definition.resource()?;
    let digest = resource.digest;
    let (head, base) = load_base(store, &request.project, &request.workspace)?;
    let base_request = open_base(&base)?;
    let mut reader = Reader::default();
    let opened = reader.open(base_request.clone(), &control)?;
    let decoded = decode(&reader, &opened, &control)?;
    if decoded.definitions.contains_key(&digest) {
        let head = head.ok_or_else(|| integrity("definitions exist without a revision"))?;
        return Ok(Defined {
            already_defined: true,
            digest,
            record: request.definition.clone(),
            revision: summarize(head.info()),
        });
    }
    let Base {
        project,
        workspace,
        generation: base_generation,
        expected,
        ..
    } = base;
    let schema = schema_identity()?;
    let (mut author, opened) = Author::open(
        base_request,
        EditTargets {
            authored: crate::plate::AUTHORED_LAYER.into(),
            derived: crate::plate::DERIVED_LAYER.into(),
        },
        schema,
        &control,
    )?;
    let prepared = author.prepare(
        Prepare {
            context: opened.context.clone(),
            op_id: request.op_id.to_string(),
            actor: request.actor.to_string(),
            schema,
            resources: vec![resource],
            changes: Vec::new(),
        },
        &control,
    )?;
    let footprint = Footprint::for_define(digest);
    let observed: BTreeSet<Entry> = prepared
        .writes
        .iter()
        .map(|item| {
            Some(Entry {
                target: Target::Definition(item.identity.parse().ok()?),
                group: Group::Relationship,
            })
        })
        .collect::<Option<_>>()
        .ok_or_else(|| integrity("layer engine footprint names an unknown target"))?;
    if observed.is_empty() || !footprint.covers_writes(&observed) {
        return Err(integrity(
            "layer engine writes other than the engine footprint declares",
        ));
    }
    let committed = publish_revision(
        store,
        &PublishRequest {
            footprint: &footprint,
            fingerprint: digest,
            prepared: &prepared,
            schema,
            project: &project,
            workspace: &workspace,
            revision_id: &request.revision_id,
            op: host_uuid(&request.op_id.to_string())?,
            base_generation,
            expected: &expected,
        },
        hook,
    )?;
    Ok(Defined {
        // A replayed commit means these very bytes were already published under this operation.
        already_defined: committed.outcome == CommitOutcome::Replayed,
        digest,
        record: request.definition.clone(),
        revision: summarize(&committed.info),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use beamcraft_layer_engine::Property;

    fn prim(path: String, properties: Vec<(&str, &str, Value)>) -> Prim {
        Prim {
            path,
            element_id: None,
            type_name: Some("Scope".into()),
            active: true,
            kind: None,
            display_name: None,
            properties: properties
                .into_iter()
                .map(|(name, type_name, value)| Property {
                    name: name.into(),
                    type_name: type_name.into(),
                    value: Some(value),
                    interpolation: None,
                })
                .collect(),
            relationships: Vec::new(),
            sources: Vec::new(),
        }
    }

    fn text(value: &str) -> Value {
        Value::Text(value.into())
    }

    fn material_prim(path_density: Option<f64>, layer_density: Option<f64>) -> Prim {
        let record = DefinitionRecord::Material {
            density_kg_m3: path_density,
            grade: None,
            standard: None,
            display_name: None,
        };
        let digest = record.resource().unwrap().digest;
        let mut properties = vec![
            ("bim:schemaVersion", "string", text("0.1.0")),
            ("bim:units", "token", text("mm")),
            ("bim:definitionKind", "token", text("material")),
            ("bim:contentDigest", "string", text(&digest.to_string())),
        ];
        if let Some(density) = layer_density {
            properties.push(("bim:densityKgM3", "double", Value::Number(density)));
        }
        prim(definition_path(&digest, "material"), properties)
    }

    #[test]
    fn a_definition_round_trips_through_its_digest() {
        let prims = [
            material_prim(Some(7850.0), Some(7850.0)),
            material_prim(None, None),
        ];
        let found = decode_definitions(&prims).unwrap();
        assert_eq!(found.len(), 2);
        assert!(found.values().any(|r| matches!(
            r,
            DefinitionRecord::Material { density_kg_m3: Some(d), .. } if *d == 7850.0
        )));
        assert!(found.values().any(|r| matches!(
            r,
            DefinitionRecord::Material {
                density_kg_m3: None,
                ..
            }
        )));
    }

    #[test]
    fn declared_section_values_and_labels_round_trip_and_are_pinned_by_the_digest() {
        let profile = DefinitionRecord::RectangleProfile {
            width_mm: 200.0,
            height_mm: 300.0,
            section: SectionProperties {
                designation: Some("RECT 200x300".into()),
                standard: Some("Synthetic".into()),
                area_mm2: Some(60_000.0),
                mass_per_meter_kg: Some(471.0),
                surface_per_meter_m2: Some(1.0),
                display_name: None,
            },
        };
        let material = DefinitionRecord::Material {
            density_kg_m3: Some(7850.0),
            grade: Some("S355".into()),
            standard: Some("EN 10025-2".into()),
            display_name: None,
        };
        let plain = DefinitionRecord::RectangleProfile {
            width_mm: 200.0,
            height_mm: 300.0,
            section: SectionProperties::default(),
        };
        let digest = |r: &DefinitionRecord| r.resource().unwrap().digest;
        // Declared values are part of the identity; a plain profile keeps its earlier digest.
        assert_ne!(digest(&profile), digest(&plain));
        let mut profile_props = vec![
            ("bim:schemaVersion", "string", text("0.1.0")),
            ("bim:units", "token", text("mm")),
            ("bim:definitionKind", "token", text("rectangle")),
            (
                "bim:contentDigest",
                "string",
                text(&digest(&profile).to_string()),
            ),
            ("steel:profile:widthMm", "double", Value::Number(200.0)),
            ("steel:profile:heightMm", "double", Value::Number(300.0)),
            ("steel:profile:areaMm2", "double", Value::Number(60_000.0)),
            (
                "steel:profile:massPerMeterKg",
                "double",
                Value::Number(471.0),
            ),
            (
                "steel:profile:surfacePerMeterM2",
                "double",
                Value::Number(1.0),
            ),
            ("steel:profile:designation", "string", text("RECT 200x300")),
            ("steel:profile:standard", "string", text("Synthetic")),
        ];
        let found = decode_definitions(&[prim(
            definition_path(&digest(&profile), profile.kind()),
            profile_props.clone(),
        )])
        .unwrap();
        assert_eq!(found.get(&digest(&profile)), Some(&profile));
        // Dropping the designation from the layer no longer matches the pinned content.
        profile_props.retain(|p| p.0 != "steel:profile:designation");
        assert_eq!(
            decode_definitions(&[prim(
                definition_path(&digest(&profile), profile.kind()),
                profile_props
            )])
            .unwrap_err()
            .code,
            ErrorCode::IntegrityError
        );
        let found = decode_definitions(&[prim(
            definition_path(&digest(&material), material.kind()),
            vec![
                ("bim:schemaVersion", "string", text("0.1.0")),
                ("bim:units", "token", text("mm")),
                ("bim:definitionKind", "token", text("material")),
                (
                    "bim:contentDigest",
                    "string",
                    text(&digest(&material).to_string()),
                ),
                ("bim:densityKgM3", "double", Value::Number(7850.0)),
                ("bim:materialGrade", "string", text("S355")),
                ("bim:materialStandard", "string", text("EN 10025-2")),
            ],
        )])
        .unwrap();
        assert_eq!(found.get(&digest(&material)), Some(&material));
    }

    #[test]
    fn a_layer_edited_after_publication_no_longer_matches_its_digest() {
        // The path pins 7850 kg/m3 but the layer says 1: the digest is recomputed from the values.
        let edited = material_prim(Some(7850.0), Some(1.0));
        assert_eq!(
            decode_definitions(&[edited]).unwrap_err().code,
            ErrorCode::IntegrityError
        );
        // The density was removed: unknown is a different content from 7850.
        let removed = material_prim(Some(7850.0), None);
        assert_eq!(
            decode_definitions(&[removed]).unwrap_err().code,
            ErrorCode::IntegrityError
        );
        // A density added to a definition that was published without one.
        let added = material_prim(None, Some(7850.0));
        assert_eq!(
            decode_definitions(&[added]).unwrap_err().code,
            ErrorCode::IntegrityError
        );
    }

    #[test]
    fn other_units_a_hostile_density_and_a_forged_label_are_rejected() {
        let mut other_units = material_prim(Some(7850.0), Some(7850.0));
        other_units.properties.retain(|p| p.name != "bim:units");
        other_units.properties.push(Property {
            name: "bim:units".into(),
            type_name: "token".into(),
            value: Some(text("m")),
            interpolation: None,
        });
        assert_eq!(
            decode_definitions(&[other_units]).unwrap_err().code,
            ErrorCode::IntegrityError
        );
        // A density outside the F04 cap is refused before it can make a mass.
        let hostile = DefinitionRecord::Material {
            density_kg_m3: Some(1e9),
            grade: None,
            standard: None,
            display_name: None,
        };
        let digest = beamcraft_model::digest::Sha256::digest(b"forged").unwrap();
        let hostile = prim(
            definition_path(&digest, "rectangle"),
            vec![
                ("bim:schemaVersion", "string", text("0.1.0")),
                ("bim:units", "token", text("mm")),
                ("bim:definitionKind", "token", text(hostile.kind())),
                ("bim:densityKgM3", "double", Value::Number(1e9)),
            ],
        );
        assert_eq!(
            decode_definitions(&[hostile]).unwrap_err().code,
            ErrorCode::LimitExceeded
        );
        let mut forged = material_prim(Some(7850.0), Some(7850.0));
        forged.properties.retain(|p| p.name != "bim:contentDigest");
        forged.properties.push(Property {
            name: "bim:contentDigest".into(),
            type_name: "string".into(),
            value: Some(text(&"0".repeat(64))),
            interpolation: None,
        });
        assert_eq!(
            decode_definitions(&[forged]).unwrap_err().code,
            ErrorCode::IntegrityError
        );
    }

    #[test]
    fn malformed_inactive_and_mistyped_definitions_are_integrity_errors() {
        let code = |prim: Prim| decode_definitions(&[prim]).unwrap_err().code;
        let steel = || material_prim(Some(7850.0), Some(7850.0));
        let replace = |prim: &mut Prim, name: &str, property: Option<Property>| {
            prim.properties.retain(|p| p.name != name);
            prim.properties.extend(property);
        };
        let property = |name: &str, ty: &str, value: Option<Value>| Property {
            name: name.into(),
            type_name: ty.into(),
            value,
            interpolation: None,
        };
        // A missing, mistyped or valueless kind is not "a newer kind".
        let mut p = steel();
        replace(&mut p, "bim:definitionKind", None);
        assert_eq!(code(p), ErrorCode::IntegrityError);
        let mut p = steel();
        replace(
            &mut p,
            "bim:definitionKind",
            Some(property(
                "bim:definitionKind",
                "token",
                Some(Value::Number(1.0)),
            )),
        );
        assert_eq!(code(p), ErrorCode::IntegrityError);
        let mut p = steel();
        replace(
            &mut p,
            "bim:definitionKind",
            Some(property("bim:definitionKind", "token", None)),
        );
        assert_eq!(code(p), ErrorCode::IntegrityError);
        // A deactivated definition cannot supply a density.
        let mut p = steel();
        p.active = false;
        assert_eq!(code(p), ErrorCode::IntegrityError);
        // The density changed from double to float, or declared without a value, is not unknown.
        let mut p = steel();
        replace(
            &mut p,
            "bim:densityKgM3",
            Some(property(
                "bim:densityKgM3",
                "float",
                Some(Value::Number(7850.0)),
            )),
        );
        assert_eq!(code(p), ErrorCode::IntegrityError);
        let mut p = steel();
        replace(
            &mut p,
            "bim:densityKgM3",
            Some(property("bim:densityKgM3", "double", None)),
        );
        assert_eq!(code(p), ErrorCode::IntegrityError);
        let mut p = steel();
        replace(
            &mut p,
            "bim:densityKgM3",
            Some(property("bim:densityKgM3", "double", Some(Value::Block))),
        );
        assert_eq!(code(p), ErrorCode::IntegrityError);
        // Units and the label must have their exact types too.
        let mut p = steel();
        replace(
            &mut p,
            "bim:units",
            Some(property("bim:units", "string", Some(text("mm")))),
        );
        assert_eq!(code(p), ErrorCode::IntegrityError);
        let mut p = steel();
        replace(&mut p, "bim:contentDigest", None);
        assert_eq!(code(p), ErrorCode::IntegrityError);
        // Only an absent density property is unknown.
        assert!(decode_definitions(&[material_prim(None, None)]).is_ok());
    }

    #[test]
    fn unknown_kinds_and_other_paths_stay_inert() {
        let digest = beamcraft_model::digest::Sha256::digest(b"future").unwrap();
        let future = prim(
            definition_path(&digest, "rectangle"),
            vec![
                ("bim:schemaVersion", "string", text("0.1.0")),
                ("bim:units", "token", text("mm")),
                ("bim:definitionKind", "token", text("catalogue")),
            ],
        );
        let child = prim(
            format!("{}/Child", definition_path(&digest, "rectangle")),
            Vec::new(),
        );
        let upper = prim(
            format!("{DEFINITION_PREFIX}{}", digest.to_string().to_uppercase()),
            Vec::new(),
        );
        let steel = prim("/World/Steel".into(), Vec::new());
        assert!(
            decode_definitions(&[future, child, upper, steel])
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            definition_digest(&definition_path(&digest, "rectangle")),
            Some(digest)
        );
        assert_eq!(definition_digest("/World/Definitions/D_abc"), None);
    }

    #[test]
    fn a_material_lookup_distinguishes_absent_from_a_profile() {
        let steel = DefinitionRecord::Material {
            density_kg_m3: Some(7850.0),
            grade: None,
            standard: None,
            display_name: None,
        };
        let profile = DefinitionRecord::RectangleProfile {
            width_mm: 200.0,
            height_mm: 300.0,
            section: SectionProperties::default(),
        };
        let (steel_digest, profile_digest) = (
            steel.resource().unwrap().digest,
            profile.resource().unwrap().digest,
        );
        let definitions = BTreeMap::from([(steel_digest, steel), (profile_digest, profile)]);
        let lookup = material_lookup(&definitions);
        assert_eq!(lookup(&steel_digest).unwrap().density_kg_m3, Some(7850.0));
        assert_eq!(
            lookup(&profile_digest).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        let absent = beamcraft_model::digest::Sha256::digest(b"absent").unwrap();
        assert_eq!(lookup(&absent).unwrap_err().code, ErrorCode::NotFound);
    }

    #[test]
    fn a_display_name_is_part_of_the_content_and_is_read_back_from_the_prim() {
        let material = |display_name: Option<&str>| DefinitionRecord::Material {
            density_kg_m3: Some(7850.0),
            grade: Some("S355".into()),
            standard: None,
            display_name: display_name.map(str::to_owned),
        };
        let digest = |r: &DefinitionRecord| r.resource().unwrap().digest;
        let named = material(Some("Structural steel"));
        assert_ne!(digest(&named), digest(&material(None)));
        let props = vec![
            ("bim:schemaVersion", "string", text("0.1.0")),
            ("bim:units", "token", text("mm")),
            ("bim:definitionKind", "token", text("material")),
            (
                "bim:contentDigest",
                "string",
                text(&digest(&named).to_string()),
            ),
            ("bim:densityKgM3", "double", Value::Number(7850.0)),
            ("bim:materialGrade", "string", text("S355")),
        ];
        let with_name = |name: Option<&str>| {
            let mut p = prim(
                definition_path(&digest(&named), named.kind()),
                props.clone(),
            );
            p.display_name = name.map(str::to_owned);
            p
        };
        let found = decode_definitions(&[with_name(Some("Structural steel"))]).unwrap();
        assert_eq!(found.get(&digest(&named)), Some(&named));
        // A name that was changed, removed or invalid after publication no longer matches.
        assert_eq!(
            decode_definitions(&[with_name(Some("Other"))])
                .unwrap_err()
                .code,
            ErrorCode::IntegrityError
        );
        assert_eq!(
            decode_definitions(&[with_name(None)]).unwrap_err().code,
            ErrorCode::IntegrityError
        );
        assert!(decode_definitions(&[with_name(Some("bad\nname"))]).is_err());
        // A definition published without a name keeps its old digest and verifies as before.
        assert!(decode_definitions(&[material_prim(Some(7850.0), Some(7850.0))]).is_ok());
    }

    #[test]
    fn default_names_use_only_what_the_definition_declares() {
        let name = |record: DefinitionRecord| record.default_display_name();
        let material = |density: Option<f64>, grade: Option<&str>| DefinitionRecord::Material {
            density_kg_m3: density,
            grade: grade.map(str::to_owned),
            standard: None,
            display_name: None,
        };
        assert_eq!(
            name(material(Some(7850.0), Some("S355"))),
            "S355 7850 kg/m3"
        );
        assert_eq!(name(material(Some(2700.5), None)), "2700.5 kg/m3");
        assert_eq!(name(material(None, Some("S355"))), "S355");
        assert_eq!(name(material(None, None)), "Material (density unknown)");
        // A grade at the label limit keeps its density but loses the suffix from the name.
        let long = "G".repeat(256);
        let fitted = name(material(Some(7850.0), Some(&long)));
        assert_eq!(fitted, long);
        let record = material(Some(7850.0), Some(&long)).named(None);
        assert!(record.validate().is_ok());
        assert!(matches!(
            record,
            DefinitionRecord::Material { density_kg_m3: Some(d), grade: Some(_), .. } if d == 7850.0
        ));
        let rectangle = |designation: Option<&str>| DefinitionRecord::RectangleProfile {
            width_mm: 200.0,
            height_mm: 300.5,
            section: SectionProperties {
                designation: designation.map(str::to_owned),
                ..SectionProperties::default()
            },
        };
        assert_eq!(name(rectangle(None)), "RECT 200x300.5");
        assert_eq!(name(rectangle(Some("FB 200"))), "FB 200");
        let section = |family, values: Vec<f64>| DefinitionRecord::SectionProfile {
            shape: SectionShape::Parametric(ParametricSection::new(family, values).unwrap()),
            section: SectionProperties::default(),
        };
        assert_eq!(
            name(section(
                SectionFamily::IShape,
                vec![200.0, 100.0, 5.0, 8.0, 10.0]
            )),
            "I 200x100"
        );
        assert_eq!(
            name(section(SectionFamily::Chs, vec![114.0, 5.0])),
            "CHS 114x5"
        );
        assert_eq!(
            name(section(
                SectionFamily::LippedC,
                vec![150.0, 60.0, 20.0, 2.0, 4.0]
            )),
            "LC 150x60x20x2"
        );
        // Setting a name never changes the other values, and a given name wins.
        let named = rectangle(None).named(Some("Flat bar"));
        assert_eq!(named.display_name(), Some("Flat bar"));
        assert_eq!(
            rectangle(None).named(None).display_name(),
            Some("RECT 200x300.5")
        );
    }

    #[test]
    fn names_resolve_exactly_by_kind_and_an_ambiguous_name_names_every_candidate() {
        let profile = |designation: &str, width: f64| {
            DefinitionRecord::RectangleProfile {
                width_mm: width,
                height_mm: 100.0,
                section: SectionProperties {
                    designation: Some(designation.into()),
                    ..SectionProperties::default()
                },
            }
            .named(None)
        };
        let material = |density: f64| {
            DefinitionRecord::Material {
                density_kg_m3: Some(density),
                grade: Some("S355".into()),
                standard: None,
                display_name: None,
            }
            .named(None)
        };
        let records = [
            profile("A", 10.0),
            profile("A", 20.0),
            profile("B", 30.0),
            material(7850.0),
            material(7800.0),
        ];
        let definitions: BTreeMap<_, _> = records
            .iter()
            .map(|r| (r.resource().unwrap().digest, r.clone()))
            .collect();
        let resolve = |name: &str, profiles: bool| resolve_name(&definitions, name, profiles);
        assert!(matches!(resolve("B", true), NameResolution::Found(_)));
        assert_eq!(resolve("b", true), NameResolution::NotFound);
        assert_eq!(resolve("B", false), NameResolution::NotFound);
        assert_eq!(resolve("S355", true), NameResolution::NotFound);
        assert!(matches!(
            resolve("S355 7800 kg/m3", false),
            NameResolution::Found(_)
        ));
        let NameResolution::Ambiguous(found) = resolve("A", true) else {
            panic!("two profiles share the designation A");
        };
        assert_eq!(found.len(), 2);
        let NameResolution::Ambiguous(found) = resolve("S355", false) else {
            panic!("two materials share the grade S355");
        };
        assert_eq!(found.len(), 2);
        assert_eq!(resolve("", true), NameResolution::NotFound);
    }

    #[test]
    fn a_definition_value_is_admitted_before_it_is_published() {
        for (record, code) in [
            (
                DefinitionRecord::Material {
                    density_kg_m3: Some(-1.0),
                    grade: None,
                    standard: None,
                    display_name: None,
                },
                ErrorCode::InvalidArgument,
            ),
            (
                DefinitionRecord::RectangleProfile {
                    width_mm: 200.0,
                    height_mm: 2e6,
                    section: SectionProperties::default(),
                },
                ErrorCode::LimitExceeded,
            ),
            (
                DefinitionRecord::RectangleProfile {
                    width_mm: 200.0,
                    height_mm: 300.0,
                    section: SectionProperties {
                        area_mm2: Some(1.0),
                        ..SectionProperties::default()
                    },
                },
                ErrorCode::InvalidArgument,
            ),
        ] {
            assert_eq!(record.resource().unwrap_err().code, code);
        }
    }
}
