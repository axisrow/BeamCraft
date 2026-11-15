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
//! The Layering work adds the layers above the environment. Every [`Source`] of a lookup names
//! its layer, strongest first: the user library named on the command line, the user-owned firm
//! layer (`libraries/` in the application data, loaded by [`load_firm`], never written by the
//! programs and never touched by an environment update), then the chosen libraries. A project's
//! own copy of its environment is the project layer of [`project_library_chain`], and the
//! shipped neutral environment ([`neutral_environment`]) is the built-in fallback every project
//! chain ends with. When a weaker layer declares a designation a stronger one already answers,
//! [`Lookup::shadowed`] reports it with both layers. Fastener entries ride the same machinery in
//! their own namespace ([`Environment::fastener_lookup`], [`ProjectChain::fastener_lookup`]);
//! material libraries join the same chain when they arrive.
//!
//! Limits, none of them hidden: a bad entry is reported and skipped, while a duplicate
//! designation refuses the whole library; a firm file that does not read is reported and skipped
//! by [`load_firm`], never silently dropped; the library provenance is reported with the entry
//! but not stored in the definition; the layer engine runs in this process (no LEW-03
//! containment), as for every other command.

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
    CustomSection, MAX_CUSTOM_CAVITIES, ParametricSection, SectionFamily, SectionShape,
    profile_attribute,
};

/// Largest library file, in bytes.
pub const MAX_LIBRARY_BYTES: usize = foreign::MAX_LAYER_BYTES;
/// Most entries one library may hold; more is `LIMIT_EXCEEDED`.
pub const MAX_LIBRARY_ENTRIES: usize = foreign::MAX_LIBRARY_ENTRIES;

