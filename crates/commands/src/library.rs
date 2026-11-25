// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A07.2 profile libraries: a USD layer of named section entries, read by the layer engine.
//!
//! A library is an input, never model state. Resolving a designation yields a complete
//! [`DefinitionRecord`] with every value; publishing that record pins it by content, so a model
//! never depends on the library file afterwards. A library file is untrusted input: it is read
//! with the bounded local-file read, opened by the layer engine under tight stage budgets (one
//! layer, no sublayer, no external reference, millimetres and Z up), and every entry is admitted
//! by the same rules as a definition published by hand.
//!
//! Layout (design note A07, section 6): the root metadata `customLayerData` carries
//! `bim:library:id` (required) and optional `title`, `version`, `source` and `licence`; each
//! entry is a prim `/Library/Profiles/<name>` with `steel:profile:designation` (string),
//! `steel:profile:family` (token: `rectangle`, a section family or `custom`), one
//! `steel:profile:<parameter>Mm` double per family parameter (a rectangle has `h` and `b`; the
//! dimensionless slope of a tapered family is `steel:profile:k`; a
//! custom outline has `outerPointsMm` and optional `cavityPointsMm` with `cavityVertexCounts`),
//! and optional declared `standard`, `areaMm2`, `massPerMeterKg` and `surfacePerMeterM2`.
//!
//! A07.3 adds the lookup over several libraries and the writer. An environment is a USD layer
//! whose `customLayerData` carries `bim:environment:id` and `bim:environment:libraries`, an
//! ordered list of contained relative paths; a user library named on the command line comes
//! before them. The first library that declares a designation answers it ([`Lookup`]). A
//! library file is written only by [`add_entry`], which renders the whole library again and reads
//! it back before it installs the file.
//!
//! Fastener libraries (A14.1, #668) and material libraries (#636) ride the same machinery under
//! `/Library/Fasteners` and `/Library/Materials`: the fastener entries carry
//! `steel:fastener:*` attributes, the material entries `steel:material:*` (grade, density,
//! optional standard and display name), and both are admitted by the same rules, provenance
//! included, when an environment lists them.
//!
//! The Layering work adds the layers above the environment. Every [`Source`] of a lookup names
//! its layer, strongest first: the user library named on the command line, the user-owned firm
//! layer (`libraries/` in the application data, loaded by [`load_firm`], never written by the
//! programs and never touched by an environment update), then the chosen libraries. A project's
//! own copy of its environment is the project layer of [`project_library_chain`], and the
//! shipped neutral environment ([`neutral_environment`]) is the built-in fallback every project
//! chain ends with. When a weaker layer declares a designation a stronger one already answers,
//! [`Lookup::shadowed`] reports it with both layers. Profiles, fasteners and materials ride the
//! same machinery in their own namespaces ([`Environment::fastener_lookup`],
//! [`Environment::material_lookup`], [`ProjectChain::fastener_lookup`],
//! [`ProjectChain::material_lookup`]), and every namespace of the firm layer feeds its own
//! chain.
//!
//! A project chain has three namespaces ([`Namespace`]): a listing walks their resolvers one
//! after another and never merges them, because the same text may be a grade and a designation
//! ([`ProjectChain::lookup_of`], [`Lookup::matching`]).
//!
//! Limits, none of them hidden: a bad entry is reported and skipped, while a duplicate
//! designation refuses the whole library; a firm file that does not read is reported and skipped
//! by [`load_firm`], never silently dropped; a project copy whose `environment/` folder survives
//! without its `environment.usda` is reported in [`ProjectChain::environment_lost`], because the
//! folder proves an environment was chosen and the fallback would substitute sections (#628), and
//! so is a recorded environment whose whole copy folder is gone: the store's root layer records
//! the chosen environment identity at creation, so the loss is named instead of looking like a
//! plain project, whose warning-free behaviour is unchanged;
//! the library provenance is reported with the entry
//! but not stored in the definition; the layer engine runs in this process (no LEW-03
//! containment), as for every other command.
//!
//! A creation command resolves a name through one resolver, [`resolve`], in the lookup order
//! of #628 for profiles, fasteners and materials alike: the model first, then the project's
//! chain. It answers with the layer and the library that resolved the name, the entries the
//! weaker libraries declare under it and the lost environment the chain reports, so a command
//! reports and warns instead of silently substituting ([`Resolved`]).

use crate::definition::{DefinitionRecord, field};
use crate::plate::{decode_polygon, decode_polygons, number, text};
use crate::source_file::read_file;
use beamcraft_layer_engine::{Prim, Reader, Value};
use beamcraft_model::command::{CommandError, label_is_valid};
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::reference::check_source_path;
use session::Refusal;
use session::environment::{EnvironmentInfo, MAX_ENVIRONMENT_BYTES, parse_environment};
use session::foreign;
use session::pipeline::digest_of;
use std::collections::{BTreeMap, BTreeSet};
use steel::model::definition::SectionProperties;
use steel::model::fastener::{FastenerKind, FastenerSpec, HeadKind, HoleMode, WasherSpec};
use steel::model::section::{
    AreaConvention, CustomSection, MAX_CUSTOM_CAVITIES, ParametricSection, SectionFamily,
    SectionShape, profile_attribute,
};

/// Largest library file, in bytes.
pub const MAX_LIBRARY_BYTES: usize = foreign::MAX_LAYER_BYTES;
/// Most entries one library may hold; more is `LIMIT_EXCEEDED`.
pub const MAX_LIBRARY_ENTRIES: usize = foreign::MAX_LIBRARY_ENTRIES;

/// Parent path of every profile entry prim.
const ENTRY_PARENT: &str = "/Library/Profiles";
/// Parent path of every fastener entry prim (A14.1).
const FASTENER_ENTRY_PARENT: &str = "/Library/Fasteners";
/// Parent path of every material entry prim (#636).
const MATERIAL_ENTRY_PARENT: &str = "/Library/Materials";

/// The layer of a source that was named on the command line: an explicit single library.
pub const LAYER_LIBRARY: &str = "library";
/// The layer of the user library named on the command line: the strongest source of a lookup.
pub const LAYER_USER_LIBRARY: &str = "user-library";
/// The layer of the user-owned firm libraries (`libraries/` in the application data): above the
/// environment, below the project, never written by the programs.
pub const LAYER_FIRM: &str = "firm";
/// The layer of a project's own copy of its environment.
pub const LAYER_PROJECT: &str = "project";
/// The layer of a chosen environment's libraries.
pub const LAYER_ENVIRONMENT: &str = "environment";
/// The layer of the shipped neutral environment: the built-in fallback every project chain
/// ends with.
pub const LAYER_NEUTRAL: &str = "neutral";
/// The layer of a definition the model already holds: the strongest layer of every lookup a
/// creation command makes ([`resolve`]).
pub const LAYER_MODEL: &str = "model";

/// What the library file declares about itself and what was read.
#[derive(Debug, Clone, PartialEq)]
pub struct LibraryInfo {
    /// `bim:library:id`.
    pub id: String,
    /// `bim:library:title`, when declared.
    pub title: Option<String>,
    /// `bim:library:version`, when declared.
    pub version: Option<String>,
    /// `bim:library:source`, when declared. Never inferred.
    pub source: Option<String>,
    /// `bim:library:licence`, when declared. Never inferred.
    pub licence: Option<String>,
    /// SHA-256 of the library file bytes.
    pub digest: ContentDigest,
    /// Size of the library file in bytes.
    pub bytes: u64,
    /// Whether the file is binary USD (`.usdc` content). Such a library is read but never
    /// rewritten by [`add_entry`].
    pub binary: bool,
}

/// One admitted library entry.
#[derive(Debug, Clone, PartialEq)]
pub struct LibraryEntry {
    /// USD path of the entry prim.
    pub prim_path: String,
    /// The designation, unique in its library.
    pub designation: String,
    /// The complete profile definition the entry resolves to (its display name is the
    /// designation).
    pub record: DefinitionRecord,
}

/// An entry that was read but refused; the other entries still load.
#[derive(Debug, Clone, PartialEq)]
pub struct RejectedEntry {
    /// USD path of the entry prim.
    pub prim_path: String,
    /// The designation, when it could be read.
    pub designation: Option<String>,
    /// Stable F04 code of the refusal.
    pub code: ErrorCode,
    /// Fixed explanation.
    pub message: &'static str,
}

/// A loaded library.
#[derive(Debug, Clone, PartialEq)]
pub struct Library {
    /// The file's own declarations and identity.
    pub info: LibraryInfo,
    /// Admitted entries in designation order.
    pub entries: Vec<LibraryEntry>,
    /// Refused entries in path order.
    pub rejected: Vec<RejectedEntry>,
}

impl Library {
    /// The entry with exactly this designation. An unknown designation is `NOT_FOUND`, never a
    /// near match; a designation whose entry was refused reports that refusal instead.
    pub fn entry(&self, designation: &str) -> Result<&LibraryEntry, Refusal> {
        if let Some(entry) = self.entries.iter().find(|e| e.designation == designation) {
            return Ok(entry);
        }
        if let Some(refused) = self
            .rejected
            .iter()
            .find(|r| r.designation.as_deref() == Some(designation))
        {
            return Err(Refusal {
                rule_id: None,
                code: refused.code,
                message: refused.message,
            });
        }
        Err(Refusal {
            rule_id: None,
            code: ErrorCode::NotFound,
            message: "the designation is not in the library",
        })
    }
}

fn refuse(code: ErrorCode, message: &'static str) -> Refusal {
    Refusal::new(code, message)
}

fn invalid(message: &'static str) -> Refusal {
    refuse(ErrorCode::InvalidArgument, message)
}

/// A library is not a store: damaged content is the caller's bad input, not an integrity error.
fn as_input(error: Refusal) -> Refusal {
    if error.code == ErrorCode::IntegrityError {
        invalid(error.message)
    } else {
        error
    }
}

fn io_error(code: ErrorCode) -> Refusal {
    match code {
        ErrorCode::NotFound => refuse(code, "the library file could not be opened"),
        ErrorCode::LimitExceeded => refuse(code, "the library file is larger than 8 MiB"),
        _ => refuse(
            ErrorCode::InvalidArgument,
            "the library path is not a readable regular local file",
        ),
    }
}

fn label(value: Option<&Value>, message: &'static str) -> Result<Option<String>, Refusal> {
    match value {
        None => Ok(None),
        Some(Value::Text(text)) if label_is_valid(text) => Ok(Some(text.clone())),
        Some(_) => Err(invalid(message)),
    }
}

fn read_info(
    metadata: &BTreeMap<String, Value>,
    digest: ContentDigest,
    bytes: u64,
    binary: bool,
) -> Result<LibraryInfo, Refusal> {
    let Some(Value::Dictionary(layer)) = metadata.get("customLayerData") else {
        return Err(invalid("the library declares no bim:library:id"));
    };
    let get = |key: &str| {
        label(
            layer.get(key),
            "a bim:library value is not a valid string label",
        )
    };
    Ok(LibraryInfo {
        id: get("bim:library:id")?
            .ok_or_else(|| invalid("the library declares no bim:library:id"))?,
        title: get("bim:library:title")?,
        version: get("bim:library:version")?,
        source: get("bim:library:source")?,
        licence: get("bim:library:licence")?,
        digest,
        bytes,
        binary,
    })
}

pub(crate) fn read_all(
    reader: &Reader,
    opened: &beamcraft_layer_engine::Opened,
    control: &beamcraft_layer_engine::Control,
) -> Result<Vec<Prim>, Refusal> {
    let mut prims = Vec::new();
    let mut cursor = None;
    loop {
        let page = reader.read(&opened.context, cursor.as_ref(), control)?;
        prims.extend(page.items);
        match page.next {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    Ok(prims)
}

fn required_text<'a>(prim: &'a Prim, name: &str, ty: &str) -> Result<&'a str, Refusal> {
    field(prim, name, ty)
        .map_err(as_input)?
        .and_then(text)
        .ok_or_else(|| invalid("a required entry attribute is missing or mistyped"))
}

fn optional_number(prim: &Prim, name: &str) -> Result<Option<f64>, Refusal> {
    field(prim, name, "double")
        .map_err(as_input)?
        .map(|v| number(v).ok_or_else(|| invalid("a number attribute has another type")))
        .transpose()
}

fn optional_string(prim: &Prim, name: &str) -> Result<Option<String>, Refusal> {
    Ok(field(prim, name, "string")
        .map_err(as_input)?
        .and_then(text)
        .map(str::to_owned))
}

/// The declared-area convention of an entry: absent is the default exact convention; anything
/// present must be a known convention token.
fn area_convention(prim: &Prim) -> Result<AreaConvention, Refusal> {
    match optional_string(prim, "steel:profile:areaConvention")? {
        None => Ok(AreaConvention::default()),
        Some(token) => AreaConvention::from_token(&token)
            .ok_or_else(|| invalid("the area convention is not a known token")),
    }
}

fn required_number(prim: &Prim, name: &str) -> Result<f64, Refusal> {
    optional_number(prim, name)?.ok_or_else(|| invalid("a required dimension is missing"))
}

fn command(error: CommandError) -> Refusal {
    Refusal::from(error)
}

const DAMAGED_OUTLINE: &str = "the custom outline attributes are inconsistent";

/// The profile definition of one entry prim, admitted by the F04 value rules. A declared area
/// that disagrees with the outline is not a reason to refuse the entry (see
/// [`DefinitionRecord::declared_area_warning`]).
fn decode_record(prim: &Prim, designation: &str) -> Result<DefinitionRecord, Refusal> {
    let mut section = SectionProperties {
        designation: Some(designation.to_owned()),
        standard: field(prim, "steel:profile:standard", "string")
            .map_err(as_input)?
            .map(|v| {
                text(v)
                    .map(str::to_owned)
                    .ok_or_else(|| invalid("the standard is not a string"))
            })
            .transpose()?,
        area_mm2: optional_number(prim, "steel:profile:areaMm2")?,
        mass_per_meter_kg: optional_number(prim, "steel:profile:massPerMeterKg")?,
        surface_per_meter_m2: optional_number(prim, "steel:profile:surfacePerMeterM2")?,
        display_name: Some(designation.to_owned()),
        area_convention: AreaConvention::default(),
    };
    let family = required_text(prim, "steel:profile:family", "token")?;
    let record = match family {
        "rectangle" => {
            if optional_string(prim, "steel:profile:areaConvention")?.is_some() {
                return Err(invalid("a rectangle profile declares no area convention"));
            }
            DefinitionRecord::RectangleProfile {
                width_mm: required_number(prim, "steel:profile:bMm")?,
                height_mm: required_number(prim, "steel:profile:hMm")?,
                section,
            }
        }
        "custom" => {
            if optional_string(prim, "steel:profile:areaConvention")?.is_some() {
                return Err(invalid("a custom profile declares no area convention"));
            }
            DefinitionRecord::SectionProfile {
                shape: SectionShape::Custom(CustomSection {
                    outer: decode_polygon(prim, "steel:profile:outerPointsMm", DAMAGED_OUTLINE)
                        .map_err(as_input)?,
                    cavities: decode_polygons(
                        prim,
                        "steel:profile:cavityPointsMm",
                        "steel:profile:cavityVertexCounts",
                        MAX_CUSTOM_CAVITIES,
                        DAMAGED_OUTLINE,
                    )
                    .map_err(as_input)?,
                }),
                section,
            }
        }
        token => {
            let family = SectionFamily::from_token(token)
                .ok_or_else(|| invalid("unknown section family"))?;
            let values = family
                .parameters()
                .iter()
                .map(|name| required_number(prim, &profile_attribute(name)))
                .collect::<Result<Vec<_>, _>>()?;
            section.area_convention = area_convention(prim)?;
            DefinitionRecord::SectionProfile {
                shape: SectionShape::Parametric(
                    ParametricSection::new(family, values).map_err(command)?,
                ),
                section,
            }
        }
    };
    record.validate().map_err(command)?;
    Ok(record)
}

/// Loads and admits a library file.
///
/// Errors: `PERMISSION_DENIED` for a network or URL path (before any I/O), `NOT_FOUND` for a
/// file that cannot be opened, `LIMIT_EXCEEDED` for a file over 8 MiB, more than 4096 entries or
/// a stage over the library budgets, `INVALID_ARGUMENT` for bytes that are not a readable USD
/// layer, a layer with sublayers or references, other units or up axis, a missing
/// `bim:library:id`, or two entries with the same designation. A refused entry does not fail the
/// load; it is listed in [`Library::rejected`].
pub fn load(path: &str) -> Result<Library, Refusal> {
    check_source_path(path).map_err(command)?;
    load_bytes(read_file(path, MAX_LIBRARY_BYTES).map_err(io_error)?)
}

/// One layer opened read-only under the foreign-layer budgets.
type OpenLayer = foreign::ForeignLayer;

/// Opens the bytes of a library or environment as one foreign, read-only, self-contained layer in
/// millimetres and Z up. `not_self_contained` is the `INVALID_ARGUMENT` text for a layer that the
/// engine cannot open on its own (sublayers, references, other units).
fn open_layer(bytes: Vec<u8>, not_self_contained: &'static str) -> Result<OpenLayer, Refusal> {
    foreign::open(bytes, not_self_contained)
}

/// [`load`] for bytes that were already read (at most 8 MiB, checked again here).
pub fn load_bytes(bytes: Vec<u8>) -> Result<Library, Refusal> {
    Ok(load_bytes_as(bytes, Some(ENTRY_PARENT), decode_record)?.0)
}

/// [`load`] for the fastener library at `path` (A14.1): the same layer format and
/// loading rules, entries under `/Library/Fasteners` carrying `steel:fastener:*` attributes.
pub fn load_fastener(path: &str) -> Result<Library, Refusal> {
    check_source_path(path).map_err(command)?;
    load_fastener_bytes(read_file(path, MAX_LIBRARY_BYTES).map_err(io_error)?)
}

/// [`load_bytes`] for the bytes of a fastener library.
pub fn load_fastener_bytes(bytes: Vec<u8>) -> Result<Library, Refusal> {
    Ok(load_bytes_as(bytes, Some(FASTENER_ENTRY_PARENT), decode_fastener_record)?.0)
}

/// [`load`] for the material library at `path` (#636): the same layer format and loading rules,
/// entries under `/Library/Materials` carrying `steel:material:*` attributes. The designation of
/// an entry is its grade.
pub fn load_material(path: &str) -> Result<Library, Refusal> {
    check_source_path(path).map_err(command)?;
    load_material_bytes(read_file(path, MAX_LIBRARY_BYTES).map_err(io_error)?)
}

/// [`load_bytes`] for the bytes of a material library.
pub fn load_material_bytes(bytes: Vec<u8>) -> Result<Library, Refusal> {
    Ok(load_bytes_as(bytes, Some(MATERIAL_ENTRY_PARENT), decode_material_record)?.0)
}

/// [`load_firm_bytes`] for a firm-layer file, whose entries may sit under any entry parent: the
/// profile, fastener and material admission runs in one pass over the same opened layer and
/// every entry joins the half of its own parent, so a mixed file feeds every namespace of the
/// firm layer. Any half may be empty; all carry the file's own declarations.
pub fn load_firm_bytes(bytes: Vec<u8>) -> Result<(Library, Library, Library), Refusal> {
    load_bytes_as(bytes, None, decode_record)
}

/// [`load_firm_bytes`] for the library file at `path`.
pub fn load_firm_file(path: &str) -> Result<(Library, Library, Library), Refusal> {
    check_source_path(path).map_err(command)?;
    load_firm_bytes(read_file(path, MAX_LIBRARY_BYTES).map_err(io_error)?)
}

/// [`load_bytes`] for one entry parent path (or `None` for a firm file, where every entry
/// parent is admitted and each entry joins the half of its own parent) and the profile entry
/// decoder; the layer budgets, identity, admission and refused-entry reporting are shared with
/// the profile loading. The second and third libraries are the fastener and material halves of
/// a firm file; when a single parent was named they are empty libraries, because the one named
/// parent is the answer.
fn load_bytes_as(
    bytes: Vec<u8>,
    entry_parent: Option<&str>,
    decode: fn(&Prim, &str) -> Result<DefinitionRecord, Refusal>,
) -> Result<(Library, Library, Library), Refusal> {
    if bytes.len() > MAX_LIBRARY_BYTES {
        return Err(io_error(ErrorCode::LimitExceeded));
    }
    let binary = bytes.starts_with(b"PXR-USDC");
    let OpenLayer {
        reader,
        opened,
        control,
        digest,
        bytes: size,
    } = open_layer(
        bytes,
        "the library is not a self-contained USD layer in millimetres, Z up",
    )?;
    let info = read_info(&opened.metadata, digest, size, binary)?;
    let prims = read_all(&reader, &opened, &control)?;
    // The three namespaces of a firm file, collected by their own entry parents in one pass.
    // Designation uniqueness is per namespace: a mixed firm file may name a profile, a bolt and
    // a grade the same, but never two of one kind.
    struct Half {
        entries: Vec<LibraryEntry>,
        rejected: Vec<RejectedEntry>,
        seen: BTreeSet<String>,
    }
    impl Half {
        fn new() -> Self {
            Self {
                entries: Vec::new(),
                rejected: Vec::new(),
                seen: BTreeSet::new(),
            }
        }
    }
    let mut profiles = Half::new();
    let mut fasteners = Half::new();
    let mut materials = Half::new();
    let mut count = 0usize;
    for prim in prims.iter().filter(|p| {
        let parent = p.path.rsplit_once('/').map(|(parent, _)| parent);
        match entry_parent {
            Some(one) => parent == Some(one),
            None => {
                parent == Some(ENTRY_PARENT)
                    || parent == Some(FASTENER_ENTRY_PARENT)
                    || parent == Some(MATERIAL_ENTRY_PARENT)
            }
        }
    }) {
        count += 1;
        if count > MAX_LIBRARY_ENTRIES {
            return Err(refuse(
                ErrorCode::LimitExceeded,
                "the library has more than 4096 entries",
            ));
        }
        let prim_parent = prim.path.rsplit_once('/').map(|(parent, _)| parent);
        // The filter above admitted this prim. In firm mode every entry joins the half of its
        // own parent; when one parent was named, it is the answer and it fills the first
        // library, whatever its kind.
        let kind_parent = match (entry_parent, prim_parent) {
            (Some(one), _) => one,
            (None, Some(parent))
                if parent == FASTENER_ENTRY_PARENT || parent == MATERIAL_ENTRY_PARENT =>
            {
                parent
            }
            _ => ENTRY_PARENT,
        };
        let half = if entry_parent.is_none() {
            match kind_parent {
                FASTENER_ENTRY_PARENT => &mut fasteners,
                MATERIAL_ENTRY_PARENT => &mut materials,
                _ => &mut profiles,
            }
        } else {
            &mut profiles
        };
        let designation = match required_text(prim, designation_attribute(kind_parent), "string") {
            Ok(text) if label_is_valid(text) => text.to_owned(),
            Ok(_) | Err(_) => {
                half.rejected.push(RejectedEntry {
                    prim_path: prim.path.clone(),
                    designation: None,
                    code: ErrorCode::InvalidArgument,
                    message: "the entry has no valid designation",
                });
                continue;
            }
        };
        if !half.seen.insert(designation.clone()) {
            return Err(invalid(
                "the library has two entries with the same designation",
            ));
        }
        if !prim.active {
            half.rejected.push(RejectedEntry {
                prim_path: prim.path.clone(),
                designation: Some(designation),
                code: ErrorCode::InvalidArgument,
                message: "the entry is inactive",
            });
            continue;
        }
        let decoded = match kind_parent {
            FASTENER_ENTRY_PARENT => decode_fastener_record(prim, &designation),
            MATERIAL_ENTRY_PARENT => decode_material_record(prim, &designation),
            _ => decode(prim, &designation),
        };
        let record = match decoded {
            Ok(record) => record,
            Err(error) => {
                half.rejected.push(RejectedEntry {
                    prim_path: prim.path.clone(),
                    designation: Some(designation.clone()),
                    code: error.code,
                    message: error.message,
                });
                continue;
            }
        };
        half.entries.push(LibraryEntry {
            prim_path: prim.path.clone(),
            designation,
            record,
        });
    }
    let half_library = |half: &mut Half, info: &LibraryInfo| {
        half.entries
            .sort_by(|a, b| a.designation.cmp(&b.designation));
        Library {
            info: info.clone(),
            entries: std::mem::take(&mut half.entries),
            rejected: std::mem::take(&mut half.rejected),
        }
    };
    let profiles = half_library(&mut profiles, &info);
    let auto = entry_parent.is_none();
    let fasteners = if auto {
        half_library(&mut fasteners, &info)
    } else {
        Library {
            info: info.clone(),
            entries: Vec::new(),
            rejected: Vec::new(),
        }
    };
    let materials = if auto {
        half_library(&mut materials, &info)
    } else {
        Library {
            info,
            entries: Vec::new(),
            rejected: Vec::new(),
        }
    };
    Ok((profiles, fasteners, materials))
}

/// The designation attribute of one entry kind: `steel:profile:designation`,
/// `steel:fastener:designation` or `steel:material:grade`.
fn designation_attribute(entry_parent: &str) -> &'static str {
    if entry_parent == FASTENER_ENTRY_PARENT {
        "steel:fastener:designation"
    } else if entry_parent == MATERIAL_ENTRY_PARENT {
        "steel:material:grade"
    } else {
        "steel:profile:designation"
    }
}

/// The fastener definition of one entry prim, admitted by the F04 value rules (A14.1). A
/// declared mass is optional; nothing is inferred from the designation.
fn decode_fastener_record(prim: &Prim, designation: &str) -> Result<DefinitionRecord, Refusal> {
    let double = |name: &str| -> Result<Option<f64>, Refusal> {
        optional_number(prim, &format!("steel:fastener:{name}"))
    };
    let required_double = |name: &'static str| -> Result<f64, Refusal> {
        double(name)?.ok_or_else(|| invalid("a required fastener dimension is missing"))
    };
    let optional_text = |name: &str| -> Result<Option<String>, Refusal> {
        field(prim, &format!("steel:fastener:{name}"), "string")
            .map_err(as_input)?
            .map(|v| {
                text(v)
                    .map(str::to_owned)
                    .ok_or_else(|| invalid("a fastener label is not a string"))
            })
            .transpose()
    };
    let token = |name: &str| -> Result<String, Refusal> {
        field(prim, &format!("steel:fastener:{name}"), "token")
            .map_err(as_input)?
            .and_then(text)
            .map(str::to_owned)
            .ok_or_else(|| invalid("a required fastener token is missing or mistyped"))
    };
    let boolean = |name: &str| -> Result<bool, Refusal> {
        match field(prim, &format!("steel:fastener:{name}"), "bool").map_err(as_input)? {
            Some(Value::Bool(value)) => Ok(*value),
            _ => Err(invalid("a required fastener switch is missing or mistyped")),
        }
    };
    let kind = FastenerKind::from_token(&token("kind")?)
        .ok_or_else(|| invalid("unknown fastener kind"))?;
    let head_kind = HeadKind::from_token(&token("headKind")?)
        .ok_or_else(|| invalid("unknown fastener head kind"))?;
    let hole_mode = HoleMode::from_token(&token("holeMode")?)
        .ok_or_else(|| invalid("unknown fastener hole mode"))?;
    let washer = match (
        double("washerOuterDiameterMm")?,
        double("washerInnerDiameterMm")?,
        double("washerThicknessMm")?,
    ) {
        (None, None, None) => None,
        (outer, inner, thickness) => Some(WasherSpec {
            outer_diameter_mm: outer.ok_or_else(|| invalid("a washer dimension is missing"))?,
            inner_diameter_mm: inner.ok_or_else(|| invalid("a washer dimension is missing"))?,
            thickness_mm: thickness.ok_or_else(|| invalid("a washer dimension is missing"))?,
        }),
    };
    let lengths = match field(prim, "steel:fastener:lengthsMm", "double[]").map_err(as_input)? {
        Some(Value::Array(values)) => values
            .iter()
            .map(|v| number(v).ok_or_else(|| invalid("a standard length is not a number")))
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Err(invalid("the standard lengths are missing or mistyped")),
    };
    let spec = FastenerSpec {
        designation: Some(designation.to_owned()),
        standard: optional_text("standard")?,
        grade: optional_text("grade")?,
        display_name: Some(designation.to_owned()),
        kind,
        head_kind,
        nominal_diameter_mm: required_double("nominalDiameterMm")?,
        head_across_flats_mm: required_double("headAcrossFlatsMm")?,
        head_height_mm: required_double("headHeightMm")?,
        nut_across_flats_mm: double("nutAcrossFlatsMm")?,
        nut_height_mm: double("nutHeightMm")?,
        washer,
        hole_clearance_mm: required_double("holeClearanceMm")?,
        lengths_mm: lengths,
        thread_protrusion_mm: required_double("threadProtrusionMm")?,
        mass_each_kg: double("massEachKg")?,
        washer_under_head: boolean("washerUnderHead")?,
        washer_under_nut: boolean("washerUnderNut")?,
        hole_mode,
    };
    spec.validate().map_err(command)?;
    Ok(DefinitionRecord::Fastener { spec })
}

/// The material definition of one entry prim, admitted by the F04 value rules (#636). The
/// density and the grade are required; the standard and the display name are optional, and a
/// display name that is not declared is the grade. Nothing is inferred from the grade: an
/// unknown density is refused, never substituted.
fn decode_material_record(prim: &Prim, designation: &str) -> Result<DefinitionRecord, Refusal> {
    let optional_text = |name: &str| -> Result<Option<String>, Refusal> {
        field(prim, &format!("steel:material:{name}"), "string")
            .map_err(as_input)?
            .map(|v| {
                text(v)
                    .map(str::to_owned)
                    .ok_or_else(|| invalid("a material label is not a string"))
            })
            .transpose()
    };
    let display_name = optional_text("displayName")?.unwrap_or_else(|| designation.to_owned());
    let density = optional_number(prim, "steel:material:densityKgM3")?
        .ok_or_else(|| invalid("a material entry has no density"))?;
    let record = DefinitionRecord::Material {
        density_kg_m3: Some(density),
        grade: Some(designation.to_owned()),
        standard: optional_text("standard")?,
        display_name: Some(display_name),
    };
    record.validate().map_err(command)?;
    Ok(record)
}

/// One library of a lookup, with the path it was read from and the layer it sits in.
#[derive(Debug, Clone, PartialEq)]
pub struct Source {
    /// The path the library was read from: as given, or resolved against its environment file.
    pub path: String,
    /// The loaded library.
    pub library: Library,
    /// The layer the library sits in, one of the [`LAYER_LIBRARY`] family of constants.
    pub layer: &'static str,
}

/// A designation that a stronger library already provides; the weaker entry is never used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shadowed {
    /// The designation.
    pub designation: String,
    /// `bim:library:id` of the stronger library, whose entry (or refusal) answers a lookup.
    pub used_library: String,
    /// The layer of the stronger library.
    pub used_layer: &'static str,
    /// `bim:library:id` of the weaker library that has the same designation.
    pub shadowed_library: String,
    /// The layer of the weaker library.
    pub shadowed_layer: &'static str,
}

/// A catalogue namespace: one of the three kinds of library an environment selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Namespace {
    /// Profiles: entries under `/Library/Profiles`.
    Profile,
    /// Fasteners: entries under `/Library/Fasteners`.
    Fastener,
    /// Materials: entries under `/Library/Materials`.
    Material,
}

impl Namespace {
    /// Every namespace, in listing order.
    pub const ALL: [Self; 3] = [Self::Profile, Self::Fastener, Self::Material];

    /// The token of `--namespace` and of a reply's `namespace`.
    pub const fn token(self) -> &'static str {
        match self {
            Self::Profile => "profile",
            Self::Fastener => "fastener",
            Self::Material => "material",
        }
    }

    /// The namespace of a token; any other text is `None`.
    pub fn parse(token: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|namespace| namespace.token() == token)
    }
}

/// Libraries in lookup order, strongest first.
///
/// The first library that has a designation answers for it, and a refused entry answers with its
/// refusal: a weaker library is never consulted for a designation that a stronger one declares,
/// valid or not. A default lookup has no library and answers nothing.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Lookup {
    sources: Vec<Source>,
}

enum Slot<'a> {
    Entry(&'a Source, &'a LibraryEntry),
    Refused,
}

/// One designation's answering library id, its layer, and the entry or refusal behind it.
type Answered<'a> = BTreeMap<&'a str, (&'a str, &'static str, Slot<'a>)>;

impl Lookup {
    /// A lookup over these libraries, strongest first.
    pub fn new(sources: Vec<Source>) -> Self {
        Self { sources }
    }

    /// The libraries in lookup order.
    pub fn sources(&self) -> &[Source] {
        &self.sources
    }

    /// The entry that answers for exactly this designation, with its library. `NOT_FOUND` when no
    /// library declares it; the refusal of the first declaring library otherwise.
    pub fn find(&self, designation: &str) -> Result<(&Source, &LibraryEntry), Refusal> {
        for source in &self.sources {
            match source.library.entry(designation) {
                Ok(entry) => return Ok((source, entry)),
                Err(error) if error.code == ErrorCode::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Err(refuse(
            ErrorCode::NotFound,
            "the designation is not in any library of the lookup",
        ))
    }

    fn slots(&self) -> (Answered<'_>, Vec<Shadowed>) {
        let mut slots: Answered<'_> = BTreeMap::new();
        let mut shadowed = Vec::new();
        for source in &self.sources {
            let id = source.library.info.id.as_str();
            let declared = source
                .library
                .entries
                .iter()
                .map(|e| (e.designation.as_str(), Some(e)))
                .chain(
                    source
                        .library
                        .rejected
                        .iter()
                        .filter_map(|r| r.designation.as_deref().map(|d| (d, None))),
                );
            for (designation, entry) in declared {
                match slots.get(designation) {
                    Some((winner, winner_layer, _)) => shadowed.push(Shadowed {
                        designation: designation.to_owned(),
                        used_library: (*winner).to_owned(),
                        used_layer: winner_layer,
                        shadowed_library: id.to_owned(),
                        shadowed_layer: source.layer,
                    }),
                    None => {
                        let slot = entry.map_or(Slot::Refused, |e| Slot::Entry(source, e));
                        slots.insert(designation, (id, source.layer, slot));
                    }
                }
            }
        }
        shadowed.sort_by(|a, b| {
            (&a.designation, &a.shadowed_library).cmp(&(&b.designation, &b.shadowed_library))
        });
        (slots, shadowed)
    }

    /// The entries that answer a lookup, one per designation, in designation order.
    pub fn effective(&self) -> Vec<(&Source, &LibraryEntry)> {
        self.matching(None)
    }

    /// The entries that answer a lookup, one per designation in designation order, whose
    /// designation matches `text` ([`designation_matches`]); every one of them when `text` is
    /// `None`.
    pub fn matching(&self, text: Option<&str>) -> Vec<(&Source, &LibraryEntry)> {
        self.slots()
            .0
            .into_values()
            .filter_map(|(_, _, slot)| match slot {
                Slot::Entry(source, entry) => Some((source, entry)),
                Slot::Refused => None,
            })
            .filter(|(_, entry)| {
                text.is_none_or(|text| designation_matches(&entry.designation, text))
            })
            .collect()
    }

    /// Designations that more than one library declares, strongest first.
    pub fn shadowed(&self) -> Vec<Shadowed> {
        self.slots().1
    }

    /// The shadow records of one designation.
    pub fn shadowed_for(&self, designation: &str) -> Vec<Shadowed> {
        let mut records = self.slots().1;
        records.retain(|s| s.designation == designation);
        records
    }
}

/// Whether a designation matches a search text: both lower-cased in every script and stripped of
/// whitespace; the folded text must occur in the folded designation.
///
/// This is a search, never a lookup: it proposes entries, and an exact lookup disposes.
pub fn designation_matches(designation: &str, text: &str) -> bool {
    fold(designation).contains(&fold(text))
}

/// The search fold of one text. Lower-casing is per character, so it is a widening of the ASCII
/// fold it replaces for every needle: a context-dependent rule (the Greek final sigma) could turn
/// a designation and a needle that agreed into ones that do not.
fn fold(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

/// A loaded environment: its libraries in lookup order.
#[derive(Debug, Clone, PartialEq)]
pub struct Environment {
    /// The environment file's own declarations.
    pub info: EnvironmentInfo,
    /// The listed profile libraries, strongest first.
    pub sources: Vec<Source>,
    /// The listed fastener libraries, strongest first (BC-ENV-002: bolts ride the same
    /// environment as profiles).
    pub fastener_sources: Vec<Source>,
    /// The listed material libraries, strongest first (#636: grades ride the same environment
    /// as profiles and bolts).
    pub material_sources: Vec<Source>,
}

impl Environment {
    /// The resolver over the environment's fastener libraries, strongest first: the same
    /// [`Lookup`] machinery that answers profile designations answers fastener ones. The two
    /// namespaces stay separate; a profile library never answers a bolt lookup.
    #[must_use]
    pub fn fastener_lookup(&self) -> Lookup {
        Lookup::new(self.fastener_sources.clone())
    }

    /// The resolver over the environment's material libraries, strongest first: the same
    /// [`Lookup`] machinery answers a grade. The namespaces stay separate; a profile library
    /// never answers a material lookup and the other way round.
    #[must_use]
    pub fn material_lookup(&self) -> Lookup {
        Lookup::new(self.material_sources.clone())
    }
}

/// Admits one listed library of an environment: its provenance is declared (`bim:library:source`
/// and `bim:library:licence`, empty is undeclared) and its `bim:library:id` is new among every
/// library of the environment, profile, fastener or material. A single-file `--library` needs no
/// provenance, so the rule lives here, at the environment's edge.
fn admit_library(
    sources: &mut Vec<Source>,
    ids: &mut Vec<String>,
    path: String,
    library: Library,
) -> Result<(), Refusal> {
    if library.info.source.as_ref().is_none_or(String::is_empty) {
        return Err(invalid(
            "a library of the environment declares no bim:library:source",
        ));
    }
    if library.info.licence.as_ref().is_none_or(String::is_empty) {
        return Err(invalid(
            "a library of the environment declares no bim:library:licence",
        ));
    }
    if ids.contains(&library.info.id) {
        return Err(invalid(
            "two libraries of the environment have the same bim:library:id",
        ));
    }
    ids.push(library.info.id.clone());
    sources.push(Source {
        path,
        library,
        layer: LAYER_ENVIRONMENT,
    });
    Ok(())
}

/// Resolves one listed reference against the environment directory.
fn resolve_reference(directory: &std::path::Path, reference: &str) -> Result<String, Refusal> {
    let resolved = directory.join(reference);
    resolved
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| invalid("an environment library path is not UTF-8"))
}

/// Loads an environment file and every library it lists, in order.
///
/// Each listed path is resolved against the directory of the environment file and loaded with
/// [`load`] (profile libraries), [`load_fastener`] (fastener libraries) or [`load_material`]
/// (material libraries), so every library keeps its own bounds and admission. A library that
/// cannot be loaded, one that declares no `source` or `licence`, or two listed libraries with
/// the same `bim:library:id`, refuse the whole environment: a missing library would silently
/// change which section, bolt or grade a designation selects. The containment of a listed path
/// is lexical; the files themselves are read like any `--library` file.
pub fn load_environment(path: &str) -> Result<Environment, Refusal> {
    check_source_path(path).map_err(command)?;
    let bytes = read_file(path, MAX_ENVIRONMENT_BYTES).map_err(|code| match code {
        ErrorCode::NotFound => refuse(code, "the environment file could not be opened"),
        ErrorCode::LimitExceeded => refuse(code, "the environment file is larger than 1 MiB"),
        _ => refuse(
            ErrorCode::InvalidArgument,
            "the environment path is not a readable regular local file",
        ),
    })?;
    let info = parse_environment(bytes)?;
    let directory = std::path::Path::new(path)
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_default();
    let mut ids: Vec<String> = Vec::new();
    let mut sources: Vec<Source> = Vec::with_capacity(info.libraries.len());
    for reference in &info.libraries {
        let resolved = resolve_reference(&directory, reference)?;
        let library = load(&resolved)?;
        admit_library(&mut sources, &mut ids, resolved, library)?;
    }
    let mut fastener_sources: Vec<Source> = Vec::with_capacity(info.fastener_libraries.len());
    for reference in &info.fastener_libraries {
        let resolved = resolve_reference(&directory, reference)?;
        let library = load_fastener(&resolved)?;
        admit_library(&mut fastener_sources, &mut ids, resolved, library)?;
    }
    let mut material_sources: Vec<Source> = Vec::with_capacity(info.material_libraries.len());
    for reference in &info.material_libraries {
        let resolved = resolve_reference(&directory, reference)?;
        let library = load_material(&resolved)?;
        admit_library(&mut material_sources, &mut ids, resolved, library)?;
    }
    Ok(Environment {
        info,
        sources,
        fastener_sources,
        material_sources,
    })
}

/// A lookup a command opened, with its provenance: the resolver, the environment declarations
/// when one was named, and the firm files that did not read (reported, never silently dropped).
#[derive(Debug, Clone, PartialEq)]
pub struct OpenedLookup {
    /// The resolver over the layered sources, strongest first.
    pub lookup: Lookup,
    /// The environment's own declarations, when an environment was named.
    pub environment: Option<EnvironmentInfo>,
    /// The firm files that did not read, in file-name order.
    pub firm_refused: Vec<FirmFileRefused>,
}

/// The material lookup that a command names (#636): exactly one of a single `--library` file
/// (a material library, no provenance required) or an `--environment` file (its material
/// libraries in order), like [`open_lookup`] for profiles. A single `--library` names every
/// source exactly; an `--environment` consults the firm material libraries above the
/// environment's own (no `--user-library` option exists for materials yet), and a firm file
/// that does not read is skipped as [`open_lookup`] describes. The environment's own
/// declarations are returned when one was named.
///
/// # Errors
/// The same refusals as [`open_lookup`], with the material loading rules.
pub fn open_material_lookup(
    library: Option<&str>,
    environment: Option<&str>,
    config_dir: Option<&std::path::Path>,
) -> Result<(Lookup, Option<EnvironmentInfo>), Refusal> {
    match (library, environment) {
        (Some(_), Some(_)) => Err(invalid("--library and --environment are alternatives")),
        (None, None) => Err(invalid("--library or --environment is required")),
        (Some(path), None) => Ok((
            Lookup::new(vec![Source {
                path: path.to_owned(),
                library: load_material(path)?,
                layer: LAYER_LIBRARY,
            }]),
            None,
        )),
        (None, Some(path)) => {
            let loaded = load_environment(path)?;
            let info = loaded.info.clone();
            // The firm layer sits above the environment, as in every other lookup of this
            // module: a named environment never beats the user's own catalog for a grade.
            let mut sources = load_firm(config_dir).material_sources;
            sources.extend(loaded.material_sources);
            Ok((Lookup::new(sources), Some(info)))
        }
    }
}

/// The lookup that a command names: exactly one of a single `--library` file or an
/// `--environment` file (its libraries in order), with an optional `--user-library` file that is
/// consulted before either. Never a hidden global; with none of them it is `INVALID_ARGUMENT`.
/// The environment's own declarations are returned when one was named.
///
/// A single `--library` names every source exactly: no firm layer joins it, so `list-library
/// --library FILE` shows the one file. An `--environment` is the layered lookup: the firm
/// libraries of `config_dir` (or of the application data when `config_dir` is `None`) are
/// consulted between an optional `--user-library` and the environment's own libraries, and a
/// firm file that does not read is reported in [`OpenedLookup::firm_refused`] and skipped.
pub fn open_lookup(
    library: Option<&str>,
    environment: Option<&str>,
    user_library: Option<&str>,
    config_dir: Option<&std::path::Path>,
) -> Result<OpenedLookup, Refusal> {
    let mut environment_info = None;
    let mut firm_refused = Vec::new();
    let base = match (library, environment) {
        (Some(_), Some(_)) => {
            return Err(invalid("--library and --environment are alternatives"));
        }
        (None, None) => {
            return Err(invalid("--library or --environment is required"));
        }
        (Some(path), None) => vec![Source {
            path: path.to_owned(),
            library: load(path)?,
            layer: LAYER_LIBRARY,
        }],
        (None, Some(path)) => {
            let loaded = load_environment(path)?;
            environment_info = Some(loaded.info);
            loaded.sources
        }
    };
    let mut sources = Vec::new();
    if let Some(path) = user_library {
        let user = Source {
            path: path.to_owned(),
            library: load(path)?,
            layer: LAYER_USER_LIBRARY,
        };
        // The same identity twice would make the library ids of entries and shadow records
        // ambiguous, so it is refused for a single library as it is for an environment.
        if base
            .iter()
            .chain(sources.iter())
            .any(|s| s.library.info.id == user.library.info.id)
        {
            return Err(invalid(
                "the user library has the same bim:library:id as another library of the lookup",
            ));
        }
        sources.push(user);
    }
    if environment.is_some() {
        let firm = load_firm(config_dir);
        firm_refused = firm.refused;
        sources.extend(firm.sources);
    }
    sources.extend(base);
    Ok(OpenedLookup {
        lookup: Lookup::new(sources),
        environment: environment_info,
        firm_refused,
    })
}

/// One firm file that did not read; the file is skipped and the other files load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirmFileRefused {
    /// The library file's path.
    pub path: String,
    /// Stable F04 code of the refusal.
    pub code: ErrorCode,
    /// Fixed explanation.
    pub message: &'static str,
}

/// The user-owned firm layer: the library files of `libraries/`, loaded, with the files that
/// did not read reported instead of dropped.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FirmLayer {
    /// The firm profile libraries, in file-name order.
    pub sources: Vec<Source>,
    /// The firm fastener libraries, in file-name order.
    pub fastener_sources: Vec<Source>,
    /// The firm material libraries, in file-name order.
    pub material_sources: Vec<Source>,
    /// The firm files that did not read, in file-name order.
    pub refused: Vec<FirmFileRefused>,
}

/// Loads the user-owned firm layer: every library file [`session::environment::firm_library_files`]
/// finds in `config_dir`'s `libraries/` (or in the current user's, when `config_dir` is `None`),
/// in file-name order. Every entry joins the list of its own parent, so a mixed file feeds
/// [`FirmLayer::sources`], [`FirmLayer::fastener_sources`] and [`FirmLayer::material_sources`]
/// all; a file that does not read, a non-UTF-8 name, or a second file of a `bim:library:id`
/// already taken is reported in [`FirmLayer::refused`] and skipped — a firm file is never
/// allowed to break the layers above or below it, and never silently ignored either. The firm
/// layer is the user's own: nothing here writes a file, and no environment update ever touches
/// it.
#[must_use]
pub fn load_firm(config_dir: Option<&std::path::Path>) -> FirmLayer {
    let mut layer = FirmLayer::default();
    let mut ids: Vec<String> = Vec::new();
    for file in session::environment::firm_library_files(config_dir) {
        let Some(path) = file.to_str() else {
            layer.refused.push(FirmFileRefused {
                path: file.to_string_lossy().into_owned(),
                code: ErrorCode::InvalidArgument,
                message: "the firm library path is not UTF-8",
            });
            continue;
        };
        let loaded = load_firm_file(path);
        match loaded {
            Ok((profiles, fasteners, materials)) => {
                if ids.contains(&profiles.info.id) {
                    layer.refused.push(FirmFileRefused {
                        path: path.to_owned(),
                        code: ErrorCode::InvalidArgument,
                        message: "two firm libraries have the same bim:library:id",
                    });
                    continue;
                }
                ids.push(profiles.info.id.clone());
                // A half of a file that holds nothing of its kind is not a source: an empty
                // library answers no lookup and would only pad the listing.
                for (half, sources) in [
                    (profiles, &mut layer.sources),
                    (fasteners, &mut layer.fastener_sources),
                    (materials, &mut layer.material_sources),
                ] {
                    if !half.entries.is_empty() || !half.rejected.is_empty() {
                        sources.push(Source {
                            path: path.to_owned(),
                            library: half,
                            layer: LAYER_FIRM,
                        });
                    }
                }
            }
            Err(error) => layer.refused.push(FirmFileRefused {
                path: path.to_owned(),
                code: error.code,
                message: error.message,
            }),
        }
    }
    layer
}

/// The shipped neutral environment, loaded: the built-in fallback layer a project chain ends
/// with. `None` when the program ships no [`session::environment::NEUTRAL_ENVIRONMENT_ID`]
/// environment where it looks for its built-in ones.
pub fn neutral_environment() -> Result<Option<Environment>, Refusal> {
    let folders: Vec<_> = session::environment::builtin_environment_dirs()
        .into_iter()
        .map(|dir| (dir, "builtin"))
        .collect();
    let Some(file) = session::environment::find_environment(
        &folders,
        session::environment::NEUTRAL_ENVIRONMENT_ID,
    ) else {
        return Ok(None);
    };
    let path = file
        .to_str()
        .ok_or_else(|| invalid("the neutral environment path is not UTF-8"))?;
    load_environment(path).map(Some)
}

/// Appends the neutral environment's libraries as the last, weakest layer of a chain. A neutral
/// library whose `bim:library:id` is already in the chain is skipped: a project seeded from the
/// neutral environment carries its byte-identical copy already, and a self-shadowing report of
/// identical ids would be noise.
pub fn append_neutral(
    sources: &mut Vec<Source>,
    fastener_sources: &mut Vec<Source>,
    material_sources: &mut Vec<Source>,
) -> Result<(), Refusal> {
    let Some(neutral) = neutral_environment()? else {
        return Ok(());
    };
    for (neutral_sources, chain_sources) in [
        (&neutral.sources, sources),
        (&neutral.fastener_sources, fastener_sources),
        (&neutral.material_sources, material_sources),
    ] {
        for source in neutral_sources {
            let id = &source.library.info.id;
            if !chain_sources.iter().any(|s| &s.library.info.id == id) {
                chain_sources.push(Source {
                    layer: LAYER_NEUTRAL,
                    ..source.clone()
                });
            }
        }
    }
    Ok(())
}

/// The project's environment is demonstrably lost, and the fallback may answer a designation
/// with another section, bolt or grade than the environment the project was created with
/// (#628). Two shapes report it: a copy whose `environment/` folder survives without its
/// `environment.usda`, and — since the store records the chosen environment identity at
/// creation — a project whose whole copy folder is gone while its root layer still names the
/// environment it was created with. A plain project without a folder and without a record is a
/// normal state, not a loss, and warns nowhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentCopyLost {
    /// The path of the environment file the project is missing: the copy's own file for an
    /// incomplete copy, and the path the copy would occupy for a recorded environment whose
    /// folder is gone.
    pub path: String,
    /// The explanation, naming the recorded environment when the store knows it.
    pub message: String,
}

/// The layered lookup of one project: the project's own copy of its environment (the project
/// layer), then the firm libraries of the application data, then the built-in neutral
/// environment. The strongest first; a designation the model already defines never reaches this
/// chain, because the model layer answers before it. A project whose environment copy is absent
/// resolves from firm and neutral alone; `environment` carries the copy's declarations when it
/// is present, `environment_lost` reports a demonstrably incomplete copy, and `firm_refused`
/// names the firm files that did not read.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectChain {
    /// The resolver over the profile libraries of the chain, strongest first.
    pub lookup: Lookup,
    /// The resolver over the fastener libraries of the chain, strongest first; `None` when no
    /// layer of the chain declares a fastener library.
    pub fastener_lookup: Option<Lookup>,
    /// The resolver over the material libraries of the chain, strongest first; `None` when no
    /// layer of the chain declares a material library.
    pub material_lookup: Option<Lookup>,
    /// The project environment copy's own declarations, when the project carries one.
    pub environment: Option<EnvironmentInfo>,
    /// The project's lost environment: the `environment/` folder that proves one was chosen
    /// with its `environment.usda` gone, or a recorded environment whose whole copy folder is
    /// gone; `None` for a plain project (no folder, no record) and for a copy that reads.
    pub environment_lost: Option<EnvironmentCopyLost>,
    /// The firm files that did not read, in file-name order.
    pub firm_refused: Vec<FirmFileRefused>,
}

impl ProjectChain {
    /// The resolver of one namespace over the chain, strongest first; `None` when no layer of
    /// the chain declares a library of that namespace. The resolvers stay apart on purpose: the
    /// same text may be a grade and a designation, so one merged lookup would report that as
    /// shadowing.
    pub fn lookup_of(&self, namespace: Namespace) -> Option<&Lookup> {
        match namespace {
            Namespace::Profile => Some(&self.lookup),
            Namespace::Fastener => self.fastener_lookup.as_ref(),
            Namespace::Material => self.material_lookup.as_ref(),
        }
    }
}

/// Builds the [`ProjectChain`] of the project folder at `store`. The project's environment copy
/// is read with [`load_environment`] (a refused copy refuses the chain, as a missing library
/// would silently change which section, bolt or grade a designation selects) and retagged as
/// the project layer; the firm layer is loaded from `config_dir`'s `libraries/` (`None` is the
/// current user's). The neutral environment joins last. A copy whose folder survives without
/// its `environment.usda` cannot be read and does not refuse — an honest chain resolves from
/// firm and neutral — but it is reported in [`ProjectChain::environment_lost`] rather than
/// silently dropped, and so is a recorded environment whose whole copy folder is gone: the
/// store's root layer names the environment the project was created with, so the loss is
/// reported with its identity instead of looking like a plain project (#628). Every namespace —
/// profiles, fasteners and materials — builds its own resolver over the same chain.
pub fn project_library_chain(
    store: &std::path::Path,
    config_dir: Option<&std::path::Path>,
) -> Result<ProjectChain, Refusal> {
    let mut sources = Vec::new();
    let mut fastener_sources = Vec::new();
    let mut material_sources = Vec::new();
    let mut environment = None;
    let mut environment_lost = None;
    match crate::project::project_environment_copy(store) {
        crate::project::ProjectEnvironmentCopy::Present(file) => {
            let file = file
                .to_str()
                .ok_or_else(|| invalid("the project environment path is not UTF-8"))?;
            let copy = load_environment(file)?;
            environment = Some(copy.info);
            sources.extend(copy.sources.into_iter().map(|s| Source {
                layer: LAYER_PROJECT,
                ..s
            }));
            fastener_sources.extend(copy.fastener_sources.into_iter().map(|s| Source {
                layer: LAYER_PROJECT,
                ..s
            }));
            material_sources.extend(copy.material_sources.into_iter().map(|s| Source {
                layer: LAYER_PROJECT,
                ..s
            }));
        }
        crate::project::ProjectEnvironmentCopy::Incomplete(file) => {
            environment_lost = Some(EnvironmentCopyLost {
                path: file.to_string_lossy().into_owned(),
                message: "the project's environment copy has lost its environment.usda; profile, fastener and material names resolve from the firm libraries and the built-in neutral environment, not the environment the project was created with".to_owned(),
            });
        }
        crate::project::ProjectEnvironmentCopy::Absent => {
            // A project whose copy folder is gone entirely looks like a plain project on the
            // file system; the root layer's record of the chosen environment tells the two
            // apart (#628). A store without the record — a plain project, or one created
            // before the record existed — keeps the warning-free plain behaviour.
            if let Some(recorded) = crate::project::recorded_project_environment(store)? {
                let version = recorded
                    .version
                    .as_ref()
                    .map(|v| format!(" version {v}"))
                    .unwrap_or_default();
                environment_lost = Some(EnvironmentCopyLost {
                    path: store
                        .join(crate::project::PROJECT_ENVIRONMENT_DIR)
                        .join(session::environment::ENVIRONMENT_FILE)
                        .to_string_lossy()
                        .into_owned(),
                    message: format!(
                        "the project was created with the environment {}{}, whose copy is no longer in the project; profile, fastener and material names resolve from the firm libraries and the built-in neutral environment, not the environment the project was created with",
                        recorded.id, version
                    ),
                });
            }
        }
    }
    let firm = load_firm(config_dir);
    let firm_refused = firm.refused;
    sources.extend(firm.sources);
    fastener_sources.extend(firm.fastener_sources);
    material_sources.extend(firm.material_sources);
    append_neutral(&mut sources, &mut fastener_sources, &mut material_sources)?;
    let fastener_lookup = (!fastener_sources.is_empty()).then(|| Lookup::new(fastener_sources));
    let material_lookup = (!material_sources.is_empty()).then(|| Lookup::new(material_sources));
    Ok(ProjectChain {
        lookup: Lookup::new(sources),
        fastener_lookup,
        material_lookup,
        environment,
        environment_lost,
        firm_refused,
    })
}

/// What answered the name of a creation command, and what was shadowed or lost under it.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    /// The complete definition the command pins; `digest` is the digest of its canonical layer.
    pub record: DefinitionRecord,
    /// The digest of `record`.
    pub digest: ContentDigest,
    /// The layer that answered: [`LAYER_MODEL`], or the layer of the answering library.
    pub layer: &'static str,
    /// The library that answered; `None` for the model layer.
    pub library: Option<LibraryInfo>,
    /// The weaker libraries of the chain that declare the same name, strongest first; empty for
    /// the model layer, which is consulted before any library.
    pub shadowed: Vec<Shadowed>,
    /// The project's lost environment, when the chain that answered reports one (#628).
    pub environment_lost: Option<EnvironmentCopyLost>,
}

/// What the name of a creation command selected.
#[derive(Debug, Clone, PartialEq)]
pub enum Resolution {
    /// Exactly one definition answers. The record is boxed: it holds the whole definition and
    /// dwarfs the other two outcomes, which a command only matches on.
    Found(Box<Resolved>),
    /// No layer declares the name; nothing is substituted.
    NotFound,
    /// The model holds more than one definition of the name; none is chosen. Every candidate
    /// travels with the refusal, so a command can name the digests and its digest option.
    Ambiguous(Vec<(ContentDigest, DefinitionRecord)>),
}

/// Resolves the name of a creation command in one `namespace`, strongest layer first (#628): the
/// definitions of the model ([`LAYER_MODEL`]), then the libraries of the chain `chain` builds —
/// project, firm, neutral. Profiles, fasteners and materials take the same walk.
///
/// `model` is the model's definitions by digest, or `None` when the model is not consulted: a
/// repin follows the current library entry, and a library file named on the command line
/// answers alone. The chain is built only when the model has no answer, so a name the model
/// defines needs no library file: a project whose environment copy is lost or cannot be read
/// still places every definition it pinned. The model layer therefore reports neither shadows
/// nor a lost environment; nothing was substituted under it.
///
/// * One definition of the model has the name: it answers.
/// * More than one has it — two generations of one designation, since definitions are
///   content-addressed and nothing makes a designation unique in a model: [`Resolution::Ambiguous`]
///   with every candidate. A definition is never chosen for the user.
/// * No layer declares the name: [`Resolution::NotFound`].
/// * Otherwise the first library that declares the name answers ([`Lookup::find`]), with the
///   shadow records of that name and the lost environment of the chain.
pub fn resolve(
    model: Option<&BTreeMap<ContentDigest, DefinitionRecord>>,
    chain: impl FnOnce() -> Result<ProjectChain, Refusal>,
    namespace: Namespace,
    name: &str,
) -> Result<Resolution, Refusal> {
    if let Some(model) = model {
        let found: Vec<(&ContentDigest, &DefinitionRecord)> = model
            .iter()
            .filter(|(_, record)| record.namespace() == namespace)
            .filter(|(_, record)| record.selectable_names().contains(&name))
            .collect();
        if let [(digest, record)] = found.as_slice() {
            return Ok(Resolution::Found(Box::new(Resolved {
                record: (*record).clone(),
                digest: **digest,
                layer: LAYER_MODEL,
                library: None,
                shadowed: Vec::new(),
                environment_lost: None,
            })));
        }
        if !found.is_empty() {
            return Ok(Resolution::Ambiguous(
                found
                    .into_iter()
                    .map(|(digest, record)| (*digest, record.clone()))
                    .collect(),
            ));
        }
    }
    let chain = chain()?;
    let Some(lookup) = chain.lookup_of(namespace) else {
        return Ok(Resolution::NotFound);
    };
    let (source, entry) = match lookup.find(name) {
        Ok(found) => found,
        Err(error) if error.code == ErrorCode::NotFound => return Ok(Resolution::NotFound),
        Err(error) => return Err(error),
    };
    Ok(Resolution::Found(Box::new(Resolved {
        digest: entry.record.resource()?.digest,
        record: entry.record.clone(),
        layer: source.layer,
        library: Some(source.library.info.clone()),
        shadowed: lookup.shadowed_for(name),
        environment_lost: chain.environment_lost.clone(),
    })))
}

/// The identity of a library that [`add_entry`] creates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewLibrary {
    /// `bim:library:id`.
    pub id: String,
    /// `bim:library:title`, when given.
    pub title: Option<String>,
}

/// The result of [`add_entry`].
#[derive(Debug, Clone, PartialEq)]
pub struct Added {
    /// Whether the library file was created by this call.
    pub created: bool,
    /// The library as read back after the write.
    pub info: LibraryInfo,
    /// The entry that was added, as read back.
    pub entry: LibraryEntry,
    /// Entries in the library after the write.
    pub entry_count: usize,
}

fn usda_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn points_text(points: &[[f64; 2]]) -> String {
    let items: Vec<String> = points
        .iter()
        .map(|[x, y]| format!("({x}, {y}, 0)"))
        .collect();
    format!("[{}]", items.join(", "))
}

/// The attribute lines of one entry; the inverse of [`decode_record`]. Fastener entries are not
/// written here: a fastener library is shipped or maintained by hand, and the shipped files are
/// the provenance record of their values (A14.1).
fn entry_body(designation: &str, record: &DefinitionRecord) -> Result<String, Refusal> {
    let mut lines = vec![format!(
        "string steel:profile:designation = {}",
        usda_string(designation)
    )];
    let section = match record {
        DefinitionRecord::RectangleProfile {
            width_mm,
            height_mm,
            section,
        } => {
            lines.push("token steel:profile:family = \"rectangle\"".into());
            lines.push(format!("double steel:profile:hMm = {height_mm}"));
            lines.push(format!("double steel:profile:bMm = {width_mm}"));
            section
        }
        DefinitionRecord::SectionProfile {
            shape: SectionShape::Parametric(parametric),
            section,
        } => {
            lines.push(format!(
                "token steel:profile:family = {}",
                usda_string(parametric.family().token())
            ));
            for (name, value) in parametric
                .family()
                .parameters()
                .iter()
                .zip(parametric.values())
            {
                lines.push(format!("double {} = {value}", profile_attribute(name)));
            }
            section
        }
        DefinitionRecord::SectionProfile {
            shape: SectionShape::Custom(custom),
            section,
        } => {
            lines.push("token steel:profile:family = \"custom\"".into());
            lines.push(format!(
                "double3[] steel:profile:outerPointsMm = {}",
                points_text(&custom.outer)
            ));
            if !custom.cavities.is_empty() {
                let all: Vec<[f64; 2]> = custom.cavities.iter().flatten().copied().collect();
                let counts: Vec<String> = custom
                    .cavities
                    .iter()
                    .map(|c| c.len().to_string())
                    .collect();
                lines.push(format!(
                    "double3[] steel:profile:cavityPointsMm = {}",
                    points_text(&all)
                ));
                lines.push(format!(
                    "int[] steel:profile:cavityVertexCounts = [{}]",
                    counts.join(", ")
                ));
            }
            section
        }
        DefinitionRecord::Material { .. } => {
            return Err(invalid("a library holds profiles, not materials"));
        }
        DefinitionRecord::Fastener { .. } => {
            return Err(invalid("a profile library holds profiles, not fasteners"));
        }
    };
    if let Some(standard) = &section.standard {
        lines.push(format!(
            "string steel:profile:standard = {}",
            usda_string(standard)
        ));
    }
    for (name, value) in [
        ("areaMm2", section.area_mm2),
        ("massPerMeterKg", section.mass_per_meter_kg),
        ("surfacePerMeterM2", section.surface_per_meter_m2),
    ] {
        if let Some(value) = value {
            lines.push(format!("double steel:profile:{name} = {value}"));
        }
    }
    // The declared-area convention stores only when it is not the default: an absent
    // attribute reads back as the exact convention.
    if section.area_convention == AreaConvention::NominalSharp {
        lines.push(format!(
            "string steel:profile:areaConvention = {}",
            usda_string(section.area_convention.token())
        ));
    }
    Ok(lines
        .iter()
        .map(|line| format!("            {line}\n"))
        .collect())
}

/// The USDA text of a library: the declared identity and every entry, in designation order.
fn render_library(
    info: &LibraryInfo,
    entries: &[(String, &LibraryEntry)],
) -> Result<String, Refusal> {
    let mut text = String::from(
        "#usda 1.0\n(\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n    customLayerData = {\n",
    );
    for (key, value) in [
        ("id", Some(&info.id)),
        ("title", info.title.as_ref()),
        ("version", info.version.as_ref()),
        ("source", info.source.as_ref()),
        ("licence", info.licence.as_ref()),
    ] {
        if let Some(value) = value {
            text.push_str(&format!(
                "        string \"bim:library:{key}\" = {}\n",
                usda_string(value)
            ));
        }
    }
    text.push_str("    }\n)\n\ndef Scope \"Library\"\n{\n    def Scope \"Profiles\"\n    {\n");
    for (index, (name, entry)) in entries.iter().enumerate() {
        if index > 0 {
            text.push('\n');
        }
        text.push_str(&format!("        def Scope \"{name}\"\n        {{\n"));
        text.push_str(&entry_body(&entry.designation, &entry.record)?);
        text.push_str("        }\n");
    }
    text.push_str("    }\n}\n");
    Ok(text)
}

/// A USD identifier for the prim of a new entry: the designation with every character outside
/// `[A-Za-z0-9_]` replaced, made unique among `used`.
fn prim_name_for(designation: &str, used: &BTreeSet<String>) -> String {
    let mut base: String = designation
        .chars()
        .take(96)
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if base.is_empty() || base.starts_with(|c: char| c.is_ascii_digit()) {
        base.insert_str(0, "E_");
    }
    let mut name = base.clone();
    let mut n = 2usize;
    while used.contains(&name) {
        name = format!("{base}_{n}");
        n += 1;
    }
    name
}

fn write_failed(_: std::io::Error) -> Refusal {
    refuse(
        ErrorCode::BackendUnavailable,
        "the library could not be written",
    )
}

/// Writes `text` next to `path` and installs it only when complete: a new file is linked into
/// place (it never replaces a file that appeared meanwhile), an existing one is renamed over.
/// Not safe against two writers of the same file at once.
fn install(path: &str, text: &str, replace: bool) -> Result<(), Refusal> {
    use std::io::Write;
    let target = std::path::Path::new(path);
    let name = target
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| invalid("the library path has no file name"))?;
    let temporary = target.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    // The replacement keeps the permissions of the file it replaces (a private 0600 library must
    // not become readable by others). They are set while the temporary file is still empty.
    let permissions = if replace {
        Some(
            std::fs::metadata(target)
                .map_err(write_failed)?
                .permissions(),
        )
    } else {
        None
    };
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(write_failed)?;
    let written = permissions
        .map_or(Ok(()), |p| file.set_permissions(p))
        .and_then(|()| file.write_all(text.as_bytes()))
        .and_then(|()| file.sync_all());
    drop(file);
    let installed = written.and_then(|()| {
        if replace {
            std::fs::rename(&temporary, target)
        } else {
            std::fs::hard_link(&temporary, target)
        }
    });
    if !replace || installed.is_err() {
        // A failed rename leaves the temporary file; a hard link leaves it beside the target.
        let _ = std::fs::remove_file(&temporary);
    }
    installed.map_err(write_failed)
}

/// Adds one profile entry to the library file at `path`, creating the file when `new` is given.
///
/// `record` is a profile definition; its designation and display name become `designation`.
/// An existing file is loaded with [`load`] first and refused when it holds refused entries
/// (a rewrite would lose them), when the designation is already in it (`INVALID_ARGUMENT`), or
/// when `new` is given for a file that exists. The whole library is rendered again from what the
/// loader understood, so comments, `doc` text and attributes it does not read are not kept; the
/// rendered text is read back with [`load_bytes`] and must give exactly the expected entries
/// before anything is written (`INTEGRITY_ERROR` otherwise). The file is replaced atomically.
pub fn add_entry(
    path: &str,
    mut record: DefinitionRecord,
    designation: &str,
    new: Option<NewLibrary>,
) -> Result<Added, Refusal> {
    check_source_path(path).map_err(command)?;
    if std::path::Path::new(path)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("usdc"))
    {
        return Err(invalid(
            "add-library writes USDA text and cannot write a .usdc library",
        ));
    }
    if !label_is_valid(designation) {
        return Err(invalid("the designation is not a valid label"));
    }
    match &mut record {
        DefinitionRecord::RectangleProfile { section, .. }
        | DefinitionRecord::SectionProfile { section, .. } => {
            section.designation = Some(designation.to_owned());
            section.display_name = Some(designation.to_owned());
        }
        DefinitionRecord::Material { .. } => {
            return Err(invalid("a library holds profiles, not materials"));
        }
        DefinitionRecord::Fastener { .. } => {
            return Err(invalid(
                "add-library writes profile entries; a fastener library is shipped or maintained by hand",
            ));
        }
    }
    record.validate().map_err(command)?;
    let (info, existing, created) = match load(path) {
        Ok(library) => {
            if new.is_some() {
                return Err(invalid(
                    "the library exists; --library-id and --title only create one",
                ));
            }
            if library.info.binary {
                return Err(invalid(
                    "the library is binary USD; add-library rewrites USDA text only",
                ));
            }
            if !library.rejected.is_empty() {
                return Err(invalid(
                    "the library has refused entries that a rewrite would lose",
                ));
            }
            (library.info, library.entries, false)
        }
        Err(error) if error.code == ErrorCode::NotFound => {
            let new = new.ok_or(error)?;
            if !label_is_valid(&new.id) || new.title.as_deref().is_some_and(|t| !label_is_valid(t))
            {
                return Err(invalid("a library id or title is not a valid label"));
            }
            let info = LibraryInfo {
                id: new.id,
                title: new.title,
                version: None,
                source: None,
                licence: None,
                digest: digest_of(b"")?,
                bytes: 0,
                binary: false,
            };
            (info, Vec::new(), true)
        }
        Err(error) => return Err(error),
    };
    if existing.iter().any(|e| e.designation == designation) {
        return Err(invalid("the designation is already in the library"));
    }
    if existing.len() >= MAX_LIBRARY_ENTRIES {
        return Err(refuse(
            ErrorCode::LimitExceeded,
            "the library has more than 4096 entries",
        ));
    }
    let mut used: BTreeSet<String> = existing
        .iter()
        .filter_map(|e| {
            e.prim_path
                .rsplit_once('/')
                .map(|(_, name)| name.to_owned())
        })
        .collect();
    let name = prim_name_for(designation, &used);
    used.insert(name.clone());
    let added = LibraryEntry {
        prim_path: format!("{ENTRY_PARENT}/{name}"),
        designation: designation.to_owned(),
        record,
    };
    let mut all: Vec<&LibraryEntry> = existing.iter().chain(std::iter::once(&added)).collect();
    all.sort_by(|a, b| a.designation.cmp(&b.designation));
    let named: Vec<(String, &LibraryEntry)> = all
        .iter()
        .filter_map(|e| {
            e.prim_path
                .rsplit_once('/')
                .map(|(_, name)| (name.to_owned(), *e))
        })
        .collect();
    let text = render_library(&info, &named)?;
    if text.len() > MAX_LIBRARY_BYTES {
        return Err(io_error(ErrorCode::LimitExceeded));
    }
    let read_back = load_bytes(text.clone().into_bytes())?;
    let faithful =
        read_back.rejected.is_empty()
            && read_back.info.id == info.id
            && read_back.info.title == info.title
            && read_back.info.version == info.version
            && read_back.info.source == info.source
            && read_back.info.licence == info.licence
            && read_back.entries.len() == all.len()
            && read_back.entries.iter().zip(&all).all(|(got, want)| {
                got.designation == want.designation && got.record == want.record
            });
    if !faithful {
        return Err(refuse(
            ErrorCode::IntegrityError,
            "the rewritten library does not read back as intended; nothing was written",
        ));
    }
    install(path, &text, !created)?;
    let entry = read_back
        .entries
        .iter()
        .find(|e| e.designation == designation)
        .cloned()
        .ok_or_else(|| {
            refuse(
                ErrorCode::IntegrityError,
                "the added entry was not read back",
            )
        })?;
    Ok(Added {
        created,
        entry_count: read_back.entries.len(),
        info: read_back.info,
        entry,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    const HEADER: &str = "#usda 1.0\n(\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n    customLayerData = {\n        string \"bim:library:id\" = \"t\"\n    }\n)\n";

    fn entry(name: &str, designation: &str, body: &str) -> String {
        format!(
            "        def Scope \"{name}\" {{\n            string steel:profile:designation = \"{designation}\"\n{body}        }}\n"
        )
    }

    fn library(entries: &[String]) -> String {
        format!(
            "{HEADER}def Scope \"Library\" {{\n    def Scope \"Profiles\" {{\n{}    }}\n}}\n",
            entries.concat()
        )
    }

    const I_BODY: &str = "            token steel:profile:family = \"i_shape\"\n            double steel:profile:hMm = 300\n            double steel:profile:bMm = 150\n            double steel:profile:twMm = 7\n            double steel:profile:tfMm = 10\n            double steel:profile:rMm = 15\n";

    fn load_text(_name: &str, text: &str) -> Result<Library, Refusal> {
        load_bytes(text.as_bytes().to_vec())
    }

    fn fastener_entry(name: &str, designation: &str, body: &str) -> String {
        format!(
            "        def Scope \"{name}\" {{\n            string steel:fastener:designation = \"{designation}\"\n{body}        }}\n"
        )
    }

    fn fastener_library(entries: &[String]) -> String {
        format!(
            "{HEADER}def Scope \"Library\" {{\n    def Scope \"Fasteners\" {{\n{}    }}\n}}\n",
            entries.concat()
        )
    }

    const BOLT_BODY: &str = "            token steel:fastener:kind = \"bolt\"\n            token steel:fastener:headKind = \"hex\"\n            double steel:fastener:nominalDiameterMm = 12\n            double steel:fastener:headAcrossFlatsMm = 18\n            double steel:fastener:headHeightMm = 7.5\n            double steel:fastener:nutAcrossFlatsMm = 18\n            double steel:fastener:nutHeightMm = 10.8\n            double steel:fastener:holeClearanceMm = 1\n            double[] steel:fastener:lengthsMm = [30, 40, 50]\n            double steel:fastener:threadProtrusionMm = 4\n            bool steel:fastener:washerUnderHead = false\n            bool steel:fastener:washerUnderNut = false\n            token steel:fastener:holeMode = \"generate\"\n";

    #[test]
    fn a_fastener_entry_resolves_to_a_complete_named_definition() {
        let with_washer = format!(
            "{BOLT_BODY}            double steel:fastener:washerOuterDiameterMm = 24\n            double steel:fastener:washerInnerDiameterMm = 13\n            double steel:fastener:washerThicknessMm = 2.5\n            string steel:fastener:standard = \"Synthetic\"\n"
        );
        let text = fastener_library(&[fastener_entry("A", "SYN-M12", &with_washer)]);
        let loaded = load_fastener_bytes(text.clone().into_bytes()).unwrap();
        assert_eq!(loaded.info.id, "t");
        assert_eq!(loaded.entries.len(), 1);
        let found = loaded.entry("SYN-M12").unwrap();
        assert_eq!(found.record.display_name(), Some("SYN-M12"));
        let DefinitionRecord::Fastener { spec } = &found.record else {
            panic!("the entry is not a fastener definition");
        };
        assert_eq!(spec.kind, FastenerKind::Bolt);
        assert_eq!(spec.nominal_diameter_mm, 12.0);
        assert_eq!(spec.lengths_mm, [30.0, 40.0, 50.0]);
        assert_eq!(
            spec.washer.as_ref().map(|w| w.inner_diameter_mm),
            Some(13.0)
        );
        assert_eq!(spec.standard.as_deref(), Some("Synthetic"));
        // A profile loader never sees a fastener entry and the other way round.
        assert_eq!(
            load_text("p.usda", &text).unwrap().entries.len(),
            0,
            "the fastener entry is not a profile entry"
        );
        assert_eq!(
            loaded.entry("SYN-M13").unwrap_err().code,
            ErrorCode::NotFound
        );
    }

    #[test]
    fn a_bad_fastener_entry_is_reported_and_the_others_load() {
        // A bolt without its nut height is refused by the definition's own rules.
        let nutless = BOLT_BODY.replace("double steel:fastener:nutHeightMm = 10.8\n", "");
        let text = fastener_library(&[
            fastener_entry("A", "SYN-M12", BOLT_BODY),
            fastener_entry("B", "SYN-BAD", &nutless),
        ]);
        let loaded = load_fastener_bytes(text.into_bytes()).unwrap();
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.rejected.len(), 1);
        assert_eq!(loaded.rejected[0].designation.as_deref(), Some("SYN-BAD"));
        assert_eq!(
            loaded.entry("SYN-BAD").unwrap_err().code,
            ErrorCode::InvalidArgument,
            "a refused designation reports its refusal, never a near match"
        );
    }

    #[test]
    fn a_fastener_record_is_not_written_by_add_entry() {
        let text = fastener_library(&[fastener_entry("A", "SYN-M12", BOLT_BODY)]);
        let loaded = load_fastener_bytes(text.into_bytes()).unwrap();
        let record = loaded.entry("SYN-M12").unwrap().record.clone();
        let error = add_entry("/tmp/a14-unwritable.usda", record, "SYN-M12", None).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidArgument);
    }

    fn material_entry(name: &str, grade: &str, body: &str) -> String {
        format!(
            "        def Scope \"{name}\" {{\n            string steel:material:grade = \"{grade}\"\n{body}        }}\n"
        )
    }

    fn material_library(entries: &[String]) -> String {
        format!(
            "{HEADER}def Scope \"Library\" {{\n    def Scope \"Materials\" {{\n{}    }}\n}}\n",
            entries.concat()
        )
    }

    const MATERIAL_BODY: &str = "            double steel:material:densityKgM3 = 7850\n            string steel:material:standard = \"EN 10025-2\"\n";

    #[test]
    fn a_material_entry_resolves_to_a_complete_named_definition() {
        let named = format!(
            "{MATERIAL_BODY}            string steel:material:displayName = \"Structural steel S355\"\n"
        );
        let text = material_library(&[
            material_entry("A", "S235JR", MATERIAL_BODY),
            material_entry("B", "S355J2", &named),
        ]);
        let loaded = load_material_bytes(text.clone().into_bytes()).unwrap();
        assert_eq!(loaded.info.id, "t");
        assert_eq!(loaded.entries.len(), 2);
        let plain = loaded.entry("S235JR").unwrap();
        let DefinitionRecord::Material {
            density_kg_m3,
            grade,
            standard,
            display_name,
        } = &plain.record
        else {
            panic!("the entry is not a material definition");
        };
        assert_eq!(*density_kg_m3, Some(7850.0));
        assert_eq!(grade.as_deref(), Some("S235JR"));
        assert_eq!(standard.as_deref(), Some("EN 10025-2"));
        // Without a declared display name the entry answers under its grade.
        assert_eq!(display_name.as_deref(), Some("S235JR"));
        let overridden = loaded.entry("S355J2").unwrap();
        let DefinitionRecord::Material { display_name, .. } = &overridden.record else {
            panic!("the entry is not a material definition");
        };
        assert_eq!(display_name.as_deref(), Some("Structural steel S355"));
        // A profile loader never sees a material entry and the other way round.
        assert_eq!(
            load_text("p.usda", &text).unwrap().entries.len(),
            0,
            "the material entry is not a profile entry"
        );
        assert_eq!(
            loaded.entry("S355").unwrap_err().code,
            ErrorCode::NotFound,
            "the grade is exact, never a near match"
        );
    }

    #[test]
    fn a_bad_material_entry_is_reported_and_the_others_load() {
        // A grade without its density is refused; so is a density the definition's own rules
        // refuse. Nothing is inferred or substituted.
        let dense = format!("{MATERIAL_BODY}            double steel:material:densityKgM3 = -1\n");
        let text = material_library(&[
            material_entry("A", "S235JR", MATERIAL_BODY),
            material_entry(
                "B",
                "NO-DENSITY",
                "            string steel:material:standard = \"X\"\n",
            ),
            material_entry("C", "NEGATIVE", &dense),
        ]);
        let loaded = load_material_bytes(text.into_bytes()).unwrap();
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.rejected.len(), 2);
        assert_eq!(
            loaded
                .rejected
                .iter()
                .map(|r| r.designation.as_deref())
                .collect::<Vec<_>>(),
            [Some("NO-DENSITY"), Some("NEGATIVE")]
        );
        assert_eq!(
            loaded.entry("NO-DENSITY").unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            loaded.entry("NEGATIVE").unwrap_err().code,
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn an_entry_resolves_to_a_complete_named_definition() {
        let text = library(&[entry("A", "T-I300", I_BODY)]);
        let loaded = load_text("ok.usda", &text).unwrap();
        assert_eq!(loaded.info.id, "t");
        assert_eq!(loaded.entries.len(), 1);
        let found = loaded.entry("T-I300").unwrap();
        assert_eq!(found.record.display_name(), Some("T-I300"));
        assert!(matches!(
            &found.record,
            DefinitionRecord::SectionProfile { shape: SectionShape::Parametric(p), .. }
                if p.values() == [300.0, 150.0, 7.0, 10.0, 15.0]
        ));
        assert_eq!(
            loaded.entry("T-I301").unwrap_err().code,
            ErrorCode::NotFound
        );
    }

    #[test]
    fn a_duplicate_designation_refuses_the_whole_library() {
        let text = library(&[entry("A", "T-I300", I_BODY), entry("B", "T-I300", I_BODY)]);
        let error = load_text("dup.usda", &text).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidArgument);
    }

    #[test]
    fn a_bad_entry_is_reported_and_the_others_load() {
        let bad = "            token steel:profile:family = \"i_shape\"\n            double steel:profile:hMm = 300\n";
        let text = library(&[entry("A", "T-I300", I_BODY), entry("B", "T-BAD", bad)]);
        let loaded = load_text("bad.usda", &text).unwrap();
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.rejected.len(), 1);
        assert_eq!(loaded.rejected[0].designation.as_deref(), Some("T-BAD"));
        assert_eq!(
            loaded.entry("T-BAD").unwrap_err().code,
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn a_declared_area_that_contradicts_the_outline_loads_and_warns() {
        let wrong = format!("{I_BODY}            double steel:profile:areaMm2 = 5000\n");
        let text = library(&[entry("A", "T-AREA", &wrong)]);
        let loaded = load_text("area.usda", &text).unwrap();
        assert!(loaded.rejected.is_empty());
        let found = loaded.entry("T-AREA").unwrap();
        assert!(matches!(
            &found.record,
            DefinitionRecord::SectionProfile { section, .. } if section.area_mm2 == Some(5000.0)
        ));
        let warning = found.record.declared_area_warning().unwrap();
        assert_eq!(warning.declared_mm2, 5000.0);
        assert!(warning.relative_difference > 0.01);
    }

    fn source(path: &str, id: &str, entries: &[(&str, &str)]) -> Source {
        let entries: Vec<String> = entries
            .iter()
            .map(|(prim, designation)| entry(prim, designation, I_BODY))
            .collect();
        let text = library(&entries).replace("\"t\"", &format!("\"{id}\""));
        Source {
            path: path.to_owned(),
            library: load_text(path, &text).unwrap(),
            layer: LAYER_LIBRARY,
        }
    }

    #[test]
    fn the_first_library_that_declares_a_designation_answers_and_the_rest_is_shadowed() {
        let lookup = Lookup::new(vec![
            source("a", "strong", &[("P", "X-1"), ("Q", "A-ONLY")]),
            source("b", "weak", &[("P", "X-1"), ("R", "B-ONLY")]),
        ]);
        assert_eq!(lookup.find("X-1").unwrap().0.library.info.id, "strong");
        assert_eq!(lookup.find("B-ONLY").unwrap().0.library.info.id, "weak");
        assert_eq!(lookup.find("NOPE").unwrap_err().code, ErrorCode::NotFound);
        let names: Vec<&str> = lookup
            .effective()
            .iter()
            .map(|(_, e)| e.designation.as_str())
            .collect();
        assert_eq!(names, ["A-ONLY", "B-ONLY", "X-1"]);
        assert_eq!(
            lookup.shadowed(),
            [Shadowed {
                designation: "X-1".into(),
                used_library: "strong".into(),
                used_layer: LAYER_LIBRARY,
                shadowed_library: "weak".into(),
                shadowed_layer: LAYER_LIBRARY,
            }]
        );
    }

    #[test]
    fn a_refused_entry_hides_the_weaker_entry_of_the_same_designation() {
        let bad = "            token steel:profile:family = \"i_shape\"\n";
        let text = library(&[entry("P", "X-1", bad)]).replace("\"t\"", "\"strong\"");
        let strong = Source {
            path: "a".into(),
            library: load_text("a", &text).unwrap(),
            layer: LAYER_LIBRARY,
        };
        let lookup = Lookup::new(vec![strong, source("b", "weak", &[("P", "X-1")])]);
        assert_eq!(
            lookup.find("X-1").unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        assert!(lookup.effective().is_empty());
        assert_eq!(lookup.shadowed().len(), 1);
    }

    #[test]
    fn namespace_tokens_round_trip_and_refuse_any_other_text() {
        let tokens: Vec<&str> = Namespace::ALL
            .iter()
            .map(|namespace| namespace.token())
            .collect();
        assert_eq!(tokens, ["profile", "fastener", "material"], "listing order");
        for namespace in Namespace::ALL {
            assert_eq!(Namespace::parse(namespace.token()), Some(namespace));
        }
        // `all` names a listing, a token is exact: neither names a namespace.
        for other in ["all", "Profile", "bolt", "", "profile "] {
            assert_eq!(Namespace::parse(other), None, "{other:?}");
        }
    }

    #[test]
    fn lookup_of_hands_every_namespace_its_own_resolver() {
        let mut chain = ProjectChain {
            lookup: Lookup::new(vec![source("a", "profiles", &[("P", "X-1")])]),
            fastener_lookup: None,
            material_lookup: None,
            environment: None,
            environment_lost: None,
            firm_refused: Vec::new(),
        };
        assert_eq!(
            chain
                .lookup_of(Namespace::Profile)
                .map(|lookup| lookup.find("X-1").is_ok()),
            Some(true),
            "the profile resolver is always there"
        );
        assert!(chain.lookup_of(Namespace::Fastener).is_none());
        assert!(chain.lookup_of(Namespace::Material).is_none());
        chain.fastener_lookup = Some(Lookup::new(vec![source("b", "bolts", &[("P", "X-1")])]));
        // The same designation in two namespaces is no shadow: every resolver answers it from
        // its own library and reports none.
        for (namespace, library) in [
            (Namespace::Profile, "profiles"),
            (Namespace::Fastener, "bolts"),
        ] {
            let lookup = chain.lookup_of(namespace).unwrap();
            assert_eq!(lookup.find("X-1").unwrap().0.library.info.id, library);
            assert!(lookup.shadowed().is_empty(), "{namespace:?}");
        }
        assert!(chain.lookup_of(Namespace::Material).is_none());
    }

    #[test]
    fn a_creation_name_resolves_through_the_model_then_the_chain_in_its_own_namespace() {
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
        let bolt = load_fastener_bytes(
            fastener_library(&[fastener_entry("A", "M12", BOLT_BODY)]).into_bytes(),
        )
        .unwrap()
        .entries[0]
            .record
            .clone();
        // Two generations of one designation: definitions are content-addressed, so a model
        // may hold several of one name.
        let records = [
            profile("A", 10.0),
            profile("A", 20.0),
            profile("B", 30.0),
            material(7850.0),
            material(7800.0),
            bolt,
        ];
        let model: BTreeMap<_, _> = records
            .iter()
            .map(|r| (r.resource().unwrap().digest, r.clone()))
            .collect();
        let lost = EnvironmentCopyLost {
            path: "environment/environment.usda".into(),
            message: "the copy is lost".into(),
        };
        let chain = || {
            Ok(ProjectChain {
                lookup: Lookup::new(vec![
                    source("a", "strong", &[("P", "X-1")]),
                    source("b", "weak", &[("P", "X-1")]),
                ]),
                fastener_lookup: None,
                material_lookup: None,
                environment: None,
                environment_lost: Some(lost.clone()),
                firm_refused: Vec::new(),
            })
        };
        let answers = |model, namespace, name| match resolve(model, chain, namespace, name).unwrap()
        {
            Resolution::Found(found) => found,
            other => panic!("{name:?} did not resolve: {other:?}"),
        };
        let digest_of = |record: &DefinitionRecord| record.resource().unwrap().digest;
        // The candidates travel in the model's digest order.
        let mut generations: Vec<ContentDigest> = records[..2].iter().map(&digest_of).collect();
        generations.sort();

        // The model answers by its digest and the chain is never built, so it reports no
        // shadow and no lost environment: nothing was substituted.
        let pinned = resolve(
            Some(&model),
            || -> Result<ProjectChain, Refusal> {
                panic!("the chain was built although the model answers")
            },
            Namespace::Profile,
            "B",
        )
        .unwrap();
        let Resolution::Found(pinned) = pinned else {
            panic!("one profile is named B");
        };
        assert_eq!(pinned.layer, LAYER_MODEL);
        assert_eq!(pinned.digest, digest_of(&records[2]));
        assert!(pinned.library.is_none());
        assert!(pinned.shadowed.is_empty() && pinned.environment_lost.is_none());

        // Two generations of one name are an ambiguity with every candidate, never a choice;
        // a name selects by display name or catalogue label, exactly and inside its namespace.
        let Resolution::Ambiguous(candidates) =
            resolve(Some(&model), chain, Namespace::Profile, "A").unwrap()
        else {
            panic!("two profiles are named A");
        };
        assert_eq!(
            candidates
                .iter()
                .map(|(digest, _)| *digest)
                .collect::<Vec<_>>(),
            generations
        );
        assert!(matches!(
            resolve(Some(&model), chain, Namespace::Material, "S355").unwrap(),
            Resolution::Ambiguous(_)
        ));
        assert!(matches!(
            resolve(Some(&model), chain, Namespace::Profile, "b").unwrap(),
            Resolution::NotFound
        ));
        assert_eq!(
            answers(Some(&model), Namespace::Fastener, "M12").digest,
            digest_of(&records[5])
        );
        assert!(matches!(
            resolve(Some(&model), chain, Namespace::Profile, "M12").unwrap(),
            Resolution::NotFound
        ));

        // A name the model does not have falls through to the chain: the strongest library
        // answers, with the weaker entry of the same name and the chain's lost environment.
        let from_chain = answers(Some(&model), Namespace::Profile, "X-1");
        assert_eq!(from_chain.layer, LAYER_LIBRARY);
        assert_eq!(
            from_chain.library.as_ref().map(|l| l.id.as_str()),
            Some("strong")
        );
        assert_eq!(from_chain.shadowed.len(), 1);
        assert_eq!(from_chain.shadowed[0].shadowed_library, "weak");
        assert_eq!(from_chain.environment_lost, Some(lost.clone()));
        // Without the model the chain answers alone, and a namespace no layer declares, or a
        // name none declares, resolves nothing.
        assert_eq!(
            answers(None, Namespace::Profile, "X-1").layer,
            LAYER_LIBRARY
        );
        assert!(matches!(
            resolve(None, chain, Namespace::Profile, "B").unwrap(),
            Resolution::NotFound
        ));
        assert!(matches!(
            resolve(None, chain, Namespace::Material, "S355").unwrap(),
            Resolution::NotFound
        ));
    }

    #[test]
    fn the_search_folds_case_script_and_whitespace_and_stays_a_search() {
        for (designation, text, matches) in [
            ("IPE300", "ipe300", true),
            ("IPE300", "IPE 300", true),
            ("I PE 300", "ipe300", true),
            ("IPE300", "ipe30", true),
            ("IPE300", "ipe3000", false),
            ("IPE300", "ipe200", false),
            ("С345", "с345", true),
            ("С345", "С345", true),
            ("C345", "с345", false),
        ] {
            assert_eq!(
                designation_matches(designation, text),
                matches,
                "{designation:?} / {text:?}"
            );
        }
        let lookup = Lookup::new(vec![source(
            "a",
            "lib",
            &[("P", "IPE300"), ("Q", "IPE200"), ("R", "Ш20К1")],
        )]);
        let names = |text: &str| -> Vec<&str> {
            lookup
                .matching(Some(text))
                .into_iter()
                .map(|(_, e)| e.designation.as_str())
                .collect()
        };
        assert_eq!(names("ipe"), ["IPE200", "IPE300"]);
        assert_eq!(names("IPE 3"), ["IPE300"]);
        assert_eq!(names("ш20"), ["Ш20К1"]);
        assert!(
            names("ipe250").is_empty(),
            "a search never proposes a near match"
        );
        // Without a text every answering entry is listed, as `effective` lists them.
        assert_eq!(lookup.matching(None).len(), 3);
        // Search proposes; the exact lookup disposes, in the designation's own spelling.
        assert_eq!(lookup.find("ipe300").unwrap_err().code, ErrorCode::NotFound);
    }

    #[test]
    fn prim_names_are_identifiers_and_unique() {
        let mut used = BTreeSet::new();
        for (designation, expected) in [
            ("W310x52", "W310x52"),
            ("300x150 \"I\"", "E_300x150__I_"),
            ("", "E_"),
            ("W310x52", "W310x52_2"),
            ("W310x52", "W310x52_3"),
        ] {
            let name = prim_name_for(designation, &used);
            assert_eq!(name, expected);
            used.insert(name);
        }
    }

    #[test]
    fn a_rendered_library_reads_back_to_the_same_records() {
        let info = LibraryInfo {
            id: "r".into(),
            title: Some("T \"q\" \\".into()),
            version: Some("2".into()),
            source: None,
            licence: Some("MPL-2.0".into()),
            digest: digest_of(b"").unwrap(),
            bytes: 0,
            binary: false,
        };
        let text = library(&[
            entry("A", "A-1", I_BODY),
            entry(
                "B",
                "B-1",
                "            token steel:profile:family = \"rectangle\"\n            double steel:profile:hMm = 80\n            double steel:profile:bMm = 20\n            double steel:profile:areaMm2 = 1600\n",
            ),
        ]);
        let original = load_text("o", &text).unwrap();
        let named: Vec<(String, &LibraryEntry)> = original
            .entries
            .iter()
            .enumerate()
            .map(|(n, e)| (format!("E{n}"), e))
            .collect();
        let rendered = render_library(&info, &named).unwrap();
        let again = load_text("r", &rendered).unwrap();
        assert_eq!(again.info.title, info.title);
        assert_eq!(again.info.version, info.version);
        assert_eq!(again.info.licence, info.licence);
        assert_eq!(again.entries.len(), 2);
        for (got, want) in again.entries.iter().zip(&original.entries) {
            assert_eq!(got.record, want.record);
            assert_eq!(got.designation, want.designation);
        }
    }

    const UPN_BODY: &str = "            token steel:profile:family = \"tapered_c\"\n            double steel:profile:hMm = 100\n            double steel:profile:bMm = 50\n            double steel:profile:sMm = 6\n            double steel:profile:tMm = 8.5\n            double steel:profile:rMm = 8.5\n            double steel:profile:r1Mm = 4.5\n            double steel:profile:k = 0.08\n            double steel:profile:areaMm2 = 1350\n            double steel:profile:massPerMeterKg = 10.6\n";

    #[test]
    fn a_declared_area_convention_round_trips_and_is_admitted_by_token_only() {
        // The EN 10365 UPN convention: the nominal sharp declaration reads back as such, and
        // a rendered library carries the attribute so the round trip keeps it.
        let nominal = format!(
            "{UPN_BODY}            string steel:profile:areaConvention = \"nominal_sharp\"\n"
        );
        let original = load_text(
            "o",
            &library(&[
                entry("A", "UPN100", UPN_BODY),
                entry("B", "UPN200", &nominal),
            ]),
        )
        .unwrap();
        assert_eq!(original.entries.len(), 2);
        for (designation, convention) in [
            ("UPN100", AreaConvention::ExactFilleted),
            ("UPN200", AreaConvention::NominalSharp),
        ] {
            let found = original.entry(designation).unwrap();
            let DefinitionRecord::SectionProfile { section, .. } = &found.record else {
                panic!("{designation} is not a section profile");
            };
            assert_eq!(section.area_convention, convention, "{designation}");
        }
        let named: Vec<(String, &LibraryEntry)> = original
            .entries
            .iter()
            .enumerate()
            .map(|(n, e)| (format!("E{n}"), e))
            .collect();
        let info = LibraryInfo {
            id: "r".into(),
            title: None,
            version: None,
            source: None,
            licence: Some("MPL-2.0".into()),
            digest: digest_of(b"").unwrap(),
            bytes: 0,
            binary: false,
        };
        let again = load_text("r", &render_library(&info, &named).unwrap()).unwrap();
        for (got, want) in again.entries.iter().zip(&original.entries) {
            assert_eq!(got.record, want.record);
        }
        // An unknown convention token refuses the entry; a rectangle or custom profile
        // declaring one has no reference and is refused as well.
        let unknown = format!(
            "{UPN_BODY}            string steel:profile:areaConvention = \"approximate\"\n"
        );
        let loaded = load_text("u", &library(&[entry("A", "UPNX", &unknown)])).unwrap();
        assert_eq!(loaded.rejected.len(), 1);
        let rectangle = "            token steel:profile:family = \"rectangle\"\n            double steel:profile:hMm = 80\n            double steel:profile:bMm = 20\n            string steel:profile:areaConvention = \"nominal_sharp\"\n";
        let loaded = load_text("x", &library(&[entry("A", "RECT", rectangle)])).unwrap();
        assert_eq!(loaded.rejected.len(), 1);
    }

    #[test]
    fn hostile_layers_are_refused_without_a_panic() {
        let no_id = "#usda 1.0\n(\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n)\n";
        assert_eq!(
            load_text("noid.usda", no_id).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            load_text("garbage.usda", "not usd at all")
                .unwrap_err()
                .code,
            ErrorCode::InvalidArgument
        );
        let sublayer = HEADER.replace(")\n", "    subLayers = [@other.usda@]\n)\n");
        assert!(load_text("sub.usda", &sublayer).is_err());
        let metres = HEADER.replace("0.001", "1");
        assert_eq!(
            load_text("units.usda", &metres).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
        assert_eq!(
            load("https://example.invalid/lib.usda").unwrap_err().code,
            ErrorCode::PermissionDenied
        );
    }

    #[test]
    fn the_shipped_europe_library_reads_with_every_entry_admitted() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/europe/profiles-en-10365.usda"
        );
        let library = load(path).unwrap();
        assert_eq!(library.info.id, "europe-en-10365-profiles");
        assert_eq!(library.info.version.as_deref(), Some("4"));
        assert!(
            library
                .info
                .licence
                .as_deref()
                .is_some_and(|l| !l.is_empty()),
            "the shipped library declares its licence"
        );
        assert!(
            library
                .info
                .source
                .as_deref()
                .is_some_and(|s| !s.is_empty()),
            "the shipped library declares its source"
        );
        // No transcription slips: every entry is admitted and none warns.
        assert_eq!(library.rejected, []);
        assert_eq!(library.entries.len(), 207);
        let mut designations = Vec::new();
        for entry in &library.entries {
            let DefinitionRecord::SectionProfile {
                shape: SectionShape::Parametric(parametric),
                section,
            } = &entry.record
            else {
                panic!("{} is not a parametric section", entry.designation);
            };
            // The UPE channels are the pack's parallel-flange channel series, the UPN channels
            // its taper-flange series, and the rest are I and H shapes.
            let want_family = if entry.designation.starts_with("UPE") {
                SectionFamily::Channel
            } else if entry.designation.starts_with("UPN") {
                SectionFamily::TaperedC
            } else {
                SectionFamily::IShape
            };
            assert_eq!(parametric.family(), want_family);
            assert_eq!(
                section.standard.as_deref(),
                Some("EN 10365:2017"),
                "{}",
                entry.designation
            );
            // The declared catalogue area matches its convention's reference (the D3 1 percent
            // warning never fires: like-for-like, EN 10365 publishes the UPN 50-300 areas as
            // the nominal sharp-cornered value and the 320-400 areas exact, and the EN tables
            // print no HL root radius, so the HL rows carry the radius reproducing the
            // published area) and the declared mass must be the area at the standard's
            // 7850 kg/m3 within its published rounding — except the HL, HLZ, HD and HP rows of
            // the #790 extension, which carry the designation nominal masses: the rounded
            // published areas track them within 2.2 percent (worst HP400x122 at 2.19 percent,
            // HD360x147 at 1.91, HL920x368 at 0.60), so those rows are bounded relatively.
            assert!(
                entry.record.declared_area_warning().is_none(),
                "{}",
                entry.designation
            );
            let area = section.area_mm2.unwrap();
            let mass = section.mass_per_meter_kg.unwrap();
            let mass_from_area = area * 7850.0 / 1.0e6;
            if entry.designation.starts_with("HL")
                || entry.designation.starts_with("HD")
                || entry.designation.starts_with("HP")
            {
                assert!(
                    (mass - mass_from_area).abs() / mass_from_area <= 0.025,
                    "{}: mass {mass} kg/m vs area-derived {mass_from_area}",
                    entry.designation
                );
            } else {
                assert!(
                    (mass - mass_from_area).abs() <= 0.55,
                    "{}: mass {mass} kg/m vs area-derived {mass_from_area}",
                    entry.designation
                );
            }
            // The convention declaration itself: nominal sharp exactly on the UPN 50-300 rows,
            // the exact default on the 320-400 rows and everywhere else.
            let want_convention = if entry.designation.starts_with("UPN") {
                let depth: f64 = entry.designation[3..].parse().unwrap();
                if depth <= 300.0 {
                    AreaConvention::NominalSharp
                } else {
                    AreaConvention::ExactFilleted
                }
            } else {
                AreaConvention::ExactFilleted
            };
            assert_eq!(
                section.area_convention, want_convention,
                "{}",
                entry.designation
            );
            designations.push(entry.designation.clone());
        }
        // The series as EN 10365 publishes it, in the loader's designation order: no IPE 250/280
        // and no HEA/HEB 250 (those designations do not exist; the sequence steps 240 to 260/270),
        // no UPE 250/260 (the channel sequence steps 240 to 270), and the UPN series runs
        // 50 to 400 with its own steps (320/350/380 after 300; no 270/330/360). The #790
        // extension adds the HEA/HEB 650-1000 rows and the HL, HLZ, HD and HP series; the
        // x-suffix rows sort lexically, so the deep HD and HP rows lead their groups and the
        // four HLZ rows close the HL one.
        assert_eq!(
            designations,
            [
                "HD260x114",
                "HD260x142",
                "HD260x172",
                "HD260x225",
                "HD260x299",
                "HD260x54.1",
                "HD260x68.2",
                "HD260x93.0",
                "HD320x127",
                "HD320x158",
                "HD320x198",
                "HD320x245",
                "HD320x300",
                "HD320x74.2",
                "HD320x97.6",
                "HD360x134",
                "HD360x147",
                "HD360x162",
                "HD360x179",
                "HD360x196",
                "HD400x1086",
                "HD400x187",
                "HD400x216",
                "HD400x237",
                "HD400x262",
                "HD400x287",
                "HD400x314",
                "HD400x347",
                "HD400x382",
                "HD400x421",
                "HD400x463",
                "HD400x509",
                "HD400x551",
                "HD400x592",
                "HD400x634",
                "HD400x677",
                "HD400x744",
                "HD400x818",
                "HD400x900",
                "HD400x990",
                "HEA100",
                "HEA1000",
                "HEA120",
                "HEA140",
                "HEA160",
                "HEA180",
                "HEA200",
                "HEA220",
                "HEA240",
                "HEA260",
                "HEA280",
                "HEA300",
                "HEA320",
                "HEA340",
                "HEA360",
                "HEA400",
                "HEA450",
                "HEA500",
                "HEA550",
                "HEA600",
                "HEA650",
                "HEA700",
                "HEA800",
                "HEA900",
                "HEB100",
                "HEB1000",
                "HEB120",
                "HEB140",
                "HEB160",
                "HEB180",
                "HEB200",
                "HEB220",
                "HEB240",
                "HEB260",
                "HEB280",
                "HEB300",
                "HEB320",
                "HEB340",
                "HEB360",
                "HEB400",
                "HEB450",
                "HEB500",
                "HEB550",
                "HEB600",
                "HEB650",
                "HEB700",
                "HEB800",
                "HEB900",
                "HL1000A",
                "HL1000AA",
                "HL1000B",
                "HL1000M",
                "HL1000x443",
                "HL1000x483",
                "HL1000x539",
                "HL1000x554",
                "HL1000x591",
                "HL1000x642",
                "HL1000x748",
                "HL1000x883",
                "HL1000x976",
                "HL1100A",
                "HL1100B",
                "HL1100M",
                "HL1100R",
                "HL1100x548",
                "HL1100x607",
                "HL920x1077",
                "HL920x1194",
                "HL920x1269",
                "HL920x1377",
                "HL920x344",
                "HL920x368",
                "HL920x390",
                "HL920x420",
                "HL920x449",
                "HL920x491",
                "HL920x537",
                "HL920x588",
                "HL920x656",
                "HL920x725",
                "HL920x787",
                "HL920x970",
                "HLZ1100A",
                "HLZ1100B",
                "HLZ1100C",
                "HLZ1100D",
                "HP200x43",
                "HP200x53",
                "HP220x57",
                "HP260x75",
                "HP260x87",
                "HP305x110",
                "HP305x126",
                "HP305x149",
                "HP305x180",
                "HP305x186",
                "HP305x223",
                "HP305x79",
                "HP305x88",
                "HP305x95",
                "HP320x103",
                "HP320x117",
                "HP320x147",
                "HP320x184",
                "HP320x88",
                "HP360x109",
                "HP360x133",
                "HP360x152",
                "HP360x174",
                "HP360x180",
                "HP400x122",
                "HP400x140",
                "HP400x158",
                "HP400x176",
                "HP400x194",
                "HP400x213",
                "HP400x231",
                "IPE100",
                "IPE120",
                "IPE140",
                "IPE160",
                "IPE180",
                "IPE200",
                "IPE220",
                "IPE240",
                "IPE270",
                "IPE300",
                "IPE330",
                "IPE360",
                "IPE400",
                "IPE450",
                "IPE500",
                "IPE550",
                "IPE600",
                "UPE100",
                "UPE120",
                "UPE140",
                "UPE160",
                "UPE180",
                "UPE200",
                "UPE220",
                "UPE240",
                "UPE270",
                "UPE300",
                "UPE330",
                "UPE360",
                "UPE400",
                "UPE80",
                "UPN100",
                "UPN120",
                "UPN140",
                "UPN160",
                "UPN180",
                "UPN200",
                "UPN220",
                "UPN240",
                "UPN260",
                "UPN280",
                "UPN300",
                "UPN320",
                "UPN350",
                "UPN380",
                "UPN400",
                "UPN50",
                "UPN65",
                "UPN80",
            ]
        );
        // Spot checks against the transcribed values.
        let ipe300 = library.entry("IPE300").unwrap();
        let DefinitionRecord::SectionProfile {
            shape: SectionShape::Parametric(p),
            section,
        } = &ipe300.record
        else {
            panic!("IPE300 is not a parametric section");
        };
        assert_eq!(p.values(), [300.0, 150.0, 7.1, 10.7, 15.0]);
        assert_eq!(section.area_mm2, Some(5381.0));
        assert_eq!(section.mass_per_meter_kg, Some(42.2));
        let hea340 = library.entry("HEA340").unwrap();
        let DefinitionRecord::SectionProfile {
            shape: SectionShape::Parametric(p),
            section,
        } = &hea340.record
        else {
            panic!("HEA340 is not a parametric section");
        };
        assert_eq!(p.values(), [330.0, 300.0, 9.5, 16.5, 27.0]);
        assert_eq!(section.area_mm2, Some(13350.0));
        assert_eq!(section.mass_per_meter_kg, Some(105.0));
        let heb100 = library.entry("HEB100").unwrap();
        let DefinitionRecord::SectionProfile {
            shape: SectionShape::Parametric(p),
            section,
        } = &heb100.record
        else {
            panic!("HEB100 is not a parametric section");
        };
        assert_eq!(p.values(), [100.0, 100.0, 6.0, 10.0, 12.0]);
        assert_eq!(section.area_mm2, Some(2600.0));
        assert_eq!(section.mass_per_meter_kg, Some(20.4));
        let upe100 = library.entry("UPE100").unwrap();
        let DefinitionRecord::SectionProfile {
            shape: SectionShape::Parametric(p),
            section,
        } = &upe100.record
        else {
            panic!("UPE100 is not a parametric section");
        };
        assert_eq!(p.values(), [100.0, 55.0, 4.5, 7.5, 10.0]);
        assert_eq!(section.area_mm2, Some(1250.0));
        assert_eq!(section.mass_per_meter_kg, Some(9.8));
        let upe300 = library.entry("UPE300").unwrap();
        let DefinitionRecord::SectionProfile {
            shape: SectionShape::Parametric(p),
            section,
        } = &upe300.record
        else {
            panic!("UPE300 is not a parametric section");
        };
        assert_eq!(p.values(), [300.0, 100.0, 9.5, 15.0, 15.0]);
        assert_eq!(section.area_mm2, Some(5660.0));
        assert_eq!(section.mass_per_meter_kg, Some(44.4));
        let upn100 = library.entry("UPN100").unwrap();
        let DefinitionRecord::SectionProfile {
            shape: SectionShape::Parametric(p),
            section,
        } = &upn100.record
        else {
            panic!("UPN100 is not a parametric section");
        };
        assert_eq!(p.values(), [100.0, 50.0, 6.0, 8.5, 8.5, 4.5, 0.08]);
        assert_eq!(section.area_mm2, Some(1350.0));
        assert_eq!(section.mass_per_meter_kg, Some(10.6));
        let upn400 = library.entry("UPN400").unwrap();
        let DefinitionRecord::SectionProfile {
            shape: SectionShape::Parametric(p),
            section,
        } = &upn400.record
        else {
            panic!("UPN400 is not a parametric section");
        };
        assert_eq!(p.values(), [400.0, 110.0, 14.0, 18.0, 18.0, 9.0, 0.05]);
        assert_eq!(section.area_mm2, Some(9150.0));
        assert_eq!(section.mass_per_meter_kg, Some(71.8));
        // One spot per family of the #790 extension, against the transcribed values: the
        // EMSTEEL-catalogue HL row, the largest-HD-gap row, the deepest HLZ row, the heaviest
        // HP row and the top of the extended HEA ladder.
        let hl920x491 = library.entry("HL920x491").unwrap();
        let DefinitionRecord::SectionProfile {
            shape: SectionShape::Parametric(p),
            section,
        } = &hl920x491.record
        else {
            panic!("HL920x491 is not a parametric section");
        };
        assert_eq!(p.values(), [957.0, 422.0, 25.9, 47.0, 19.0]);
        assert_eq!(section.area_mm2, Some(62300.0));
        assert_eq!(section.mass_per_meter_kg, Some(491.0));
        let hd400x818 = library.entry("HD400x818").unwrap();
        let DefinitionRecord::SectionProfile {
            shape: SectionShape::Parametric(p),
            section,
        } = &hd400x818.record
        else {
            panic!("HD400x818 is not a parametric section");
        };
        assert_eq!(p.values(), [514.0, 437.0, 60.5, 97.0, 24.0]);
        assert_eq!(section.area_mm2, Some(104630.0));
        assert_eq!(section.mass_per_meter_kg, Some(818.0));
        let hlz1100d = library.entry("HLZ1100D").unwrap();
        let DefinitionRecord::SectionProfile {
            shape: SectionShape::Parametric(p),
            section,
        } = &hlz1100d.record
        else {
            panic!("HLZ1100D is not a parametric section");
        };
        assert_eq!(p.values(), [1087.0, 460.0, 22.0, 37.0, 35.0]);
        assert_eq!(section.area_mm2, Some(57800.0));
        assert_eq!(section.mass_per_meter_kg, Some(453.0));
        let hp400x231 = library.entry("HP400x231").unwrap();
        let DefinitionRecord::SectionProfile {
            shape: SectionShape::Parametric(p),
            section,
        } = &hp400x231.record
        else {
            panic!("HP400x231 is not a parametric section");
        };
        assert_eq!(p.values(), [372.0, 402.0, 26.0, 26.0, 24.0]);
        assert_eq!(section.area_mm2, Some(29720.0));
        assert_eq!(section.mass_per_meter_kg, Some(231.0));
        let hea1000 = library.entry("HEA1000").unwrap();
        let DefinitionRecord::SectionProfile {
            shape: SectionShape::Parametric(p),
            section,
        } = &hea1000.record
        else {
            panic!("HEA1000 is not a parametric section");
        };
        assert_eq!(p.values(), [990.0, 300.0, 16.5, 31.0, 30.0]);
        assert_eq!(section.area_mm2, Some(34685.0));
        assert_eq!(section.mass_per_meter_kg, Some(272.3));
        for absent in [
            "IPE250", "IPE280", "HEA250", "HEB250", "IPE650", "HEM200", "HEM300", "UPE250",
            "UPE260", "UPE450", "UPN250", "UPN270", "UPN330", "UPN360", "UPN450",
        ] {
            assert_eq!(
                library.entry(absent).unwrap_err().code,
                ErrorCode::NotFound,
                "{absent} is not in the series"
            );
        }
    }

    #[test]
    fn the_shipped_europe_environment_admits_its_fastener_library() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/europe/environment.usda"
        );
        let environment = load_environment(path).unwrap();
        assert_eq!(environment.info.id, "europe");
        assert_eq!(environment.info.fastener_libraries, ["din-fasteners.usda"]);
        // The profile namespace is unchanged by the fastener listing: both profile libraries
        // keep their entries.
        assert_eq!(environment.sources.len(), 2);
        assert_eq!(environment.sources[0].library.entries.len(), 207);
        assert_eq!(environment.sources[1].library.entries.len(), 52);
        // The DIN fastener set rides the same environment, admitted with declared provenance;
        // the pack carries the M12-30 designations since the EN 1090-2 hole-series rows.
        assert_eq!(environment.fastener_sources.len(), 1);
        let bolts = &environment.fastener_sources[0];
        assert_eq!(bolts.library.info.id, "beamcraft-din-fasteners");
        assert_eq!(bolts.library.entries.len(), 6);
        assert!(bolts.library.rejected.is_empty());
        assert!(
            bolts
                .library
                .info
                .source
                .as_deref()
                .is_some_and(|s| !s.is_empty())
                && bolts
                    .library
                    .info
                    .licence
                    .as_deref()
                    .is_some_and(|l| !l.is_empty()),
            "the shipped fastener library declares its provenance"
        );
        // Bolt lookups ride the same resolver machinery, in their own namespace.
        let lookup = environment.fastener_lookup();
        let m12 = lookup.find("M12").unwrap().1;
        assert!(
            matches!(&m12.record, DefinitionRecord::Fastener { spec } if spec.nominal_diameter_mm == 12.0 && spec.lengths_mm.contains(&80.0))
        );
        // A designation the set does not publish is NOT_FOUND, never a near match.
        for absent in ["M36", "M12x80", "m12"] {
            assert_eq!(
                lookup.find(absent).unwrap_err().code,
                ErrorCode::NotFound,
                "{absent}"
            );
        }
    }

    #[test]
    fn the_shipped_usa_environment_admits_its_fastener_library() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/usa/environment.usda"
        );
        let environment = load_environment(path).unwrap();
        assert_eq!(environment.info.id, "usa");
        assert_eq!(
            environment.info.fastener_libraries,
            ["fasteners-rcsc-2025.usda"]
        );
        // The profile namespace is unchanged by the fastener listing: the A6 pack keeps its
        // entries.
        assert_eq!(environment.sources.len(), 1);
        assert_eq!(environment.sources[0].library.entries.len(), 285);
        // The RCSC fastener set rides the same environment, admitted with declared provenance.
        assert_eq!(environment.fastener_sources.len(), 1);
        let bolts = &environment.fastener_sources[0];
        assert_eq!(bolts.library.info.id, "beamcraft-rcsc-fasteners");
        assert_eq!(bolts.library.entries.len(), 18);
        assert!(bolts.library.rejected.is_empty());
        assert!(
            bolts
                .library
                .info
                .source
                .as_deref()
                .is_some_and(|s| !s.is_empty())
                && bolts
                    .library
                    .info
                    .licence
                    .as_deref()
                    .is_some_and(|l| !l.is_empty()),
            "the shipped fastener library declares its provenance"
        );
        // Bolt lookups ride the same resolver machinery, in their own namespace.
        let lookup = environment.fastener_lookup();
        let three_quarter = lookup.find("A325-3/4").unwrap().1;
        assert!(
            matches!(&three_quarter.record, DefinitionRecord::Fastener { spec }
                if spec.nominal_diameter_mm == 19.05 && spec.hole_clearance_mm == 1.5875)
        );
        // A designation the set does not publish is NOT_FOUND, never a near match: the metric
        // A325M series is excluded by the slice, the washers are paywalled and deferred, no
        // oversize or slot row exists, and the lookup never answers a lowercase variant.
        for absent in [
            "A325M-16",
            "A325M20",
            "M20",
            "F436-3/4",
            "A325-3/4-OVS",
            "a325-3/4",
        ] {
            assert_eq!(
                lookup.find(absent).unwrap_err().code,
                ErrorCode::NotFound,
                "{absent}"
            );
        }
    }

    #[test]
    fn the_shipped_usa_fastener_pack_matches_the_rcsc_2025_tables() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/usa/fasteners-rcsc-2025.usda"
        );
        let library = load_fastener(path).unwrap();
        assert_eq!(library.info.id, "beamcraft-rcsc-fasteners");
        assert_eq!(library.info.version.as_deref(), Some("1"));
        assert_eq!(library.rejected, []);
        assert_eq!(library.entries.len(), 18);
        // One row per size and grade: the C-2.1 inch fractions converted exactly (x 25.4),
        // the Table 3.1 standard hole as the clearance — 1/16 in over the bolt through 7/8 in
        // and 1/8 in from 1 in up, the band the 2025 edition moved — and no washer anywhere.
        let sizes = [
            ("1/2", 12.7, 22.225, 7.9375, 12.303125, 1.5875, 4.0),
            ("5/8", 15.875, 26.9875, 9.921875, 15.478125, 1.5875, 5.0),
            ("3/4", 19.05, 31.75, 11.90625, 18.653125, 1.5875, 6.0),
            ("7/8", 22.225, 36.5125, 13.890625, 21.828125, 1.5875, 7.0),
            ("1", 25.4, 41.275, 15.478125, 25.003125, 3.175, 8.0),
            ("1-1/8", 28.575, 46.0375, 17.4625, 28.178125, 3.175, 9.0),
            ("1-1/4", 31.75, 50.8, 19.84375, 30.95625, 3.175, 10.0),
            ("1-3/8", 34.925, 55.5625, 21.43125, 34.13125, 3.175, 11.0),
            ("1-1/2", 38.1, 60.325, 23.8125, 37.30625, 3.175, 12.0),
        ];
        // The offered lengths are the declared contributor grid, exact millimetres of the
        // inch steps: 1-1/4 to 5 in quarter steps (31.75 to 127 mm), then half steps to 8 in
        // (139.7 to 203.2 mm).
        let lengths: Vec<f64> = vec![
            31.75, 38.1, 44.45, 50.8, 57.15, 63.5, 69.85, 76.2, 82.55, 88.9, 95.25, 101.6, 107.95,
            114.3, 120.65, 127., 139.7, 152.4, 165.1, 177.8, 190.5, 203.2,
        ];
        assert_eq!(lengths.len(), 22);
        for grade in ["A325", "A490"] {
            for (size, nominal, flats, head, nut, clearance, protrusion) in sizes {
                let designation = format!("{grade}-{size}");
                let entry = library
                    .entries
                    .iter()
                    .find(|e| e.designation == designation)
                    .unwrap_or_else(|| panic!("the pack lists {designation}"));
                let DefinitionRecord::Fastener { spec } = &entry.record else {
                    panic!("{designation} is a fastener record");
                };
                assert_eq!(spec.designation.as_deref(), Some(designation.as_str()));
                assert_eq!(spec.standard.as_deref(), Some("ASTM F3125"));
                assert_eq!(spec.grade.as_deref(), Some(grade));
                assert_eq!(spec.kind, FastenerKind::Bolt);
                assert_eq!(spec.head_kind, HeadKind::Hex);
                assert_eq!(spec.nominal_diameter_mm, nominal, "{designation}");
                assert_eq!(spec.head_across_flats_mm, flats, "{designation}");
                assert_eq!(spec.head_height_mm, head, "{designation}");
                assert_eq!(spec.nut_across_flats_mm, Some(flats), "{designation}");
                assert_eq!(spec.nut_height_mm, Some(nut), "{designation}");
                assert_eq!(spec.hole_clearance_mm, clearance, "{designation}");
                assert_eq!(spec.thread_protrusion_mm, protrusion, "{designation}");
                assert!(spec.washer.is_none(), "{designation} carries no washer");
                assert!(!spec.washer_under_head && !spec.washer_under_nut);
                assert_eq!(spec.hole_mode, HoleMode::Generate);
                assert_eq!(spec.lengths_mm, lengths, "{designation}");
            }
        }
    }

    #[test]
    fn the_shipped_europe_materials_library_reads_with_every_entry_admitted() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/europe/materials-en-10025.usda"
        );
        let library = load_material(path).unwrap();
        assert_eq!(library.info.id, "europe-en-10025-materials");
        assert_eq!(library.info.version.as_deref(), Some("1"));
        assert!(
            library
                .info
                .licence
                .as_deref()
                .is_some_and(|l| !l.is_empty()),
            "the shipped library declares its licence"
        );
        assert!(
            library
                .info
                .source
                .as_deref()
                .is_some_and(|s| !s.is_empty()),
            "the shipped library declares its source"
        );
        // No transcription slips: every entry is admitted.
        assert_eq!(library.rejected, []);
        assert_eq!(library.entries.len(), 33);
        // The complete structural steel grade sets of the EN 10025-2/-3/-4:2019 editions,
        // S235 to S500, in the loader's designation order, each with its part and the pack
        // density: part 2 grew in 2019 by the S460 qualities and S500J0 (and dropped S450J0,
        // which is not shipped), part 4 by S500M/ML; part 3 is unchanged. The weathering
        // steels of part 5 and the quenched and tempered flat products of part 6 are not in
        // the pack.
        let part2 = "EN 10025-2:2019";
        let part3 = "EN 10025-3:2019";
        let part4 = "EN 10025-4:2019";
        let expected: [(&str, &str); 33] = [
            ("S235J0", part2),
            ("S235J2", part2),
            ("S235JR", part2),
            ("S275J0", part2),
            ("S275J2", part2),
            ("S275JR", part2),
            ("S275M", part4),
            ("S275ML", part4),
            ("S275N", part3),
            ("S275NL", part3),
            ("S355J0", part2),
            ("S355J2", part2),
            ("S355JR", part2),
            ("S355K2", part2),
            ("S355M", part4),
            ("S355ML", part4),
            ("S355N", part3),
            ("S355NL", part3),
            ("S420M", part4),
            ("S420ML", part4),
            ("S420N", part3),
            ("S420NL", part3),
            ("S460J0", part2),
            ("S460J2", part2),
            ("S460JR", part2),
            ("S460K2", part2),
            ("S460M", part4),
            ("S460ML", part4),
            ("S460N", part3),
            ("S460NL", part3),
            ("S500J0", part2),
            ("S500M", part4),
            ("S500ML", part4),
        ];
        for ((designation, want_standard), entry) in expected.iter().zip(&library.entries) {
            let DefinitionRecord::Material {
                density_kg_m3,
                grade,
                standard,
                display_name,
            } = &entry.record
            else {
                panic!("{} is not a material", entry.designation);
            };
            assert_eq!(entry.designation, *designation);
            assert_eq!(grade.as_deref(), Some(*designation));
            assert_eq!(*density_kg_m3, Some(7850.0), "{}", designation);
            assert_eq!(standard.as_deref(), Some(*want_standard), "{}", designation);
            assert_eq!(display_name.as_deref(), Some(*designation));
        }
        assert_eq!(library.entries.len(), expected.len());
    }

    #[test]
    fn the_shipped_europe_environment_admits_its_material_library() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/europe/environment.usda"
        );
        let environment = load_environment(path).unwrap();
        assert_eq!(
            environment.info.material_libraries,
            ["materials-en-10025.usda"]
        );
        // The profile and fastener namespaces are unchanged by the material listing.
        assert_eq!(environment.sources.len(), 2);
        assert_eq!(environment.fastener_sources.len(), 1);
        assert_eq!(environment.material_sources.len(), 1);
        let materials = &environment.material_sources[0];
        assert_eq!(materials.library.info.id, "europe-en-10025-materials");
        assert_eq!(materials.library.entries.len(), 33);
        assert!(materials.library.rejected.is_empty());
        // A grade rides the same resolver machinery, in its own namespace.
        let lookup = environment.material_lookup();
        let (source, s355) = lookup.find("S355J2").unwrap();
        assert_eq!(source.library.info.id, "europe-en-10025-materials");
        assert!(matches!(
            &s355.record,
            DefinitionRecord::Material { density_kg_m3: Some(7850.0), standard, .. }
                if standard.as_deref() == Some("EN 10025-2:2019")
        ));
        // A grade the set does not publish is NOT_FOUND, never a near match: no bare strength
        // class, no lowercase, no grade of another part or of the superseded 2004 edition.
        for absent in ["S235", "S355", "s355j2", "S355K2+N", "S500N", "S450J0"] {
            assert_eq!(
                lookup.find(absent).unwrap_err().code,
                ErrorCode::NotFound,
                "{absent}"
            );
        }
    }

    #[test]
    fn the_shipped_south_africa_ipe_library_reads_with_every_entry_admitted() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/south-africa/profiles-en-10365-ipe.usda"
        );
        let library = load(path).unwrap();
        assert_eq!(library.info.id, "south-africa-en-10365-ipe-profiles");
        assert_eq!(library.info.version.as_deref(), Some("1"));
        assert!(
            library
                .info
                .licence
                .as_deref()
                .is_some_and(|l| !l.is_empty()),
            "the shipped library declares its licence"
        );
        assert!(
            library
                .info
                .source
                .as_deref()
                .is_some_and(|s| !s.is_empty()),
            "the shipped library declares its source"
        );
        // No transcription slips: every entry is admitted and none warns.
        assert_eq!(library.rejected, []);
        assert_eq!(library.entries.len(), 17);
        for entry in &library.entries {
            let DefinitionRecord::SectionProfile {
                shape: SectionShape::Parametric(parametric),
                section,
            } = &entry.record
            else {
                panic!("{} is not a parametric section", entry.designation);
            };
            assert_eq!(parametric.family(), SectionFamily::IShape);
            // The live EN string is the declared standard: EN 10365:2017 carries the IPE series
            // that DIN 1025-5 specified before EN 10365 superseded it.
            assert_eq!(
                section.standard.as_deref(),
                Some("EN 10365:2017"),
                "{}",
                entry.designation
            );
            // The D3 declared-area check: the declared catalogue area matches the exact
            // filleted reference (the 1 percent warning never fires) and the declared mass is
            // the area at 7850 kg/m3 within the standard's published rounding.
            assert!(
                entry.record.declared_area_warning().is_none(),
                "{}",
                entry.designation
            );
            let area = section.area_mm2.unwrap();
            let mass = section.mass_per_meter_kg.unwrap();
            let mass_from_area = area * 7850.0 / 1.0e6;
            assert!(
                (mass - mass_from_area).abs() <= 0.55,
                "{}: mass {mass} kg/m vs area-derived {mass_from_area}",
                entry.designation
            );
            assert_eq!(section.area_convention, AreaConvention::ExactFilleted);
        }
        // The series as EN 10365 publishes it, in the loader's designation order: the sequence
        // steps 240 to 270 (no IPE 250 or 280) and ends at 600.
        let designations: Vec<&str> = library
            .entries
            .iter()
            .map(|e| e.designation.as_str())
            .collect();
        assert_eq!(
            designations,
            [
                "IPE100", "IPE120", "IPE140", "IPE160", "IPE180", "IPE200", "IPE220", "IPE240",
                "IPE270", "IPE300", "IPE330", "IPE360", "IPE400", "IPE450", "IPE500", "IPE550",
                "IPE600",
            ]
        );
        // Every row is the europe pack's IPE row of the same designation, value for value: one
        // transcription of the series serves both packs.
        let europe = load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/europe/profiles-en-10365.usda"
        ))
        .unwrap();
        let twins: Vec<&LibraryEntry> = europe
            .entries
            .iter()
            .filter(|e| e.designation.starts_with("IPE"))
            .collect();
        assert_eq!(twins.len(), 17);
        let rows: Vec<&LibraryEntry> = library.entries.iter().collect();
        assert_eq!(rows, twins, "the IPE rows are the europe pack's");
        // Spot checks against the standard's published values.
        let ipe300 = library.entry("IPE300").unwrap();
        let DefinitionRecord::SectionProfile {
            shape: SectionShape::Parametric(p),
            section,
        } = &ipe300.record
        else {
            panic!("IPE300 is not a parametric section");
        };
        assert_eq!(p.values(), [300.0, 150.0, 7.1, 10.7, 15.0]);
        assert_eq!(section.area_mm2, Some(5381.0));
        assert_eq!(section.mass_per_meter_kg, Some(42.2));
        let ipe100 = library.entry("IPE100").unwrap();
        let DefinitionRecord::SectionProfile {
            shape: SectionShape::Parametric(p),
            section,
        } = &ipe100.record
        else {
            panic!("IPE100 is not a parametric section");
        };
        assert_eq!(p.values(), [100.0, 55.0, 4.1, 5.7, 7.0]);
        assert_eq!(section.area_mm2, Some(1032.0));
        assert_eq!(section.mass_per_meter_kg, Some(8.1));
        // Out of this library: the BS UB/UC/UBP/PFC series of South African practice (an open
        // owner question), the other EN 10365 series, the sizes beyond IPE 600, and any other
        // spelling of a shipped designation.
        for absent in [
            "IPE250",
            "IPE280",
            "IPE750",
            "HEA200",
            "HEB200",
            "UPE200",
            "UPN200",
            "UB203x133x25",
            "UC203x203x46",
            "UBP254x254x71",
            "PFC200",
            "IPE 300",
            "ipe300",
        ] {
            assert_eq!(
                library.entry(absent).unwrap_err().code,
                ErrorCode::NotFound,
                "{absent} is not in the series"
            );
        }
    }

    #[test]
    fn the_shipped_south_africa_sans_50025_materials_library_reads_with_every_entry_admitted() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/south-africa/materials-sans-50025.usda"
        );
        let library = load_material(path).unwrap();
        assert_eq!(library.info.id, "south-africa-sans-50025-materials");
        assert_eq!(library.info.version.as_deref(), Some("1"));
        assert!(
            library
                .info
                .licence
                .as_deref()
                .is_some_and(|l| !l.is_empty()),
            "the shipped library declares its licence"
        );
        assert!(
            library
                .info
                .source
                .as_deref()
                .is_some_and(|s| !s.is_empty()),
            "the shipped library declares its source"
        );
        // No transcription slips: every entry is admitted.
        assert_eq!(library.rejected, []);
        assert_eq!(library.entries.len(), 27);
        // The structural grade sets of SANS 50025-2/-3/-4:2009, in the loader's designation
        // order, each with its part and the pack density. The 2009 SANS parts adopt the 2004
        // EN editions: S450J0 ships, the S460 JR/J0/J2/K2 qualities, S500J0, S500M and S500ML
        // the 2019 EN revision added do not, and S185 and the E295/E335/E360 engineering steels,
        // whose chemical composition the standard does not specify, ride no rows.
        let part2 = "SANS 50025-2:2009";
        let part3 = "SANS 50025-3:2009";
        let part4 = "SANS 50025-4:2009";
        let expected: [(&str, &str); 27] = [
            ("S235J0", part2),
            ("S235J2", part2),
            ("S235JR", part2),
            ("S275J0", part2),
            ("S275J2", part2),
            ("S275JR", part2),
            ("S275M", part4),
            ("S275ML", part4),
            ("S275N", part3),
            ("S275NL", part3),
            ("S355J0", part2),
            ("S355J2", part2),
            ("S355JR", part2),
            ("S355K2", part2),
            ("S355M", part4),
            ("S355ML", part4),
            ("S355N", part3),
            ("S355NL", part3),
            ("S420M", part4),
            ("S420ML", part4),
            ("S420N", part3),
            ("S420NL", part3),
            ("S450J0", part2),
            ("S460M", part4),
            ("S460ML", part4),
            ("S460N", part3),
            ("S460NL", part3),
        ];
        for ((designation, want_standard), entry) in expected.iter().zip(&library.entries) {
            let DefinitionRecord::Material {
                density_kg_m3,
                grade,
                standard,
                display_name,
            } = &entry.record
            else {
                panic!("{} is not a material", entry.designation);
            };
            assert_eq!(entry.designation, *designation);
            assert_eq!(grade.as_deref(), Some(*designation));
            assert_eq!(*density_kg_m3, Some(7850.0), "{}", designation);
            assert_eq!(standard.as_deref(), Some(*want_standard), "{}", designation);
            assert_eq!(display_name.as_deref(), Some(*designation));
        }
        assert_eq!(library.entries.len(), expected.len());
        // Cross-check against the shipped `europe` pack, the independently transcribed 2019 EN
        // editions: every SANS grade but S450J0 is a 2019 EN grade too, and the 2019 EN grades
        // the pack lacks are exactly those the 2019 revision added.
        let europe = load_material(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/europe/materials-en-10025.usda"
        ))
        .unwrap();
        let sans: Vec<&str> = library
            .entries
            .iter()
            .map(|e| e.designation.as_str())
            .collect();
        let added_in_2019 = [
            "S460JR", "S460J0", "S460J2", "S460K2", "S500J0", "S500M", "S500ML",
        ];
        for entry in &europe.entries {
            assert_eq!(
                sans.contains(&entry.designation.as_str()),
                !added_in_2019.contains(&entry.designation.as_str()),
                "{}",
                entry.designation
            );
        }
        // A grade the set does not publish is NOT_FOUND, never a near match: no bare strength
        // class, no lowercase, no delivery-condition suffix and no grade of another edition.
        for absent in [
            "S235", "S355", "s355j2", "S355K2+N", "S460JR", "S500J0", "S500M", "S185", "E295",
        ] {
            assert_eq!(
                library.entry(absent).unwrap_err().code,
                ErrorCode::NotFound,
                "{absent} is not in the series"
            );
        }
    }

    #[test]
    fn the_shipped_south_africa_environment_admits_its_libraries() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/south-africa/environment.usda"
        );
        let environment = load_environment(path).unwrap();
        assert_eq!(environment.info.id, "south-africa");
        assert_eq!(environment.info.version.as_deref(), Some("1"));
        assert_eq!(environment.info.libraries, ["profiles-en-10365-ipe.usda"]);
        assert_eq!(
            environment.info.material_libraries,
            ["materials-sans-50025.usda"]
        );
        // The environment names no fastener library (#708), so that namespace stays empty.
        assert!(environment.info.fastener_libraries.is_empty());
        assert_eq!(environment.sources.len(), 1);
        assert_eq!(environment.sources[0].library.entries.len(), 17);
        assert!(environment.sources[0].library.rejected.is_empty());
        assert!(environment.fastener_sources.is_empty());
        assert_eq!(environment.material_sources.len(), 1);
        assert_eq!(environment.material_sources[0].library.entries.len(), 27);
        assert!(environment.material_sources[0].library.rejected.is_empty());
        // A grade rides the same resolver machinery, in its own namespace.
        let lookup = environment.material_lookup();
        let (source, s355) = lookup.find("S355J2").unwrap();
        assert_eq!(source.library.info.id, "south-africa-sans-50025-materials");
        assert!(matches!(
            &s355.record,
            DefinitionRecord::Material { density_kg_m3: Some(7850.0), standard, .. }
                if standard.as_deref() == Some("SANS 50025-2:2009")
        ));
        // A bolt designation stays NOT_FOUND: the pack carries no fastener set.
        assert_eq!(
            environment.fastener_lookup().find("M20").unwrap_err().code,
            ErrorCode::NotFound
        );
    }

    #[test]
    fn the_shipped_russia_gost_27772_materials_library_reads_with_every_entry_admitted() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/russia/materials-gost-27772.usda"
        );
        let library = load_material(path).unwrap();
        assert_eq!(library.info.id, "russia-gost-27772-materials");
        assert_eq!(library.info.version.as_deref(), Some("1"));
        assert!(
            library
                .info
                .licence
                .as_deref()
                .is_some_and(|l| !l.is_empty()),
            "the shipped library declares its licence"
        );
        assert!(
            library
                .info
                .source
                .as_deref()
                .is_some_and(|s| !s.is_empty()),
            "the shipped library declares its source"
        );
        // No transcription slips: every entry is admitted, Cyrillic prim names included.
        assert_eq!(library.rejected, []);
        assert_eq!(library.entries.len(), 14);
        // The complete grade set of GOST 27772-2015 clause 4.1, in the loader's designation
        // order (byte order: the Cyrillic К, П suffixes sort after the ASCII ones), each with
        // the standard and the pack density. The designations are the standard's own Cyrillic
        // forms: the leading letter is the Cyrillic С (U+0421), where С means structural
        // construction steel, the number the conventional yield strength in N/mm2, 1 a
        // chemistry variant, К an increased corrosion resistance, П an increased fire
        // resistance.
        let gost = "GOST 27772-2015";
        let expected: [&str; 14] = [
            "С235", "С245", "С255", "С345", "С345К", "С355", "С355-1", "С355К", "С355П", "С390",
            "С390-1", "С440", "С550", "С590",
        ];
        for (designation, entry) in expected.iter().zip(&library.entries) {
            let DefinitionRecord::Material {
                density_kg_m3,
                grade,
                standard,
                display_name,
            } = &entry.record
            else {
                panic!("{} is not a material", entry.designation);
            };
            assert_eq!(entry.designation, *designation);
            assert_eq!(grade.as_deref(), Some(*designation));
            assert_eq!(*density_kg_m3, Some(7850.0), "{}", designation);
            assert_eq!(standard.as_deref(), Some(gost), "{}", designation);
            assert_eq!(display_name.as_deref(), Some(*designation));
        }
        assert_eq!(library.entries.len(), expected.len());
    }

    #[test]
    fn the_shipped_russia_environment_admits_its_material_library() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/russia/environment.usda"
        );
        let environment = load_environment(path).unwrap();
        assert_eq!(
            environment.info.material_libraries,
            ["materials-gost-27772.usda"]
        );
        // The profile and fastener namespaces are unchanged by the material listing.
        assert_eq!(environment.sources.len(), 3);
        assert_eq!(environment.fastener_sources.len(), 1);
        assert_eq!(environment.material_sources.len(), 1);
        let materials = &environment.material_sources[0];
        assert_eq!(materials.library.info.id, "russia-gost-27772-materials");
        assert_eq!(materials.library.entries.len(), 14);
        assert!(materials.library.rejected.is_empty());
        // A grade rides the same resolver machinery, in its own namespace.
        let lookup = environment.material_lookup();
        let (source, s345) = lookup.find("С345").unwrap();
        assert_eq!(source.library.info.id, "russia-gost-27772-materials");
        assert!(matches!(
            &s345.record,
            DefinitionRecord::Material { density_kg_m3: Some(7850.0), standard, .. }
                if standard.as_deref() == Some("GOST 27772-2015")
        ));
        // A grade the set does not publish is NOT_FOUND, never a near match: no bare strength
        // class, no Latin look-alike of the Cyrillic С (the С of the standard's own
        // designation is the Cyrillic letter, so neither the Latin C nor the transliteration
        // S answers), no lowercase variant and no grade of the superseded 1988 edition.
        for absent in [
            "С275", "С375", "С285", "C245", "S245", "C345", "s355", "С245 ", "С440Д",
        ] {
            assert_eq!(
                lookup.find(absent).unwrap_err().code,
                ErrorCode::NotFound,
                "{absent}"
            );
        }
    }

    #[test]
    fn the_shipped_usa_astm_materials_library_reads_with_every_entry_admitted() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/usa/materials-astm.usda"
        );
        let library = load_material(path).unwrap();
        assert_eq!(library.info.id, "usa-astm-materials");
        assert_eq!(library.info.version.as_deref(), Some("1"));
        assert!(
            library
                .info
                .licence
                .as_deref()
                .is_some_and(|l| !l.is_empty()),
            "the shipped library declares its licence"
        );
        assert!(
            library
                .info
                .source
                .as_deref()
                .is_some_and(|s| !s.is_empty()),
            "the shipped library declares its source"
        );
        // No transcription slips: every entry is admitted.
        assert_eq!(library.rejected, []);
        assert_eq!(library.entries.len(), 7);
        // The grades of the three standards in the loader's designation order: the single
        // A36, the five A572 strength grades (42 to 65, the grade number the minimum yield
        // strength in ksi) with the Grade word the standard designates with, and the W-shape
        // grade A992. The standards are the consulted editions; the density is the metric
        // 7850 kg/m3 of every shipped pack.
        let a36 = "ASTM A36/A36M-19";
        let a992 = "ASTM A992/A992M-22";
        let a572 = "ASTM A572/A572M-21";
        let expected: [(&str, &str); 7] = [
            ("A36", a36),
            ("A572 Grade 42", a572),
            ("A572 Grade 50", a572),
            ("A572 Grade 55", a572),
            ("A572 Grade 60", a572),
            ("A572 Grade 65", a572),
            ("A992", a992),
        ];
        for ((designation, want_standard), entry) in expected.iter().zip(&library.entries) {
            let DefinitionRecord::Material {
                density_kg_m3,
                grade,
                standard,
                display_name,
            } = &entry.record
            else {
                panic!("{} is not a material", entry.designation);
            };
            assert_eq!(entry.designation, *designation);
            assert_eq!(grade.as_deref(), Some(*designation));
            assert_eq!(*density_kg_m3, Some(7850.0), "{}", designation);
            assert_eq!(standard.as_deref(), Some(*want_standard), "{}", designation);
            assert_eq!(display_name.as_deref(), Some(*designation));
        }
        assert_eq!(library.entries.len(), expected.len());
    }

    #[test]
    fn the_shipped_usa_environment_admits_its_material_library() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/usa/environment.usda"
        );
        let environment = load_environment(path).unwrap();
        assert_eq!(environment.info.libraries, ["profiles-astm-a6-w-hp.usda"]);
        assert_eq!(environment.info.material_libraries, ["materials-astm.usda"]);
        // The environment carries its profile library beside its material library and its
        // RCSC fastener set (the fastener slice of #708).
        assert_eq!(environment.sources.len(), 1);
        assert_eq!(
            environment.sources[0].library.info.id,
            "usa-astm-a6-w-hp-profiles"
        );
        assert_eq!(environment.fastener_sources.len(), 1);
        assert_eq!(
            environment.fastener_sources[0].library.info.id,
            "beamcraft-rcsc-fasteners"
        );
        assert_eq!(environment.material_sources.len(), 1);
        let materials = &environment.material_sources[0];
        assert_eq!(materials.library.info.id, "usa-astm-materials");
        assert_eq!(materials.library.entries.len(), 7);
        assert!(materials.library.rejected.is_empty());
        // A grade rides the same resolver machinery, in its own namespace.
        let lookup = environment.material_lookup();
        let (source, grade_50) = lookup.find("A572 Grade 50").unwrap();
        assert_eq!(source.library.info.id, "usa-astm-materials");
        assert!(matches!(
            &grade_50.record,
            DefinitionRecord::Material { density_kg_m3: Some(7850.0), standard, .. }
                if standard.as_deref() == Some("ASTM A572/A572M-21")
        ));
        // A grade the set does not publish is NOT_FOUND, never a near match: no bare grade
        // number, no hyphenated or compact industry shorthand (the shipped form is the
        // standard's own Grade form), no lowercase, no grade outside the standard's set and
        // no grade of another specification.
        for absent in [
            "A572",
            "A572-50",
            "A572Gr50",
            "A36M",
            "a992",
            "A572 Grade 70",
            "A500",
        ] {
            assert_eq!(
                lookup.find(absent).unwrap_err().code,
                ErrorCode::NotFound,
                "{absent}"
            );
        }
    }

    #[test]
    fn the_shipped_canada_csa_materials_library_reads_with_every_entry_admitted() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/canada/materials-csa-g4021.usda"
        );
        let library = load_material(path).unwrap();
        assert_eq!(library.info.id, "canada-csa-g4021-materials");
        assert_eq!(library.info.version.as_deref(), Some("1"));
        assert!(
            library
                .info
                .licence
                .as_deref()
                .is_some_and(|l| !l.is_empty()),
            "the shipped library declares its licence"
        );
        assert!(
            library
                .info
                .source
                .as_deref()
                .is_some_and(|s| !s.is_empty()),
            "the shipped library declares its source"
        );
        // No transcription slips: every entry is admitted.
        assert_eq!(library.rejected, []);
        assert_eq!(library.entries.len(), 7);
        // The complete weldable category of CSA G40.20-13/G40.21-13 in the loader's
        // designation order: the metric W series, the number the minimum yield strength in
        // MPa and W the weldable category. 380W is available only as hollow structural
        // sections, angles and bars, the scope note the standard's grade table carries; the
        // impact-tested, atmospheric-corrosion-resistant and quenched-and-tempered categories
        // are not in the pack.
        let csa = "CSA G40.20-13/G40.21-13";
        let expected: [(&str, &str); 7] = [
            ("260W", csa),
            ("300W", csa),
            ("350W", csa),
            ("380W", csa),
            ("400W", csa),
            ("480W", csa),
            ("550W", csa),
        ];
        for ((designation, want_standard), entry) in expected.iter().zip(&library.entries) {
            let DefinitionRecord::Material {
                density_kg_m3,
                grade,
                standard,
                display_name,
            } = &entry.record
            else {
                panic!("{} is not a material", entry.designation);
            };
            assert_eq!(entry.designation, *designation);
            assert_eq!(grade.as_deref(), Some(*designation));
            assert_eq!(*density_kg_m3, Some(7850.0), "{}", designation);
            assert_eq!(standard.as_deref(), Some(*want_standard), "{}", designation);
            assert_eq!(display_name.as_deref(), Some(*designation));
        }
        assert_eq!(library.entries.len(), expected.len());
    }

    #[test]
    fn the_shipped_canada_environment_admits_its_material_library() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/canada/environment.usda"
        );
        let environment = load_environment(path).unwrap();
        assert_eq!(
            environment.info.material_libraries,
            ["materials-csa-g4021.usda"]
        );
        // One profile source beside the material one (the #709 pack), no fastener libraries.
        assert_eq!(environment.sources.len(), 1);
        assert_eq!(
            environment.sources[0].library.info.id,
            "canada-astm-a6-w-hp-profiles"
        );
        assert_eq!(environment.fastener_sources.len(), 0);
        assert_eq!(environment.material_sources.len(), 1);
        let materials = &environment.material_sources[0];
        assert_eq!(materials.library.info.id, "canada-csa-g4021-materials");
        assert_eq!(materials.library.entries.len(), 7);
        assert!(materials.library.rejected.is_empty());
        // A grade rides the same resolver machinery, in its own namespace.
        let lookup = environment.material_lookup();
        let (source, grade_350) = lookup.find("350W").unwrap();
        assert_eq!(source.library.info.id, "canada-csa-g4021-materials");
        assert!(matches!(
            &grade_350.record,
            DefinitionRecord::Material { density_kg_m3: Some(7850.0), standard, .. }
                if standard.as_deref() == Some("CSA G40.20-13/G40.21-13")
        ));
        // A grade the set does not publish is NOT_FOUND, never a near match: no bare strength
        // level, no imperial-ksi designation of the same grade, no other category (the
        // impact-tested 350WT and the weathering 350A of the same strength level included),
        // no lowercase and no strength level outside the standard's set.
        for absent in [
            "350", "50W", "44W", "350WT", "350A", "350R", "w350", "600W", "700W",
        ] {
            assert_eq!(
                lookup.find(absent).unwrap_err().code,
                ErrorCode::NotFound,
                "{absent}"
            );
        }
    }

    #[test]
    fn the_shipped_south_america_nch203_materials_library_reads_with_every_entry_admitted() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/south-america/materials-nch203.usda"
        );
        let library = load_material(path).unwrap();
        assert_eq!(library.info.id, "south-america-nch203-materials");
        assert_eq!(library.info.version.as_deref(), Some("1"));
        assert!(
            library
                .info
                .licence
                .as_deref()
                .is_some_and(|l| !l.is_empty()),
            "the shipped library declares its licence"
        );
        assert!(
            library
                .info
                .source
                .as_deref()
                .is_some_and(|s| !s.is_empty()),
            "the shipped library declares its source"
        );
        // No transcription slips: every entry is admitted.
        assert_eq!(library.rejected, []);
        assert_eq!(library.entries.len(), 7);
        // The complete grade set of NCh203.Of2006 in the loader's designation order: the
        // general-construction class ES (the carbon steels A240ES, A270ES and A345ES, the
        // microalloyed M345ES and the high-strength low-alloy Y345ES, where the number is the
        // minimum yield strength in MPa) and the dynamic-load class ESP (A250ESP, A345ESP).
        // The national designations only: no ASTM alias and no designation of the annulled
        // NCh203.Of77 edition is a row.
        let nch = "NCh203.Of2006";
        let expected: [(&str, &str); 7] = [
            ("A240ES", nch),
            ("A250ESP", nch),
            ("A270ES", nch),
            ("A345ES", nch),
            ("A345ESP", nch),
            ("M345ES", nch),
            ("Y345ES", nch),
        ];
        for ((designation, want_standard), entry) in expected.iter().zip(&library.entries) {
            let DefinitionRecord::Material {
                density_kg_m3,
                grade,
                standard,
                display_name,
            } = &entry.record
            else {
                panic!("{} is not a material", entry.designation);
            };
            assert_eq!(entry.designation, *designation);
            assert_eq!(grade.as_deref(), Some(*designation));
            assert_eq!(*density_kg_m3, Some(7850.0), "{}", designation);
            assert_eq!(standard.as_deref(), Some(*want_standard), "{}", designation);
            assert_eq!(display_name.as_deref(), Some(*designation));
        }
        assert_eq!(library.entries.len(), expected.len());
    }

    #[test]
    fn the_shipped_south_america_environment_admits_its_material_library() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/south-america/environment.usda"
        );
        let environment = load_environment(path).unwrap();
        assert_eq!(environment.info.id, "south-america");
        assert_eq!(
            environment.info.material_libraries,
            ["materials-nch203.usda"]
        );
        // The #708 route landed beside the grades (#711): the environment carries the W-shape
        // profile library with its material library and no fastener library.
        assert_eq!(environment.info.libraries, ["profiles-astm-a6-w.usda"]);
        assert_eq!(environment.sources.len(), 1);
        assert_eq!(
            environment.sources[0].library.info.id,
            "south-america-astm-a6-w-profiles"
        );
        assert_eq!(environment.fastener_sources.len(), 0);
        assert_eq!(environment.material_sources.len(), 1);
        let materials = &environment.material_sources[0];
        assert_eq!(materials.library.info.id, "south-america-nch203-materials");
        assert_eq!(materials.library.entries.len(), 7);
        assert!(materials.library.rejected.is_empty());
        // A grade rides the same resolver machinery, in its own namespace.
        let lookup = environment.material_lookup();
        let (source, grade_270) = lookup.find("A270ES").unwrap();
        assert_eq!(source.library.info.id, "south-america-nch203-materials");
        assert!(matches!(
            &grade_270.record,
            DefinitionRecord::Material { density_kg_m3: Some(7850.0), standard, .. }
                if standard.as_deref() == Some("NCh203.Of2006")
        ));
        // A grade the set does not publish is NOT_FOUND, never a near match: no ASTM
        // designation of the standard's informative Annex C relations, no designation of the
        // annulled NCh203.Of77 edition, no bare strength level, no full designation carrying
        // the optional condition letters, no lowercase and no strength level outside the set.
        for absent in [
            "A36",
            "A572 Grade 50",
            "A37-24ES",
            "A42-27ES",
            "A270",
            "A270ES N",
            "a270es",
            "A300ES",
        ] {
            assert_eq!(
                lookup.find(absent).unwrap_err().code,
                ErrorCode::NotFound,
                "{absent}"
            );
        }
    }

    #[test]
    fn the_shipped_philippines_materials_library_reads_with_every_entry_admitted() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/philippines/materials-philippines.usda"
        );
        let library = load_material(path).unwrap();
        assert_eq!(library.info.id, "philippines-esc-materials");
        assert_eq!(library.info.version.as_deref(), Some("1"));
        assert!(
            library
                .info
                .licence
                .as_deref()
                .is_some_and(|l| !l.is_empty()),
            "the shipped library declares its licence"
        );
        assert!(
            library
                .info
                .source
                .as_deref()
                .is_some_and(|s| !s.is_empty()),
            "the shipped library declares its source"
        );
        // No transcription slips: every entry is admitted.
        assert_eq!(library.rejected, []);
        assert_eq!(library.entries.len(), 35);
        // The grade designations the ESC Steel Philippines page names for locally supplied
        // sections, each with the publicly documented edition of its standard: the ASTM
        // grades the `usa` pack declares (A36, the complete A572 strength-grade set), the
        // EN 10025-2:2019 set the `europe` pack re-entered, and the JIS grades — SS400 of
        // G 3101:2015, the SM set ESC names of G 3106:2020 and the complete G 3136:2012 set
        // (SN490A is no grade of that standard). The KS D 3503 and KS D 3515 designations
        // the page names are the same strings the JIS rows carry and add no row.
        let expected: [(&str, &str); 35] = [
            ("A36", "ASTM A36/A36M-19"),
            ("A572 Grade 42", "ASTM A572/A572M-21"),
            ("A572 Grade 50", "ASTM A572/A572M-21"),
            ("A572 Grade 55", "ASTM A572/A572M-21"),
            ("A572 Grade 60", "ASTM A572/A572M-21"),
            ("A572 Grade 65", "ASTM A572/A572M-21"),
            ("S235J0", "EN 10025-2:2019"),
            ("S235J2", "EN 10025-2:2019"),
            ("S235JR", "EN 10025-2:2019"),
            ("S275J0", "EN 10025-2:2019"),
            ("S275J2", "EN 10025-2:2019"),
            ("S275JR", "EN 10025-2:2019"),
            ("S355J0", "EN 10025-2:2019"),
            ("S355J2", "EN 10025-2:2019"),
            ("S355JR", "EN 10025-2:2019"),
            ("S355K2", "EN 10025-2:2019"),
            ("S460J0", "EN 10025-2:2019"),
            ("S460J2", "EN 10025-2:2019"),
            ("S460JR", "EN 10025-2:2019"),
            ("S460K2", "EN 10025-2:2019"),
            ("S500J0", "EN 10025-2:2019"),
            ("SM400A", "JIS G 3106:2020"),
            ("SM400B", "JIS G 3106:2020"),
            ("SM400C", "JIS G 3106:2020"),
            ("SM490A", "JIS G 3106:2020"),
            ("SM490B", "JIS G 3106:2020"),
            ("SM490C", "JIS G 3106:2020"),
            ("SM490YA", "JIS G 3106:2020"),
            ("SM490YB", "JIS G 3106:2020"),
            ("SN400A", "JIS G 3136:2012"),
            ("SN400B", "JIS G 3136:2012"),
            ("SN400C", "JIS G 3136:2012"),
            ("SN490B", "JIS G 3136:2012"),
            ("SN490C", "JIS G 3136:2012"),
            ("SS400", "JIS G 3101:2015"),
        ];
        for ((designation, want_standard), entry) in expected.iter().zip(&library.entries) {
            let DefinitionRecord::Material {
                density_kg_m3,
                grade,
                standard,
                display_name,
            } = &entry.record
            else {
                panic!("{} is not a material", entry.designation);
            };
            assert_eq!(entry.designation, *designation);
            assert_eq!(grade.as_deref(), Some(*designation));
            assert_eq!(*density_kg_m3, Some(7850.0), "{}", designation);
            assert_eq!(standard.as_deref(), Some(*want_standard), "{}", designation);
            assert_eq!(display_name.as_deref(), Some(*designation));
        }
        assert_eq!(library.entries.len(), expected.len());
    }

    #[test]
    fn the_shipped_philippines_environment_admits_its_libraries() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/philippines/environment.usda"
        );
        let environment = load_environment(path).unwrap();
        assert_eq!(environment.info.id, "philippines");
        assert_eq!(environment.info.libraries, ["profiles-astm-a6-w-hp.usda"]);
        assert_eq!(
            environment.info.material_libraries,
            ["materials-philippines.usda"]
        );
        // One profile source beside one material source; no fastener library.
        assert_eq!(environment.sources.len(), 1);
        assert_eq!(environment.fastener_sources.len(), 0);
        assert_eq!(environment.material_sources.len(), 1);
        // The profile source re-declares the `usa` pack A6 facts: the same 285 admitted
        // rows, every designation with the same dimensions, area, mass and radius, under
        // the Philippines library id.
        let profiles = &environment.sources[0];
        assert_eq!(
            profiles.library.info.id,
            "philippines-astm-a6-w-hp-profiles"
        );
        assert!(profiles.library.rejected.is_empty());
        assert_eq!(profiles.library.entries.len(), 285);
        let usa = load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/usa/profiles-astm-a6-w-hp.usda"
        ))
        .unwrap();
        assert_eq!(usa.entries.len(), 285);
        for (philippines, usa) in profiles.library.entries.iter().zip(&usa.entries) {
            assert_eq!(philippines.designation, usa.designation);
            assert_eq!(philippines.record, usa.record);
        }
        // The material source binds the ESC-named grades; the environment lookup resolves
        // through both namespaces.
        let materials = &environment.material_sources[0];
        assert_eq!(materials.library.info.id, "philippines-esc-materials");
        assert!(materials.library.rejected.is_empty());
        assert_eq!(materials.library.entries.len(), 35);
        let lookup = environment.material_lookup();
        let (source, ss400) = lookup.find("SS400").unwrap();
        assert_eq!(source.library.info.id, "philippines-esc-materials");
        assert!(matches!(
            &ss400.record,
            DefinitionRecord::Material { standard, .. }
                if standard.as_deref() == Some("JIS G 3101:2015")
        ));
        assert!(matches!(
            &lookup.find("SM490YB").unwrap().1.record,
            DefinitionRecord::Material { standard, .. }
                if standard.as_deref() == Some("JIS G 3106:2020")
        ));
        assert!(matches!(
            &lookup.find("SN490C").unwrap().1.record,
            DefinitionRecord::Material { standard, .. }
                if standard.as_deref() == Some("JIS G 3136:2012")
        ));
        // A grade the ESC list does not name is NOT_FOUND, never a near match: no SN490A
        // (no grade of G 3136), no SM520B or SS540 of the full JIS sets, no A992 the usa
        // pack ships, no S450J0 the 2019 EN revision deleted, no bare SM400, and no
        // designation of the KS standards the page names (the same strings the JIS rows
        // carry already resolve there).
        for absent in [
            "SN490A", "SM520B", "SS540", "A992", "S450J0", "SM400", "SN400",
        ] {
            assert_eq!(
                lookup.find(absent).unwrap_err().code,
                ErrorCode::NotFound,
                "{absent}"
            );
        }
        // The material namespace never answers a profile name and the other way round: the
        // EDI inch designations, the jumbo rows of the newer A6 editions and the S family of
        // the annex stay NOT_FOUND in the material lookup too.
        for absent in ["W1100X499", "W44X408", "HP16X183", "S24X121"] {
            assert_eq!(
                lookup.find(absent).unwrap_err().code,
                ErrorCode::NotFound,
                "{absent}"
            );
        }
    }

    #[test]
    fn the_shipped_china_gb_t_1591_materials_library_reads_with_every_entry_admitted() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/china/materials-gb-t-1591.usda"
        );
        let library = load_material(path).unwrap();
        assert_eq!(library.info.id, "china-gb-t-1591-materials");
        assert_eq!(library.info.version.as_deref(), Some("1"));
        assert!(
            library
                .info
                .licence
                .as_deref()
                .is_some_and(|l| !l.is_empty()),
            "the shipped library declares its licence"
        );
        assert!(
            library
                .info
                .source
                .as_deref()
                .is_some_and(|s| !s.is_empty()),
            "the shipped library declares its source"
        );
        // No transcription slips: every entry is admitted, fifty-three designations.
        assert_eq!(library.rejected, []);
        assert_eq!(library.entries.len(), 53);
        // The complete published grade set of GB/T 1591-2018 in the loader's designation
        // order: per strength level the as-rolled grades (the AR code omitted) first, then
        // the thermomechanical M and the normalizing N families, each with the quality
        // grades its table lists — the sections-and-bars-only Q460 as-rolled grade has C
        // only, and the Q500-to-Q690 levels exist in the thermomechanical family only. The
        // designation system is clause 4: Q + the minimum upper yield strength in MPa + the
        // delivery status code + the quality grade.
        let gb = "GB/T 1591-2018";
        let expected: [&str; 53] = [
            "Q355B", "Q355C", "Q355D", "Q355MB", "Q355MC", "Q355MD", "Q355ME", "Q355MF", "Q355NB",
            "Q355NC", "Q355ND", "Q355NE", "Q355NF", "Q390B", "Q390C", "Q390D", "Q390MB", "Q390MC",
            "Q390MD", "Q390ME", "Q390NB", "Q390NC", "Q390ND", "Q390NE", "Q420B", "Q420C", "Q420MB",
            "Q420MC", "Q420MD", "Q420ME", "Q420NB", "Q420NC", "Q420ND", "Q420NE", "Q460C",
            "Q460MC", "Q460MD", "Q460ME", "Q460NC", "Q460ND", "Q460NE", "Q500MC", "Q500MD",
            "Q500ME", "Q550MC", "Q550MD", "Q550ME", "Q620MC", "Q620MD", "Q620ME", "Q690MC",
            "Q690MD", "Q690ME",
        ];
        for (designation, entry) in expected.iter().zip(&library.entries) {
            let DefinitionRecord::Material {
                density_kg_m3,
                grade,
                standard,
                display_name,
            } = &entry.record
            else {
                panic!("{} is not a material", entry.designation);
            };
            assert_eq!(entry.designation, *designation);
            assert_eq!(grade.as_deref(), Some(*designation));
            assert_eq!(*density_kg_m3, Some(7850.0), "{}", designation);
            assert_eq!(standard.as_deref(), Some(gb), "{}", designation);
            assert_eq!(display_name.as_deref(), Some(*designation));
        }
        assert_eq!(library.entries.len(), expected.len());
    }

    #[test]
    fn the_shipped_china_environment_admits_its_material_library() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/china/environment.usda"
        );
        let environment = load_environment(path).unwrap();
        assert_eq!(
            environment.info.material_libraries,
            ["materials-gb-t-1591.usda"]
        );
        // The profile and fastener namespaces are unchanged by the material listing.
        assert_eq!(environment.sources.len(), 2);
        assert_eq!(environment.fastener_sources.len(), 1);
        assert_eq!(environment.material_sources.len(), 1);
        let materials = &environment.material_sources[0];
        assert_eq!(materials.library.info.id, "china-gb-t-1591-materials");
        assert_eq!(materials.library.entries.len(), 53);
        assert!(materials.library.rejected.is_empty());
        // A grade rides the same resolver machinery, in its own namespace.
        let lookup = environment.material_lookup();
        let (source, q355) = lookup.find("Q355ND").unwrap();
        assert_eq!(source.library.info.id, "china-gb-t-1591-materials");
        assert!(matches!(
            &q355.record,
            DefinitionRecord::Material { density_kg_m3: Some(7850.0), standard, .. }
                if standard.as_deref() == Some("GB/T 1591-2018")
        ));
        // A grade the set does not publish is NOT_FOUND, never a near match: no bare
        // steel-grade without a quality grade, no designation of the withdrawn 2008 edition
        // (Q345 became Q355), no lowercase, no quality grade a table omits (the as-rolled
        // Q420 stops at C, the high-strength levels exist only in the thermomechanical
        // family) and no grade of another standard.
        for absent in [
            "Q355", "Q345B", "Q345", "q355b", "Q420D", "Q420F", "Q460B", "Q500B", "Q500NB",
            "Q690NE", "Q960", "S355",
        ] {
            assert_eq!(
                lookup.find(absent).unwrap_err().code,
                ErrorCode::NotFound,
                "{absent}"
            );
        }
    }

    #[test]
    fn the_shipped_australia_as_nzs_3678_materials_library_reads_with_every_entry_admitted() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/australia/materials-as-nzs-3678.usda"
        );
        let library = load_material(path).unwrap();
        assert_eq!(library.info.id, "australia-as-nzs-3678-materials");
        assert_eq!(library.info.version.as_deref(), Some("1"));
        assert!(
            library
                .info
                .licence
                .as_deref()
                .is_some_and(|l| !l.is_empty()),
            "the shipped library declares its licence"
        );
        assert!(
            library
                .info
                .source
                .as_deref()
                .is_some_and(|s| !s.is_empty()),
            "the shipped library declares its source"
        );
        // No transcription slips: every entry is admitted.
        assert_eq!(library.rejected, []);
        assert_eq!(library.entries.len(), 5);
        // The structural base grades of AS/NZS 3678-2016 in the loader's designation order,
        // the bare strength-level numbers (the nominal minimum yield in MPa) the ASI
        // specification lists for plates and flats. The 300 and 400 grades are the steels the
        // welded beams and columns of AS/NZS 3679.2 are welded from; the impact-test L
        // suffixes, the Z values, the AS/NZS 1594 product grades and the weathering WR350
        // product are not in the pack.
        let asnzs = "AS/NZS 3678-2016";
        let expected: [(&str, &str); 5] = [
            ("250", asnzs),
            ("300", asnzs),
            ("350", asnzs),
            ("400", asnzs),
            ("450", asnzs),
        ];
        for ((designation, want_standard), entry) in expected.iter().zip(&library.entries) {
            let DefinitionRecord::Material {
                density_kg_m3,
                grade,
                standard,
                display_name,
            } = &entry.record
            else {
                panic!("{} is not a material", entry.designation);
            };
            assert_eq!(entry.designation, *designation);
            assert_eq!(grade.as_deref(), Some(*designation));
            assert_eq!(*density_kg_m3, Some(7850.0), "{}", designation);
            assert_eq!(standard.as_deref(), Some(*want_standard), "{}", designation);
            assert_eq!(display_name.as_deref(), Some(*designation));
        }
        assert_eq!(library.entries.len(), expected.len());
    }

    #[test]
    fn the_shipped_australia_environment_admits_its_material_library() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/australia/environment.usda"
        );
        let environment = load_environment(path).unwrap();
        assert_eq!(
            environment.info.material_libraries,
            ["materials-as-nzs-3678.usda"]
        );
        // The profile and fastener namespaces are unchanged by the material listing.
        assert_eq!(environment.sources.len(), 2);
        assert_eq!(environment.fastener_sources.len(), 1);
        assert_eq!(environment.material_sources.len(), 1);
        let materials = &environment.material_sources[0];
        assert_eq!(materials.library.info.id, "australia-as-nzs-3678-materials");
        assert_eq!(materials.library.entries.len(), 5);
        assert!(materials.library.rejected.is_empty());
        // A grade rides the same resolver machinery, in its own namespace.
        let lookup = environment.material_lookup();
        let (source, grade_350) = lookup.find("350").unwrap();
        assert_eq!(source.library.info.id, "australia-as-nzs-3678-materials");
        assert!(matches!(
            &grade_350.record,
            DefinitionRecord::Material { density_kg_m3: Some(7850.0), standard, .. }
                if standard.as_deref() == Some("AS/NZS 3678-2016")
        ));
        // A grade the set does not publish is NOT_FOUND, never a near match: no qualifier
        // forms (the NZ catalogue G-prefix and the Grade word, the AS/NZS 1594 HA form), no
        // impact suffix, no Z value, no lighter grade outside the shipped set and no grade of
        // another standard.
        for absent in [
            "G350",
            "Grade 350",
            "HA250",
            "HW350",
            "C350L0",
            "350L15",
            "350Z25",
            "200",
            "500",
            "A36",
        ] {
            assert_eq!(
                lookup.find(absent).unwrap_err().code,
                ErrorCode::NotFound,
                "{absent}"
            );
        }
    }

    #[test]
    fn the_shipped_india_is_2062_materials_library_reads_with_every_entry_admitted() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/india/materials-is-2062.usda"
        );
        let library = load_material(path).unwrap();
        assert_eq!(library.info.id, "india-is-2062-materials");
        assert_eq!(library.info.version.as_deref(), Some("1"));
        assert!(
            library
                .info
                .licence
                .as_deref()
                .is_some_and(|l| !l.is_empty()),
            "the shipped library declares its licence"
        );
        assert!(
            library
                .info
                .source
                .as_deref()
                .is_some_and(|s| !s.is_empty()),
            "the shipped library declares its source"
        );
        // No transcription slips: every entry is admitted, twenty-eight designations.
        assert_eq!(library.rejected, []);
        assert_eq!(library.entries.len(), 28);
        // The complete published grade set of IS 2062:2011 in the loader's designation order
        // (byte order: the B0 digit sorts before the BR letter): the five lower levels carry
        // the four sub-qualities A, BR, B0 and C, the higher levels E450 to E650 only A and
        // BR — the sub-qualities indicate impact-test requirement and de-oxidation mode.
        // The 2011 edition replaced the 2006 Fe designations with the minimum-yield E system,
        // and its top levels are E600 and E650, not the E590 some second-hand lists name.
        let india = "IS 2062-2011";
        let expected: [&str; 28] = [
            "E250A", "E250B0", "E250BR", "E250C", "E275A", "E275B0", "E275BR", "E275C", "E300A",
            "E300B0", "E300BR", "E300C", "E350A", "E350B0", "E350BR", "E350C", "E410A", "E410B0",
            "E410BR", "E410C", "E450A", "E450BR", "E550A", "E550BR", "E600A", "E600BR", "E650A",
            "E650BR",
        ];
        for (designation, entry) in expected.iter().zip(&library.entries) {
            let DefinitionRecord::Material {
                density_kg_m3,
                grade,
                standard,
                display_name,
            } = &entry.record
            else {
                panic!("{} is not a material", entry.designation);
            };
            assert_eq!(entry.designation, *designation);
            assert_eq!(grade.as_deref(), Some(*designation));
            assert_eq!(*density_kg_m3, Some(7850.0), "{}", designation);
            assert_eq!(standard.as_deref(), Some(india), "{}", designation);
            assert_eq!(display_name.as_deref(), Some(*designation));
        }
        assert_eq!(library.entries.len(), expected.len());
    }

    #[test]
    fn the_shipped_india_environment_admits_its_material_library() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/india/environment.usda"
        );
        let environment = load_environment(path).unwrap();
        assert_eq!(
            environment.info.material_libraries,
            ["materials-is-2062.usda"]
        );
        // The profile and fastener namespaces are unchanged by the material listing.
        assert_eq!(environment.sources.len(), 4);
        assert_eq!(environment.fastener_sources.len(), 1);
        assert_eq!(environment.material_sources.len(), 1);
        let materials = &environment.material_sources[0];
        assert_eq!(materials.library.info.id, "india-is-2062-materials");
        assert_eq!(materials.library.entries.len(), 28);
        assert!(materials.library.rejected.is_empty());
        // A grade rides the same resolver machinery, in its own namespace.
        let lookup = environment.material_lookup();
        let (source, e350) = lookup.find("E350C").unwrap();
        assert_eq!(source.library.info.id, "india-is-2062-materials");
        assert!(matches!(
            &e350.record,
            DefinitionRecord::Material { density_kg_m3: Some(7850.0), standard, .. }
                if standard.as_deref() == Some("IS 2062-2011")
        ));
        // A grade the set does not publish is NOT_FOUND, never a near match: no 2006-edition
        // Fe designations, no bare grade without a sub-quality, no lowercase, no level the
        // edition does not publish, no sub-quality the higher levels omit and no grade of
        // another standard.
        for absent in [
            "Fe410WA", "Fe490A", "Fe590", "E250", "e350c", "E250D", "E590", "E450C", "E550C",
            "E600C", "E650C", "S355",
        ] {
            assert_eq!(
                lookup.find(absent).unwrap_err().code,
                ErrorCode::NotFound,
                "{absent}"
            );
        }
    }

    #[test]
    fn the_shipped_middle_east_library_reads_with_every_entry_admitted() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/middle-east/profiles-en-10365.usda"
        );
        let library = load(path).unwrap();
        assert_eq!(library.info.id, "middle-east-en-10365-profiles");
        assert_eq!(library.info.version.as_deref(), Some("2"));
        assert!(
            library
                .info
                .licence
                .as_deref()
                .is_some_and(|l| !l.is_empty()),
            "the shipped library declares its licence"
        );
        assert!(
            library
                .info
                .source
                .as_deref()
                .is_some_and(|s| !s.is_empty()),
            "the shipped library declares its source"
        );
        // No transcription slips: every entry is admitted and none warns.
        assert_eq!(library.rejected, []);
        assert_eq!(library.entries.len(), 175);
        let mut designations = Vec::new();
        for entry in &library.entries {
            let DefinitionRecord::SectionProfile {
                shape: SectionShape::Parametric(parametric),
                section,
            } = &entry.record
            else {
                panic!("{} is not a parametric section", entry.designation);
            };
            assert_eq!(parametric.family(), SectionFamily::IShape);
            // The curated mix declares the edition the regional producer prints, on every row.
            assert_eq!(
                section.standard.as_deref(),
                Some("EN 10365:2017"),
                "{}",
                entry.designation
            );
            // The D3 declared-area check stays silent (the declared areas are the standard's
            // published values, the exact-filleted convention by default; the EN tables print
            // no HL root radius, so the HL rows carry the radius reproducing the published
            // area) and the declared mass is that area at 7850 kg/m3 within the published
            // rounding — except the HL, HLZ, HD and HP rows, which carry the designation
            // nominal masses the rounded published areas track within 2.2 percent (worst
            // HP400x122 at 2.19 percent, HD360x147 at 1.91, HL920x368 at 0.60), so those rows
            // are bounded relatively, as in the `europe` pack.
            assert!(
                entry.record.declared_area_warning().is_none(),
                "{}",
                entry.designation
            );
            let area = section.area_mm2.unwrap();
            let mass = section.mass_per_meter_kg.unwrap();
            let mass_from_area = area * 7850.0 / 1.0e6;
            if entry.designation.starts_with("HL")
                || entry.designation.starts_with("HD")
                || entry.designation.starts_with("HP")
            {
                assert!(
                    (mass - mass_from_area).abs() / mass_from_area <= 0.025,
                    "{}: mass {mass} kg/m vs area-derived {mass_from_area}",
                    entry.designation
                );
            } else {
                assert!(
                    (mass - mass_from_area).abs() <= 0.55,
                    "{}: mass {mass} kg/m vs area-derived {mass_from_area}",
                    entry.designation
                );
            }
            designations.push(entry.designation.clone());
        }
        // The rows of the extended `europe` pack, in the loader's designation order: the HEA and
        // HEB ladder gains its 650-1000 rows (still one shared ladder, no 250 or 270) and IPE
        // keeps its own (no 250 or 280; 270 and 330 included); the HD, HL and HP designations
        // the #790 extension adds sort around them, the x-suffix rows lexically (the deep HD
        // rows lead their groups), and the HL list closes with the four HLZ rows, its lexical
        // position.
        let hd = [
            "HD260x114",
            "HD260x142",
            "HD260x172",
            "HD260x225",
            "HD260x299",
            "HD260x54.1",
            "HD260x68.2",
            "HD260x93.0",
            "HD320x127",
            "HD320x158",
            "HD320x198",
            "HD320x245",
            "HD320x300",
            "HD320x74.2",
            "HD320x97.6",
            "HD360x134",
            "HD360x147",
            "HD360x162",
            "HD360x179",
            "HD360x196",
            "HD400x1086",
            "HD400x187",
            "HD400x216",
            "HD400x237",
            "HD400x262",
            "HD400x287",
            "HD400x314",
            "HD400x347",
            "HD400x382",
            "HD400x421",
            "HD400x463",
            "HD400x509",
            "HD400x551",
            "HD400x592",
            "HD400x634",
            "HD400x677",
            "HD400x744",
            "HD400x818",
            "HD400x900",
            "HD400x990",
        ];
        let hl = [
            "HL1000A",
            "HL1000AA",
            "HL1000B",
            "HL1000M",
            "HL1000x443",
            "HL1000x483",
            "HL1000x539",
            "HL1000x554",
            "HL1000x591",
            "HL1000x642",
            "HL1000x748",
            "HL1000x883",
            "HL1000x976",
            "HL1100A",
            "HL1100B",
            "HL1100M",
            "HL1100R",
            "HL1100x548",
            "HL1100x607",
            "HL920x1077",
            "HL920x1194",
            "HL920x1269",
            "HL920x1377",
            "HL920x344",
            "HL920x368",
            "HL920x390",
            "HL920x420",
            "HL920x449",
            "HL920x491",
            "HL920x537",
            "HL920x588",
            "HL920x656",
            "HL920x725",
            "HL920x787",
            "HL920x970",
            "HLZ1100A",
            "HLZ1100B",
            "HLZ1100C",
            "HLZ1100D",
        ];
        let hp = [
            "HP200x43",
            "HP200x53",
            "HP220x57",
            "HP260x75",
            "HP260x87",
            "HP305x110",
            "HP305x126",
            "HP305x149",
            "HP305x180",
            "HP305x186",
            "HP305x223",
            "HP305x79",
            "HP305x88",
            "HP305x95",
            "HP320x103",
            "HP320x117",
            "HP320x147",
            "HP320x184",
            "HP320x88",
            "HP360x109",
            "HP360x133",
            "HP360x152",
            "HP360x174",
            "HP360x180",
            "HP400x122",
            "HP400x140",
            "HP400x158",
            "HP400x176",
            "HP400x194",
            "HP400x213",
            "HP400x231",
        ];
        let he = [
            100, 120, 140, 160, 180, 200, 220, 240, 260, 280, 300, 320, 340, 360, 400, 450, 500,
            550, 600, 650, 700, 800, 900, 1000,
        ];
        let ipe = [
            100, 120, 140, 160, 180, 200, 220, 240, 270, 300, 330, 360, 400, 450, 500, 550, 600,
        ];
        let mut expected: Vec<String> = hd.iter().map(|d| d.to_string()).collect();
        expected.extend(he.iter().map(|h| format!("HEA{h}")));
        expected.extend(he.iter().map(|h| format!("HEB{h}")));
        expected.extend(hl.iter().map(|d| d.to_string()));
        expected.extend(hp.iter().map(|d| d.to_string()));
        expected.extend(ipe.iter().map(|h| format!("IPE{h}")));
        // The loader's order is lexical, so the 1000 rows slot in right after the 100 rows.
        expected.sort();
        assert_eq!(designations, expected);
        // Spot checks against the standard's published values, one per family of the #790
        // extension on top of the original three.
        for (designation, values, area, mass) in [
            ("IPE300", [300.0, 150.0, 7.1, 10.7, 15.0], 5381.0, 42.2),
            ("HEA340", [330.0, 300.0, 9.5, 16.5, 27.0], 13350.0, 105.0),
            ("HEB100", [100.0, 100.0, 6.0, 10.0, 12.0], 2600.0, 20.4),
            (
                "HL920x491",
                [957.0, 422.0, 25.9, 47.0, 19.0],
                62300.0,
                491.0,
            ),
            (
                "HD400x818",
                [514.0, 437.0, 60.5, 97.0, 24.0],
                104630.0,
                818.0,
            ),
            (
                "HLZ1100D",
                [1087.0, 460.0, 22.0, 37.0, 35.0],
                57800.0,
                453.0,
            ),
            (
                "HP400x231",
                [372.0, 402.0, 26.0, 26.0, 24.0],
                29720.0,
                231.0,
            ),
            ("HEA1000", [990.0, 300.0, 16.5, 31.0, 30.0], 34685.0, 272.3),
        ] {
            let DefinitionRecord::SectionProfile {
                shape: SectionShape::Parametric(p),
                section,
            } = &library.entry(designation).unwrap().record
            else {
                panic!("{designation} is not a parametric section");
            };
            assert_eq!(p.values(), values, "{designation}");
            assert_eq!(section.area_mm2, Some(area), "{designation}");
            assert_eq!(section.mass_per_meter_kg, Some(mass), "{designation}");
        }
        // NOT_FOUND, never a near match: designations the series does not publish (no IPE 250 or
        // 280, no HEA 250), sizes beyond the shipped range (IPE 650, HEA 1100), the HEM series
        // the #790 extension leaves out, and the channel series the curated mix leaves to the
        // `europe` pack.
        for absent in [
            "IPE250", "IPE280", "HEA250", "IPE650", "HEA1100", "HEM300", "UPE100", "UPN100",
        ] {
            assert_eq!(
                library.entry(absent).unwrap_err().code,
                ErrorCode::NotFound,
                "{absent} is not in the pack"
            );
        }
    }

    #[test]
    fn the_shipped_middle_east_materials_library_reads_with_every_entry_admitted() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/middle-east/materials-astm.usda"
        );
        let library = load_material(path).unwrap();
        assert_eq!(library.info.id, "middle-east-astm-materials");
        assert_eq!(library.info.version.as_deref(), Some("1"));
        assert!(
            library
                .info
                .licence
                .as_deref()
                .is_some_and(|l| !l.is_empty()),
            "the shipped library declares its licence"
        );
        assert!(
            library
                .info
                .source
                .as_deref()
                .is_some_and(|s| !s.is_empty()),
            "the shipped library declares its source"
        );
        // No transcription slips: every entry is admitted.
        assert_eq!(library.rejected, []);
        assert_eq!(library.entries.len(), 7);
        // The grades of the three standards in the loader's designation order: the single A36,
        // the five A572 strength grades (42 to 65, the grade number the minimum yield strength
        // in ksi) with the Grade word the standard designates with, and the W-shape grade A992.
        // Each row declares the edition the regional producer prints on its grade rows (owner
        // decision of 2026-11-23); the density is the metric 7850 kg/m3 of every shipped pack.
        let a36 = "ASTM A36/A36M-19";
        let a572 = "ASTM A572/A572M-21";
        let a992 = "ASTM A992/A992M-20";
        let expected: [(&str, &str); 7] = [
            ("A36", a36),
            ("A572 Grade 42", a572),
            ("A572 Grade 50", a572),
            ("A572 Grade 55", a572),
            ("A572 Grade 60", a572),
            ("A572 Grade 65", a572),
            ("A992", a992),
        ];
        for ((designation, want_standard), entry) in expected.iter().zip(&library.entries) {
            let DefinitionRecord::Material {
                density_kg_m3,
                grade,
                standard,
                display_name,
            } = &entry.record
            else {
                panic!("{} is not a material", entry.designation);
            };
            assert_eq!(entry.designation, *designation);
            assert_eq!(grade.as_deref(), Some(*designation));
            assert_eq!(*density_kg_m3, Some(7850.0), "{}", designation);
            assert_eq!(standard.as_deref(), Some(*want_standard), "{}", designation);
            assert_eq!(display_name.as_deref(), Some(*designation));
        }
        assert_eq!(library.entries.len(), expected.len());
    }

    #[test]
    fn the_shipped_middle_east_environment_admits_its_libraries() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/middle-east/environment.usda"
        );
        let environment = load_environment(path).unwrap();
        assert_eq!(environment.info.id, "middle-east");
        assert_eq!(environment.info.version.as_deref(), Some("1"));
        assert_eq!(environment.info.libraries, ["profiles-en-10365.usda"]);
        assert_eq!(environment.info.material_libraries, ["materials-astm.usda"]);
        // The curated mix: one EN 10365 profile library and one ASTM material library. The
        // environment lists no fastener library; the regional fastener set waits on #708.
        assert_eq!(environment.sources.len(), 1);
        assert_eq!(environment.fastener_sources.len(), 0);
        assert_eq!(environment.material_sources.len(), 1);
        let profiles = &environment.sources[0];
        assert_eq!(profiles.library.info.id, "middle-east-en-10365-profiles");
        assert_eq!(profiles.library.entries.len(), 175);
        assert!(profiles.library.rejected.is_empty());
        let materials = &environment.material_sources[0];
        assert_eq!(materials.library.info.id, "middle-east-astm-materials");
        assert_eq!(materials.library.entries.len(), 7);
        assert!(materials.library.rejected.is_empty());
        // A grade rides the same resolver machinery, in its own namespace.
        let lookup = environment.material_lookup();
        let (source, a992) = lookup.find("A992").unwrap();
        assert_eq!(source.library.info.id, "middle-east-astm-materials");
        assert!(matches!(
            &a992.record,
            DefinitionRecord::Material { density_kg_m3: Some(7850.0), standard, .. }
                if standard.as_deref() == Some("ASTM A992/A992M-20")
        ));
        // A grade the set does not publish is NOT_FOUND, never a near match: no bare spec
        // number, no hyphenated or compact industry shorthand (the shipped A572 form is the
        // standard's own Grade form), no lowercase, no grade outside the standard's set, no
        // grade of another ASTM specification and no EN 10025 grade (the curated mix ships the
        // ASTM grades alone).
        for absent in [
            "A572",
            "A572-50",
            "A572Gr50",
            "A36M",
            "a992",
            "A572 Grade 70",
            "A709",
            "S355J2",
        ] {
            assert_eq!(
                lookup.find(absent).unwrap_err().code,
                ErrorCode::NotFound,
                "{absent}"
            );
        }
    }

    #[test]
    fn an_environment_library_without_provenance_is_refused() {
        let dir =
            std::env::temp_dir().join(format!("library-env-provenance-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let write = |name: &str, text: &str| {
            std::fs::write(dir.join(name), text).unwrap();
        };
        // A profile library whose licence is missing, and a fastener one whose source is.
        let library = |id: &str, scope: &str, entry_body: &str, declared: &str| {
            format!(
                "#usda 1.0\n(\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n    customLayerData = {{\n        string \"bim:library:id\" = \"{id}\"{declared}\n    }}\n)\ndef Scope \"Library\" {{\n    def Scope \"{scope}\" {{\n{entry_body}\n    }}\n}}\n"
            )
        };
        let profile = library(
            "p",
            "Profiles",
            &entry("A", "P-1", I_BODY),
            "\n        string \"bim:library:source\" = \"s\"",
        );
        let complete = library(
            "pc",
            "Profiles",
            &entry("A", "P-1", I_BODY),
            "\n        string \"bim:library:source\" = \"s\"\n        string \"bim:library:licence\" = \"MPL-2.0\"",
        );
        let fastener = library(
            "f",
            "Fasteners",
            &fastener_entry("B", "F-1", BOLT_BODY),
            "\n        string \"bim:library:licence\" = \"MPL-2.0\"",
        );
        // A material library whose source is missing rides the same provenance edge (#636).
        let material = library(
            "m",
            "Materials",
            &material_entry("C", "S235JR", MATERIAL_BODY),
            "\n        string \"bim:library:source\" = \"s\"",
        );
        write("profiles-nolicence.usda", &profile);
        write("profiles-complete.usda", &complete);
        write("fasteners-nosource.usda", &fastener);
        write("materials-nolicence.usda", &material);
        let env = |profiles: &str, fasteners: Option<&str>, materials: Option<&str>| {
            format!(
                "#usda 1.0\n(\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n    customLayerData = {{\n        string \"bim:environment:id\" = \"e\"\n        string[] \"bim:environment:libraries\" = [\"{profiles}\"]{}{}\n    }}\n)\n",
                match fasteners {
                    Some(name) => format!(
                        "\n        string[] \"bim:environment:fastener-libraries\" = [\"{name}\"]"
                    ),
                    None => String::new(),
                },
                match materials {
                    Some(name) => format!(
                        "\n        string[] \"bim:environment:material-libraries\" = [\"{name}\"]"
                    ),
                    None => String::new(),
                }
            )
        };
        write(
            "no-profile-licence.usda",
            &env("profiles-nolicence.usda", None, None),
        );
        write(
            "no-fastener-source.usda",
            &env(
                "profiles-complete.usda",
                Some("fasteners-nosource.usda"),
                None,
            ),
        );
        write(
            "no-material-licence.usda",
            &env(
                "profiles-complete.usda",
                None,
                Some("materials-nolicence.usda"),
            ),
        );
        let refused =
            load_environment(dir.join("no-profile-licence.usda").to_str().unwrap()).unwrap_err();
        assert_eq!(refused.code, ErrorCode::InvalidArgument);
        assert!(
            refused.message.contains("licence"),
            "the missing record is named: {refused:?}"
        );
        let refused =
            load_environment(dir.join("no-fastener-source.usda").to_str().unwrap()).unwrap_err();
        assert_eq!(refused.code, ErrorCode::InvalidArgument);
        assert!(
            refused.message.contains("source"),
            "the fastener library's missing record is named: {refused:?}"
        );
        let refused =
            load_environment(dir.join("no-material-licence.usda").to_str().unwrap()).unwrap_err();
        assert_eq!(refused.code, ErrorCode::InvalidArgument);
        assert!(
            refused.message.contains("licence"),
            "the material library's missing record is named: {refused:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_shipped_russia_library_reads_with_every_entry_admitted() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/russia/profiles-gost-26020.usda"
        );
        let library = load(path).unwrap();
        assert_eq!(library.info.id, "russia-gost-26020-profiles");
        assert_eq!(library.info.version.as_deref(), Some("2"));
        assert!(
            library
                .info
                .licence
                .as_deref()
                .is_some_and(|l| !l.is_empty()),
            "the shipped library declares its licence"
        );
        assert!(
            library
                .info
                .source
                .as_deref()
                .is_some_and(|s| !s.is_empty()),
            "the shipped library declares its source"
        );
        // No transcription slips: every entry is admitted and none warns.
        assert_eq!(library.rejected, []);
        assert_eq!(library.entries.len(), 81);
        let mut designations = Vec::new();
        for entry in &library.entries {
            let DefinitionRecord::SectionProfile {
                shape: SectionShape::Parametric(parametric),
                section,
            } = &entry.record
            else {
                panic!("{} is not a parametric section", entry.designation);
            };
            assert_eq!(parametric.family(), SectionFamily::IShape);
            assert_eq!(
                section.standard.as_deref(),
                Some("GOST 26020-83"),
                "{}",
                entry.designation
            );
            // The closed-form area of the declared dimensions must match the declared catalogue
            // area (the D3 1 percent warning never fires) and the declared mass must be that
            // area at 7850 kg/m3 within the standard's one-decimal rounding. 40K3 and 70B2 sit
            // 0.073 and 0.074 off, and among the Ш rows 50Sh2, 50Sh4, 60Sh1, 70Sh3 and 70Sh4 sit
            // 0.05..0.07 off, because GOST itself truncated or rounded from unrounded geometry
            // there; the published pairs are taken as-is.
            assert!(
                entry.record.declared_area_warning().is_none(),
                "{}",
                entry.designation
            );
            let area = section.area_mm2.unwrap();
            let mass = section.mass_per_meter_kg.unwrap();
            let mass_from_area = area * 7850.0 / 1.0e6;
            assert!(
                (mass - mass_from_area).abs() <= 0.08,
                "{}: mass {mass} kg/m vs area-derived {mass_from_area}",
                entry.designation
            );
            designations.push(entry.designation.clone());
        }
        // The beam (B), wide-flange (Sh) and column (K) series exactly as GOST 26020-83 publishes
        // them: 37 + 26 + 18 designations. The designations that belong only to the newer
        // GOST R 57837-2017 (30K4, 35K4, 50K1..50K5, 80Sh1..100Sh5) and the unshipped Д series
        // are not here.
        let mut expected = vec![
            "10B1", "12B1", "12B2", "14B1", "14B2", "16B1", "16B2", "18B1", "18B2", "20B1", "23B1",
            "26B1", "26B2", "30B1", "30B2", "35B1", "35B2", "40B1", "40B2", "45B1", "45B2", "50B1",
            "50B2", "55B1", "55B2", "60B1", "60B2", "70B1", "70B2", "80B1", "80B2", "90B1", "90B2",
            "100B1", "100B2", "100B3", "100B4", "20Sh1", "23Sh1", "26Sh1", "26Sh2", "30Sh1",
            "30Sh2", "30Sh3", "35Sh1", "35Sh2", "35Sh3", "40Sh1", "40Sh2", "40Sh3", "50Sh1",
            "50Sh2", "50Sh3", "50Sh4", "60Sh1", "60Sh2", "60Sh3", "60Sh4", "70Sh1", "70Sh2",
            "70Sh3", "70Sh4", "70Sh5", "20K1", "20K2", "23K1", "23K2", "26K1", "26K2", "26K3",
            "30K1", "30K2", "30K3", "35K1", "35K2", "35K3", "40K1", "40K2", "40K3", "40K4", "40K5",
        ];
        expected.sort();
        assert_eq!(designations, expected);
        // Spot checks against the transcribed values, including the owner's mass-decision
        // designations of this standard.
        let spot = |designation: &str, values: [f64; 5], area: f64, mass: f64| {
            let entry = library.entry(designation).unwrap();
            let DefinitionRecord::SectionProfile {
                shape: SectionShape::Parametric(p),
                section,
            } = &entry.record
            else {
                panic!("{designation} is not a parametric section");
            };
            assert_eq!(p.values(), values, "{designation}");
            assert_eq!(section.area_mm2, Some(area), "{designation}");
            assert_eq!(section.mass_per_meter_kg, Some(mass), "{designation}");
        };
        spot("30B1", [296.0, 140.0, 5.8, 8.5, 15.0], 4192.0, 32.9);
        spot("100B4", [1013.0, 320.0, 19.5, 32.5, 30.0], 40060.0, 314.5);
        spot("20Sh1", [193.0, 150.0, 6.0, 9.0, 13.0], 3895.0, 30.6);
        spot("30Sh1", [291.0, 200.0, 8.0, 11.0, 18.0], 6831.0, 53.6);
        spot("70Sh5", [718.0, 320.0, 23.0, 36.5, 30.0], 38970.0, 305.9);
        spot("20K1", [195.0, 200.0, 6.5, 10.0, 13.0], 5282.0, 41.5);
        spot("40K5", [431.0, 400.0, 23.0, 35.5, 22.0], 37100.0, 291.2);
        for absent in [
            "30K4", "35K4", "50K1", "20B2", "25B1", "80Sh1", "30Ш1", "24ДБ1",
        ] {
            assert_eq!(
                library.entry(absent).unwrap_err().code,
                ErrorCode::NotFound,
                "{absent} is not in the shipped series"
            );
        }
    }

    #[test]
    fn the_shipped_russia_gost_30245_library_reads_with_every_entry_admitted() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/russia/profiles-gost-30245.usda"
        );
        let library = load(path).unwrap();
        assert_eq!(library.info.id, "russia-gost-30245-profiles");
        assert_eq!(library.info.version.as_deref(), Some("7"));
        assert!(
            library
                .info
                .licence
                .as_deref()
                .is_some_and(|l| !l.is_empty()),
            "the shipped library declares its licence"
        );
        assert!(
            library
                .info
                .source
                .as_deref()
                .is_some_and(|s| !s.is_empty()),
            "the shipped library declares its source"
        );
        // No transcription slips: every entry is admitted and none warns.
        assert_eq!(library.rejected, []);
        assert_eq!(library.entries.len(), 580);
        let mut designations = Vec::new();
        for entry in &library.entries {
            let DefinitionRecord::SectionProfile {
                shape: SectionShape::Parametric(parametric),
                section,
            } = &entry.record
            else {
                panic!("{} is not a parametric section", entry.designation);
            };
            assert_eq!(parametric.family(), SectionFamily::Rhs);
            assert_eq!(
                section.standard.as_deref(),
                Some("GOST 30245-2003"),
                "{}",
                entry.designation
            );
            // The exact per-row validator: the declared radius carries the standard's corner
            // rule (notes to tables 1 and 2: R = 2t up to 6 mm wall, 2.5t from 6 to 10 mm,
            // 3t above 10 mm), so the closed-form area of the declared dimensions must
            // reproduce the declared catalogue area within the table's print rounding (two
            // decimals below 100 cm2, one decimal at and above it), and the declared mass
            // must be that closed-form area at the standard's 0.785 kg per m per cm2.
            let values = parametric.values();
            let [h, b, t, r] = [values[0], values[1], values[2], values[3]];
            assert!(h >= b, "{}: the tables publish h >= b", entry.designation);
            let area_exact_mm2 = h * b
                - (h - 2.0 * t) * (b - 2.0 * t)
                - (4.0 - std::f64::consts::PI) * t * (2.0 * r - t);
            let declared = section.area_mm2.unwrap();
            let area_tolerance = if declared < 10_000.0 { 0.5 } else { 5.0 };
            assert!(
                (declared - area_exact_mm2).abs() <= area_tolerance,
                "{}: declared area {declared} mm2 vs closed form {area_exact_mm2}",
                entry.designation
            );
            assert!(
                entry.record.declared_area_warning().is_none(),
                "{}",
                entry.designation
            );
            let mass = section.mass_per_meter_kg.unwrap();
            let mass_from_area = 0.785 * area_exact_mm2 / 100.0;
            let mass_tolerance = if mass < 100.0 { 0.011 } else { 0.055 };
            assert!(
                (mass - mass_from_area).abs() <= mass_tolerance,
                "{}: mass {mass} kg/m vs 0.785*A {mass_from_area}",
                entry.designation
            );
            designations.push(entry.designation.clone());
        }
        // Tables 1 and 2 exactly as GOST 30245-2003 publishes them: the complete square
        // series (table 1, sizes 40, 50, 60, 70, 80, 90, 100, 120, 140, 150, 160, 180, 200,
        // 250 and 300) and the rectangular series (table 2) entering by height bands, this
        // layer carrying the table 2 groups with h up to 80 mm, each group with its published
        // wall thicknesses in 0.5 mm steps.
        let mut expected = vec![
            "40x40x2",
            "40x40x2.5",
            "40x40x3",
            "40x40x3.5",
            "40x40x4",
            "50x50x2",
            "50x50x2.5",
            "50x50x3",
            "50x50x3.5",
            "50x50x4",
            "50x50x4.5",
            "50x50x5",
            "50x50x5.5",
            "50x50x6",
            "60x60x2",
            "60x60x2.5",
            "60x60x3",
            "60x60x3.5",
            "60x60x4",
            "60x60x4.5",
            "60x60x5",
            "60x60x5.5",
            "60x60x6",
            "70x70x2",
            "70x70x2.5",
            "70x70x3",
            "70x70x3.5",
            "70x70x4",
            "70x70x4.5",
            "70x70x5",
            "70x70x5.5",
            "70x70x6",
            "70x70x6.5",
            "70x70x7",
            "80x80x3",
            "80x80x3.5",
            "80x80x4",
            "80x80x4.5",
            "80x80x5",
            "80x80x5.5",
            "80x80x6",
            "80x80x6.5",
            "80x80x7",
            "80x80x7.5",
            "80x80x8",
            "90x90x3",
            "90x90x3.5",
            "90x90x4",
            "90x90x4.5",
            "90x90x5",
            "90x90x5.5",
            "90x90x6",
            "90x90x6.5",
            "90x90x7",
            "90x90x7.5",
            "90x90x8",
            "100x100x3",
            "100x100x3.5",
            "100x100x4",
            "100x100x4.5",
            "100x100x5",
            "100x100x5.5",
            "100x100x6",
            "100x100x6.5",
            "100x100x7",
            "100x100x7.5",
            "100x100x8",
            "120x120x3",
            "120x120x3.5",
            "120x120x4",
            "120x120x4.5",
            "120x120x5",
            "120x120x5.5",
            "120x120x6",
            "120x120x6.5",
            "120x120x7",
            "120x120x7.5",
            "120x120x8",
            "140x140x4",
            "140x140x4.5",
            "140x140x5",
            "140x140x5.5",
            "140x140x6",
            "140x140x6.5",
            "140x140x7",
            "140x140x7.5",
            "140x140x8",
            "150x150x4",
            "150x150x4.5",
            "150x150x5",
            "150x150x5.5",
            "150x150x6",
            "150x150x6.5",
            "150x150x7",
            "150x150x7.5",
            "150x150x8",
            "160x160x4",
            "160x160x4.5",
            "160x160x5",
            "160x160x5.5",
            "160x160x6",
            "160x160x6.5",
            "160x160x7",
            "160x160x7.5",
            "160x160x8",
            "180x180x5",
            "180x180x5.5",
            "180x180x6",
            "180x180x6.5",
            "180x180x7",
            "180x180x7.5",
            "180x180x8",
            "180x180x8.5",
            "180x180x9",
            "180x180x9.5",
            "180x180x10",
            "200x200x6",
            "200x200x6.5",
            "200x200x7",
            "200x200x7.5",
            "200x200x8",
            "200x200x8.5",
            "200x200x9",
            "200x200x9.5",
            "200x200x10",
            "200x200x10.5",
            "200x200x11",
            "200x200x11.5",
            "200x200x12",
            "250x250x6",
            "250x250x6.5",
            "250x250x7",
            "250x250x7.5",
            "250x250x8",
            "250x250x8.5",
            "250x250x9",
            "250x250x9.5",
            "250x250x10",
            "250x250x10.5",
            "250x250x11",
            "250x250x11.5",
            "250x250x12",
            "300x300x6",
            "300x300x6.5",
            "300x300x7",
            "300x300x7.5",
            "300x300x8",
            "300x300x8.5",
            "300x300x9",
            "300x300x9.5",
            "300x300x10",
            "300x300x10.5",
            "300x300x11",
            "300x300x11.5",
            "300x300x12",
            "50x25x2",
            "50x25x2.5",
            "50x25x3",
            "50x25x3.5",
            "50x25x4",
            "50x30x2",
            "50x30x2.5",
            "50x30x3",
            "50x30x3.5",
            "50x30x4",
            "50x30x5",
            "50x40x2",
            "50x40x2.5",
            "50x40x3",
            "50x40x3.5",
            "50x40x4",
            "50x40x4.5",
            "50x40x5",
            "60x30x2",
            "60x30x2.5",
            "60x30x3",
            "60x30x3.5",
            "60x30x4",
            "60x30x4.5",
            "60x30x5",
            "60x30x5.5",
            "60x30x6",
            "60x40x2",
            "60x40x2.5",
            "60x40x3",
            "60x40x3.5",
            "60x40x4",
            "60x40x4.5",
            "60x40x5",
            "60x40x5.5",
            "60x40x6",
            "70x50x2",
            "70x50x2.5",
            "70x50x3",
            "70x50x3.5",
            "70x50x4",
            "70x50x4.5",
            "70x50x5",
            "70x50x5.5",
            "70x50x6",
            "80x40x2",
            "80x40x2.5",
            "80x40x3",
            "80x40x3.5",
            "80x40x4",
            "80x40x4.5",
            "80x40x5",
            "80x40x5.5",
            "80x40x6",
            "80x60x2",
            "80x60x2.5",
            "80x60x3",
            "80x60x3.5",
            "80x60x4",
            "80x60x4.5",
            "80x60x5",
            "80x60x5.5",
            "80x60x6",
            "80x60x6.5",
            "80x60x7",
            "80x70x3",
            "80x70x3.5",
            "80x70x4",
            "80x70x4.5",
            "80x70x5",
            "80x70x5.5",
            "80x70x6",
            "80x70x6.5",
            "80x70x7",
            "90x50x3",
            "90x50x3.5",
            "90x50x4",
            "90x50x4.5",
            "90x50x5",
            "90x50x5.5",
            "90x50x6",
            "90x50x6.5",
            "90x50x7",
            "90x60x3",
            "90x60x3.5",
            "90x60x4",
            "90x60x4.5",
            "90x60x5",
            "90x60x5.5",
            "90x60x6",
            "90x60x6.5",
            "90x60x7",
            "100x40x3",
            "100x40x3.5",
            "100x40x4",
            "100x40x4.5",
            "100x40x5",
            "100x40x5.5",
            "100x40x6",
            "100x40x6.5",
            "100x40x7",
            "100x50x3",
            "100x50x3.5",
            "100x50x4",
            "100x50x4.5",
            "100x50x5",
            "100x50x5.5",
            "100x50x6",
            "100x50x6.5",
            "100x50x7",
            "100x60x3",
            "100x60x3.5",
            "100x60x4",
            "100x60x4.5",
            "100x60x5",
            "100x60x5.5",
            "100x60x6",
            "100x60x6.5",
            "100x60x7",
            "120x40x3",
            "120x40x3.5",
            "120x40x4",
            "120x40x4.5",
            "120x40x5",
            "120x40x5.5",
            "120x40x6",
            "120x40x6.5",
            "120x40x7",
            "120x60x3",
            "120x60x3.5",
            "120x60x4",
            "120x60x4.5",
            "120x60x5",
            "120x60x5.5",
            "120x60x6",
            "120x60x6.5",
            "120x60x7",
            "120x80x3",
            "120x80x3.5",
            "120x80x4",
            "120x80x4.5",
            "120x80x5",
            "120x80x5.5",
            "120x80x6",
            "120x80x6.5",
            "120x80x7",
            "140x60x3",
            "140x60x3.5",
            "140x60x4",
            "140x60x4.5",
            "140x60x5",
            "140x60x5.5",
            "140x60x6",
            "140x60x6.5",
            "140x60x7",
            "140x100x4",
            "140x100x4.5",
            "140x100x5",
            "140x100x5.5",
            "140x100x6",
            "140x100x6.5",
            "140x100x7",
            "140x120x4",
            "140x120x4.5",
            "140x120x5",
            "140x120x5.5",
            "140x120x6",
            "140x120x6.5",
            "140x120x7",
            "140x120x7.5",
            "140x120x8",
            "150x100x4",
            "150x100x4.5",
            "150x100x5",
            "150x100x5.5",
            "150x100x6",
            "150x100x6.5",
            "150x100x7",
            "160x40x3",
            "160x40x3.5",
            "160x40x4",
            "160x40x4.5",
            "160x40x5",
            "160x40x5.5",
            "160x40x6",
            "160x40x6.5",
            "160x40x7",
            "160x80x4",
            "160x80x4.5",
            "160x80x5",
            "160x80x5.5",
            "160x80x6",
            "160x80x6.5",
            "160x80x7",
            "160x100x4",
            "160x100x4.5",
            "160x100x5",
            "160x100x5.5",
            "160x100x6",
            "160x100x6.5",
            "160x100x7",
            "160x100x7.5",
            "160x100x8",
            "160x120x4",
            "160x120x4.5",
            "160x120x5",
            "160x120x5.5",
            "160x120x6",
            "160x120x6.5",
            "160x120x7",
            "160x120x7.5",
            "160x120x8",
            "160x140x5",
            "160x140x5.5",
            "160x140x6",
            "160x140x6.5",
            "160x140x7",
            "160x140x7.5",
            "160x140x8",
            "180x60x4",
            "180x60x4.5",
            "180x60x5",
            "180x60x5.5",
            "180x60x6",
            "180x60x6.5",
            "180x60x7",
            "180x60x7.5",
            "180x60x8",
            "180x80x4",
            "180x80x4.5",
            "180x80x5",
            "180x80x5.5",
            "180x80x6",
            "180x80x6.5",
            "180x80x7",
            "180x80x7.5",
            "180x80x8",
            "180x100x4",
            "180x100x4.5",
            "180x100x5",
            "180x100x5.5",
            "180x100x6",
            "180x100x6.5",
            "180x100x7",
            "180x100x7.5",
            "180x100x8",
            "180x140x4",
            "180x140x4.5",
            "180x140x5",
            "180x140x5.5",
            "180x140x6",
            "180x140x6.5",
            "180x140x7",
            "180x140x7.5",
            "180x140x8",
            "200x40x4",
            "200x40x4.5",
            "200x40x5",
            "200x40x5.5",
            "200x40x6",
            "200x40x6.5",
            "200x40x7",
            "200x80x4",
            "200x80x4.5",
            "200x80x5",
            "200x80x5.5",
            "200x80x6",
            "200x80x6.5",
            "200x80x7",
            "200x80x7.5",
            "200x80x8",
            "200x100x4",
            "200x100x4.5",
            "200x100x5",
            "200x100x5.5",
            "200x100x6",
            "200x100x6.5",
            "200x100x7",
            "200x100x7.5",
            "200x100x8",
            "200x120x4",
            "200x120x4.5",
            "200x120x5",
            "200x120x5.5",
            "200x120x6",
            "200x120x6.5",
            "200x120x7",
            "200x120x7.5",
            "200x120x8",
            "200x160x5",
            "200x160x5.5",
            "200x160x6",
            "200x160x6.5",
            "200x160x7",
            "200x160x7.5",
            "200x160x8",
            "200x160x8.5",
            "200x160x9",
            "200x160x9.5",
            "200x160x10",
            "220x100x4",
            "220x100x4.5",
            "220x100x5",
            "220x100x5.5",
            "220x100x6",
            "220x100x6.5",
            "220x100x7",
            "220x100x7.5",
            "220x100x8",
            "220x140x5",
            "220x140x5.5",
            "220x140x6",
            "220x140x6.5",
            "220x140x7",
            "220x140x7.5",
            "220x140x8",
            "240x120x5",
            "240x120x5.5",
            "240x120x6",
            "240x120x6.5",
            "240x120x7",
            "240x120x7.5",
            "240x120x8",
            "240x160x6",
            "240x160x6.5",
            "240x160x7",
            "240x160x7.5",
            "240x160x8",
            "240x160x8.5",
            "240x160x9",
            "240x160x9.5",
            "240x160x10",
            "240x160x10.5",
            "240x160x11",
            "240x160x11.5",
            "240x160x12",
            "250x150x6",
            "250x150x6.5",
            "250x150x7",
            "250x150x7.5",
            "250x150x8",
            "260x130x6",
            "260x130x6.5",
            "260x130x7",
            "260x130x7.5",
            "260x130x8",
            "260x130x8.5",
            "260x130x9",
            "260x130x9.5",
            "260x130x10",
            "260x130x10.5",
            "260x130x11",
            "260x130x11.5",
            "260x130x12",
            "300x100x6",
            "300x100x6.5",
            "300x100x7",
            "300x100x7.5",
            "300x100x8",
            "300x100x8.5",
            "300x100x9",
            "300x100x9.5",
            "300x100x10",
            "300x200x6",
            "300x200x6.5",
            "300x200x7",
            "300x200x7.5",
            "300x200x8",
            "300x200x8.5",
            "300x200x9",
            "300x200x9.5",
            "300x200x10",
            "300x200x10.5",
            "300x200x11",
            "300x200x11.5",
            "300x200x12",
            "320x180x6",
            "320x180x6.5",
            "320x180x7",
            "320x180x7.5",
            "320x180x8",
            "320x180x8.5",
            "320x180x9",
            "320x180x9.5",
            "320x180x10",
            "320x180x10.5",
            "320x180x11",
            "320x180x11.5",
            "320x180x12",
            "350x250x6",
            "350x250x6.5",
            "350x250x7",
            "350x250x7.5",
            "350x250x8",
            "350x250x8.5",
            "350x250x9",
            "350x250x9.5",
            "350x250x10",
            "350x250x10.5",
            "350x250x11",
            "350x250x11.5",
            "350x250x12",
            "350x300x6",
            "350x300x6.5",
            "350x300x7",
            "350x300x7.5",
            "350x300x8",
            "350x300x8.5",
            "350x300x9",
            "350x300x9.5",
            "350x300x10",
            "350x300x10.5",
            "350x300x11",
            "350x300x11.5",
            "350x300x12",
            "380x220x6",
            "380x220x6.5",
            "380x220x7",
            "380x220x7.5",
            "380x220x8",
            "400x200x10",
            "400x200x10.5",
            "400x200x11",
            "400x200x11.5",
            "400x200x12",
        ];
        expected.sort();
        assert_eq!(designations, expected);
        // Spot checks against the transcribed values, including the three cells where the
        // scans' printed area/mass contradict the print's own m = 0.785*A relation and the
        // closed form; the library carries the relation's value there.
        let spot = |designation: &str, values: [f64; 4], area: f64, mass: f64| {
            let entry = library.entry(designation).unwrap();
            let DefinitionRecord::SectionProfile {
                shape: SectionShape::Parametric(p),
                section,
            } = &entry.record
            else {
                panic!("{designation} is not a parametric section");
            };
            assert_eq!(p.values(), values, "{designation}");
            assert_eq!(section.area_mm2, Some(area), "{designation}");
            assert_eq!(section.mass_per_meter_kg, Some(mass), "{designation}");
        };
        spot("60x60x3", [60.0, 60.0, 3.0, 6.0], 661.0, 5.19);
        // The row the stroyinf typeset drops (its 70x70 group steps 5.0 to 6.0); the raster
        // and the text scans carry it and the closed form confirms it.
        spot("70x70x5.5", [70.0, 70.0, 5.5, 11.0], 1341.0, 10.53);
        // The scans print area 17.86 while their own mass 13.86 is 0.785 * 17.66, and no
        // published corner radius reproduces 17.86; the library carries 17.66.
        spot("80x80x6.5", [80.0, 80.0, 6.5, 16.25], 1766.0, 13.86);
        spot("100x100x7.5", [100.0, 100.0, 7.5, 18.75], 2582.0, 20.27);
        // The scans print area 25.47 while their own mass 20.15 is 0.785 * 25.67 and the
        // printed Ix 896.1 cm4 matches 25.67 too; the library carries 25.67.
        spot("150x150x4.5", [150.0, 150.0, 4.5, 9.0], 2567.0, 20.15);
        // The scans print mass 24.54 where 0.785 times their own printed area 31.01 gives
        // 24.34; the library carries 24.34.
        spot("150x150x5.5", [150.0, 150.0, 5.5, 11.0], 3101.0, 24.34);
        spot("200x200x12", [200.0, 200.0, 12.0, 36.0], 8406.0, 65.99);
        spot("40x40x2", [40.0, 40.0, 2.0, 4.0], 294.0, 2.31);
        // 50x50x4: the scans' printed mass 5.45 truncates the relation (0.785 * 6.95 =
        // 5.4558 rounds to 5.46); the text layer and the raster print 5.45, the typeset
        // 5.46, and the library carries the majority print.
        spot("50x50x4", [50.0, 50.0, 4.0, 8.0], 695.0, 5.45);
        // 250x250x11.5: the row the raster scan drops at its page break; the text layer and
        // the typeset carry it and the closed form confirms it. Areas of 100 cm2 and above
        // print with one decimal.
        spot("250x250x11.5", [250.0, 250.0, 11.5, 34.5], 10400.0, 81.67);
        spot("300x300x12", [300.0, 300.0, 12.0, 36.0], 13210.0, 103.7);
        // The rectangular band spots against the transcribed values: the smallest row,
        // the 50x30 wall range that steps over 4.5 mm, the 80x60 top wall at the 2.5t
        // radius band, and the 80x70 row only the raster scan prints cleanly.
        spot("50x25x2", [50.0, 25.0, 2.0, 4.0], 274.0, 2.15);
        spot("50x25x4", [50.0, 25.0, 4.0, 8.0], 495.0, 3.88);
        spot("50x30x5", [50.0, 30.0, 5.0, 10.0], 636.0, 4.99);
        spot("80x60x7", [80.0, 60.0, 7.0, 17.5], 1596.0, 12.53);
        spot("80x70x5", [80.0, 70.0, 5.0, 10.0], 1336.0, 10.48);
        // Band 2 spots: the band opener, the 90x60 row both text scans drop (only the
        // raster scan prints it; the closed form and the row's own mass confirm it),
        // and the band's widest group at its top wall.
        spot("90x50x3", [90.0, 50.0, 3.0, 6.0], 781.0, 6.13);
        spot("90x60x6.5", [90.0, 60.0, 6.5, 16.25], 1636.0, 12.84);
        spot("100x40x7", [100.0, 40.0, 7.0, 17.5], 1596.0, 12.53);
        spot("120x80x7", [120.0, 80.0, 7.0, 17.5], 2436.0, 19.12);
        // Band 3 spots: the band opener, the widest group's top wall at the 2.5t radius
        // band, and the tall-thin 160x40 pair against the scans.
        spot("140x60x3", [140.0, 60.0, 3.0, 6.0], 1141.0, 8.96);
        spot("140x120x5", [140.0, 120.0, 5.0, 10.0], 2436.0, 19.12);
        spot("150x100x4", [150.0, 100.0, 4.0, 8.0], 1895.0, 14.87);
        spot("160x40x7", [160.0, 40.0, 7.0, 17.5], 2436.0, 19.12);
        spot("160x120x8", [160.0, 120.0, 8.0, 20.0], 4004.0, 31.43);
        // Band 4 spots: the band opener, the 200x160 group at its opening wall, and the
        // band's largest wall at the 2.5t radius band.
        spot("180x60x4", [180.0, 60.0, 4.0, 8.0], 1815.0, 14.25);
        spot("200x160x5", [200.0, 160.0, 5.0, 10.0], 3436.0, 26.97);
        spot("200x120x8", [200.0, 120.0, 8.0, 20.0], 4644.0, 36.46);
        // Band 5 spots, including the two rectangular print errata: the scans' typeset
        // prints 220x100x7 as 46.16 while its own mass 32.31 is 0.785 * 41.16 (the raster
        // and the closed form carry 41.16), and the scans print 350x300x9 as 110.0 while
        // its own mass 87.12 and Ix 20347 are those of 111.0 (the raster and the closed
        // form carry 111.0); the library carries the relation's values, the misprints are
        // named here. The 400x200x12 spot closes the assortment at its largest row, whose
        // area prints at one decimal.
        spot("220x100x7", [220.0, 100.0, 7.0, 17.5], 4116.0, 32.31);
        spot("350x300x9", [350.0, 300.0, 9.0, 22.5], 11100.0, 87.12);
        spot("240x160x6", [240.0, 160.0, 6.0, 12.0], 4563.0, 35.82);
        spot("260x130x12", [260.0, 130.0, 12.0, 36.0], 8166.0, 64.1);
        spot("400x200x12", [400.0, 200.0, 12.0, 36.0], 13210.0, 103.7);
        // Designations the standard does not publish are NOT_FOUND, never a near match:
        // no 65/110/130/170/210/220/240/260/280 mm squares, and each size keeps its
        // published thickness range. The Cyrillic authenticity form is not a lookup key.
        for absent in [
            "65x65x4",
            "110x110x5",
            "130x130x5",
            "170x170x6",
            "210x210x8",
            "220x220x10",
            "240x240x10",
            "260x260x8",
            "280x280x10",
            "60x60x6.5",
            "70x70x7.5",
            "80x80x8.5",
            "100x100x8.5",
            "120x120x8.5",
            "140x140x3.5",
            "140x140x8.5",
            "150x150x8.5",
            "160x160x8.5",
            "180x180x4.5",
            "180x180x10.5",
            "200x200x5.5",
            "200x200x12.5",
            "60х60х3",
            // Rectangular facts table 2 does not publish: no 90x40, 150x75, 180x120,
            // 200x140 or 260x140 group; the 50x30 group steps over the 4.5 mm wall; and
            // every published group keeps its printed wall range. The Cyrillic form is
            // not a lookup key for a rectangle either.
            "50x25x4.5",
            "50x30x4.5",
            "60x30x6.5",
            "80x70x2.5",
            "90x40x3",
            "150x75x4",
            "180x120x5",
            "200x140x5",
            "260x140x9.5",
            // Band 2 facts table 2 does not publish: the 90x50 wall range ends at 7 mm,
            // the 100x40 range opens at 3 mm, the 120x80 range ends at 7 mm, and no
            // 110 mm height group exists.
            "90x50x7.5",
            "100x40x2.5",
            "110x60x4",
            "120x80x7.5",
            // Band 3 facts table 2 does not publish: the 140x60 wall range ends at 7 mm,
            // the 140x100 and 150x100 ranges end at 7 mm too, the 160x40 range opens at
            // 3 mm, and neither a 140x80 nor a 170x100 group exists.
            "140x60x7.5",
            "140x100x7.5",
            "150x100x7.5",
            "160x40x2.5",
            "140x80x4",
            "170x100x5",
            // Band 4 facts table 2 does not publish: the 180x60 wall range ends at 8 mm,
            // the 200x40 range ends at 7 mm, the 200x160 range ends at 10 mm, and no
            // 190 mm height group exists.
            "180x60x8.5",
            "200x40x7.5",
            "200x160x10.5",
            "190x100x5",
            // Band 5 facts table 2 does not publish: the 220x100, 240x120, 250x150 and
            // 380x220 wall ranges end at 8 mm, the 300x100 range ends at 10 mm, and the
            // 400x200 range opens at 10 mm.
            "220x100x8.5",
            "240x120x8.5",
            "250x150x8.5",
            "380x220x8.5",
            "300x100x10.5",
            "400x200x9.5",
        ] {
            assert_eq!(
                library.entry(absent).unwrap_err().code,
                ErrorCode::NotFound,
                "{absent} is not in the shipped series"
            );
        }
    }

    #[test]
    fn the_shipped_russia_gost_8239_8240_library_reads_with_every_entry_admitted() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/russia/profiles-gost-8239-8240.usda"
        );
        let library = load(path).unwrap();
        assert_eq!(library.info.id, "russia-gost-8239-8240-profiles");
        assert_eq!(library.info.version.as_deref(), Some("1"));
        assert!(
            library
                .info
                .licence
                .as_deref()
                .is_some_and(|l| !l.is_empty()),
            "the shipped library declares its licence"
        );
        assert!(
            library
                .info
                .source
                .as_deref()
                .is_some_and(|s| !s.is_empty()),
            "the shipped library declares its source"
        );
        // No transcription slips: every entry is admitted and none warns.
        assert_eq!(library.rejected, []);
        assert_eq!(library.entries.len(), 35);
        // Neither standard prints a per-row slope: the note under the clause 1 figure fixes the
        // inner-face slope as a roll-pass range (8239: 6-12 %, 8240: 4-10 %), excluded from
        // finished-product inspection by note 2, and the rows carry the middle of the range.
        const I_SLOPE: f64 = 0.09;
        const C_SLOPE: f64 = 0.07;
        let mut designations = Vec::new();
        for entry in &library.entries {
            let DefinitionRecord::SectionProfile {
                shape: SectionShape::Parametric(parametric),
                section,
            } = &entry.record
            else {
                panic!("{} is not a parametric section", entry.designation);
            };
            let (slope, range, standard, family) = match parametric.family() {
                SectionFamily::TaperedI => (
                    I_SLOPE,
                    [0.06, 0.12],
                    "GOST 8239-89",
                    SectionFamily::TaperedI,
                ),
                SectionFamily::TaperedC => (
                    C_SLOPE,
                    [0.04, 0.10],
                    "GOST 8240-89",
                    SectionFamily::TaperedC,
                ),
                family => panic!("{}: unexpected family {family:?}", entry.designation),
            };
            assert_eq!(
                parametric.values()[6],
                slope,
                "{}: the series slope",
                entry.designation
            );
            assert_eq!(
                section.standard.as_deref(),
                Some(standard),
                "{}",
                entry.designation
            );
            // The exact closed-form area of the declared dimensions stays within 0.8 percent of
            // the published catalogue area at the shipped slope (tighter than the D3 1 percent
            // warning) and at both ends of the printed roll-pass range, so no slope the
            // standards allow can push a row over the warning.
            let declared = section.area_mm2.unwrap();
            let exact = parametric.exact_area_mm2();
            let gap = (declared - exact).abs() / declared;
            assert!(
                gap <= 0.008,
                "{}: exact {exact} mm2 vs declared {declared} (gap {:.3} %)",
                entry.designation,
                gap * 100.0
            );
            let mut values = parametric.values().to_vec();
            for end in range {
                values[6] = end;
                let exact = ParametricSection::new(family, values.clone())
                    .unwrap()
                    .exact_area_mm2();
                let gap = (declared - exact).abs() / declared;
                assert!(
                    gap <= 0.008,
                    "{} at slope {end}: exact {exact} mm2 vs declared {declared} (gap {:.3} %)",
                    entry.designation,
                    gap * 100.0
                );
            }
            // The declared mass is the declared area at the standards' own 7.85 g/cm3 note,
            // within the printed rounding. I60 is the one row whose printed mass corresponds to
            // the unrounded roll-pass area instead of the printed one (0.785 x 138.0 is 108.33,
            // the standard prints 108.00); the published pair is taken as-is.
            let mass = section.mass_per_meter_kg.unwrap();
            let mass_from_area = declared * 7850.0 / 1.0e6;
            if entry.designation == "I60" {
                assert_eq!(mass, 108.0, "I60 carries the printed mass");
            } else {
                assert!(
                    (mass - mass_from_area).abs() <= 0.08,
                    "{}: mass {mass} kg/m vs area-derived {mass_from_area}",
                    entry.designation
                );
            }
            designations.push(entry.designation.clone());
        }
        // Both assortments exactly as the standards publish them: the 17 I-beam designations
        // 10 to 60 and the 18 slope-series channels of table 1 (16a and 18a included), in the
        // library's lexicographic designation order. The parallel-flange П series (table 2 of
        // GOST 8240-89) is a different geometry family and ships with none of this pack.
        assert_eq!(
            designations,
            [
                "C10", "C12", "C14", "C16", "C16a", "C18", "C18a", "C20", "C22", "C24", "C27",
                "C30", "C33", "C36", "C40", "C5", "C6.5", "C8", "I10", "I12", "I14", "I16", "I18",
                "I20", "I22", "I24", "I27", "I30", "I33", "I36", "I40", "I45", "I50", "I55", "I60",
            ]
        );
        // Spot checks against the transcribed values.
        let spot = |designation: &str, values: [f64; 7], area: f64, mass: f64| {
            let entry = library.entry(designation).unwrap();
            let DefinitionRecord::SectionProfile {
                shape: SectionShape::Parametric(p),
                section,
            } = &entry.record
            else {
                panic!("{designation} is not a parametric section");
            };
            assert_eq!(p.values(), values, "{designation}");
            assert_eq!(section.area_mm2, Some(area), "{designation}");
            assert_eq!(section.mass_per_meter_kg, Some(mass), "{designation}");
        };
        spot(
            "I10",
            [100.0, 55.0, 4.5, 7.2, 7.0, 2.5, I_SLOPE],
            1200.0,
            9.46,
        );
        spot(
            "I60",
            [600.0, 190.0, 12.0, 17.8, 20.0, 8.0, I_SLOPE],
            13800.0,
            108.0,
        );
        spot(
            "C16a",
            [160.0, 68.0, 5.0, 9.0, 8.5, 3.5, C_SLOPE],
            1950.0,
            15.3,
        );
        // The assortment gaps: numbers the standards do not publish are refused, not silently
        // substituted.
        for absent in ["I11", "I21", "I25", "I32", "I65", "C15", "C24a", "C45"] {
            assert_eq!(
                library.entry(absent).unwrap_err().code,
                ErrorCode::NotFound,
                "{absent} is not in the shipped series"
            );
        }
    }

    #[test]
    fn the_shipped_australia_library_reads_with_every_entry_admitted() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/australia/profiles-as-nzs-3679-1.usda"
        );
        let library = load(path).unwrap();
        assert_eq!(library.info.id, "australia-asnzs-3679-profiles");
        assert_eq!(library.info.version.as_deref(), Some("1"));
        assert!(
            library
                .info
                .licence
                .as_deref()
                .is_some_and(|l| !l.is_empty()),
            "the shipped library declares its licence"
        );
        assert!(
            library
                .info
                .source
                .as_deref()
                .is_some_and(|s| !s.is_empty()),
            "the shipped library declares its source"
        );
        // No transcription slips: every entry is admitted and none warns.
        assert_eq!(library.rejected, []);
        assert_eq!(library.entries.len(), 41);
        let mut designations = Vec::new();
        for entry in &library.entries {
            let DefinitionRecord::SectionProfile {
                shape: SectionShape::Parametric(parametric),
                section,
            } = &entry.record
            else {
                panic!("{} is not a parametric section", entry.designation);
            };
            assert_eq!(parametric.family(), SectionFamily::IShape);
            assert_eq!(
                section.standard.as_deref(),
                Some("AS/NZS 3679.1:2016"),
                "{}",
                entry.designation
            );
            // The closed-form area of the declared dimensions must match the declared catalogue
            // area (the D3 1 percent warning never fires) and the designation mass must be that
            // area at 7850 kg/m3 within the rounding the published series uses. Three rows sit
            // widest off because the assortment names carry masses rounded from an earlier
            // re-measurement generation (310UB40.4 1.22 percent, 610UB101 1.03 percent,
            // 200UB22.3 1.02 percent); the published pairs are taken as-is.
            assert!(
                entry.record.declared_area_warning().is_none(),
                "{}",
                entry.designation
            );
            let area = section.area_mm2.unwrap();
            let mass = section.mass_per_meter_kg.unwrap();
            let mass_from_area = area * 7850.0 / 1.0e6;
            assert!(
                (mass - mass_from_area).abs() / mass_from_area <= 0.013,
                "{}: mass {mass} kg/m vs area-derived {mass_from_area}",
                entry.designation
            );
            designations.push(entry.designation.clone());
        }
        // The UB and UC series exactly as AS/NZS 3679.1:2016 publishes them: 28 + 13
        // designations, each carrying its published mass per metre. The older designation
        // generation (150UB17.1, 200UB29.7, 250UB31.3, 460UB82.4, 360UB44.3, 310UC97.7,
        // 150UC30.6) and the sizes no longer rolled (530UB72.4, 460UB61.2, 310UC107,
        // 250UC66.9, 250UC80.5, 200UC41.7, the 350UC series) are not here.
        let mut expected = vec![
            "150UB14.0",
            "150UB18.0",
            "180UB16.1",
            "180UB18.1",
            "180UB22.2",
            "200UB18.2",
            "200UB22.3",
            "200UB25.4",
            "200UB29.8",
            "250UB25.7",
            "250UB31.4",
            "250UB37.3",
            "310UB32.0",
            "310UB40.4",
            "310UB46.2",
            "360UB44.7",
            "360UB50.7",
            "360UB56.7",
            "410UB53.7",
            "410UB59.7",
            "460UB67.1",
            "460UB74.6",
            "460UB82.1",
            "530UB82.0",
            "530UB92.4",
            "610UB101",
            "610UB113",
            "610UB125",
            "100UC14.8",
            "150UC23.4",
            "150UC30.0",
            "150UC37.2",
            "200UC46.2",
            "200UC52.2",
            "200UC59.5",
            "250UC72.9",
            "250UC89.5",
            "310UC96.8",
            "310UC118",
            "310UC137",
            "310UC158",
        ];
        expected.sort();
        designations.sort();
        assert_eq!(designations, expected);
        // Spot checks against the transcribed values, spanning both series.
        let spot = |designation: &str, values: [f64; 5], area: f64, mass: f64| {
            let entry = library.entry(designation).unwrap();
            let DefinitionRecord::SectionProfile {
                shape: SectionShape::Parametric(p),
                section,
            } = &entry.record
            else {
                panic!("{designation} is not a parametric section");
            };
            assert_eq!(p.values(), values, "{designation}");
            assert_eq!(section.area_mm2, Some(area), "{designation}");
            assert_eq!(section.mass_per_meter_kg, Some(mass), "{designation}");
        };
        spot("610UB125", [611.6, 229.0, 11.9, 19.6, 14.0], 16000.0, 125.0);
        spot("150UB14.0", [150.0, 75.0, 5.0, 7.0, 8.0], 1780.0, 14.0);
        spot("310UC158", [327.2, 311.0, 15.7, 25.0, 16.5], 20100.0, 158.0);
        spot("100UC14.8", [97.0, 99.0, 5.0, 7.0, 10.0], 1890.0, 14.8);
        for absent in [
            "150UB17.1",
            "200UB29.7",
            "250UB31.3",
            "460UB82.4",
            "360UB44.3",
            "530UB72.4",
            "460UB61.2",
            "310UC97.7",
            "150UC30.6",
            "310UC107",
            "250UC66.9",
            "250UC80.5",
            "200UC41.7",
            "350UC153",
            "IPE300",
        ] {
            assert_eq!(
                library.entry(absent).unwrap_err().code,
                ErrorCode::NotFound,
                "{absent} is not in the shipped series"
            );
        }
    }

    #[test]
    fn the_shipped_australia_hollows_read_with_every_entry_admitted() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/australia/hollows-as-nzs-1163.usda"
        );
        let library = load(path).unwrap();
        assert_eq!(library.info.id, "australia-asnzs-1163-hollows");
        assert_eq!(library.info.version.as_deref(), Some("1"));
        assert!(
            library
                .info
                .licence
                .as_deref()
                .is_some_and(|l| !l.is_empty()),
            "the shipped library declares its licence"
        );
        assert!(
            library
                .info
                .source
                .as_deref()
                .is_some_and(|s| !s.is_empty()),
            "the shipped library declares its source"
        );
        // No transcription slips: every entry is admitted and none warns.
        assert_eq!(library.rejected, []);
        assert_eq!(library.entries.len(), 145);
        let mut designations = Vec::new();
        for entry in &library.entries {
            let DefinitionRecord::SectionProfile {
                shape: SectionShape::Parametric(_),
                section,
            } = &entry.record
            else {
                panic!("{} is not a parametric section", entry.designation);
            };
            assert_eq!(
                section.standard.as_deref(),
                Some("AS/NZS 1163:2016"),
                "{}",
                entry.designation
            );
            // The closed-form area of the declared dimensions must match the declared catalogue
            // area (the D3 1 percent warning never fires); for the hollows the closed form is the
            // hollow formula of Appendix E with the corner radii the standard fixes for the
            // series (2.0t to 3.0 mm, 2.5t above). The mass must be that area at 7850 kg/m3
            // within the three significant figures the series publishes (worst 0.46 percent on
            // SHS89x89x6.0).
            assert!(
                entry.record.declared_area_warning().is_none(),
                "{}",
                entry.designation
            );
            let area = section.area_mm2.unwrap();
            let mass = section.mass_per_meter_kg.unwrap();
            let mass_from_area = area * 7850.0 / 1.0e6;
            assert!(
                (mass - mass_from_area).abs() / mass_from_area <= 0.01,
                "{}: mass {mass} kg/m vs area-derived {mass_from_area}",
                entry.designation
            );
            designations.push(entry.designation.clone());
        }
        // The CHS, RHS and SHS series exactly as AS/NZS 1163:2016 Appendix D publishes them:
        // 66 + 41 + 38 designations. The mill stock lists carry sizes outside the standard's
        // Appendix D (SHS 300x300 and smaller than 40x40, RHS 300x200 and 152x76, the heavier
        // thicknesses of the small series); those follow the manufacturer, not the standard, and
        // are not shipped here.
        let mut expected = vec![
            "CHS610x12.7",
            "CHS610x9.5",
            "CHS610x6.4",
            "CHS508x12.7",
            "CHS508x9.5",
            "CHS508x6.4",
            "CHS457x12.7",
            "CHS457x9.5",
            "CHS457x6.4",
            "CHS406.4x12.7",
            "CHS406.4x9.5",
            "CHS406.4x6.4",
            "CHS355.6x12.7",
            "CHS355.6x9.5",
            "CHS355.6x6.4",
            "CHS323.9x12.7",
            "CHS323.9x9.5",
            "CHS323.9x6.4",
            "CHS273.1x9.3",
            "CHS273.1x6.4",
            "CHS273.1x4.8",
            "CHS219.1x8.2",
            "CHS219.1x6.4",
            "CHS219.1x4.8",
            "CHS168.3x7.1",
            "CHS168.3x6.4",
            "CHS168.3x4.8",
            "CHS165.1x5.4",
            "CHS165.1x5",
            "CHS165.1x3.5",
            "CHS165.1x3",
            "CHS139.7x5.4",
            "CHS139.7x5",
            "CHS139.7x3.5",
            "CHS139.7x3",
            "CHS114.3x6",
            "CHS114.3x5.4",
            "CHS114.3x4.8",
            "CHS114.3x4.5",
            "CHS114.3x3.6",
            "CHS114.3x3.2",
            "CHS101.6x5",
            "CHS101.6x4",
            "CHS101.6x3.2",
            "CHS101.6x2.6",
            "CHS88.9x5.9",
            "CHS88.9x5.5",
            "CHS88.9x5",
            "CHS88.9x4.8",
            "CHS88.9x4",
            "CHS88.9x3.2",
            "CHS88.9x2.6",
            "CHS76.1x5.9",
            "CHS76.1x4.5",
            "CHS76.1x3.6",
            "CHS76.1x3.2",
            "CHS76.1x2.3",
            "CHS60.3x5.4",
            "CHS60.3x4.5",
            "CHS60.3x3.6",
            "CHS48.3x5.4",
            "CHS48.3x4",
            "CHS48.3x3.2",
            "CHS42.4x4.9",
            "CHS42.4x4",
            "CHS42.4x3.2",
            "RHS250x150x9",
            "RHS250x150x6",
            "RHS250x150x5",
            "RHS200x100x9",
            "RHS200x100x6",
            "RHS200x100x5",
            "RHS200x100x4",
            "RHS150x100x6",
            "RHS150x100x5",
            "RHS150x100x4",
            "RHS150x50x5",
            "RHS150x50x4",
            "RHS150x50x3",
            "RHS125x75x5",
            "RHS125x75x4",
            "RHS125x75x3",
            "RHS100x50x6",
            "RHS100x50x5",
            "RHS100x50x4",
            "RHS100x50x3.5",
            "RHS100x50x3",
            "RHS100x50x2.5",
            "RHS100x50x2",
            "RHS75x50x4",
            "RHS75x50x3",
            "RHS75x50x2.5",
            "RHS75x50x2",
            "RHS75x25x2.5",
            "RHS75x25x2",
            "RHS75x25x1.6",
            "RHS65x35x3",
            "RHS65x35x2.5",
            "RHS65x35x2",
            "RHS50x25x3",
            "RHS50x25x2.5",
            "RHS50x25x2",
            "RHS50x25x1.6",
            "RHS50x20x3",
            "RHS50x20x2.5",
            "RHS50x20x2",
            "RHS50x20x1.6",
            "SHS250x250x9",
            "SHS250x250x6",
            "SHS200x200x9",
            "SHS200x200x6",
            "SHS200x200x5",
            "SHS150x150x9",
            "SHS150x150x6",
            "SHS150x150x5",
            "SHS125x125x9",
            "SHS125x125x6",
            "SHS125x125x5",
            "SHS125x125x4",
            "SHS100x100x9",
            "SHS100x100x6",
            "SHS100x100x5",
            "SHS100x100x4",
            "SHS100x100x3",
            "SHS89x89x6",
            "SHS89x89x5",
            "SHS89x89x3.5",
            "SHS75x75x6",
            "SHS75x75x5",
            "SHS75x75x4",
            "SHS75x75x3.5",
            "SHS75x75x3",
            "SHS75x75x2.5",
            "SHS65x65x3",
            "SHS65x65x2.5",
            "SHS65x65x2",
            "SHS50x50x4",
            "SHS50x50x3",
            "SHS50x50x2.5",
            "SHS50x50x2",
            "SHS50x50x1.6",
            "SHS40x40x4",
            "SHS40x40x2.5",
            "SHS40x40x2",
            "SHS40x40x1.6",
        ];
        expected.sort();
        designations.sort();
        assert_eq!(designations, expected);
        // Spot checks against the transcribed values, one per series. The SHS radius is the one
        // the standard fixes (3.0 mm thickness or less: 2.0t; above: 2.5t).
        let spot =
            |designation: &str, family: SectionFamily, values: &[f64], area: f64, mass: f64| {
                let entry = library.entry(designation).unwrap();
                let DefinitionRecord::SectionProfile {
                    shape: SectionShape::Parametric(p),
                    section,
                } = &entry.record
                else {
                    panic!("{designation} is not a parametric section");
                };
                assert_eq!(p.family(), family, "{designation}");
                assert_eq!(p.values(), values, "{designation}");
                assert_eq!(section.area_mm2, Some(area), "{designation}");
                assert_eq!(section.mass_per_meter_kg, Some(mass), "{designation}");
            };
        spot(
            "SHS100x100x3",
            SectionFamily::Rhs,
            &[100.0, 100.0, 3.0, 6.0],
            1140.0,
            8.96,
        );
        spot(
            "RHS200x100x9",
            SectionFamily::Rhs,
            &[200.0, 100.0, 9.0, 22.5],
            4800.0,
            37.7,
        );
        spot("CHS76.1x3.2", SectionFamily::Chs, &[76.1, 3.2], 733.0, 5.75);
        for absent in [
            "SHS300x300x8",
            "SHS30x30x2",
            "SHS20x20x1.6",
            "SHS50x50x6",
            "SHS65x65x6",
            "RHS300x200x6",
            "RHS400x200x8",
            "RHS152x76x5",
            "RHS150x100x8",
            "CHS60.3x2.6",
            "CHS114.3x2",
            "RHS-100x60x4",
        ] {
            assert_eq!(
                library.entry(absent).unwrap_err().code,
                ErrorCode::NotFound,
                "{absent} is not in the shipped series"
            );
        }
    }

    #[test]
    fn the_shipped_europe_hollow_library_reads_with_every_entry_admitted() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/europe/profiles-en-10219.usda"
        );
        let library = load(path).unwrap();
        assert_eq!(library.info.id, "europe-en-10219-profiles");
        assert_eq!(library.info.version.as_deref(), Some("1"));
        assert!(
            library
                .info
                .licence
                .as_deref()
                .is_some_and(|l| !l.is_empty()),
            "the shipped library declares its licence"
        );
        assert!(
            library
                .info
                .source
                .as_deref()
                .is_some_and(|s| !s.is_empty()),
            "the shipped library declares its source"
        );
        // No transcription slips: every entry is admitted and none warns.
        assert_eq!(library.rejected, []);
        assert_eq!(library.entries.len(), 52);
        let mut designations = Vec::new();
        for entry in &library.entries {
            let DefinitionRecord::SectionProfile {
                shape: SectionShape::Parametric(parametric),
                section,
            } = &entry.record
            else {
                panic!("{} is not a parametric section", entry.designation);
            };
            assert_eq!(
                section.standard.as_deref(),
                Some("EN 10219-2:2006"),
                "{}",
                entry.designation
            );
            // These are closed (hollow) sections, so the declared catalogue area is checked
            // against the hollow closed form, not the I/H formula. EN 10219-2 computes the
            // sectional properties with the outer corner radius 2t up to a 6 mm wall, 2.5t
            // from 6 to 10 mm and 3t above 10 mm, and the inner radius is the outer minus
            // the wall; the closed form of that geometry must reproduce the declared area
            // within the standard's three-significant-figure print rounding (0.55 percent),
            // and the declared mass must be that area at 7850 kg/m3 within the same rounding.
            let (exact_area, exact_mass) = match (parametric.family(), parametric.values()) {
                (SectionFamily::Rhs, &[h, b, t, r]) => {
                    let want_radius = if t <= 6.0 {
                        2.0 * t
                    } else if t <= 10.0 {
                        2.5 * t
                    } else {
                        3.0 * t
                    };
                    assert!(
                        (r - want_radius).abs() <= 1e-9,
                        "{}: corner radius {r} for a {t} mm wall",
                        entry.designation
                    );
                    let inner = (r - t).max(0.0);
                    let k = 1.0 - core::f64::consts::PI / 4.0;
                    let area = (h * b - 4.0 * k * r * r)
                        - ((h - 2.0 * t) * (b - 2.0 * t) - 4.0 * k * inner * inner);
                    (area, area * 7850.0 / 1.0e6)
                }
                (SectionFamily::Chs, &[d, t]) => {
                    let area = core::f64::consts::PI * t * (d - t);
                    (area, area * 7850.0 / 1.0e6)
                }
                _ => panic!("{} is not a hollow section", entry.designation),
            };
            let area = section.area_mm2.unwrap();
            let mass = section.mass_per_meter_kg.unwrap();
            assert!(
                (area - exact_area).abs() / exact_area <= 0.0055,
                "{}: area {area} mm2 vs hollow closed form {exact_area}",
                entry.designation
            );
            assert!(
                (mass - exact_mass).abs() / exact_mass <= 0.0055,
                "{}: mass {mass} kg/m vs hollow closed form {exact_mass}",
                entry.designation
            );
            // The D3 1 percent warning never fires for these rows.
            assert!(
                entry.record.declared_area_warning().is_none(),
                "{}",
                entry.designation
            );
            designations.push(entry.designation.clone());
        }
        // The shipped rows are a curated subset of the common EN 10219-2 sizes; the loader
        // reports them in designation order.
        let mut expected = vec![
            "CHS114.3x4.0",
            "CHS114.3x5.0",
            "CHS139.7x5.0",
            "CHS168.3x5.0",
            "CHS219.1x6.0",
            "CHS273.0x8.0",
            "CHS323.9x8.0",
            "CHS33.7x3.0",
            "CHS406.4x10.0",
            "CHS42.4x3.0",
            "CHS48.3x3.0",
            "CHS60.3x3.0",
            "CHS76.1x3.0",
            "CHS76.1x4.0",
            "CHS88.9x4.0",
            "RHS100x50x4",
            "RHS100x50x5",
            "RHS120x60x5",
            "RHS120x80x5",
            "RHS150x100x5",
            "RHS150x100x8",
            "RHS160x80x6",
            "RHS200x100x6",
            "RHS200x120x8",
            "RHS250x150x8",
            "RHS300x200x8",
            "RHS60x40x3",
            "RHS80x40x4",
            "SHS100x100x4",
            "SHS100x100x5",
            "SHS100x100x6",
            "SHS120x120x5",
            "SHS120x120x6",
            "SHS140x140x5",
            "SHS150x150x5",
            "SHS150x150x6",
            "SHS150x150x8",
            "SHS160x160x6",
            "SHS180x180x6",
            "SHS200x200x10",
            "SHS200x200x12",
            "SHS200x200x6",
            "SHS200x200x8",
            "SHS250x250x8",
            "SHS300x300x6",
            "SHS350x350x10",
            "SHS50x50x3",
            "SHS50x50x4",
            "SHS60x60x3",
            "SHS60x60x4",
            "SHS80x80x4",
            "SHS80x80x5",
        ];
        expected.sort();
        assert_eq!(designations, expected);
        // Spot checks against the transcribed values: one square, one rectangular and one
        // circular row, including the 2.5t and 3t corner-radius bands.
        let spot = |designation: &str, values: &[f64], area: f64, mass: f64| {
            let entry = library.entry(designation).unwrap();
            let DefinitionRecord::SectionProfile {
                shape: SectionShape::Parametric(p),
                section,
            } = &entry.record
            else {
                panic!("{designation} is not a parametric section");
            };
            assert_eq!(p.values(), values, "{designation}");
            assert_eq!(section.area_mm2, Some(area), "{designation}");
            assert_eq!(section.mass_per_meter_kg, Some(mass), "{designation}");
        };
        spot("SHS100x100x5", &[100.0, 100.0, 5.0, 10.0], 1840.0, 14.4);
        spot("SHS200x200x10", &[200.0, 200.0, 10.0, 25.0], 7260.0, 57.0);
        spot("SHS200x200x12", &[200.0, 200.0, 12.0, 36.0], 8410.0, 66.0);
        spot("RHS200x100x6", &[200.0, 100.0, 6.0, 12.0], 3360.0, 26.4);
        spot("CHS219.1x6.0", &[219.1, 6.0], 4020.0, 31.4);
        // Designations and wall thicknesses the shipped series does not publish are refused,
        // not silently substituted.
        for absent in [
            "SHS110x110x5",
            "SHS50x50x3.2",
            "SHS200x200x9",
            "RHS100x70x4",
            "RHS600x400x16",
            "CHS100x5",
            "CHS219.1x6.3",
        ] {
            assert_eq!(
                library.entry(absent).unwrap_err().code,
                ErrorCode::NotFound,
                "{absent} is not in the shipped series"
            );
        }
    }

    #[test]
    fn the_shipped_india_npb_library_reads_with_every_entry_admitted() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/india/profiles-is808-npb.usda"
        );
        let library = load(path).unwrap();
        assert_eq!(library.info.id, "india-is808-npb-profiles");
        assert_eq!(library.info.version.as_deref(), Some("1"));
        assert!(
            library
                .info
                .licence
                .as_deref()
                .is_some_and(|l| !l.is_empty()),
            "the shipped library declares its licence"
        );
        assert!(
            library
                .info
                .source
                .as_deref()
                .is_some_and(|s| !s.is_empty()),
            "the shipped library declares its source"
        );
        // No transcription slips: every entry is admitted and none warns.
        assert_eq!(library.rejected, []);
        assert_eq!(library.entries.len(), 70);
        let mut designations = Vec::new();
        for entry in &library.entries {
            let DefinitionRecord::SectionProfile {
                shape: SectionShape::Parametric(parametric),
                section,
            } = &entry.record
            else {
                panic!("{} is not a parametric section", entry.designation);
            };
            assert_eq!(parametric.family(), SectionFamily::IShape);
            assert_eq!(
                section.standard.as_deref(),
                Some("IS 808:2021"),
                "{}",
                entry.designation
            );
            // The closed-form area of the declared dimensions must match the declared catalogue
            // area (the D3 1 percent warning never fires; the worst row sits at 0.94 percent,
            // an area column rounded to three significant digits) and the declared mass must
            // match the closed-form area at 7850 kg/m3: the published masses are computed from
            // unrounded geometry, so they follow the geometry and not the rounded printed area
            // (worst observed 0.0144 kg/m).
            assert!(
                entry.record.declared_area_warning().is_none(),
                "{}",
                entry.designation
            );
            let mass = section.mass_per_meter_kg.unwrap();
            let mass_from_geometry = parametric.exact_area_mm2() * 7850.0 / 1.0e6;
            assert!(
                (mass - mass_from_geometry).abs() <= 0.05,
                "{}: mass {mass} kg/m vs geometry-derived {mass_from_geometry}",
                entry.designation
            );
            designations.push(entry.designation.clone());
        }
        // The narrow parallel flange beam (NPB) series exactly as IS 808:2021 table 3
        // publishes it: 70 rows, the assortment of IS 12778:2004 amalgamated into the
        // 2021 edition. The designation mass is the 2021 printed value (the superseded
        // IS 12778:2004 rounding of the same row, 36.52, is not here).
        let mut expected = vec![
            "NPB100x55x8.10",
            "NPB120x60x10.37",
            "NPB140x70x12.89",
            "NPB160x80x15.77",
            "NPB180x90x15.37",
            "NPB180x90x18.80",
            "NPB180x90x21.27",
            "NPB200x100x18.43",
            "NPB200x100x22.36",
            "NPB200x100x25.09",
            "NPB200x130x27.37",
            "NPB200x130x31.56",
            "NPB200x150x30.46",
            "NPB200x165x35.69",
            "NPB200x165x42.48",
            "NPB200x165x48.00",
            "NPB220x110x22.18",
            "NPB220x110x26.20",
            "NPB220x110x29.35",
            "NPB240x120x26.15",
            "NPB240x120x30.71",
            "NPB240x120x34.32",
            "NPB250x125x30.11",
            "NPB250x150x34.08",
            "NPB250x150x39.78",
            "NPB250x150x46.48",
            "NPB250x175x43.94",
            "NPB270x135x30.73",
            "NPB270x135x36.07",
            "NPB270x135x42.26",
            "NPB300x150x36.53",
            "NPB300x150x42.24",
            "NPB300x150x49.32",
            "NPB300x165x39.88",
            "NPB300x165x45.76",
            "NPB300x165x53.46",
            "NPB300x200x59.57",
            "NPB300x200x66.75",
            "NPB300x200x75.37",
            "NPB330x160x42.97",
            "NPB330x160x49.15",
            "NPB330x160x57.01",
            "NPB350x170x50.22",
            "NPB350x170x57.10",
            "NPB350x170x66.05",
            "NPB350x250x79.18",
            "NPB400x180x57.38",
            "NPB400x180x66.31",
            "NPB400x180x75.67",
            "NPB400x200x67.28",
            "NPB450x190x67.16",
            "NPB450x190x77.58",
            "NPB450x190x92.37",
            "NPB500x200x107.32",
            "NPB500x200x79.36",
            "NPB500x200x90.69",
            "NPB550x210x105.52",
            "NPB550x210x122.52",
            "NPB550x210x92.08",
            "NPB600x220x107.57",
            "NPB600x220x122.45",
            "NPB600x220x154.47",
            "NPB700x250x113.46",
            "NPB700x250x128.41",
            "NPB700x250x143.42",
            "NPB700x250x153.87",
            "NPB700x250x171.48",
            "NPB750x270x145.29",
            "NPB750x270x174.54",
            "NPB750x270x202.49",
        ];
        expected.sort();
        designations.sort();
        assert_eq!(designations, expected);
        // Spot checks against the transcribed values, including the deepest row with its
        // production-technology root radius of 17 mm and the row whose 2021 area re-rounding
        // differs from IS 12778:2004 by one final digit.
        let spot = |designation: &str, values: [f64; 5], area: f64, mass: f64| {
            let entry = library.entry(designation).unwrap();
            let DefinitionRecord::SectionProfile {
                shape: SectionShape::Parametric(p),
                section,
            } = &entry.record
            else {
                panic!("{designation} is not a parametric section");
            };
            assert_eq!(p.values(), values, "{designation}");
            assert_eq!(section.area_mm2, Some(area), "{designation}");
            assert_eq!(section.mass_per_meter_kg, Some(mass), "{designation}");
        };
        spot("NPB100x55x8.10", [100.0, 55.0, 4.1, 5.7, 7.0], 1030.0, 8.1);
        spot(
            "NPB300x150x42.24",
            [300.0, 150.0, 7.1, 10.7, 15.0],
            5380.0,
            42.24,
        );
        spot(
            "NPB400x200x67.28",
            [400.0, 200.0, 8.0, 13.0, 21.0],
            8570.0,
            67.28,
        );
        spot(
            "NPB750x270x202.49",
            [770.0, 270.0, 15.6, 26.6, 17.0],
            25700.0,
            202.49,
        );
        // Independent cross-check against the shipped `europe` pack: seventeen NPB rows are the
        // EN 10365 IPE sizes rolled under the Indian designation, and their dimensions, areas
        // and masses must agree with that separately transcribed edition (within its printed
        // rounding).
        let europe = load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/europe/profiles-en-10365.usda"
        ))
        .unwrap();
        let ipe_twins: Vec<(&str, &[f64], f64, f64)> = europe
            .entries
            .iter()
            .filter_map(|e| match &e.record {
                DefinitionRecord::SectionProfile {
                    shape: SectionShape::Parametric(p),
                    section,
                } => Some((
                    e.designation.as_str(),
                    p.values(),
                    section.area_mm2?,
                    section.mass_per_meter_kg?,
                )),
                _ => None,
            })
            .collect();
        let mut ipe_rows = 0;
        for entry in &library.entries {
            let DefinitionRecord::SectionProfile {
                shape: SectionShape::Parametric(p),
                section,
            } = &entry.record
            else {
                continue;
            };
            let values = p.values();
            if let Some((designation, _, twin_area, twin_mass)) = ipe_twins
                .iter()
                .find(|(_, twin_values, _, _)| *twin_values == values)
            {
                ipe_rows += 1;
                assert!(
                    (section.area_mm2.unwrap() - twin_area).abs() <= 100.0,
                    "{} vs {designation}: area",
                    entry.designation
                );
                // The europe pack rounds the heaviest sections to three significant digits
                // (IPE550: 106 against the standard's 105.5, carried here as 105.52), so the
                // mass agreement is 0.5 kg/m on those rows.
                assert!(
                    (section.mass_per_meter_kg.unwrap() - twin_mass).abs() <= 0.5,
                    "{} vs {designation}: mass",
                    entry.designation
                );
            }
        }
        // Seventeen, not eighteen: IPE 250 is not part of the shipped `europe` pack.
        assert_eq!(
            ipe_rows, 17,
            "the IPE-exact NPB rows agree with the europe pack"
        );
        // The sloping-flange series of the same edition (MB/WB/SC, tables 1 and 5), the
        // toe-radius channels (table 8), the angles and the bearing piles are not shipped.
        for absent in [
            "MB300",
            "SC200",
            "WB200",
            "MCP75",
            "ISA50x50x6",
            "PBP200x43.85",
            "NPB300x150x36.52",
        ] {
            assert_eq!(
                library.entry(absent).unwrap_err().code,
                ErrorCode::NotFound,
                "{absent} is not in the shipped series"
            );
        }
    }

    #[test]
    fn the_shipped_india_wpb_100_260_library_reads_with_every_entry_admitted() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/india/profiles-is808-wpb-100-260.usda"
        );
        let library = load(path).unwrap();
        assert_eq!(library.info.id, "india-is808-wpb-100-260-profiles");
        assert_eq!(library.info.version.as_deref(), Some("1"));
        assert!(
            library
                .info
                .licence
                .as_deref()
                .is_some_and(|l| !l.is_empty()),
            "the shipped library declares its licence"
        );
        assert!(
            library
                .info
                .source
                .as_deref()
                .is_some_and(|s| !s.is_empty()),
            "the shipped library declares its source"
        );
        assert_eq!(library.rejected, []);
        assert_eq!(library.entries.len(), 52);
        let mut designations = Vec::new();
        for entry in &library.entries {
            let DefinitionRecord::SectionProfile {
                shape: SectionShape::Parametric(parametric),
                section,
            } = &entry.record
            else {
                panic!("{} is not a parametric section", entry.designation);
            };
            assert_eq!(parametric.family(), SectionFamily::IShape);
            assert_eq!(
                section.standard.as_deref(),
                Some("IS 808:2021"),
                "{}",
                entry.designation
            );
            // Same discipline as the NPB library: the closed-form area never trips the D3 1
            // percent warning (worst row 0.94 percent, an area column rounded to three
            // significant digits) and every declared mass follows the closed-form geometry
            // rather than the rounded printed area.
            assert!(
                entry.record.declared_area_warning().is_none(),
                "{}",
                entry.designation
            );
            let mass = section.mass_per_meter_kg.unwrap();
            let mass_from_geometry = parametric.exact_area_mm2() * 7850.0 / 1.0e6;
            assert!(
                (mass - mass_from_geometry).abs() <= 0.05,
                "{}: mass {mass} kg/m vs geometry-derived {mass_from_geometry}",
                entry.designation
            );
            designations.push(entry.designation.clone());
        }
        // The wide parallel flange beam (WPB) series of IS 808:2021 table 4, depths 100 to
        // 260: 52 rows. The 280-deep and larger rows live in the follow-up library, the
        // text-layer phantom WPB200x200x37.34 (absent from the printed table) is not here,
        // and the reshaped heavy 220 row carries the 2021 depth of 226 mm (IS 12778:2004
        // printed 240 mm with mass 117.31).
        let mut expected = vec![
            "WPB100x100x12.24",
            "WPB100x100x16.67",
            "WPB100x100x20.44",
            "WPB100x100x41.79",
            "WPB120x120x14.56",
            "WPB120x120x19.89",
            "WPB120x120x26.70",
            "WPB120x120x52.13",
            "WPB140x140x18.08",
            "WPB140x140x24.66",
            "WPB140x140x33.72",
            "WPB140x140x63.24",
            "WPB150x150x23.50",
            "WPB150x150x30.11",
            "WPB150x150x36.97",
            "WPB160x160x22.75",
            "WPB160x160x30.44",
            "WPB160x160x42.59",
            "WPB160x160x76.19",
            "WPB180x180x28.68",
            "WPB180x180x35.52",
            "WPB180x180x51.22",
            "WPB180x180x88.90",
            "WPB200x200x103.06",
            "WPB200x200x34.65",
            "WPB200x200x42.26",
            "WPB200x200x50.92",
            "WPB200x200x61.30",
            "WPB200x200x74.01",
            "WPB200x200x83.52",
            "WPB220x220x115.61",
            "WPB220x220x40.40",
            "WPB220x220x50.51",
            "WPB220x220x71.47",
            "WPB240x240x156.68",
            "WPB240x240x47.40",
            "WPB240x240x60.32",
            "WPB240x240x83.20",
            "WPB250x250x103.97",
            "WPB250x250x117.58",
            "WPB250x250x133.92",
            "WPB250x250x148.38",
            "WPB250x250x67.22",
            "WPB250x250x73.15",
            "WPB250x250x85.04",
            "WPB250x250x97.04",
            "WPB260x260x114.40",
            "WPB260x260x141.52",
            "WPB260x260x172.43",
            "WPB260x260x54.15",
            "WPB260x260x68.16",
            "WPB260x260x92.99",
        ];
        expected.sort();
        designations.sort();
        assert_eq!(designations, expected);
        let spot = |designation: &str, values: [f64; 5], area: f64, mass: f64| {
            let entry = library.entry(designation).unwrap();
            let DefinitionRecord::SectionProfile {
                shape: SectionShape::Parametric(p),
                section,
            } = &entry.record
            else {
                panic!("{designation} is not a parametric section");
            };
            assert_eq!(p.values(), values, "{designation}");
            assert_eq!(section.area_mm2, Some(area), "{designation}");
            assert_eq!(section.mass_per_meter_kg, Some(mass), "{designation}");
        };
        spot(
            "WPB100x100x12.24",
            [91.0, 100.0, 4.2, 5.5, 12.0],
            1550.0,
            12.24,
        );
        spot(
            "WPB150x150x23.50",
            [152.0, 152.0, 5.8, 6.8, 12.0],
            2990.0,
            23.5,
        );
        spot(
            "WPB220x220x115.61",
            [226.0, 226.0, 15.5, 26.0, 18.0],
            14700.0,
            115.61,
        );
        spot(
            "WPB250x250x67.22",
            [247.0, 252.0, 11.0, 11.1, 24.0],
            8560.0,
            67.22,
        );
        // The two 2021 root-radius updates against IS 12778:2004 (its foreword notes the fillet
        // radii were modified for the newer production technology; the 2021 area and mass follow
        // the new radius and both editions are self-consistent):
        // WPB150x150x23.50 carries the raised radius (12 mm against 12778's 8 mm, area 29.9
        // against 29.2 cm2, mass 23.50 against 22.96) and WPB160x160x22.75 the lowered one
        // (8 mm against 15 mm, area 28.9 against 30.4 cm2, mass 22.75 against 23.83);
        // both are spot-checked above by their shipped values.
        for absent in [
            "WPB280x280x61.26",
            "WPB200x200x37.34",
            "WPB220x220x117.31",
            "MB300",
            "SC200",
            "MCP75",
        ] {
            assert_eq!(
                library.entry(absent).unwrap_err().code,
                ErrorCode::NotFound,
                "{absent} is not in the shipped series"
            );
        }
    }

    #[test]
    fn the_shipped_india_wpb_280_900_library_reads_with_every_entry_admitted() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/india/profiles-is808-wpb-280-900.usda"
        );
        let library = load(path).unwrap();
        assert_eq!(library.info.id, "india-is808-wpb-280-900-profiles");
        assert_eq!(library.info.version.as_deref(), Some("1"));
        assert!(
            library
                .info
                .licence
                .as_deref()
                .is_some_and(|l| !l.is_empty()),
            "the shipped library declares its licence"
        );
        assert!(
            library
                .info
                .source
                .as_deref()
                .is_some_and(|s| !s.is_empty()),
            "the shipped library declares its source"
        );
        assert_eq!(library.rejected, []);
        assert_eq!(library.entries.len(), 69);
        let mut designations = Vec::new();
        for entry in &library.entries {
            let DefinitionRecord::SectionProfile {
                shape: SectionShape::Parametric(parametric),
                section,
            } = &entry.record
            else {
                panic!("{} is not a parametric section", entry.designation);
            };
            assert_eq!(parametric.family(), SectionFamily::IShape);
            assert_eq!(
                section.standard.as_deref(),
                Some("IS 808:2021"),
                "{}",
                entry.designation
            );
            // Same discipline as the two shallower libraries: the closed-form area never trips
            // the D3 1 percent warning and every declared mass follows the closed-form geometry
            // at 7850 kg/m3.
            assert!(
                entry.record.declared_area_warning().is_none(),
                "{}",
                entry.designation
            );
            let mass = section.mass_per_meter_kg.unwrap();
            let mass_from_geometry = parametric.exact_area_mm2() * 7850.0 / 1.0e6;
            assert!(
                (mass - mass_from_geometry).abs() <= 0.05,
                "{}: mass {mass} kg/m vs geometry-derived {mass_from_geometry}",
                entry.designation
            );
            designations.push(entry.designation.clone());
        }
        // The wide parallel flange beam (WPB) series of IS 808:2021 table 4, depths 280 to
        // 900: 69 rows. Four rows carry a corrected mass: the 2021 print contradicts the same
        // row's printed area column, printed rz/ry and the closed form of the printed
        // dimensions, while IS 12778:2004's printed mass matches the closed form; the
        // designation uses that corrected mass (WPB360x300x83.69 against the printed 91.04,
        // WPB360x300x112.06 against 125.81, WPB360x300x141.80 against 163.00, and
        // WPB340x300x247.92 against 290.64). The unresolved WPB280x280x284.13 row is absent:
        // the 2021 text layer carries it but it is not visible in the printed and rendered
        // table and its text-layer web thickness contradicts IS 12778:2004 (10.5 against
        // 105.0), so it stays out pending the official printed text.
        let mut expected = vec![
            "WPB280x280x188.54",
            "WPB280x280x61.26",
            "WPB280x280x76.36",
            "WPB300x300x100.85",
            "WPB300x300x117.03",
            "WPB300x300x237.92",
            "WPB300x300x69.80",
            "WPB300x300x88.34",
            "WPB320x300x126.66",
            "WPB320x300x244.97",
            "WPB320x300x74.25",
            "WPB320x300x97.64",
            "WPB340x300x104.78",
            "WPB340x300x134.16",
            "WPB340x300x247.92",
            "WPB340x300x78.90",
            "WPB360x300x112.06",
            "WPB360x300x141.80",
            "WPB360x300x250.27",
            "WPB360x300x83.69",
            "WPB360x370x136.21",
            "WPB360x370x150.87",
            "WPB360x370x165.35",
            "WPB360x370x182.02",
            "WPB360x370x197.66",
            "WPB400x300x124.81",
            "WPB400x300x155.26",
            "WPB400x300x255.74",
            "WPB400x300x92.40",
            "WPB400x400x191.11",
            "WPB400x400x219.67",
            "WPB400x400x239.62",
            "WPB450x300x139.76",
            "WPB450x300x171.12",
            "WPB450x300x263.33",
            "WPB450x300x99.75",
            "WPB500x300x107.46",
            "WPB500x300x129.78",
            "WPB500x300x155.08",
            "WPB500x300x187.34",
            "WPB500x300x270.28",
            "WPB550x300x119.99",
            "WPB550x300x166.24",
            "WPB550x300x199.44",
            "WPB550x300x278.19",
            "WPB600x300x128.79",
            "WPB600x300x177.78",
            "WPB600x300x211.92",
            "WPB600x300x285.48",
            "WPB650x300x137.98",
            "WPB650x300x189.69",
            "WPB650x300x224.78",
            "WPB650x300x293.39",
            "WPB700x300x149.89",
            "WPB700x300x204.48",
            "WPB700x300x240.51",
            "WPB700x300x300.68",
            "WPB800x300x171.52",
            "WPB800x300x179.90",
            "WPB800x300x224.38",
            "WPB800x300x262.34",
            "WPB800x300x317.36",
            "WPB850x300x195.74",
            "WPB850x300x214.25",
            "WPB850x300x230.56",
            "WPB850x300x253.69",
            "WPB900x300x198.01",
            "WPB900x300x251.62",
            "WPB900x300x291.46",
        ];
        expected.sort();
        designations.sort();
        assert_eq!(designations, expected);
        let spot = |designation: &str, values: [f64; 5], area: f64, mass: f64| {
            let entry = library.entry(designation).unwrap();
            let DefinitionRecord::SectionProfile {
                shape: SectionShape::Parametric(p),
                section,
            } = &entry.record
            else {
                panic!("{designation} is not a parametric section");
            };
            assert_eq!(p.values(), values, "{designation}");
            assert_eq!(section.area_mm2, Some(area), "{designation}");
            assert_eq!(section.mass_per_meter_kg, Some(mass), "{designation}");
        };
        spot(
            "WPB280x280x61.26",
            [264.0, 280.0, 7.0, 10.0, 24.0],
            7800.0,
            61.26,
        );
        spot(
            "WPB360x300x141.80",
            [360.0, 300.0, 12.5, 22.5, 27.0],
            18000.0,
            141.8,
        );
        spot(
            "WPB900x300x291.46",
            [900.0, 300.0, 18.5, 35.0, 30.0],
            37100.0,
            291.46,
        );
        for absent in [
            "WPB360x300x163.00",
            "WPB360x300x91.04",
            "WPB360x300x125.81",
            "WPB340x300x290.64",
            "WPB280x280x284.13",
            "MB300",
            "SC200",
        ] {
            assert_eq!(
                library.entry(absent).unwrap_err().code,
                ErrorCode::NotFound,
                "{absent} is not in the shipped series"
            );
        }
    }

    #[test]
    fn the_shipped_india_sloping_flange_library_reads_with_every_entry_admitted() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/india/profiles-is808-sloping-flange.usda"
        );
        let library = load(path).unwrap();
        assert_eq!(library.info.id, "india-is808-sloping-flange-profiles");
        assert_eq!(library.info.version.as_deref(), Some("1"));
        assert!(
            library
                .info
                .licence
                .as_deref()
                .is_some_and(|l| !l.is_empty()),
            "the shipped library declares its licence"
        );
        assert!(
            library
                .info
                .source
                .as_deref()
                .is_some_and(|s| !s.is_empty()),
            "the shipped library declares its source"
        );
        // No transcription slips: every entry is admitted and none warns.
        assert_eq!(library.rejected, []);
        assert_eq!(library.entries.len(), 102);
        // The slope column prints the angle the inner flange face makes with the flange plane
        // (parallel rows print 90 degrees); every row carries k = tan of the angle over 90
        // degrees, as printed. The shipped rows split as: 8 degrees - MB, SC and the heavy
        // WB200x52.09 row; 6 degrees - the other WB rows, the provisional LBP175/LBP200, MC,
        // LC225 and deeper, and LCP125-LCP300; 4 degrees - HB; 1.5 degrees - JB, JC, the other
        // LB rows (LB75-LB200 with LBP100) and the shallow plain LC rows (LC75-LC200). The deep
        // LB rows that also print 8 degrees are the excluded toe-fit rows. The legacy
        // IS 808:1989 edition prints the same column as "flange slope, max", a roll-pass
        // maximum, so unlike the GB/T 706 tables there is no tighter-than-D3 bound: the
        // declared areas reproduce at the printed slope within 0.8 percent (the worst rows
        // MB500, JB175 and JC125 sit 0.54 to 0.76 percent above the closed form).
        let k98 = 8.0_f64.to_radians().tan();
        let k96 = 6.0_f64.to_radians().tan();
        let k94 = 4.0_f64.to_radians().tan();
        let k915 = 1.5_f64.to_radians().tan();
        let mut designations = Vec::new();
        for entry in &library.entries {
            let DefinitionRecord::SectionProfile {
                shape: SectionShape::Parametric(parametric),
                section,
            } = &entry.record
            else {
                panic!("{} is not a parametric section", entry.designation);
            };
            match parametric.family() {
                SectionFamily::TaperedI | SectionFamily::TaperedC => {}
                family => panic!("{}: unexpected family {family:?}", entry.designation),
            }
            let k = parametric.values()[6];
            // tan() of the printed angle matches the row's stored decimal value only to
            // the last ulp, and that last ulp differs between libm platforms (the exact
            // equality failed on x86_64 Linux), so the printed-angle check compares
            // within the same 1e-12 tolerance the india_environment e2e test uses.
            assert!(
                [k98, k96, k94, k915]
                    .iter()
                    .any(|printed| (k - printed).abs() < 1e-12),
                "{}: slope {k} is not one of the printed angles",
                entry.designation
            );
            assert_eq!(
                section.standard.as_deref(),
                Some("IS 808:2021"),
                "{}",
                entry.designation
            );
            // The D3 discipline: the declared catalogue area never warns against the exact
            // filleted area of the admitted geometry at the printed slope.
            assert!(
                entry.record.declared_area_warning().is_none(),
                "{}",
                entry.designation
            );
            // Every declared mass follows its declared area at 7850 kg/m3 within 1 percent,
            // except two named print errata: the mass column disagrees with the same row's
            // printed area in BOTH the 2021 and the 1989 editions, while the areas track the
            // closed form of the printed dimensions (MC175x22.7 prints 22.7 against an area
            // worth 21.4 to 21.5 kg/m, a 5.6 percent gap; MC225x30.7 prints 30.7 against 30.3
            // to 30.6, 1.04 percent). The printed values ship: the engine's quantities compute
            // from the catalogue area, not from this metadata.
            let area = section.area_mm2.unwrap();
            let mass = section.mass_per_meter_kg.unwrap();
            let mass_from_area = area * 7850.0 / 1.0e6;
            let gap = (mass - mass_from_area).abs() / mass;
            if matches!(entry.designation.as_str(), "MC175x22.7" | "MC225x30.7") {
                assert!(
                    gap > 0.01,
                    "{}: {mass} kg/m vs area-derived {mass_from_area}: the documented print \
                     erratum no longer reproduces",
                    entry.designation
                );
            } else {
                assert!(
                    gap <= 0.01,
                    "{}: mass {mass} kg/m vs area-derived {mass_from_area}",
                    entry.designation
                );
            }
            designations.push(entry.designation.clone());
        }
        // Both families exactly as tables 1, 2, 5, 6 and 7 of IS 808:2021 publish them: 62
        // sloping flange beams, columns and heavy beams (tapered_i) and 40 sloping flange
        // channels (tapered_c). Where one depth carries several rows the heavier ones take the
        // depth-x-mass form the standard requires when ordering them (WB200x52.09), the star of
        // the print is a footnote marker and is dropped (SC150 is the printed SC 150*), and the
        // provisional sections' (P) marker is normalized to P (LB(P) 100 ships as LBP100).
        designations.sort();
        let expected = [
            "HB150",
            "HB150x30.15",
            "HB150x33.66",
            "HB200",
            "HB200x39.73",
            "HB225",
            "HB225x46.52",
            "HB250",
            "HB250x54.41",
            "HB300",
            "HB300x62.67",
            "HB350",
            "HB350x72.03",
            "HB400",
            "HB400x81.83",
            "HB450",
            "HB450x92.19",
            "JB150",
            "JB175",
            "JB200",
            "JB225",
            "JC100",
            "JC125",
            "JC150",
            "JC175",
            "JC200",
            "LB100",
            "LB125",
            "LB150",
            "LB175",
            "LB200",
            "LB75",
            "LBP100",
            "LBP175",
            "LBP200",
            "LC100",
            "LC125",
            "LC150",
            "LC175",
            "LC200",
            "LC225",
            "LC250",
            "LC300",
            "LC350",
            "LC400",
            "LC75",
            "LCP125",
            "LCP150",
            "LCP200",
            "LCP300",
            "MB100",
            "MB125",
            "MB150",
            "MB175",
            "MB200",
            "MB225",
            "MB250",
            "MB300",
            "MB350",
            "MB400",
            "MB450",
            "MB500",
            "MB550",
            "MB600",
            "MC100",
            "MC125",
            "MC125x13.7",
            "MC150",
            "MC150x17.7",
            "MC175",
            "MC175x22.7",
            "MC200",
            "MC200x24.3",
            "MC225",
            "MC225x30.7",
            "MC250",
            "MC250x34.2",
            "MC250x38.1",
            "MC300",
            "MC300x41.5",
            "MC300x46.2",
            "MC350",
            "MC400",
            "MC75",
            "SC100",
            "SC120",
            "SC140",
            "SC150",
            "SC160",
            "SC180",
            "WB150",
            "WB175",
            "WB200",
            "WB200x52.09",
            "WB225",
            "WB350",
            "WB400",
            "WB450",
            "WB500",
            "WB550",
            "WB600",
            "WB600x145.06",
        ];
        assert_eq!(designations, expected);
        // Seventeen printed rows are absent: their published toe radius R2 exceeds the toe
        // thickness the standard's own annex A computes for the same row (T - ((B-t)/4) tan of
        // the slope), so the exact tapered outline the kernel admits cannot express them
        // (WB250, WB300, SC200, SC220, SC250, and the deep light beams LB225 to LB600 with
        // LB(P)300). The depths no series publishes stay absent, as do the parallel flange
        // channel (MCP/MPC, table 8) and bearing pile (PBP, table 13) series - different
        // families than the tapered ones this library ships.
        for absent in [
            "WB250", "WB300", "SC200", "SC220", "SC250", "LB225", "LB250", "LB275", "LB300",
            "LBP300", "LB325", "LB350", "LB400", "LB450", "LB500", "LB550", "LB600", "MB275",
            "MB325", "WB275", "SC190", "JB250", "HB175", "HB500", "JC75", "JC250", "LC450",
            "MC450", "MPC200", "MCP300", "PBP300",
        ] {
            assert_eq!(
                library.entry(absent).unwrap_err().code,
                ErrorCode::NotFound,
                "{absent} is not in the shipped series"
            );
        }
        // Spot rows against the print: dimensions, catalogue area and mass.
        let spot = |designation: &str, values: [f64; 7], area: f64, mass: f64| {
            let entry = library.entry(designation).unwrap();
            let DefinitionRecord::SectionProfile {
                shape: SectionShape::Parametric(p),
                section,
            } = &entry.record
            else {
                panic!("{designation} is not a parametric section");
            };
            // The dimensions parse exactly; the stored slope is the printed
            // angle's tan to the last ulp, which differs between libm
            // platforms, so the slope element compares within the same 1e-12
            // tolerance as the printed-angle check above.
            let stored = p.values();
            assert_eq!(stored[..6], values[..6], "{designation}");
            assert!(
                (stored[6] - values[6]).abs() < 1e-12,
                "{designation}: slope {} != {}",
                stored[6],
                values[6]
            );
            assert_eq!(section.area_mm2, Some(area), "{designation}");
            assert_eq!(section.mass_per_meter_kg, Some(mass), "{designation}");
        };
        spot(
            "MB200",
            [200.0, 100.0, 5.7, 10.0, 11.0, 5.5, k98],
            3080.0,
            24.17,
        );
        spot(
            "MC200",
            [200.0, 75.0, 6.2, 11.4, 11.0, 3.2, k96],
            2840.0,
            22.3,
        );
        spot(
            "HB450x92.19",
            [450.0, 250.0, 11.3, 13.7, 15.0, 7.5, k94],
            11700.0,
            92.19,
        );
        spot(
            "LBP100",
            [100.0, 50.0, 4.3, 7.0, 8.0, 3.0, k915],
            1110.0,
            8.75,
        );
        spot(
            "LC400",
            [400.0, 100.0, 8.0, 14.0, 14.0, 4.8, k96],
            5820.0,
            45.8,
        );
    }

    #[test]
    fn the_shipped_china_library_reads_with_every_entry_admitted() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/china/profiles-gb-t-706.usda"
        );
        let library = load(path).unwrap();
        assert_eq!(library.info.id, "china-gb-t-706-profiles");
        assert_eq!(library.info.version.as_deref(), Some("2"));
        assert!(
            library
                .info
                .licence
                .as_deref()
                .is_some_and(|l| !l.is_empty()),
            "the shipped library declares its licence"
        );
        assert!(
            library
                .info
                .source
                .as_deref()
                .is_some_and(|s| !s.is_empty()),
            "the shipped library declares its source"
        );
        // No transcription slips: every entry is admitted and none warns.
        assert_eq!(library.rejected, []);
        assert_eq!(library.entries.len(), 86);
        // GB/T 706-2016 prints the slopes exactly: 1:6 for the I-beams, 1:10 for the channels.
        const I_SLOPE: f64 = 1.0 / 6.0;
        const C_SLOPE: f64 = 1.0 / 10.0;
        let mut designations = Vec::new();
        for entry in &library.entries {
            let DefinitionRecord::SectionProfile {
                shape: SectionShape::Parametric(parametric),
                section,
            } = &entry.record
            else {
                panic!("{} is not a parametric section", entry.designation);
            };
            let (slope, identity_k, series) = match parametric.family() {
                SectionFamily::TaperedI => (I_SLOPE, 0.615, "I"),
                SectionFamily::TaperedC => (C_SLOPE, 0.3075, "C"),
                family => panic!("{}: unexpected family {family:?}", entry.designation),
            };
            assert_eq!(
                parametric.values()[6],
                slope,
                "{}: the {series} slope",
                entry.designation
            );
            assert_eq!(
                section.standard.as_deref(),
                Some("GB/T 706-2016"),
                "{}",
                entry.designation
            );
            // The table's own identity A = h*s + 2*t*(b - s) + k*(r^2 - r1^2) -- the standard's
            // slope coefficient, 0.615 (I) or 0.3075 (C) -- reproduces every published area of
            // both tables within 0.17 %, so a slip in any transcribed dimension breaks here,
            // before the section geometry is even consulted.
            let v = parametric.values();
            let identity =
                v[0] * v[2] + 2.0 * v[3] * (v[1] - v[2]) + identity_k * (v[4] * v[4] - v[5] * v[5]);
            let declared = section.area_mm2.unwrap();
            assert!(
                (identity - declared).abs() <= 0.002 * declared,
                "{}: identity area {identity} mm2 vs declared {declared}",
                entry.designation
            );
            // The closed-form area of the declared dimensions must match the declared catalogue
            // area (the D3 1 percent warning never fires) and the declared mass must be that
            // area at 7850 kg/m3 within the standard's published rounding. The mass prints
            // with one decimal to 100 kg/m and as an integer above it (50b "101" is 101.42
            // rounded), so 0.55 kg/m covers every row.
            assert!(
                entry.record.declared_area_warning().is_none(),
                "{}",
                entry.designation
            );
            let area = section.area_mm2.unwrap();
            let mass = section.mass_per_meter_kg.unwrap();
            let mass_from_area = area * 7850.0 / 1.0e6;
            assert!(
                (mass - mass_from_area).abs() <= 0.55,
                "{}: mass {mass} kg/m vs area-derived {mass_from_area}",
                entry.designation
            );
            designations.push(entry.designation.clone());
        }
        // Both series exactly as tables A.1 and A.2 of GB/T 706-2016 publish them: 45 I-beam
        // designations (the 2016-edition 25a/b beams and the 55 series beside the classic 56
        // series) and 41 channel designations (the 2016-edition 24c/25/27c/28c channels), in
        // the library's lexicographic designation order.
        assert_eq!(
            designations,
            [
                "C10", "C12", "C12.6", "C14a", "C14b", "C16a", "C16b", "C18a", "C18b", "C20a",
                "C20b", "C22a", "C22b", "C24a", "C24b", "C24c", "C25a", "C25b", "C25c", "C27a",
                "C27b", "C27c", "C28a", "C28b", "C28c", "C30a", "C30b", "C30c", "C32a", "C32b",
                "C32c", "C36a", "C36b", "C36c", "C40a", "C40b", "C40c", "C5", "C6.3", "C6.5", "C8",
                "I10", "I12", "I12.6", "I14", "I16", "I18", "I20a", "I20b", "I22a", "I22b", "I24a",
                "I24b", "I25a", "I25b", "I27a", "I27b", "I28a", "I28b", "I30a", "I30b", "I30c",
                "I32a", "I32b", "I32c", "I36a", "I36b", "I36c", "I40a", "I40b", "I40c", "I45a",
                "I45b", "I45c", "I50a", "I50b", "I50c", "I55a", "I55b", "I55c", "I56a", "I56b",
                "I56c", "I63a", "I63b", "I63c",
            ]
        );
        // Spot checks against the transcribed values.
        let spot = |designation: &str, values: [f64; 7], area: f64, mass: f64| {
            let entry = library.entry(designation).unwrap();
            let DefinitionRecord::SectionProfile {
                shape: SectionShape::Parametric(p),
                section,
            } = &entry.record
            else {
                panic!("{designation} is not a parametric section");
            };
            assert_eq!(p.values(), values, "{designation}");
            assert_eq!(section.area_mm2, Some(area), "{designation}");
            assert_eq!(section.mass_per_meter_kg, Some(mass), "{designation}");
        };
        spot(
            "I10",
            [100.0, 68.0, 4.5, 7.6, 6.5, 3.3, I_SLOPE],
            1433.0,
            11.3,
        );
        spot(
            "I25a",
            [250.0, 116.0, 8.0, 13.0, 10.0, 5.0, I_SLOPE],
            4851.0,
            38.1,
        );
        spot(
            "I55a",
            [550.0, 166.0, 12.5, 21.0, 14.5, 7.3, I_SLOPE],
            13410.0,
            105.0,
        );
        spot(
            "I63c",
            [630.0, 180.0, 17.0, 22.0, 15.0, 7.5, I_SLOPE],
            17980.0,
            141.0,
        );
        spot("C5", [50.0, 37.0, 4.5, 7.0, 7.0, 3.5, C_SLOPE], 692.5, 5.44);
        spot(
            "C25c",
            [250.0, 82.0, 11.0, 12.0, 12.0, 6.0, C_SLOPE],
            4491.0,
            35.3,
        );
        spot(
            "C40c",
            [400.0, 104.0, 14.5, 18.0, 18.0, 9.0, C_SLOPE],
            9104.0,
            71.5,
        );
        // The series gaps the standard does not publish stay NOT_FOUND -- for the channels
        // including the b/c variants the flawed web transcription left blank: the official
        // table A.2 has no channel 18c or 22c.
        for absent in [
            "C5.5", "C7", "C9", "C11", "C13", "C13.5", "C15", "C17", "C18c", "C19", "C21", "C22c",
            "C23", "C26", "C29", "C31", "C33", "C41", "C45", "C63", "I11", "I13", "I21", "I23",
            "I26", "I25c", "I29", "I31", "I33", "I38", "I42", "I47", "I52", "I57", "I58", "I65",
        ] {
            assert_eq!(
                library.entry(absent).unwrap_err().code,
                ErrorCode::NotFound,
                "{absent} is not in the shipped series"
            );
        }
    }

    fn env_layer(profiles: &str, fasteners: Option<&str>) -> String {
        format!(
            "#usda 1.0\n(\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n    customLayerData = {{\n        string \"bim:environment:id\" = \"e\"\n        string[] \"bim:environment:libraries\" = [{profiles}]{}\n    }}\n)\n",
            match fasteners {
                Some(list) =>
                    format!("\n        string[] \"bim:environment:fastener-libraries\" = [{list}]"),
                None => String::new(),
            }
        )
    }

    fn provenance(id: &str) -> String {
        format!(
            "        string \"bim:library:id\" = \"{id}\"\n        string \"bim:library:source\" = \"Contributor-original synthetic values for tests\"\n        string \"bim:library:licence\" = \"MPL-2.0\"\n"
        )
    }

    /// Writes `text` under `dir/name`, creating folders, and answers the absolute path.
    fn put(dir: &Path, name: &str, text: &str) -> String {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, text).unwrap();
        path.to_str().unwrap().to_owned()
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("library-firm-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_firm_file_splits_by_its_own_entries() {
        let only_profiles =
            load_firm_bytes(library(&[entry("A", "F-1", I_BODY)]).into_bytes()).unwrap();
        assert_eq!(only_profiles.0.entries.len(), 1);
        assert_eq!(only_profiles.0.entries[0].designation, "F-1");
        assert_eq!(
            only_profiles.1.entries.len(),
            0,
            "no fastener half of a profile file"
        );
        assert_eq!(
            only_profiles.2.entries.len(),
            0,
            "no material half of a profile file"
        );
        let only_bolts = load_firm_bytes(
            fastener_library(&[fastener_entry("B", "SYN-M12", BOLT_BODY)]).into_bytes(),
        )
        .unwrap();
        assert_eq!(
            only_bolts.0.entries.len(),
            0,
            "no profile half of a fastener file"
        );
        assert_eq!(only_bolts.1.entries.len(), 1);
        assert_eq!(only_bolts.2.entries.len(), 0);
        // Every parent in one file: every entry decodes by its own parent into its own half.
        let mixed = format!(
            "{}def Scope \"Library\" {{\n    def Scope \"Profiles\" {{\n{}    }}\n    def Scope \"Fasteners\" {{\n{}    }}\n    def Scope \"Materials\" {{\n{}    }}\n}}\n",
            HEADER.replace("    }\n)\n", "    }\n)\n\n"),
            entry("A", "F-1", I_BODY),
            fastener_entry("B", "SYN-M12", BOLT_BODY),
            "        def Scope \"C\"\n        {\nstring steel:material:grade = \"S355\"\ndouble steel:material:densityKgM3 = 7850\n        }\n"
        );
        let mixed = load_firm_bytes(mixed.into_bytes()).unwrap();
        assert_eq!(mixed.0.entries.len(), 1);
        assert_eq!(mixed.1.entries.len(), 1);
        assert_eq!(mixed.2.entries.len(), 1);
        assert_eq!(mixed.2.entries[0].designation, "S355");
        assert_eq!(
            load_firm_bytes(b"not usd".to_vec()).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
    }

    #[test]
    fn a_mixed_firm_file_feeds_both_namespaces_of_the_layer() {
        let dir = scratch("mixed");
        let config = dir.join("config");
        put(
            &config,
            "libraries/mixed.usda",
            &format!(
                "{}def Scope \"Library\" {{\n    def Scope \"Profiles\" {{\n{}    }}\n    def Scope \"Fasteners\" {{\n{}    }}\n    def Scope \"Materials\" {{\n{}    }}\n}}\n",
                HEADER.replace("    }\n)\n", "    }\n)\n\n"),
                entry("A", "F-1", I_BODY),
                fastener_entry("B", "SYN-M12", BOLT_BODY),
                "        def Scope \"C\"\n        {\nstring steel:material:grade = \"S355\"\ndouble steel:material:densityKgM3 = 7850\n        }\n"
            ),
        );
        let layer = load_firm(Some(&config));
        assert!(layer.refused.is_empty());
        assert_eq!(layer.sources.len(), 1, "the profile half of the mixed file");
        assert_eq!(layer.sources[0].library.info.id, "t");
        assert_eq!(layer.sources[0].library.entries[0].designation, "F-1");
        assert_eq!(
            layer.fastener_sources.len(),
            1,
            "the fastener half of the mixed file"
        );
        assert_eq!(
            layer.fastener_sources[0].library.info.id, "t",
            "every half carries the file's own id"
        );
        assert_eq!(
            layer.fastener_sources[0].library.entries[0].designation,
            "SYN-M12"
        );
        assert_eq!(
            layer.material_sources.len(),
            1,
            "the material half of the mixed file"
        );
        assert_eq!(
            layer.material_sources[0].library.entries[0].designation,
            "S355"
        );
        let lookup = Lookup::new(layer.sources);
        assert_eq!(
            lookup.find("F-1").map(|(_, e)| e.designation.as_str()),
            Ok("F-1")
        );
        let bolts = Lookup::new(layer.fastener_sources);
        assert_eq!(
            bolts.find("SYN-M12").map(|(_, e)| e.designation.as_str()),
            Ok("SYN-M12")
        );
        let grades = Lookup::new(layer.material_sources);
        assert_eq!(
            grades.find("S355").map(|(_, e)| e.designation.as_str()),
            Ok("S355")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_firm_layer_loads_in_order_reports_and_writes_nothing() {
        let dir = scratch("layer");
        let config = dir.join("config");
        // Files load in file-name order: the bolt file takes its id first, the second file of
        // the same id is refused as a duplicate.
        put(
            &config,
            "libraries/a-bolts.usda",
            &fastener_library(&[fastener_entry("B", "SYN-M12", BOLT_BODY)]).replace(
                "        string \"bim:library:id\" = \"t\"\n",
                &provenance("firm-bolts"),
            ),
        );
        put(
            &config,
            "libraries/again.usda",
            &library(&[entry("A", "F-2", I_BODY)]).replace(
                "        string \"bim:library:id\" = \"t\"\n",
                &provenance("firm-profiles"),
            ),
        );
        put(
            &config,
            "libraries/z-profiles.usda",
            &library(&[entry("A", "F-ONLY", I_BODY)]).replace(
                "        string \"bim:library:id\" = \"t\"\n",
                &provenance("firm-profiles"),
            ),
        );
        put(&config, "libraries/garbage.usda", "not usd");
        let bolts_before = std::fs::read(config.join("libraries/a-bolts.usda")).unwrap();
        let layer = load_firm(Some(&config));
        let names: Vec<&str> = layer
            .sources
            .iter()
            .map(|s| s.library.info.id.as_str())
            .collect();
        assert_eq!(names, ["firm-profiles"]);
        assert_eq!(
            layer.sources[0].layer, LAYER_FIRM,
            "every firm source names its layer"
        );
        let bolts: Vec<&str> = layer
            .fastener_sources
            .iter()
            .map(|s| s.library.info.id.as_str())
            .collect();
        assert_eq!(
            bolts,
            ["firm-bolts"],
            "fastener entries join their own list"
        );
        let refused: Vec<&str> = layer
            .refused
            .iter()
            // The stored path is the platform's own spelling, so the file name comes off
            // the path the same way (`Path::file_name`), never by splitting on `/`.
            .map(|r| {
                std::path::Path::new(r.path.as_str())
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap()
            })
            .collect();
        assert_eq!(
            refused,
            ["garbage.usda", "z-profiles.usda"],
            "the unreadable file and the duplicate id, in file-name order"
        );
        assert_eq!(layer.refused[0].code, ErrorCode::InvalidArgument);
        assert_eq!(
            std::fs::read(config.join("libraries/a-bolts.usda")).unwrap(),
            bolts_before,
            "loading never writes a firm file"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_firm_layer_shadows_an_environment_and_names_both_layers() {
        let dir = scratch("shadow");
        let config = dir.join("config");
        put(
            &dir,
            "libs/p.usda",
            &library(&[entry("A", "X-1", I_BODY), entry("B", "ENV-ONLY", I_BODY)]).replace(
                "        string \"bim:library:id\" = \"t\"\n",
                &provenance("env-lib"),
            ),
        );
        let env = put(&dir, "env.usda", &env_layer("\"libs/p.usda\"", None));
        put(
            &config,
            "libraries/mine.usda",
            &library(&[entry("A", "X-1", I_BODY), entry("B", "FIRM-ONLY", I_BODY)]).replace(
                "        string \"bim:library:id\" = \"t\"\n",
                &provenance("firm-lib"),
            ),
        );
        let opened = open_lookup(None, Some(&env), None, Some(&config)).unwrap();
        assert_eq!(opened.environment.unwrap().id, "e");
        assert!(opened.firm_refused.is_empty());
        assert_eq!(opened.lookup.find("X-1").unwrap().0.layer, LAYER_FIRM);
        assert_eq!(
            opened.lookup.find("ENV-ONLY").unwrap().0.layer,
            LAYER_ENVIRONMENT
        );
        assert_eq!(opened.lookup.find("FIRM-ONLY").unwrap().0.layer, LAYER_FIRM);
        assert_eq!(
            opened.lookup.shadowed_for("X-1"),
            [Shadowed {
                designation: "X-1".into(),
                used_library: "firm-lib".into(),
                used_layer: LAYER_FIRM,
                shadowed_library: "env-lib".into(),
                shadowed_layer: LAYER_ENVIRONMENT,
            }]
        );
        // A single --library lookup names every source exactly: no firm layer joins it.
        let alone = open_lookup(
            Some(dir.join("libs/p.usda").to_str().unwrap()),
            None,
            None,
            Some(&config),
        )
        .unwrap();
        assert_eq!(alone.lookup.sources().len(), 1);
        assert_eq!(alone.lookup.sources()[0].layer, LAYER_LIBRARY);
        assert!(alone.firm_refused.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_firm_file_that_does_not_read_is_reported_not_fatal() {
        let dir = scratch("refused");
        let config = dir.join("config");
        put(&config, "libraries/broken.usda", "not usd");
        put(
            &dir,
            "libs/p.usda",
            &library(&[entry("A", "ENV-ONLY", I_BODY)]).replace(
                "        string \"bim:library:id\" = \"t\"\n",
                &provenance("env-lib"),
            ),
        );
        let env = put(&dir, "env.usda", &env_layer("\"libs/p.usda\"", None));
        let opened = open_lookup(None, Some(&env), None, Some(&config)).unwrap();
        assert_eq!(
            opened.lookup.find("ENV-ONLY").unwrap().0.layer,
            LAYER_ENVIRONMENT
        );
        assert_eq!(opened.firm_refused.len(), 1);
        assert_eq!(opened.firm_refused[0].code, ErrorCode::InvalidArgument);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_named_material_environment_takes_the_firm_layer_above_it() {
        let dir = scratch("material-order");
        let config = dir.join("config");
        let material_library = |id: &str, grades: &[(&str, f64)]| {
            let entries: String = grades
                .iter()
                .enumerate()
                .map(|(n, (grade, density))| {
                    format!(
                        "        def Scope \"E{n}\"\n        {{\nstring steel:material:grade = \"{grade}\"\ndouble steel:material:densityKgM3 = {density}\n        }}\n"
                    )
                })
                .collect();
            format!(
                "#usda 1.0\n(\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n    customLayerData = {{\n        string \"bim:library:id\" = \"{id}\"\n        string \"bim:library:source\" = \"Contributor-original synthetic values for tests\"\n        string \"bim:library:licence\" = \"MPL-2.0\"\n    }}\n)\ndef Scope \"Library\"\n{{\n    def Scope \"Materials\"\n    {{\n{entries}    }}\n}}\n"
            )
        };
        // The named environment declares S355 at 7800 and ENV-GRADE; the firm layer declares
        // S355 at 7850. The firm layer answers S355, as it does for sections and bolts.
        put(
            &dir,
            "libs/p.usda",
            &library(&[entry("A", "P-1", I_BODY)]).replace(
                "        string \"bim:library:id\" = \"t\"\n",
                &provenance("env-lib"),
            ),
        );
        put(
            &dir,
            "libs/grades.usda",
            &material_library("env-grades", &[("S355", 7800.0), ("ENV-GRADE", 7700.0)]),
        );
        put(
            &config,
            "libraries/my-grades.usda",
            &material_library("firm-grades", &[("S355", 7850.0)]),
        );
        let env = put(
            &dir,
            "material-env.usda",
            "#usda 1.0\n(\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n    customLayerData = {\n        string \"bim:environment:id\" = \"e\"\n        string[] \"bim:environment:libraries\" = [\"libs/p.usda\"]\n        string[] \"bim:environment:material-libraries\" = [\"libs/grades.usda\"]\n    }\n)\n",
        );
        let opened = open_material_lookup(None, Some(&env), Some(&config)).unwrap();
        let (source, entry) = opened.0.find("S355").unwrap();
        assert_eq!(source.layer, LAYER_FIRM);
        let density = match &entry.record {
            DefinitionRecord::Material { density_kg_m3, .. } => density_kg_m3.unwrap(),
            other => panic!("not a material entry: {other:?}"),
        };
        assert_eq!(density, 7850.0);
        assert_eq!(
            opened.0.find("ENV-GRADE").unwrap().0.layer,
            LAYER_ENVIRONMENT
        );
        let shadows = opened.0.shadowed_for("S355");
        assert_eq!(shadows.len(), 1);
        assert_eq!(shadows[0].used_layer, LAYER_FIRM);
        assert_eq!(shadows[0].shadowed_layer, LAYER_ENVIRONMENT);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_neutral_layer_fills_the_gaps_and_skips_taken_ids() {
        let neutral = neutral_environment().unwrap().unwrap();
        assert_eq!(
            neutral.info.id,
            session::environment::NEUTRAL_ENVIRONMENT_ID,
            "the shipped neutral environment is the fallback layer"
        );
        let mut sources: Vec<Source> = Vec::new();
        let mut fasteners: Vec<Source> = Vec::new();
        let mut materials: Vec<Source> = Vec::new();
        append_neutral(&mut sources, &mut fasteners, &mut materials).unwrap();
        assert!(
            fasteners.is_empty() && materials.is_empty(),
            "the neutral environment ships no fastener or material libraries"
        );
        assert!(sources.iter().all(|s| s.layer == LAYER_NEUTRAL));
        let count = sources.len();
        assert!(
            count > 0,
            "the shipped neutral environment has profile libraries"
        );
        // A chain that already holds a library of the same id takes no neutral copy of it.
        let mut taken = vec![Source {
            path: "copy".into(),
            library: sources[0].library.clone(),
            layer: LAYER_PROJECT,
        }];
        let taken_count = taken.len();
        append_neutral(&mut taken, &mut fasteners, &mut materials).unwrap();
        assert_eq!(
            taken.len(),
            taken_count,
            "the id already in the chain is skipped"
        );
        assert_eq!(sources.len(), count);
    }

    #[test]
    fn the_project_chain_orders_project_firm_and_neutral() {
        let dir = scratch("chain");
        let config = dir.join("config");
        let store = dir.join("store");
        // The project's copy of its environment: one profile library, one fastener library,
        // both listed by contained relative references next to the environment file.
        put(
            &store,
            "environment/p.usda",
            &library(&[entry("A", "X-1", I_BODY)]).replace(
                "        string \"bim:library:id\" = \"t\"\n",
                &provenance("copy-lib"),
            ),
        );
        put(
            &store,
            "environment/b.usda",
            &fastener_library(&[fastener_entry("B", "SYN-M12", BOLT_BODY)]).replace(
                "        string \"bim:library:id\" = \"t\"\n",
                &provenance("copy-bolts"),
            ),
        );
        put(
            &store,
            "environment/environment.usda",
            &env_layer("\"p.usda\"", Some("\"b.usda\"")),
        );
        // A firm profile library of the same designation: the project copy is stronger.
        put(
            &config,
            "libraries/mine.usda",
            &library(&[entry("A", "X-1", I_BODY), entry("B", "FIRM-ONLY", I_BODY)]).replace(
                "        string \"bim:library:id\" = \"t\"\n",
                &provenance("firm-lib"),
            ),
        );
        let chain = project_library_chain(&store, Some(&config)).unwrap();
        let layers: Vec<&str> = chain.lookup.sources().iter().map(|s| s.layer).collect();
        assert_eq!(
            layers,
            [LAYER_PROJECT, LAYER_FIRM, LAYER_NEUTRAL],
            "the project copy, then firm, then the neutral fallback"
        );
        assert_eq!(chain.lookup.find("X-1").unwrap().0.layer, LAYER_PROJECT);
        assert_eq!(chain.lookup.find("FIRM-ONLY").unwrap().0.layer, LAYER_FIRM);
        assert_eq!(
            chain.lookup.shadowed_for("X-1").len(),
            1,
            "the firm entry of the same designation is reported, the neutral one does not declare it"
        );
        assert!(chain.environment.is_some());
        assert!(chain.firm_refused.is_empty());
        // Fasteners ride the same chain in their own namespace.
        let bolts = chain.fastener_lookup.unwrap();
        assert_eq!(bolts.find("SYN-M12").unwrap().0.layer, LAYER_PROJECT);
        // A project without a copy resolves from firm and neutral alone.
        let bare = project_library_chain(&dir.join("no-copy"), Some(&config)).unwrap();
        assert!(bare.environment.is_none());
        let layers: Vec<&str> = bare.lookup.sources().iter().map(|s| s.layer).collect();
        assert_eq!(layers, [LAYER_FIRM, LAYER_NEUTRAL]);
        assert!(
            bare.fastener_lookup.is_none(),
            "no fastener layer of this chain declares bolts"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_recorded_environment_whose_copy_folder_is_gone_warns_with_its_identity() {
        use crate::project::{ProjectContext, ProjectEnvironmentChoice, create_project};
        use beamcraft_host::revision::NoFault;
        use beamcraft_model::id::{OpId, ProjectId, RevisionId, Uuid7, WorkspaceId};

        let dir = scratch("recorded");
        let config = dir.join("config");
        put(
            &config,
            "libraries/mine.usda",
            &library(&[entry("A", "FIRM-ONLY", I_BODY)]).replace(
                "        string \"bim:library:id\" = \"t\"\n",
                &provenance("firm-lib"),
            ),
        );
        let context = ProjectContext {
            op_id: OpId::parse("0199e7a0-0000-7000-8000-0000000000c1").unwrap(),
            revision_id: RevisionId::parse("0199e7a0-0000-7000-8000-0000000000c2").unwrap(),
            actor: Uuid7::parse("0199e7a0-0000-7000-8000-0000000000c3").unwrap(),
            project: ProjectId::parse("0199e7a0-0000-7000-8000-0000000000c4").unwrap(),
            workspace: WorkspaceId::parse("0199e7a0-0000-7000-8000-0000000000c5").unwrap(),
        };
        let next = |n: u64| ProjectContext {
            op_id: OpId::parse(&format!("0199e7a0-0000-7000-8000-0000000000{n:x}")).unwrap(),
            revision_id: RevisionId::parse(&format!("0199e7a0-0000-7000-8000-0000000000{n:x}"))
                .unwrap(),
            project: ProjectId::parse(&format!("0199e7a0-0000-7000-8000-0000000000{n:x}")).unwrap(),
            ..context.clone()
        };
        // A store whose root records the chosen environment, with no copy folder at all: the
        // loss is reported with the identity the store carried, and the chain resolves from
        // firm and neutral alone.
        let recorded = create_project(
            &dir,
            "Recorded",
            Some(&ProjectEnvironmentChoice {
                id: "acme".to_owned(),
                version: Some("7".to_owned()),
            }),
            &next(0xd1),
            &mut NoFault,
        )
        .unwrap();
        let chain = project_library_chain(&recorded.path, Some(&config)).unwrap();
        let layers: Vec<&str> = chain.lookup.sources().iter().map(|s| s.layer).collect();
        assert_eq!(layers, [LAYER_FIRM, LAYER_NEUTRAL]);
        let lost = chain
            .environment_lost
            .expect("the recorded loss is reported");
        assert!(
            // Component comparison, not a raw string suffix: the recorded loss path is a
            // platform path, so on Windows it carries `\` separators.
            std::path::Path::new(&lost.path).ends_with("environment/environment.usda"),
            "{lost:?}"
        );
        assert!(
            lost.message.contains("acme") && lost.message.contains("version 7"),
            "{lost:?}"
        );
        // A plain project without a record warns nowhere, exactly as before.
        let plain = create_project(&dir, "Plain", None, &next(0xd2), &mut NoFault).unwrap();
        let chain = project_library_chain(&plain.path, Some(&config)).unwrap();
        assert!(chain.environment_lost.is_none(), "{chain:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_shipped_canada_w_and_hp_library_reads_with_every_entry_admitted() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/canada/profiles-astm-a6-w-hp.usda"
        );
        let library = load(path).unwrap();
        assert_eq!(library.info.id, "canada-astm-a6-w-hp-profiles");
        assert_eq!(library.info.version.as_deref(), Some("1"));
        assert!(
            library
                .info
                .licence
                .as_deref()
                .is_some_and(|l| !l.is_empty()),
            "the shipped library declares its licence"
        );
        assert!(
            library
                .info
                .source
                .as_deref()
                .is_some_and(|s| !s.is_empty()),
            "the shipped library declares its source"
        );
        // No transcription slips: every entry is admitted.
        assert_eq!(library.rejected, []);
        // Every row of the USA pack appears twice: once under the metric designation the
        // standard prints for it (284 rows; the -05a tables print W920 X 345 for two distinct
        // rows, and the name is carried once, see below) and once under the EDI inch
        // designation the USA pack ships, as the alias.
        assert_eq!(library.entries.len(), 569);
        // The thirteen shapes the Nucor-Yamato catalog does not roll have no printed radius
        // and carry a sharp corner; every other row carries one of the catalog's radii. Each
        // sharp row is sharp in both of its designation forms.
        let sharp = [
            "W1000x494",
            "W40X331",
            "W920x1191",
            "W36X800",
            "W310x21.0",
            "W12X14",
            "W250x28.4",
            "W10X19",
            "W250x25.3",
            "W10X17",
            "W250x22.3",
            "W10X15",
            "W250x17.9",
            "W10X12",
            "W200x22.5",
            "W8X15",
            "W200x19.3",
            "W8X13",
            "W200x15.0",
            "W8X10",
            "W130x28.1",
            "W5X19",
            "W130x23.8",
            "W5X16",
            "W100x19.3",
            "W4X13",
        ];
        let catalog_radii = [8.0, 8.5, 10.0, 13.0, 15.0, 19.0, 20.0, 30.0];
        let mut designations = Vec::new();
        let mut warned = 0;
        for entry in &library.entries {
            let DefinitionRecord::SectionProfile {
                shape: SectionShape::Parametric(parametric),
                section,
            } = &entry.record
            else {
                panic!("{} is not a parametric section", entry.designation);
            };
            // W and HP shapes have parallel flanges: the i_shape family.
            assert_eq!(parametric.family(), SectionFamily::IShape);
            assert_eq!(
                section.standard.as_deref(),
                Some("ASTM A6/A6M-05a"),
                "{}",
                entry.designation
            );
            let radius = parametric.values()[4];
            if sharp.contains(&entry.designation.as_str()) {
                assert_eq!(radius, 0.0, "{}", entry.designation);
            } else {
                assert!(
                    catalog_radii.contains(&radius),
                    "{}: radius {radius}",
                    entry.designation
                );
            }
            // Like the USA pack, this one expects the D3 warning (the standard prints no
            // radius and rounds its SI dimensions; owner decision of 2026-10-18: a warning,
            // never a refusal). Each of the 150 USA-warned rows warns in both of its
            // designation forms: 300 of the 569 entries, none beyond 5 percent. The nominal
            // SI mass of the designation follows the area at 7850 kg/m3 within 2.2 percent
            // (worst W14X38, whose nominal 58 kg/m rounds up from the 56.7 its area gives).
            let area = section.area_mm2.unwrap();
            let mass = section.mass_per_meter_kg.unwrap();
            let mass_from_area = area * 7850.0 / 1.0e6;
            assert!(
                (mass - mass_from_area).abs() / mass_from_area <= 0.022,
                "{}: mass {mass} kg/m vs area-derived {mass_from_area}",
                entry.designation
            );
            if let Some(warning) = entry.record.declared_area_warning() {
                warned += 1;
                assert_eq!(warning.declared_mm2, area, "{}", entry.designation);
                assert!(
                    warning.relative_difference < 0.05,
                    "{}: declared area {area} mm2 is {} off the outline",
                    entry.designation,
                    warning.relative_difference
                );
            }
            designations.push(entry.designation.clone());
        }
        assert_eq!(warned, 300, "entries carrying the declared-area warning");
        // Spot checks against the standard's printed SI column and the catalog's radius:
        // designation, [h, b, tw, tf, r], area mm2, nominal mass kg/m. Every row is spot
        // checked in both designation forms; the values are the USA pack's, verified there
        // against both public copies of the standard.
        let spot = |designation: &str, values: [f64; 5], area: f64, mass: f64| {
            let entry = library.entry(designation).unwrap();
            let DefinitionRecord::SectionProfile {
                shape: SectionShape::Parametric(p),
                section,
            } = &entry.record
            else {
                panic!("{designation} is not a parametric section");
            };
            assert_eq!(p.values(), values, "{designation}");
            assert_eq!(section.area_mm2, Some(area), "{designation}");
            assert_eq!(section.mass_per_meter_kg, Some(mass), "{designation}");
        };
        spot("W310x60", [303.0, 203.0, 7.5, 13.1, 19.0], 7610.0, 60.0);
        spot("W12X40", [303.0, 203.0, 7.5, 13.1, 19.0], 7610.0, 60.0);
        spot(
            "W1100x499",
            [1118.0, 405.0, 26.0, 45.0, 30.0],
            63500.0,
            499.0,
        );
        spot("W44X335", [1118.0, 405.0, 26.0, 45.0, 30.0], 63500.0, 499.0);
        // The W 36 X 800 designation cell prints its area before its weight in both public
        // copies (`W36 X 236.4800`); the row is read from its metric designation.
        spot(
            "W920x1191",
            [1081.0, 457.0, 60.5, 109.0, 0.0],
            152500.0,
            1191.0,
        );
        spot(
            "W36X800",
            [1081.0, 457.0, 60.5, 109.0, 0.0],
            152500.0,
            1191.0,
        );
        spot(
            "HP360x174",
            [361.0, 378.0, 20.4, 20.4, 30.0],
            22200.0,
            174.0,
        );
        spot("HP14X117", [361.0, 378.0, 20.4, 20.4, 30.0], 22200.0, 174.0);
        spot("W100x19.3", [106.0, 103.0, 7.1, 8.8, 0.0], 2470.0, 19.3);
        spot("W4X13", [106.0, 103.0, 7.1, 8.8, 0.0], 2470.0, 19.3);
        // The one ambiguous designation of the declared edition: the -05a tables print
        // W920 X 345 for two distinct rows (W36X231: d 927, bf 418 — printed first — and
        // W36X232: d 943, bf 308). The metric name is carried once, for the row the table
        // prints first; W36X232 answers only through its inch alias. The newer-edition size
        // list of the Nucor-Yamato catalog distinguishes the two (W920 x 344 / W920 x 345),
        // but those masses belong to that edition.
        spot("W920x345", [927.0, 418.0, 19.3, 32.0, 30.0], 44200.0, 345.0);
        spot("W36X232", [943.0, 308.0, 22.1, 39.9, 30.0], 44000.0, 345.0);
        // The declared-area warning: widest on W24X68 / W610x101, where the catalog radius is
        // larger than the fillet the standard's area allows; absent where the two agree.
        let difference = |designation: &str| {
            library
                .entry(designation)
                .unwrap()
                .record
                .declared_area_warning()
                .map(|w| w.relative_difference)
        };
        assert!((difference("W610x101").unwrap() - 0.0431).abs() < 0.001);
        assert!((difference("W24X68").unwrap() - 0.0431).abs() < 0.001);
        assert!((difference("W310x60").unwrap() - 0.0123).abs() < 0.0005);
        assert_eq!(difference("W150x13.0"), None);
        // The W and HP rows of the declared tables under their metric designations,
        // 274 + 11 minus the one ambiguous name, exactly as Annex A2 prints them.
        let metric_assortment = [
            "W1100x499 W1100x433 W1100x390 W1100x343",
            "W1000x883 W1000x748 W1000x642 W1000x591 W1000x554 W1000x539 W1000x483 W1000x443 W1000x412 W1000x371",
            "W1000x321 W1000x296 W1000x584 W1000x494 W1000x486 W1000x438 W1000x415 W1000x393 W1000x350 W1000x314",
            "W1000x272 W1000x249 W1000x222",
            "W920x1191 W920x970 W920x787 W920x725 W920x656 W920x588 W920x537 W920x491 W920x449 W920x420",
            "W920x390 W920x368 W920x345 W920x381 W920x313 W920x289 W920x271 W920x253 W920x238 W920x223",
            "W920x201",
            "W840x576 W840x527 W840x473 W840x433 W840x392 W840x359 W840x329 W840x299 W840x251 W840x226",
            "W840x210 W840x193 W840x176",
            "W760x582 W760x531 W760x484 W760x434 W760x389 W760x350 W760x314 W760x284 W760x257 W760x220",
            "W760x196 W760x185 W760x173 W760x161 W760x147 W760x134",
            "W690x802 W690x548 W690x500 W690x457 W690x419 W690x384 W690x350 W690x323 W690x289 W690x265",
            "W690x240 W690x217 W690x192 W690x170 W690x152 W690x140 W690x125",
            "W610x551 W610x498 W610x455 W610x415 W610x372 W610x341 W610x307 W610x285 W610x262 W610x241",
            "W610x217 W610x195 W610x174 W610x155 W610x153 W610x140 W610x125 W610x113 W610x101 W610x92",
            "W610x82",
            "W530x300 W530x272 W530x248 W530x219 W530x196 W530x182 W530x165 W530x150 W530x138 W530x123",
            "W530x109 W530x101 W530x92 W530x82 W530x72 W530x85 W530x74 W530x66",
            "W460x464 W460x421 W460x384 W460x349 W460x315 W460x286 W460x260 W460x235 W460x213 W460x193",
            "W460x177 W460x158 W460x144 W460x128 W460x113 W460x106 W460x97 W460x89 W460x82 W460x74",
            "W460x68 W460x60 W460x52",
            "W410x149 W410x132 W410x114 W410x100 W410x85 W410x75 W410x67 W410x60 W410x53 W410x46.1",
            "W410x38.8",
            "W360x1086 W360x990 W360x900 W360x818 W360x744 W360x677 W360x634 W360x592 W360x551 W360x509",
            "W360x463 W360x421 W360x382 W360x347 W360x314 W360x287 W360x262 W360x237 W360x216 W360x196",
            "W360x179 W360x162 W360x147 W360x134 W360x122 W360x110 W360x101 W360x91 W360x79 W360x72",
            "W360x64 W360x58 W360x51 W360x44.6 W360x39.0 W360x32.9",
            "W310x500 W310x454 W310x415 W310x375 W310x342 W310x313 W310x283 W310x253 W310x226 W310x202",
            "W310x179 W310x158 W310x143 W310x129 W310x117 W310x107 W310x97 W310x86 W310x79 W310x74",
            "W310x67 W310x60 W310x52 W310x44.5 W310x38.7 W310x32.7 W310x28.3 W310x23.8 W310x21.0",
            "W250x167 W250x149 W250x131 W250x115 W250x101 W250x89 W250x80 W250x73 W250x67 W250x58",
            "W250x49.1 W250x44.8 W250x38.5 W250x32.7 W250x28.4 W250x25.3 W250x22.3 W250x17.9",
            "W200x100 W200x86 W200x71 W200x59 W200x52 W200x46.1 W200x41.7 W200x35.9 W200x31.3 W200x26.6",
            "W200x22.5 W200x19.3 W200x15.0",
            "W150x37.1 W150x29.8 W150x22.5 W150x24.0 W150x18.0 W150x13.5 W150x13.0",
            "W130x28.1 W130x23.8",
            "W100x19.3",
            "HP360x174 HP360x152 HP360x132 HP360x108",
            "HP310x125 HP310x110 HP310x93 HP310x79",
            "HP250x85 HP250x62",
            "HP200x53",
        ];
        // The alias designations are exactly the USA pack's EDI inch designations, so the
        // same rows resolve in Canada by either name.
        let alias_assortment = [
            "W44X335 W44X290 W44X262 W44X230",
            "W40X593 W40X503 W40X431 W40X397 W40X372 W40X362 W40X324 W40X297 W40X277 W40X249",
            "W40X215 W40X199 W40X392 W40X331 W40X327 W40X294 W40X278 W40X264 W40X235 W40X211",
            "W40X183 W40X167 W40X149",
            "W36X800 W36X652 W36X529 W36X487 W36X441 W36X395 W36X361 W36X330 W36X302 W36X282",
            "W36X262 W36X247 W36X231 W36X256 W36X232 W36X210 W36X194 W36X182 W36X170 W36X160",
            "W36X150 W36X135",
            "W33X387 W33X354 W33X318 W33X291 W33X263 W33X241 W33X221 W33X201 W33X169 W33X152",
            "W33X141 W33X130 W33X118",
            "W30X391 W30X357 W30X326 W30X292 W30X261 W30X235 W30X211 W30X191 W30X173 W30X148",
            "W30X132 W30X124 W30X116 W30X108 W30X99 W30X90",
            "W27X539 W27X368 W27X336 W27X307 W27X281 W27X258 W27X235 W27X217 W27X194 W27X178",
            "W27X161 W27X146 W27X129 W27X114 W27X102 W27X94 W27X84",
            "W24X370 W24X335 W24X306 W24X279 W24X250 W24X229 W24X207 W24X192 W24X176 W24X162",
            "W24X146 W24X131 W24X117 W24X104 W24X103 W24X94 W24X84 W24X76 W24X68 W24X62 W24X55",
            "W21X201 W21X182 W21X166 W21X147 W21X132 W21X122 W21X111 W21X101 W21X93 W21X83 W21X73",
            "W21X68 W21X62 W21X55 W21X48 W21X57 W21X50 W21X44",
            "W18X311 W18X283 W18X258 W18X234 W18X211 W18X192 W18X175 W18X158 W18X143 W18X130",
            "W18X119 W18X106 W18X97 W18X86 W18X76 W18X71 W18X65 W18X60 W18X55 W18X50 W18X46",
            "W18X40 W18X35",
            "W16X100 W16X89 W16X77 W16X67 W16X57 W16X50 W16X45 W16X40 W16X36 W16X31 W16X26",
            "W14X730 W14X665 W14X605 W14X550 W14X500 W14X455 W14X426 W14X398 W14X370 W14X342",
            "W14X311 W14X283 W14X257 W14X233 W14X211 W14X193 W14X176 W14X159 W14X145 W14X132",
            "W14X120 W14X109 W14X99 W14X90 W14X82 W14X74 W14X68 W14X61 W14X53 W14X48 W14X43",
            "W14X38 W14X34 W14X30 W14X26 W14X22",
            "W12X336 W12X305 W12X279 W12X252 W12X230 W12X210 W12X190 W12X170 W12X152 W12X136",
            "W12X120 W12X106 W12X96 W12X87 W12X79 W12X72 W12X65 W12X58 W12X53 W12X50 W12X45",
            "W12X40 W12X35 W12X30 W12X26 W12X22 W12X19 W12X16 W12X14",
            "W10X112 W10X100 W10X88 W10X77 W10X68 W10X60 W10X54 W10X49 W10X45 W10X39 W10X33",
            "W10X30 W10X26 W10X22 W10X19 W10X17 W10X15 W10X12",
            "W8X67 W8X58 W8X48 W8X40 W8X35 W8X31 W8X28 W8X24 W8X21 W8X18 W8X15 W8X13 W8X10",
            "W6X25 W6X20 W6X15 W6X16 W6X12 W6X9 W6X8.5",
            "W5X19 W5X16",
            "W4X13",
            "HP14X117 HP14X102 HP14X89 HP14X73",
            "HP12X84 HP12X74 HP12X63 HP12X53",
            "HP10X57 HP10X42",
            "HP8X36",
        ];
        let mut expected: Vec<&str> = metric_assortment
            .iter()
            .flat_map(|g| g.split(' '))
            .chain(alias_assortment.iter().flat_map(|g| g.split(' ')))
            .collect();
        assert_eq!(expected.len(), 569, "the metric and alias assortments");
        expected.sort();
        designations.sort();
        assert_eq!(designations, expected);
        // Refused, never a near match: the jumbo shapes newer editions of the standard carry
        // (their metric names too), the other shape families of the same annex, and every
        // other spelling — including the capital-X spelling of the metric form and the
        // lowercase spelling of the alias form.
        for absent in [
            "W44X408",
            "W44X368",
            "W40X655",
            "W30X433",
            "W21X223",
            "W14X808",
            "HP12X89",
            "HP16X183",
            "HP18X204",
            "W1100x607",
            "S24X121",
            "M12X11.6",
            "C15X50",
            "MC18X58",
            "L8X8X1",
            "W310X60",
            "W 310 x 60",
            "W12x40",
            "IPE300",
        ] {
            assert_eq!(
                library.entry(absent).unwrap_err().code,
                ErrorCode::NotFound,
                "{absent} is not in the shipped series"
            );
        }
    }

    #[test]
    fn the_shipped_usa_w_and_hp_library_reads_with_every_entry_admitted() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/usa/profiles-astm-a6-w-hp.usda"
        );
        let library = load(path).unwrap();
        assert_eq!(library.info.id, "usa-astm-a6-w-hp-profiles");
        assert_eq!(library.info.version.as_deref(), Some("1"));
        assert!(
            library
                .info
                .licence
                .as_deref()
                .is_some_and(|l| !l.is_empty()),
            "the shipped library declares its licence"
        );
        assert!(
            library
                .info
                .source
                .as_deref()
                .is_some_and(|s| !s.is_empty()),
            "the shipped library declares its source"
        );
        // No transcription slips: every entry is admitted.
        assert_eq!(library.rejected, []);
        assert_eq!(library.entries.len(), 285);
        // The thirteen shapes the Nucor-Yamato catalog does not roll have no printed radius
        // and carry a sharp corner; every other row carries one of the catalog's radii.
        let sharp = [
            "W40X331", "W36X800", "W12X14", "W10X19", "W10X17", "W10X15", "W10X12", "W8X15",
            "W8X13", "W8X10", "W5X19", "W5X16", "W4X13",
        ];
        let catalog_radii = [8.0, 8.5, 10.0, 13.0, 15.0, 19.0, 20.0, 30.0];
        let mut designations = Vec::new();
        let mut warned = 0;
        for entry in &library.entries {
            let DefinitionRecord::SectionProfile {
                shape: SectionShape::Parametric(parametric),
                section,
            } = &entry.record
            else {
                panic!("{} is not a parametric section", entry.designation);
            };
            // W and HP shapes have parallel flanges: the i_shape family.
            assert_eq!(parametric.family(), SectionFamily::IShape);
            assert_eq!(
                section.standard.as_deref(),
                Some("ASTM A6/A6M-05a"),
                "{}",
                entry.designation
            );
            let radius = parametric.values()[4];
            if sharp.contains(&entry.designation.as_str()) {
                assert_eq!(radius, 0.0, "{}", entry.designation);
            } else {
                assert!(
                    catalog_radii.contains(&radius),
                    "{}: radius {radius}",
                    entry.designation
                );
            }
            // Unlike the other packs, this one expects the D3 warning: the standard prints
            // no radius, and its SI column rounds the printed dimensions to the millimetre
            // and the tenth, so the closed-form area of a row sits up to 4.4 percent from
            // the declared area even with the catalog radius (owner decision of 2026-10-18:
            // a warning, never a refusal). The bound is the typo detector: 150 rows warn,
            // none beyond 5 percent. The nominal SI mass of the designation follows the
            // area at 7850 kg/m3 within 2.2 percent (worst W14X38, whose nominal 58 kg/m
            // rounds up from the 56.7 its area gives).
            let area = section.area_mm2.unwrap();
            let mass = section.mass_per_meter_kg.unwrap();
            let mass_from_area = area * 7850.0 / 1.0e6;
            assert!(
                (mass - mass_from_area).abs() / mass_from_area <= 0.022,
                "{}: mass {mass} kg/m vs area-derived {mass_from_area}",
                entry.designation
            );
            if let Some(warning) = entry.record.declared_area_warning() {
                warned += 1;
                assert_eq!(warning.declared_mm2, area, "{}", entry.designation);
                assert!(
                    warning.relative_difference < 0.05,
                    "{}: declared area {area} mm2 is {} off the outline",
                    entry.designation,
                    warning.relative_difference
                );
            }
            designations.push(entry.designation.clone());
        }
        assert_eq!(warned, 150, "rows carrying the declared-area warning");
        // Spot checks against the standard's printed SI column and the catalog's radius:
        // designation, [h, b, tw, tf, r], area mm2, nominal mass kg/m.
        let spot = |designation: &str, values: [f64; 5], area: f64, mass: f64| {
            let entry = library.entry(designation).unwrap();
            let DefinitionRecord::SectionProfile {
                shape: SectionShape::Parametric(p),
                section,
            } = &entry.record
            else {
                panic!("{designation} is not a parametric section");
            };
            assert_eq!(p.values(), values, "{designation}");
            assert_eq!(section.area_mm2, Some(area), "{designation}");
            assert_eq!(section.mass_per_meter_kg, Some(mass), "{designation}");
        };
        spot("W44X335", [1118.0, 405.0, 26.0, 45.0, 30.0], 63500.0, 499.0);
        // The W 36 X 800 designation cell prints its area before its weight in both public
        // copies (`W36 X 236.4800`); the row is read from its metric designation W920 X 1191.
        spot(
            "W36X800",
            [1081.0, 457.0, 60.5, 109.0, 0.0],
            152500.0,
            1191.0,
        );
        spot("W24X68", [603.0, 228.0, 10.5, 14.9, 30.0], 13000.0, 101.0);
        spot("W12X40", [303.0, 203.0, 7.5, 13.1, 19.0], 7610.0, 60.0);
        spot("W6X8.5", [148.0, 100.0, 4.3, 4.9, 8.0], 1630.0, 13.0);
        spot("W4X13", [106.0, 103.0, 7.1, 8.8, 0.0], 2470.0, 19.3);
        spot("HP14X117", [361.0, 378.0, 20.4, 20.4, 30.0], 22200.0, 174.0);
        spot("HP8X36", [204.0, 207.0, 11.3, 11.3, 13.0], 6840.0, 53.0);
        // The declared-area warning: widest on W24X68, where the catalog radius is larger
        // than the fillet the standard's area allows; absent where the two agree.
        let difference = |designation: &str| {
            library
                .entry(designation)
                .unwrap()
                .record
                .declared_area_warning()
                .map(|w| w.relative_difference)
        };
        assert!((difference("W24X68").unwrap() - 0.0431).abs() < 0.001);
        assert!((difference("W12X40").unwrap() - 0.0123).abs() < 0.0005);
        assert_eq!(difference("W6X8.5"), None);
        // The W and HP rows exactly as ASTM A6/A6M-05a Annex A2 prints them, 274 + 11.
        let assortment = [
            "W44X335 W44X290 W44X262 W44X230",
            "W40X593 W40X503 W40X431 W40X397 W40X372 W40X362 W40X324 W40X297 W40X277 W40X249",
            "W40X215 W40X199 W40X392 W40X331 W40X327 W40X294 W40X278 W40X264 W40X235 W40X211",
            "W40X183 W40X167 W40X149",
            "W36X800 W36X652 W36X529 W36X487 W36X441 W36X395 W36X361 W36X330 W36X302 W36X282",
            "W36X262 W36X247 W36X231 W36X256 W36X232 W36X210 W36X194 W36X182 W36X170 W36X160",
            "W36X150 W36X135",
            "W33X387 W33X354 W33X318 W33X291 W33X263 W33X241 W33X221 W33X201 W33X169 W33X152",
            "W33X141 W33X130 W33X118",
            "W30X391 W30X357 W30X326 W30X292 W30X261 W30X235 W30X211 W30X191 W30X173 W30X148",
            "W30X132 W30X124 W30X116 W30X108 W30X99 W30X90",
            "W27X539 W27X368 W27X336 W27X307 W27X281 W27X258 W27X235 W27X217 W27X194 W27X178",
            "W27X161 W27X146 W27X129 W27X114 W27X102 W27X94 W27X84",
            "W24X370 W24X335 W24X306 W24X279 W24X250 W24X229 W24X207 W24X192 W24X176 W24X162",
            "W24X146 W24X131 W24X117 W24X104 W24X103 W24X94 W24X84 W24X76 W24X68 W24X62 W24X55",
            "W21X201 W21X182 W21X166 W21X147 W21X132 W21X122 W21X111 W21X101 W21X93 W21X83 W21X73",
            "W21X68 W21X62 W21X55 W21X48 W21X57 W21X50 W21X44",
            "W18X311 W18X283 W18X258 W18X234 W18X211 W18X192 W18X175 W18X158 W18X143 W18X130",
            "W18X119 W18X106 W18X97 W18X86 W18X76 W18X71 W18X65 W18X60 W18X55 W18X50 W18X46",
            "W18X40 W18X35",
            "W16X100 W16X89 W16X77 W16X67 W16X57 W16X50 W16X45 W16X40 W16X36 W16X31 W16X26",
            "W14X730 W14X665 W14X605 W14X550 W14X500 W14X455 W14X426 W14X398 W14X370 W14X342",
            "W14X311 W14X283 W14X257 W14X233 W14X211 W14X193 W14X176 W14X159 W14X145 W14X132",
            "W14X120 W14X109 W14X99 W14X90 W14X82 W14X74 W14X68 W14X61 W14X53 W14X48 W14X43",
            "W14X38 W14X34 W14X30 W14X26 W14X22",
            "W12X336 W12X305 W12X279 W12X252 W12X230 W12X210 W12X190 W12X170 W12X152 W12X136",
            "W12X120 W12X106 W12X96 W12X87 W12X79 W12X72 W12X65 W12X58 W12X53 W12X50 W12X45",
            "W12X40 W12X35 W12X30 W12X26 W12X22 W12X19 W12X16 W12X14",
            "W10X112 W10X100 W10X88 W10X77 W10X68 W10X60 W10X54 W10X49 W10X45 W10X39 W10X33",
            "W10X30 W10X26 W10X22 W10X19 W10X17 W10X15 W10X12",
            "W8X67 W8X58 W8X48 W8X40 W8X35 W8X31 W8X28 W8X24 W8X21 W8X18 W8X15 W8X13 W8X10",
            "W6X25 W6X20 W6X15 W6X16 W6X12 W6X9 W6X8.5",
            "W5X19 W5X16",
            "W4X13",
            "HP14X117 HP14X102 HP14X89 HP14X73",
            "HP12X84 HP12X74 HP12X63 HP12X53",
            "HP10X57 HP10X42",
            "HP8X36",
        ];
        let mut expected: Vec<&str> = assortment.iter().flat_map(|g| g.split(' ')).collect();
        expected.sort();
        designations.sort();
        assert_eq!(designations, expected);
        // Refused, never a near match: the jumbo shapes newer editions of the standard carry
        // (the Nucor-Yamato size list names them; the declared 05a tables do not), the other
        // shape families of the same annex (not in this slice), the metric designation of the
        // same rows (no aliases), and every other spelling.
        for absent in [
            "W44X408",
            "W44X368",
            "W40X655",
            "W30X433",
            "W21X223",
            "W14X808",
            "HP12X89",
            "HP16X183",
            "HP18X204",
            "S24X121",
            "M12X11.6",
            "C15X50",
            "MC18X58",
            "L8X8X1",
            "W1100X499",
            "W44x335",
            "W 44 X 335",
            "IPE300",
        ] {
            assert_eq!(
                library.entry(absent).unwrap_err().code,
                ErrorCode::NotFound,
                "{absent} is not in the shipped series"
            );
        }
    }

    #[test]
    fn the_shipped_brazil_w_and_hp_library_reads_with_every_entry_admitted() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/brazil/profiles-abnt-nbr-15980.usda"
        );
        let library = load(path).unwrap();
        assert_eq!(library.info.id, "brazil-abnt-nbr-15980-profiles");
        assert_eq!(library.info.version.as_deref(), Some("1"));
        assert!(
            library
                .info
                .licence
                .as_deref()
                .is_some_and(|l| !l.is_empty()),
            "the shipped library declares its licence"
        );
        assert!(
            library
                .info
                .source
                .as_deref()
                .is_some_and(|s| !s.is_empty()),
            "the shipped library declares its source"
        );
        // No transcription slips: every entry is admitted.
        assert_eq!(library.rejected, []);
        // The pack carries the usa pack's A6/A6M-05a W and HP rows (274 + 11 minus the one row
        // whose printed SI designation another row already owns: the annex prints W920 X 345
        // for W36X231 and W36X232 alike, the Nucor-Yamato size list names W920 x 345 for W36
        // x 232 and lists no W36 x 231, and one library cannot carry two rows under one
        // designation).
        assert_eq!(library.entries.len(), 284);
        // The thirteen shapes the Nucor-Yamato catalog does not roll carry a sharp corner;
        // every other row carries one of the catalog's radii, exactly as in the usa pack.
        let sharp = [
            "W 1000 x 494,0", // W40X331
            "W 920 x 1191,0", // W36X800
            "W 310 x 21,0",   // W12X14
            "W 250 x 28,4",   // W10X19
            "W 250 x 25,3",   // W10X17
            "W 250 x 22,3",   // W10X15
            "W 250 x 17,9",   // W10X12
            "W 200 x 22,5",   // W8X15
            "W 200 x 19,3",   // W8X13
            "W 200 x 15,0",   // W8X10
            "W 130 x 28,1",   // W5X19
            "W 130 x 23,8",   // W5X16
            "W 100 x 19,3",   // W4X13
        ];
        let catalog_radii = [8.0, 8.5, 10.0, 13.0, 15.0, 19.0, 20.0, 30.0];
        let mut designations = Vec::new();
        let mut warned = 0;
        for entry in &library.entries {
            let DefinitionRecord::SectionProfile {
                shape: SectionShape::Parametric(parametric),
                section,
            } = &entry.record
            else {
                panic!("{} is not a parametric section", entry.designation);
            };
            // W and HP shapes have parallel flanges: the i_shape family.
            assert_eq!(parametric.family(), SectionFamily::IShape);
            // Every row declares the edition of the national standard the regional producers
            // print for these dimensions; the values behind it are the annex's SI column.
            assert_eq!(
                section.standard.as_deref(),
                Some("ABNT NBR 15980:2024"),
                "{}",
                entry.designation
            );
            // The designation is the producers' print form, and its mass tail is the row's
            // nominal SI mass, decimal comma, one decimal place.
            let mass = section.mass_per_meter_kg.unwrap();
            let tail = format!("{mass:.1}").replace('.', ",");
            assert!(
                entry.designation.contains(" x ") && entry.designation.ends_with(&tail),
                "{} does not carry its mass {tail}",
                entry.designation
            );
            let radius = parametric.values()[4];
            if sharp.contains(&entry.designation.as_str()) {
                assert_eq!(radius, 0.0, "{}", entry.designation);
            } else {
                assert!(
                    catalog_radii.contains(&radius),
                    "{}: radius {radius}",
                    entry.designation
                );
            }
            // The same declared-area expectation as the usa pack, row for row: the standards
            // print no radius and round their SI dimensions, so 150 rows sit more than one
            // percent from the closed-form area, none beyond five (owner decision of
            // 2026-10-18: a warning, never a refusal). The nominal SI mass of the designation
            // follows the area at 7850 kg/m3 within 2.2 percent.
            let area = section.area_mm2.unwrap();
            let mass_from_area = area * 7850.0 / 1.0e6;
            assert!(
                (mass - mass_from_area).abs() / mass_from_area <= 0.022,
                "{}: mass {mass} kg/m vs area-derived {mass_from_area}",
                entry.designation
            );
            if let Some(warning) = entry.record.declared_area_warning() {
                warned += 1;
                assert_eq!(warning.declared_mm2, area, "{}", entry.designation);
                assert!(
                    warning.relative_difference < 0.05,
                    "{}: declared area {area} mm2 is {} off the outline",
                    entry.designation,
                    warning.relative_difference
                );
            }
            designations.push(entry.designation.clone());
        }
        assert_eq!(warned, 150, "rows carrying the declared-area warning");
        // Spot checks: the usa pack's re-entry value for value, under the metric designation.
        // designation, [h, b, tw, tf, r], area mm2, nominal mass kg/m.
        let spot = |designation: &str, values: [f64; 5], area: f64, mass: f64| {
            let entry = library.entry(designation).unwrap();
            let DefinitionRecord::SectionProfile {
                shape: SectionShape::Parametric(p),
                section,
            } = &entry.record
            else {
                panic!("{designation} is not a parametric section");
            };
            assert_eq!(p.values(), values, "{designation}");
            assert_eq!(section.area_mm2, Some(area), "{designation}");
            assert_eq!(section.mass_per_meter_kg, Some(mass), "{designation}");
        };
        spot(
            "W 1100 x 499,0",
            [1118.0, 405.0, 26.0, 45.0, 30.0],
            63500.0,
            499.0,
        );
        // The W 36 X 800 designation cell prints its area before its weight in both public
        // copies; the row is read from its metric designation W920 X 1191.
        spot(
            "W 920 x 1191,0",
            [1081.0, 457.0, 60.5, 109.0, 0.0],
            152500.0,
            1191.0,
        );
        spot(
            "W 920 x 345,0",
            [943.0, 308.0, 22.1, 39.9, 30.0],
            44000.0,
            345.0,
        );
        spot(
            "W 610 x 101,0",
            [603.0, 228.0, 10.5, 14.9, 30.0],
            13000.0,
            101.0,
        );
        spot(
            "W 310 x 60,0",
            [303.0, 203.0, 7.5, 13.1, 19.0],
            7610.0,
            60.0,
        );
        spot("W 150 x 13,0", [148.0, 100.0, 4.3, 4.9, 8.0], 1630.0, 13.0);
        spot("W 100 x 19,3", [106.0, 103.0, 7.1, 8.8, 0.0], 2470.0, 19.3);
        spot(
            "HP 360 x 174,0",
            [361.0, 378.0, 20.4, 20.4, 30.0],
            22200.0,
            174.0,
        );
        spot(
            "HP 200 x 53,0",
            [204.0, 207.0, 11.3, 11.3, 13.0],
            6840.0,
            53.0,
        );
        spot("W 250 x 22,3", [254.0, 102.0, 5.8, 6.9, 0.0], 2850.0, 22.3);
        spot("W 130 x 23,8", [127.0, 127.0, 6.1, 9.1, 0.0], 3040.0, 23.8);
        // The declared-area warning: widest on W 610 x 101,0, where the catalog radius is
        // larger than the fillet the standard's area allows; absent where the two agree.
        let difference = |designation: &str| {
            library
                .entry(designation)
                .unwrap()
                .record
                .declared_area_warning()
                .map(|w| w.relative_difference)
        };
        assert!((difference("W 610 x 101,0").unwrap() - 0.0431).abs() < 0.001);
        assert!((difference("W 310 x 60,0").unwrap() - 0.0123).abs() < 0.0005);
        assert_eq!(difference("W 150 x 13,0"), None);
        // The W and HP rows in the producers' designation form, 273 + 11.
        let assortment = [
            "W 1100 x 499,0 W 1100 x 433,0 W 1100 x 390,0 W 1100 x 343,0",
            "W 1000 x 883,0 W 1000 x 748,0 W 1000 x 642,0 W 1000 x 591,0 W 1000 x 554,0 W 1000 x 539,0 W 1000 x 483,0 W 1000 x 443,0 W 1000 x 412,0 W 1000 x 371,0 W 1000 x 321,0 W 1000 x 296,0 W 1000 x 584,0 W 1000 x 494,0 W 1000 x 486,0 W 1000 x 438,0 W 1000 x 415,0 W 1000 x 393,0 W 1000 x 350,0 W 1000 x 314,0 W 1000 x 272,0 W 1000 x 249,0 W 1000 x 222,0",
            "W 920 x 1191,0 W 920 x 970,0 W 920 x 787,0 W 920 x 725,0 W 920 x 656,0 W 920 x 588,0 W 920 x 537,0 W 920 x 491,0 W 920 x 449,0 W 920 x 420,0 W 920 x 390,0 W 920 x 368,0 W 920 x 381,0 W 920 x 345,0 W 920 x 313,0 W 920 x 289,0 W 920 x 271,0 W 920 x 253,0 W 920 x 238,0 W 920 x 223,0 W 920 x 201,0",
            "W 840 x 576,0 W 840 x 527,0 W 840 x 473,0 W 840 x 433,0 W 840 x 392,0 W 840 x 359,0 W 840 x 329,0 W 840 x 299,0 W 840 x 251,0 W 840 x 226,0 W 840 x 210,0 W 840 x 193,0 W 840 x 176,0",
            "W 760 x 582,0 W 760 x 531,0 W 760 x 484,0 W 760 x 434,0 W 760 x 389,0 W 760 x 350,0 W 760 x 314,0 W 760 x 284,0 W 760 x 257,0 W 760 x 220,0 W 760 x 196,0 W 760 x 185,0 W 760 x 173,0 W 760 x 161,0 W 760 x 147,0 W 760 x 134,0",
            "W 690 x 802,0 W 690 x 548,0 W 690 x 500,0 W 690 x 457,0 W 690 x 419,0 W 690 x 384,0 W 690 x 350,0 W 690 x 323,0 W 690 x 289,0 W 690 x 265,0 W 690 x 240,0 W 690 x 217,0 W 690 x 192,0 W 690 x 170,0 W 690 x 152,0 W 690 x 140,0 W 690 x 125,0",
            "W 610 x 551,0 W 610 x 498,0 W 610 x 455,0 W 610 x 415,0 W 610 x 372,0 W 610 x 341,0 W 610 x 307,0 W 610 x 285,0 W 610 x 262,0 W 610 x 241,0 W 610 x 217,0 W 610 x 195,0 W 610 x 174,0 W 610 x 155,0 W 610 x 153,0 W 610 x 140,0 W 610 x 125,0 W 610 x 113,0 W 610 x 101,0 W 610 x 92,0 W 610 x 82,0",
            "W 530 x 300,0 W 530 x 272,0 W 530 x 248,0 W 530 x 219,0 W 530 x 196,0 W 530 x 182,0 W 530 x 165,0 W 530 x 150,0 W 530 x 138,0 W 530 x 123,0 W 530 x 109,0 W 530 x 101,0 W 530 x 92,0 W 530 x 82,0 W 530 x 72,0 W 530 x 85,0 W 530 x 74,0 W 530 x 66,0",
            "W 460 x 464,0 W 460 x 421,0 W 460 x 384,0 W 460 x 349,0 W 460 x 315,0 W 460 x 286,0 W 460 x 260,0 W 460 x 235,0 W 460 x 213,0 W 460 x 193,0 W 460 x 177,0 W 460 x 158,0 W 460 x 144,0 W 460 x 128,0 W 460 x 113,0 W 460 x 106,0 W 460 x 97,0 W 460 x 89,0 W 460 x 82,0 W 460 x 74,0 W 460 x 68,0 W 460 x 60,0 W 460 x 52,0",
            "W 410 x 149,0 W 410 x 132,0 W 410 x 114,0 W 410 x 100,0 W 410 x 85,0 W 410 x 75,0 W 410 x 67,0 W 410 x 60,0 W 410 x 53,0 W 410 x 46,1 W 410 x 38,8",
            "W 360 x 1086,0 W 360 x 990,0 W 360 x 900,0 W 360 x 818,0 W 360 x 744,0 W 360 x 677,0 W 360 x 634,0 W 360 x 592,0 W 360 x 551,0 W 360 x 509,0 W 360 x 463,0 W 360 x 421,0 W 360 x 382,0 W 360 x 347,0 W 360 x 314,0 W 360 x 287,0 W 360 x 262,0 W 360 x 237,0 W 360 x 216,0 W 360 x 196,0 W 360 x 179,0 W 360 x 162,0 W 360 x 147,0 W 360 x 134,0 W 360 x 122,0 W 360 x 110,0 W 360 x 101,0 W 360 x 91,0 W 360 x 79,0 W 360 x 72,0 W 360 x 64,0 W 360 x 58,0 W 360 x 51,0 W 360 x 44,6 W 360 x 39,0 W 360 x 32,9",
            "W 310 x 500,0 W 310 x 454,0 W 310 x 415,0 W 310 x 375,0 W 310 x 342,0 W 310 x 313,0 W 310 x 283,0 W 310 x 253,0 W 310 x 226,0 W 310 x 202,0 W 310 x 179,0 W 310 x 158,0 W 310 x 143,0 W 310 x 129,0 W 310 x 117,0 W 310 x 107,0 W 310 x 97,0 W 310 x 86,0 W 310 x 79,0 W 310 x 74,0 W 310 x 67,0 W 310 x 60,0 W 310 x 52,0 W 310 x 44,5 W 310 x 38,7 W 310 x 32,7 W 310 x 28,3 W 310 x 23,8 W 310 x 21,0",
            "W 250 x 167,0 W 250 x 149,0 W 250 x 131,0 W 250 x 115,0 W 250 x 101,0 W 250 x 89,0 W 250 x 80,0 W 250 x 73,0 W 250 x 67,0 W 250 x 58,0 W 250 x 49,1 W 250 x 44,8 W 250 x 38,5 W 250 x 32,7 W 250 x 28,4 W 250 x 25,3 W 250 x 22,3 W 250 x 17,9",
            "W 200 x 100,0 W 200 x 86,0 W 200 x 71,0 W 200 x 59,0 W 200 x 52,0 W 200 x 46,1 W 200 x 41,7 W 200 x 35,9 W 200 x 31,3 W 200 x 26,6 W 200 x 22,5 W 200 x 19,3 W 200 x 15,0",
            "W 150 x 37,1 W 150 x 29,8 W 150 x 22,5 W 150 x 24,0 W 150 x 18,0 W 150 x 13,5 W 150 x 13,0",
            "W 130 x 28,1 W 130 x 23,8",
            "W 100 x 19,3",
            "HP 360 x 174,0 HP 360 x 152,0 HP 360 x 132,0 HP 360 x 108,0",
            "HP 310 x 125,0 HP 310 x 110,0 HP 310 x 93,0 HP 310 x 79,0",
            "HP 250 x 85,0 HP 250 x 62,0",
            "HP 200 x 53,0",
        ];
        // The designations carry spaces themselves, so the group strings split on the series
        // prefix that starts every one, never on a bare space.
        let mut expected: Vec<&str> = Vec::new();
        for group in &assortment {
            let mut rest: &str = group;
            loop {
                match rest.find(" W ").or_else(|| rest.find(" HP ")) {
                    Some(index) => {
                        expected.push(rest[..index].trim_end());
                        rest = &rest[index + 1..];
                    }
                    None => {
                        expected.push(rest.trim_end());
                        break;
                    }
                }
            }
        }
        expected.sort();
        designations.sort();
        assert_eq!(designations, expected);
        // Refused, never a near match: the EDI inch form of a shipped row and its compact
        // metric designation (the library has no aliases; the inch form resolves through the
        // `usa` pack), the shapes of the newer editions of the standard, the two light W 150
        // rows Gerdau rolls outside both standards, the designation W 920 x 345,0's sibling
        // is named above, and every other spelling or family of the same annex.
        for absent in [
            "W44X335",
            "W12X40",
            "HP14X117",
            "W36X232",
            "W1100X499",
            "W44x335",
            "W 44 X 335",
            "w 310 x 60,0",
            "W 310 x 60",
            "W 1100 x 607,0",
            "W 360 x 1202,0",
            "HP 410 x 272,0",
            "HP 460 x 304,0",
            "W 150 x 9,2",
            "W 150 x 10,4",
            "S 310 x 74,0",
            "C 250 x 38,1",
            "L 102 x 102 x 12,7",
            "BT 100 x 10",
            "IPE300",
        ] {
            assert_eq!(
                library.entry(absent).unwrap_err().code,
                ErrorCode::NotFound,
                "{absent} is not in the shipped series"
            );
        }
    }

    #[test]
    fn the_brazil_and_usa_libraries_resolve_one_shape_under_both_designation_forms() {
        // The library has no alias field: the same row resolves under the producers' metric
        // designation through the `brazil` pack and under the EDI inch form through the `usa`
        // pack, and never the other way round.
        let brazil = load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/brazil/profiles-abnt-nbr-15980.usda"
        ))
        .unwrap();
        let usa = load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/usa/profiles-astm-a6-w-hp.usda"
        ))
        .unwrap();
        let lookup = Lookup::new(vec![
            Source {
                path: "brazil".to_owned(),
                library: brazil,
                layer: LAYER_ENVIRONMENT,
            },
            Source {
                path: "usa".to_owned(),
                library: usa,
                layer: LAYER_ENVIRONMENT,
            },
        ]);
        let area = |designation: &str| match &lookup.find(designation).unwrap().1.record {
            DefinitionRecord::SectionProfile { section, .. } => section.area_mm2,
            _ => None,
        };
        assert_eq!(area("W 310 x 60,0"), area("W12X40"), "the same row's area");
        assert_eq!(area("W 310 x 60,0"), Some(7610.0));
        assert_eq!(area("HP 360 x 174,0"), area("HP14X117"));
    }

    #[test]
    fn the_shipped_south_america_a6_w_library_reads_with_every_entry_admitted() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/south-america/profiles-astm-a6-w.usda"
        );
        let library = load(path).unwrap();
        assert_eq!(library.info.id, "south-america-astm-a6-w-profiles");
        assert_eq!(library.info.version.as_deref(), Some("1"));
        assert!(
            library
                .info
                .licence
                .as_deref()
                .is_some_and(|l| !l.is_empty()),
            "the shipped library declares its licence"
        );
        assert!(
            library
                .info
                .source
                .as_deref()
                .is_some_and(|s| !s.is_empty()),
            "the shipped library declares its source"
        );
        // No transcription slips: every entry is admitted.
        assert_eq!(library.rejected, []);
        assert_eq!(library.entries.len(), 274);
        // The thirteen shapes the Nucor-Yamato catalog does not roll have no printed radius
        // and carry a sharp corner; every other row carries one of the catalog's radii.
        let sharp = [
            "W1000x494",
            "W920x1191",
            "W310x21",
            "W250x28.4",
            "W250x25.3",
            "W250x22.3",
            "W250x17.9",
            "W200x22.5",
            "W200x19.3",
            "W200x15",
            "W130x28.1",
            "W130x23.8",
            "W100x19.3",
        ];
        let catalog_radii = [8.0, 8.5, 10.0, 13.0, 15.0, 19.0, 20.0, 30.0];
        let mut designations = Vec::new();
        let mut warned = 0;
        for entry in &library.entries {
            let DefinitionRecord::SectionProfile {
                shape: SectionShape::Parametric(parametric),
                section,
            } = &entry.record
            else {
                panic!("{} is not a parametric section", entry.designation);
            };
            // W shapes have parallel flanges: the i_shape family.
            assert_eq!(parametric.family(), SectionFamily::IShape);
            assert_eq!(
                section.standard.as_deref(),
                Some("ASTM A6/A6M-05a"),
                "{}",
                entry.designation
            );
            let radius = parametric.values()[4];
            if sharp.contains(&entry.designation.as_str()) {
                assert_eq!(radius, 0.0, "{}", entry.designation);
            } else {
                assert!(
                    catalog_radii.contains(&radius),
                    "{}: radius {radius}",
                    entry.designation
                );
            }
            // As in the USA pack, this one expects the D3 warning: the standard prints no
            // radius and rounds its SI column, so the closed-form area of a row sits up to
            // 4.4 percent from the declared area even with the catalog radius (owner
            // decision of 2026-10-18: a warning, never a refusal). The bound is the typo
            // detector: the pin below counts the warning rows, none beyond 5 percent. The
            // nominal SI mass of the designation follows the area at 7850 kg/m3 within 2.2
            // percent (worst W360x58, whose nominal 58 kg/m rounds up from the 56.7 its
            // area gives).
            let area = section.area_mm2.unwrap();
            let mass = section.mass_per_meter_kg.unwrap();
            let mass_from_area = area * 7850.0 / 1.0e6;
            assert!(
                (mass - mass_from_area).abs() / mass_from_area <= 0.022,
                "{}: mass {mass} kg/m vs area-derived {mass_from_area}",
                entry.designation
            );
            if let Some(warning) = entry.record.declared_area_warning() {
                warned += 1;
                assert_eq!(warning.declared_mm2, area, "{}", entry.designation);
                assert!(
                    warning.relative_difference < 0.05,
                    "{}: declared area {area} mm2 is {} off the outline",
                    entry.designation,
                    warning.relative_difference
                );
            }
            designations.push(entry.designation.clone());
        }
        assert_eq!(warned, 142, "rows carrying the declared-area warning");
        // Spot checks against the standard's printed SI column and the catalog's radius:
        // designation, [h, b, tw, tf, r], area mm2, nominal mass kg/m.
        let spot = |designation: &str, values: [f64; 5], area: f64, mass: f64| {
            let entry = library.entry(designation).unwrap();
            let DefinitionRecord::SectionProfile {
                shape: SectionShape::Parametric(p),
                section,
            } = &entry.record
            else {
                panic!("{designation} is not a parametric section");
            };
            assert_eq!(p.values(), values, "{designation}");
            assert_eq!(section.area_mm2, Some(area), "{designation}");
            assert_eq!(section.mass_per_meter_kg, Some(mass), "{designation}");
        };
        spot(
            "W1100x499",
            [1118.0, 405.0, 26.0, 45.0, 30.0],
            63500.0,
            499.0,
        );
        // The W 36 X 800 designation cell prints its area before its weight in both public
        // copies; the row is read from its metric designation W920 X 1191, which the pack
        // keeps as its name.
        spot(
            "W920x1191",
            [1081.0, 457.0, 60.5, 109.0, 0.0],
            152500.0,
            1191.0,
        );
        spot("W610x101", [603.0, 228.0, 10.5, 14.9, 30.0], 13000.0, 101.0);
        spot("W310x60", [303.0, 203.0, 7.5, 13.1, 19.0], 7610.0, 60.0);
        // The light rows keep the printed decimal of the metric mass where the standard
        // prints one.
        spot("W150x13", [148.0, 100.0, 4.3, 4.9, 8.0], 1630.0, 13.0);
        spot("W150x13.5", [150.0, 100.0, 4.3, 5.5, 8.0], 1730.0, 13.5);
        spot("W100x19.3", [106.0, 103.0, 7.1, 8.8, 0.0], 2470.0, 19.3);
        // The declared-area warning: widest on W610x101, where the catalog radius is larger
        // than the fillet the standard's area allows; absent where the two agree.
        let difference = |designation: &str| {
            library
                .entry(designation)
                .unwrap()
                .record
                .declared_area_warning()
                .map(|w| w.relative_difference)
        };
        assert!((difference("W610x101").unwrap() - 0.0431).abs() < 0.001);
        assert!((difference("W310x60").unwrap() - 0.0123).abs() < 0.0005);
        assert_eq!(difference("W150x13"), None);
        // The W rows exactly as ASTM A6/A6M-05a Annex A2 prints them, under the standard's
        // own metric designations. The one exception is the metric-name collision the
        // standard itself prints: W 36 X 231 and W 36 X 232 both carry W920 X 345, so the
        // two rows keep their EDI inch designations and no row is named W920x345.
        let assortment = [
            "W1100x499 W1100x433 W1100x390 W1100x343",
            "W1000x883 W1000x748 W1000x642 W1000x591 W1000x554 W1000x539 W1000x483 W1000x443 W1000x412 W1000x371",
            "W1000x321 W1000x296 W1000x584 W1000x494 W1000x486 W1000x438 W1000x415 W1000x393 W1000x350 W1000x314",
            "W1000x272 W1000x249 W1000x222",
            "W920x1191 W920x970 W920x787 W920x725 W920x656 W920x588 W920x537 W920x491 W920x449 W920x420",
            "W920x390 W920x368 W36X231 W920x381 W36X232 W920x313 W920x289 W920x271 W920x253 W920x238",
            "W920x223 W920x201",
            "W840x576 W840x527 W840x473 W840x433 W840x392 W840x359 W840x329 W840x299 W840x251 W840x226",
            "W840x210 W840x193 W840x176",
            "W760x582 W760x531 W760x484 W760x434 W760x389 W760x350 W760x314 W760x284 W760x257 W760x220",
            "W760x196 W760x185 W760x173 W760x161 W760x147 W760x134",
            "W690x802 W690x548 W690x500 W690x457 W690x419 W690x384 W690x350 W690x323 W690x289 W690x265",
            "W690x240 W690x217 W690x192 W690x170 W690x152 W690x140 W690x125",
            "W610x551 W610x498 W610x455 W610x415 W610x372 W610x341 W610x307 W610x285 W610x262 W610x241",
            "W610x217 W610x195 W610x174 W610x155 W610x153 W610x140 W610x125 W610x113 W610x101 W610x92 W610x82",
            "W530x300 W530x272 W530x248 W530x219 W530x196 W530x182 W530x165 W530x150 W530x138 W530x123 W530x109",
            "W530x101 W530x92 W530x82 W530x72 W530x85 W530x74 W530x66",
            "W460x464 W460x421 W460x384 W460x349 W460x315 W460x286 W460x260 W460x235 W460x213 W460x193",
            "W460x177 W460x158 W460x144 W460x128 W460x113 W460x106 W460x97 W460x89 W460x82 W460x74 W460x68",
            "W460x60 W460x52",
            "W410x149 W410x132 W410x114 W410x100 W410x85 W410x75 W410x67 W410x60 W410x53 W410x46.1 W410x38.8",
            "W360x1086 W360x990 W360x900 W360x818 W360x744 W360x677 W360x634 W360x592 W360x551 W360x509",
            "W360x463 W360x421 W360x382 W360x347 W360x314 W360x287 W360x262 W360x237 W360x216 W360x196",
            "W360x179 W360x162 W360x147 W360x134 W360x122 W360x110 W360x101 W360x91 W360x79 W360x72 W360x64",
            "W360x58 W360x51 W360x44.6 W360x39 W360x32.9",
            "W310x500 W310x454 W310x415 W310x375 W310x342 W310x313 W310x283 W310x253 W310x226 W310x202",
            "W310x179 W310x158 W310x143 W310x129 W310x117 W310x107 W310x97 W310x86 W310x79 W310x74 W310x67",
            "W310x60 W310x52 W310x44.5 W310x38.7 W310x32.7 W310x28.3 W310x23.8 W310x21",
            "W250x167 W250x149 W250x131 W250x115 W250x101 W250x89 W250x80 W250x73 W250x67 W250x58 W250x49.1",
            "W250x44.8 W250x38.5 W250x32.7 W250x28.4 W250x25.3 W250x22.3 W250x17.9",
            "W200x100 W200x86 W200x71 W200x59 W200x52 W200x46.1 W200x41.7 W200x35.9 W200x31.3 W200x26.6 W200x22.5 W200x19.3 W200x15",
            "W150x37.1 W150x29.8 W150x22.5 W150x24 W150x18 W150x13.5 W150x13",
            "W130x28.1 W130x23.8",
            "W100x19.3",
        ];
        let mut expected: Vec<&str> = assortment.iter().flat_map(|g| g.split(' ')).collect();
        expected.sort();
        designations.sort();
        assert_eq!(designations, expected);
        // Refused, never a near match: the jumbo shapes newer editions of the standard carry
        // (the Nucor-Yamato size list names them; the declared 05a tables do not), the HP
        // shapes of the same annex (no public Chilean source lists them), the inch designations
        // of the rows the pack names in the metric form, the collision designation the standard
        // prints twice, and every other spelling.
        for absent in [
            "W44X408",
            "W44X368",
            "W40X655",
            "W30X433",
            "W21X223",
            "W14X808",
            "W18X41",
            "W24X56",
            "HP14X117",
            "HP8X36",
            "HP12X89",
            "HP16X183",
            "S24X121",
            "W12X40",
            "W44X335",
            "W36X247",
            "W920x345",
            "W310X60",
            "w310x60",
            "W 310 x 60",
            "IPE300",
        ] {
            assert_eq!(
                library.entry(absent).unwrap_err().code,
                ErrorCode::NotFound,
                "{absent} is not in the shipped series"
            );
        }
        // The two collision rows answer under their inch designations, and under no other
        // name: the same standard prints W920 X 345 for both, so the metric name cannot
        // resolve one row.
        assert!(library.entry("W36X231").is_ok());
        assert!(library.entry("W36X232").is_ok());
    }

    #[test]
    fn the_shipped_brazil_nbr_7007_materials_library_reads_with_every_entry_admitted() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/brazil/materials-nbr-7007.usda"
        );
        let library = load_material(path).unwrap();
        assert_eq!(library.info.id, "brazil-nbr-7007-materials");
        assert_eq!(library.info.version.as_deref(), Some("1"));
        assert!(
            library
                .info
                .licence
                .as_deref()
                .is_some_and(|l| !l.is_empty()),
            "the shipped library declares its licence"
        );
        assert!(
            library
                .info
                .source
                .as_deref()
                .is_some_and(|s| !s.is_empty()),
            "the shipped library declares its source"
        );
        // No transcription slips: every entry is admitted.
        assert_eq!(library.rejected, []);
        assert_eq!(library.entries.len(), 4);
        // The grades the open public listings and the producers' prints corroborate, in the
        // loader's designation order. Every row declares the edition the public listings name
        // (10/2022, vigente) and the metric 7850 kg/m3 density of every shipped pack.
        let expected: [&str; 4] = ["AR 350", "AR 350 COR", "AR 415", "MR 250"];
        let grades: Vec<&str> = library
            .entries
            .iter()
            .map(|entry| match &entry.record {
                DefinitionRecord::Material { grade, .. } => grade.as_deref().unwrap(),
                _ => panic!("{} is not a material grade", entry.designation),
            })
            .collect();
        assert_eq!(grades, expected);
        for entry in &library.entries {
            assert!(
                matches!(
                    &entry.record,
                    DefinitionRecord::Material { density_kg_m3: Some(7850.0), standard, .. }
                        if standard.as_deref() == Some("ABNT NBR 7007:2022")
                ),
                "{}",
                entry.designation
            );
            // The designation is the producers' print form: the strength prefix and the yield
            // number, spaced.
            assert_eq!(entry.designation, entry.designation.to_uppercase());
            assert!(entry.designation.contains(' '));
        }
        // Refused, never a near match: no compact spelling (the current Gerdau print runs the
        // strength prefix and the number together; the shipped rows are the spaced forms the
        // listings print), no grade one listing names and no producer corroborates (BR 190),
        // no strength level the public listings do not carry, and no ASTM name — the A36,
        // A572 and A588 neighbours of the standard's informative mapping are not rows.
        for absent in [
            "AR350",
            "AR 350COR",
            "MR250",
            "BR 190",
            "BR190",
            "AR 345",
            "AR 500",
            "MR 325",
            "A36",
            "A572 Grade 50",
            "A588",
        ] {
            assert_eq!(
                library.entry(absent).unwrap_err().code,
                ErrorCode::NotFound,
                "{absent} is not a shipped grade"
            );
        }
    }

    #[test]
    fn the_shipped_china_h_section_library_reads_with_every_entry_admitted() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../environments/china/profiles-gb-t-11263.usda"
        );
        let library = load(path).unwrap();
        assert_eq!(library.info.id, "china-gb-t-11263-profiles");
        assert_eq!(library.info.version.as_deref(), Some("1"));
        assert!(
            library
                .info
                .licence
                .as_deref()
                .is_some_and(|l| !l.is_empty()),
            "the shipped library declares its licence"
        );
        assert!(
            library
                .info
                .source
                .as_deref()
                .is_some_and(|s| !s.is_empty()),
            "the shipped library declares its source"
        );
        // No transcription slips: every entry is admitted and none warns.
        assert_eq!(library.rejected, []);
        assert_eq!(library.entries.len(), 103);
        let mut designations = Vec::new();
        for entry in &library.entries {
            let DefinitionRecord::SectionProfile {
                shape: SectionShape::Parametric(parametric),
                section,
            } = &entry.record
            else {
                panic!("{} is not a parametric section", entry.designation);
            };
            // The HW/HM/HN flanges are parallel: the i_shape family, no slope, no toe radius.
            assert_eq!(parametric.family(), SectionFamily::IShape);
            assert_eq!(
                section.standard.as_deref(),
                Some("GB/T 11263-2017"),
                "{}",
                entry.designation
            );
            // The closed-form area of the declared dimensions must match the declared catalogue
            // area (the D3 1 percent warning never fires) and the declared mass must be that
            // area at 7850 kg/m3 within the standard's published rounding. The mass prints
            // with one decimal to 100 kg/m and as an integer above it (HW400x400 "172" is
            // 171.68 rounded), so 0.55 kg/m covers every row.
            assert!(
                entry.record.declared_area_warning().is_none(),
                "{}",
                entry.designation
            );
            let area = section.area_mm2.unwrap();
            let mass = section.mass_per_meter_kg.unwrap();
            let mass_from_area = area * 7850.0 / 1.0e6;
            assert!(
                (mass - mass_from_area).abs() <= 0.55,
                "{}: mass {mass} kg/m vs area-derived {mass_from_area}",
                entry.designation
            );
            designations.push(entry.designation.clone());
        }
        // The HW, HM and HN series exactly as table 1 of GB/T 11263-2017 publishes them: 103
        // designations (the lookup lists them lexicographically). The thin-wall HT series
        // (27 rows) and the HP pile series of the same table stay outside this pack.
        assert_eq!(
            designations,
            [
                "HM148x100",
                "HM194x150",
                "HM244x175",
                "HM294x200",
                "HM298x201",
                "HM340x250",
                "HM390x300",
                "HM440x300",
                "HM482x300",
                "HM488x300",
                "HM544x300",
                "HM550x300",
                "HM582x300",
                "HM588x300",
                "HM594x302",
                "HN1000x300",
                "HN1008x302",
                "HN100x50",
                "HN125x60",
                "HN150x75",
                "HN175x90",
                "HN198x99",
                "HN200x100",
                "HN248x124",
                "HN250x125",
                "HN298x149",
                "HN300x150",
                "HN346x174",
                "HN350x175",
                "HN396x199",
                "HN400x150",
                "HN400x200",
                "HN446x150",
                "HN446x199",
                "HN450x151",
                "HN450x200",
                "HN470x150",
                "HN475x151.5",
                "HN482x153.5",
                "HN492x150",
                "HN496x199",
                "HN500x152",
                "HN500x200",
                "HN504x153",
                "HN506x201",
                "HN546x199",
                "HN550x200",
                "HN596x199",
                "HN600x200",
                "HN606x201",
                "HN625x198.5",
                "HN630x200",
                "HN638x202",
                "HN646x299",
                "HN650x300",
                "HN656x301",
                "HN692x300",
                "HN700x300",
                "HN734x299",
                "HN742x300",
                "HN750x300",
                "HN758x303",
                "HN792x300",
                "HN800x300",
                "HN834x298",
                "HN842x299",
                "HN850x300",
                "HN858x301",
                "HN890x299",
                "HN900x300",
                "HN912x302",
                "HN970x297",
                "HN980x298",
                "HN990x298",
                "HW100x100",
                "HW125x125",
                "HW150x150",
                "HW175x175",
                "HW200x200",
                "HW200x204",
                "HW244x252",
                "HW250x250",
                "HW250x255",
                "HW294x302",
                "HW300x300",
                "HW300x305",
                "HW338x351",
                "HW344x348",
                "HW344x354",
                "HW350x350",
                "HW350x357",
                "HW388x402",
                "HW394x398",
                "HW394x405",
                "HW400x400",
                "HW400x408",
                "HW414x405",
                "HW428x407",
                "HW458x417",
                "HW492x465",
                "HW498x432",
                "HW502x465",
                "HW502x470",
            ]
        );
        // Spot checks against the transcribed values.
        let spot = |designation: &str, values: [f64; 5], area: f64, mass: f64| {
            let entry = library.entry(designation).unwrap();
            let DefinitionRecord::SectionProfile {
                shape: SectionShape::Parametric(p),
                section,
            } = &entry.record
            else {
                panic!("{designation} is not a parametric section");
            };
            assert_eq!(p.values(), values, "{designation}");
            assert_eq!(section.area_mm2, Some(area), "{designation}");
            assert_eq!(section.mass_per_meter_kg, Some(mass), "{designation}");
        };
        spot("HW100x100", [100.0, 100.0, 6.0, 8.0, 8.0], 2158.0, 16.9);
        spot(
            "HM594x302",
            [594.0, 302.0, 14.0, 23.0, 13.0],
            21710.0,
            170.0,
        );
        spot(
            "HN1008x302",
            [1008.0, 302.0, 21.0, 40.0, 18.0],
            43930.0,
            345.0,
        );
        // The 625x200 and 650x300 groups where a 2017-labeled web transcription disagrees with
        // the official text: the 2017 foreword changes no table 1 value against the 2010
        // printing, so the official values stand (HN630x200 tw 13, not 15).
        spot(
            "HN630x200",
            [630.0, 200.0, 13.0, 20.0, 13.0],
            15820.0,
            124.0,
        );
        spot(
            "HN646x299",
            [646.0, 299.0, 10.0, 15.0, 13.0],
            15280.0,
            120.0,
        );
        // The HT thin-wall series, the HP pile series and the depths no series publishes stay
        // NOT_FOUND, and the bare prefix without the series letter is no designation.
        for absent in [
            "HT100x50",
            "HT200x100",
            "HP200x200",
            "HP250x250",
            "H200x100",
            "HW110x110",
            "HM160x150",
            "HN225x125",
            "HW500x500",
            "HN550x300",
            "HW498x417",
        ] {
            assert_eq!(
                library.entry(absent).unwrap_err().code,
                ErrorCode::NotFound,
                "{absent} is not in the shipped series"
            );
        }
    }
}