/// Parent path of every profile entry prim.
const ENTRY_PARENT: &str = "/Library/Profiles";
/// Parent path of every fastener entry prim (A14.1).
const FASTENER_ENTRY_PARENT: &str = "/Library/Fasteners";

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
    let section = SectionProperties {
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
    };
    let family = required_text(prim, "steel:profile:family", "token")?;
    let record = match family {
        "rectangle" => DefinitionRecord::RectangleProfile {
            width_mm: required_number(prim, "steel:profile:bMm")?,
            height_mm: required_number(prim, "steel:profile:hMm")?,
            section,
        },
        "custom" => DefinitionRecord::SectionProfile {
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
        },
        token => {
            let family = SectionFamily::from_token(token)
                .ok_or_else(|| invalid("unknown section family"))?;
            let values = family
                .parameters()
                .iter()
                .map(|name| required_number(prim, &profile_attribute(name)))
                .collect::<Result<Vec<_>, _>>()?;
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
    load_bytes_as(bytes, Some(ENTRY_PARENT), decode_record).map(|(library, _)| library)
}

/// [`load`] for the fastener library at `path` (A14.1): the same layer format and
/// loading rules, entries under `/Library/Fasteners` carrying `steel:fastener:*` attributes.
pub fn load_fastener(path: &str) -> Result<Library, Refusal> {
    check_source_path(path).map_err(command)?;
    load_fastener_bytes(read_file(path, MAX_LIBRARY_BYTES).map_err(io_error)?)
}

/// [`load_bytes`] for the bytes of a fastener library.
pub fn load_fastener_bytes(bytes: Vec<u8>) -> Result<Library, Refusal> {
    load_bytes_as(bytes, Some(FASTENER_ENTRY_PARENT), decode_fastener_record)
        .map(|(library, _)| library)
}

/// [`load_bytes`] for a firm-layer file, whose entries may sit under either entry parent: the
/// profile and fastener admission runs in one pass over the same opened layer, and the second
/// answer says whether any admitted entry is a fastener (a file without entries is a profile
/// library). A mixed file is not refused here; its entries decode by their own parent.
pub fn load_firm_bytes(bytes: Vec<u8>) -> Result<(Library, bool), Refusal> {
    load_bytes_as(bytes, None, decode_record)
}

/// [`load_firm_bytes`] for the library file at `path`.
pub fn load_firm_file(path: &str) -> Result<(Library, bool), Refusal> {
    check_source_path(path).map_err(command)?;
    load_firm_bytes(read_file(path, MAX_LIBRARY_BYTES).map_err(io_error)?)
}

/// [`load_bytes`] for one entry parent path (or `None` for a firm file, where both entry
/// parents are admitted and each prim decodes by its own) and the profile entry decoder; the
/// layer budgets, identity, admission and refused-entry reporting are shared with the profile
/// loading. The boolean answers whether any admitted entry is a fastener.
fn load_bytes_as(
    bytes: Vec<u8>,
    entry_parent: Option<&str>,
    decode: fn(&Prim, &str) -> Result<DefinitionRecord, Refusal>,
) -> Result<(Library, bool), Refusal> {
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
    let mut entries = Vec::new();
    let mut rejected = Vec::new();
    let mut seen = BTreeSet::new();
    let mut count = 0usize;
    let mut any_fastener = false;
    for prim in prims.iter().filter(|p| {
        let parent = p.path.rsplit_once('/').map(|(parent, _)| parent);
        match entry_parent {
            Some(one) => parent == Some(one),
            None => parent == Some(ENTRY_PARENT) || parent == Some(FASTENER_ENTRY_PARENT),
        }
    }) {
        count += 1;
        if count > MAX_LIBRARY_ENTRIES {
            return Err(refuse(
                ErrorCode::LimitExceeded,
                "the library has more than 4096 entries",
            ));
        }
        let fastener_prim = entry_parent.is_none()
            && prim.path.rsplit_once('/').map(|(parent, _)| parent) == Some(FASTENER_ENTRY_PARENT);
        if fastener_prim {
            any_fastener = true;
        }
        let attribute = if fastener_prim {
            "steel:fastener:designation"
        } else {
            designation_attribute(entry_parent.unwrap_or(ENTRY_PARENT))
        };
        let designation = match required_text(prim, attribute, "string") {
            Ok(text) if label_is_valid(text) => text.to_owned(),
            Ok(_) | Err(_) => {
                rejected.push(RejectedEntry {
                    prim_path: prim.path.clone(),
                    designation: None,
                    code: ErrorCode::InvalidArgument,
                    message: "the entry has no valid designation",
                });
                continue;
            }
        };
        if !seen.insert(designation.clone()) {
            return Err(invalid(
                "the library has two entries with the same designation",
            ));
        }
        if !prim.active {
            rejected.push(RejectedEntry {
                prim_path: prim.path.clone(),
                designation: Some(designation),
                code: ErrorCode::InvalidArgument,
                message: "the entry is inactive",
            });
            continue;
        }
        let decoded = if fastener_prim {
            decode_fastener_record(prim, &designation)
        } else {
            decode(prim, &designation)
        };
        match decoded {
            Ok(record) => entries.push(LibraryEntry {
                prim_path: prim.path.clone(),
                designation,
                record,
            }),
            Err(error) => rejected.push(RejectedEntry {
                prim_path: prim.path.clone(),
                designation: Some(designation),
                code: error.code,
                message: error.message,
            }),
        }
    }
    entries.sort_by(|a, b| a.designation.cmp(&b.designation));
    Ok((
        Library {
            info,
            entries,
            rejected,
        },
        any_fastener,
    ))
}

/// The designation attribute of one entry kind: `steel:profile:designation` or
/// `steel:fastener:designation`.
fn designation_attribute(entry_parent: &str) -> &'static str {
    if entry_parent == FASTENER_ENTRY_PARENT {
        "steel:fastener:designation"
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

/// Libraries in lookup order, strongest first.
///
/// The first library that has a designation answers for it, and a refused entry answers with its
/// refusal: a weaker library is never consulted for a designation that a stronger one declares,
/// valid or not.
#[derive(Debug, Clone, PartialEq)]
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
        self.slots()
            .0
            .into_values()
            .filter_map(|(_, _, slot)| match slot {
                Slot::Entry(source, entry) => Some((source, entry)),
                Slot::Refused => None,
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
}

impl Environment {
    /// The resolver over the environment's fastener libraries, strongest first: the same
    /// [`Lookup`] machinery that answers profile designations answers fastener ones. The two
    /// namespaces stay separate; a profile library never answers a bolt lookup.
    #[must_use]
    pub fn fastener_lookup(&self) -> Lookup {
        Lookup::new(self.fastener_sources.clone())
    }
}

/// Admits one listed library of an environment: its provenance is declared (`bim:library:source`
/// and `bim:library:licence`, empty is undeclared) and its `bim:library:id` is new among every
/// library of the environment, profile or fastener. A single-file `--library` needs no
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
/// [`load`] (profile libraries) or [`load_fastener`] (fastener libraries), so every library
/// keeps its own bounds and admission. A library that cannot be loaded, one that declares no
/// `source` or `licence`, or two listed libraries with the same `bim:library:id`, refuse the
/// whole environment: a missing library would silently change which section or bolt a
/// designation selects. The containment of a listed path is lexical; the files themselves are
/// read like any `--library` file.
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
    Ok(Environment {
        info,
        sources,
        fastener_sources,
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
    /// The firm files that did not read, in file-name order.
    pub refused: Vec<FirmFileRefused>,
}

/// Loads the user-owned firm layer: every library file [`session::environment::firm_library_files`]
/// finds in `config_dir`'s `libraries/` (or in the current user's, when `config_dir` is `None`),
/// in file-name order. A file whose entries live under `/Library/Fasteners` joins
/// [`FirmLayer::fastener_sources`], every other file joins [`FirmLayer::sources`]; a file that
/// does not read, a non-UTF-8 name, or a second file of a `bim:library:id` already taken is
/// reported in [`FirmLayer::refused`] and skipped — a firm file is never allowed to break the
/// layers above or below it, and never silently ignored either. The firm layer is the user's
/// own: nothing here writes a file, and no environment update ever touches it.
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
            Ok((library, fastener)) => {
                if ids.contains(&library.info.id) {
                    layer.refused.push(FirmFileRefused {
                        path: path.to_owned(),
                        code: ErrorCode::InvalidArgument,
                        message: "two firm libraries have the same bim:library:id",
                    });
                    continue;
                }
                ids.push(library.info.id.clone());
                let source = Source {
                    path: path.to_owned(),
                    library,
                    layer: LAYER_FIRM,
                };
                if fastener {
                    layer.fastener_sources.push(source);
                } else {
                    layer.sources.push(source);
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
) -> Result<(), Refusal> {
    let Some(neutral) = neutral_environment()? else {
        return Ok(());
    };
    for source in neutral.sources {
        let id = &source.library.info.id;
        if !sources.iter().any(|s| &s.library.info.id == id) {
            sources.push(Source {
                layer: LAYER_NEUTRAL,
                ..source
            });
        }
    }
    for source in neutral.fastener_sources {
        let id = &source.library.info.id;
        if !fastener_sources.iter().any(|s| &s.library.info.id == id) {
            fastener_sources.push(Source {
                layer: LAYER_NEUTRAL,
                ..source
            });
        }
    }
    Ok(())
}

/// The layered lookup of one project: the project's own copy of its environment (the project
/// layer), then the firm libraries of the application data, then the built-in neutral
/// environment. The strongest first; a designation the model already defines never reaches this
/// chain, because the model layer answers before it. A project whose environment copy is absent
/// resolves from firm and neutral alone; `environment` carries the copy's declarations when it
/// is present, and `firm_refused` names the firm files that did not read.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectChain {
    /// The resolver over the profile libraries of the chain, strongest first.
    pub lookup: Lookup,
    /// The resolver over the fastener libraries of the chain, strongest first; `None` when no
    /// layer of the chain declares a fastener library.
    pub fastener_lookup: Option<Lookup>,
    /// The project environment copy's own declarations, when the project carries one.
    pub environment: Option<EnvironmentInfo>,
    /// The firm files that did not read, in file-name order.
    pub firm_refused: Vec<FirmFileRefused>,
}

/// Builds the [`ProjectChain`] of the project folder at `store`. The project's environment copy
/// is read with [`load_environment`] (a refused copy refuses the chain, as a missing library
/// would silently change which section a designation selects) and retagged as the project
/// layer; the firm layer is loaded from `config_dir`'s `libraries/` (`None` is the current
/// user's). The neutral environment joins last.
pub fn project_library_chain(
    store: &std::path::Path,
    config_dir: Option<&std::path::Path>,
) -> Result<ProjectChain, Refusal> {
    let mut sources = Vec::new();
    let mut fastener_sources = Vec::new();
    let mut environment = None;
    if let Some(file) = crate::project::project_environment_file(store) {
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
    }
    let firm = load_firm(config_dir);
    let firm_refused = firm.refused;
    sources.extend(firm.sources);
    fastener_sources.extend(firm.fastener_sources);
    append_neutral(&mut sources, &mut fastener_sources)?;
    let fastener_lookup = (!fastener_sources.is_empty()).then(|| Lookup::new(fastener_sources));
    Ok(ProjectChain {
        lookup: Lookup::new(sources),
        fastener_lookup,
        environment,
        firm_refused,
    })
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
            // The UPE channels are the pack's parallel-flange channel series; the rest are
            // I and H shapes.
            let want_family = if entry.designation.starts_with("UPE") {
                SectionFamily::Channel
            } else {
                SectionFamily::IShape
            };
            assert_eq!(parametric.family(), want_family);
            assert_eq!(
                section.standard.as_deref(),
                Some("EN 10365:2022"),
                "{}",
                entry.designation
            );
            // The closed-form area of the declared dimensions must match the declared catalogue
            // area (the D3 1 percent warning never fires) and the declared mass must be that
            // area at the standard's 7850 kg/m3 within its published rounding.
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
        // The series as EN 10365 publishes it, in the loader's designation order: no IPE 250/280
        // and no HEA/HEB 250 (those designations do not exist; the sequence steps 240 to 260/270),
        // and no UPE 250/260 (the channel sequence steps 240 to 270). The taper-flange UPN series
        // of the same standard is a different geometry family and is not part of the pack.
        assert_eq!(
            designations,
            [
                "HEA100", "HEA120", "HEA140", "HEA160", "HEA180", "HEA200", "HEA220", "HEA240",
                "HEA260", "HEA280", "HEA300", "HEA320", "HEA340", "HEA360", "HEA400", "HEA450",
                "HEA500", "HEA550", "HEA600", "HEB100", "HEB120", "HEB140", "HEB160", "HEB180",
                "HEB200", "HEB220", "HEB240", "HEB260", "HEB280", "HEB300", "HEB320", "HEB340",
                "HEB360", "HEB400", "HEB450", "HEB500", "HEB550", "HEB600", "IPE100", "IPE120",
                "IPE140", "IPE160", "IPE180", "IPE200", "IPE220", "IPE240", "IPE270", "IPE300",
                "IPE330", "IPE360", "IPE400", "IPE450", "IPE500", "IPE550", "IPE600", "UPE100",
                "UPE120", "UPE140", "UPE160", "UPE180", "UPE200", "UPE220", "UPE240", "UPE270",
                "UPE300", "UPE330", "UPE360", "UPE400", "UPE80",
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
        for absent in [
            "IPE250", "IPE280", "HEA250", "HEB250", "IPE650", "HEA700", "UPE250", "UPE260",
            "UPE450", "UPN100", "UPN200",
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
        assert_eq!(environment.sources[0].library.entries.len(), 69);
        assert_eq!(environment.sources[1].library.entries.len(), 52);
        // The DIN fastener set rides the same environment, admitted with declared provenance.
        assert_eq!(environment.fastener_sources.len(), 1);
        let bolts = &environment.fastener_sources[0];
        assert_eq!(bolts.library.info.id, "beamcraft-din-fasteners");
        assert_eq!(bolts.library.entries.len(), 3);
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
        for absent in ["M24", "M12x80", "m12"] {
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
        write("profiles-nolicence.usda", &profile);
        write("profiles-complete.usda", &complete);
        write("fasteners-nosource.usda", &fastener);
        let env = |profiles: &str, fasteners: Option<&str>| {
            format!(
                "#usda 1.0\n(\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n    customLayerData = {{\n        string \"bim:environment:id\" = \"e\"\n        string[] \"bim:environment:libraries\" = [\"{profiles}\"]{}\n    }}\n)\n",
                match fasteners {
                    Some(name) => format!(
                        "\n        string[] \"bim:environment:fastener-libraries\" = [\"{name}\"]"
                    ),
                    None => String::new(),
                }
            )
        };
        write(
            "no-profile-licence.usda",
            &env("profiles-nolicence.usda", None),
        );
        write(
            "no-fastener-source.usda",
            &env("profiles-complete.usda", Some("fasteners-nosource.usda")),
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
    fn a_firm_file_classifies_by_its_own_entries() {
        let profiles = load_firm_bytes(library(&[entry("A", "F-1", I_BODY)]).into_bytes()).unwrap();
        assert!(!profiles.1, "profile entries make a profile library");
        let bolts = load_firm_bytes(
            fastener_library(&[fastener_entry("B", "SYN-M12", BOLT_BODY)]).into_bytes(),
        )
        .unwrap();
        assert!(bolts.1, "fastener entries make a fastener library");
        assert_eq!(bolts.0.entries.len(), 1);
        // Both parents in one file: every entry decodes by its own parent.
        let mixed = format!(
            "{}def Scope \"Library\" {{\n    def Scope \"Profiles\" {{\n{}    }}\n    def Scope \"Fasteners\" {{\n{}    }}\n}}\n",
            HEADER.replace("    }\n)\n", "    }\n)\n\n"),
            entry("A", "F-1", I_BODY),
            fastener_entry("B", "SYN-M12", BOLT_BODY)
        );
        let mixed = load_firm_bytes(mixed.into_bytes()).unwrap();
        assert!(mixed.1);
        assert_eq!(mixed.0.entries.len(), 2);
        assert_eq!(
            load_firm_bytes(b"not usd".to_vec()).unwrap_err().code,
            ErrorCode::InvalidArgument
        );
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
            .map(|r| r.path.rsplit('/').next().unwrap())
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
    fn the_neutral_layer_fills_the_gaps_and_skips_taken_ids() {
        let neutral = neutral_environment().unwrap().unwrap();
        assert_eq!(
            neutral.info.id,
            session::environment::NEUTRAL_ENVIRONMENT_ID,
            "the shipped neutral environment is the fallback layer"
        );
        let mut sources: Vec<Source> = Vec::new();
        let mut fasteners: Vec<Source> = Vec::new();
        append_neutral(&mut sources, &mut fasteners).unwrap();
        assert!(
            fasteners.is_empty(),
            "the neutral environment ships no fastener libraries"
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
        append_neutral(&mut taken, &mut fasteners).unwrap();
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
}
