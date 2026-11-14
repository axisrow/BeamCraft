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
//! `steel:profile:<parameter>Mm` double per family parameter (a rectangle has `h` and `b`; a
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
//! Limits, none of them hidden: a project-level library is not read; the environment is chosen on
//! the command line, there is no application setting or data directory yet; a bad entry is
//! reported and skipped, while a duplicate designation refuses the whole library; the library
//! provenance is reported with the entry but not stored in the definition; the layer engine runs
//! in this process (no LEW-03 containment), as for every other command.

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
};

/// Largest library file, in bytes.
pub const MAX_LIBRARY_BYTES: usize = foreign::MAX_LAYER_BYTES;
/// Most entries one library may hold; more is `LIMIT_EXCEEDED`.
pub const MAX_LIBRARY_ENTRIES: usize = foreign::MAX_LIBRARY_ENTRIES;

/// Parent path of every profile entry prim.
const ENTRY_PARENT: &str = "/Library/Profiles";
/// Parent path of every fastener entry prim (A14.1).
const FASTENER_ENTRY_PARENT: &str = "/Library/Fasteners";

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
                .map(|name| required_number(prim, &format!("steel:profile:{name}Mm")))
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
    load_bytes_as(bytes, ENTRY_PARENT, decode_record)
}

/// [`load`] for the fastener library at `path` (A14.1): the same layer format and
/// loading rules, entries under `/Library/Fasteners` carrying `steel:fastener:*` attributes.
pub fn load_fastener(path: &str) -> Result<Library, Refusal> {
    check_source_path(path).map_err(command)?;
    load_fastener_bytes(read_file(path, MAX_LIBRARY_BYTES).map_err(io_error)?)
}

/// [`load_bytes`] for the bytes of a fastener library.
pub fn load_fastener_bytes(bytes: Vec<u8>) -> Result<Library, Refusal> {
    load_bytes_as(bytes, FASTENER_ENTRY_PARENT, decode_fastener_record)
}

/// [`load_bytes`] for one entry parent path and its entry decoder; everything else (layer
/// budgets, identity, admission, refused-entry reporting) is shared with the profile loading.
fn load_bytes_as(
    bytes: Vec<u8>,
    entry_parent: &str,
    decode: fn(&Prim, &str) -> Result<DefinitionRecord, Refusal>,
) -> Result<Library, Refusal> {
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
    for prim in prims
        .iter()
        .filter(|p| p.path.rsplit_once('/').map(|(parent, _)| parent) == Some(entry_parent))
    {
        count += 1;
        if count > MAX_LIBRARY_ENTRIES {
            return Err(refuse(
                ErrorCode::LimitExceeded,
                "the library has more than 4096 entries",
            ));
        }
        let designation = match required_text(prim, designation_attribute(entry_parent), "string") {
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
        match decode(prim, &designation) {
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
    Ok(Library {
        info,
        entries,
        rejected,
    })
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

/// One library of a lookup, with the path it was read from.
#[derive(Debug, Clone, PartialEq)]
pub struct Source {
    /// The path the library was read from: as given, or resolved against its environment file.
    pub path: String,
    /// The loaded library.
    pub library: Library,
}

/// A designation that a stronger library already provides; the weaker entry is never used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shadowed {
    /// The designation.
    pub designation: String,
    /// `bim:library:id` of the stronger library, whose entry (or refusal) answers a lookup.
    pub used_library: String,
    /// `bim:library:id` of the weaker library that has the same designation.
    pub shadowed_library: String,
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

    fn slots(&self) -> (BTreeMap<&str, (&str, Slot<'_>)>, Vec<Shadowed>) {
        let mut slots: BTreeMap<&str, (&str, Slot<'_>)> = BTreeMap::new();
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
                    Some((winner, _)) => shadowed.push(Shadowed {
                        designation: designation.to_owned(),
                        used_library: (*winner).to_owned(),
                        shadowed_library: id.to_owned(),
                    }),
                    None => {
                        let slot = entry.map_or(Slot::Refused, |e| Slot::Entry(source, e));
                        slots.insert(designation, (id, slot));
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
            .filter_map(|(_, slot)| match slot {
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
    sources.push(Source { path, library });
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

/// The lookup that a command names: exactly one of a single `--library` file or an
/// `--environment` file (its libraries in order), with an optional `--user-library` file that is
/// consulted before either. Never a hidden global; with none of them it is `INVALID_ARGUMENT`.
/// The environment's own declarations are returned when one was named.
pub fn open_lookup(
    library: Option<&str>,
    environment: Option<&str>,
    user_library: Option<&str>,
) -> Result<(Lookup, Option<EnvironmentInfo>), Refusal> {
    let mut environment_info = None;
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
        }],
        (None, Some(path)) => {
            let loaded = load_environment(path)?;
            environment_info = Some(loaded.info);
            loaded.sources
        }
    };
    let mut sources = Vec::with_capacity(base.len().saturating_add(1));
    if let Some(path) = user_library {
        let user = Source {
            path: path.to_owned(),
            library: load(path)?,
        };
        // The same identity twice would make the library ids of entries and shadow records
        // ambiguous, so it is refused for a single library as it is for an environment.
        if base
            .iter()
            .any(|s| s.library.info.id == user.library.info.id)
        {
            return Err(invalid(
                "the user library has the same bim:library:id as another library of the lookup",
            ));
        }
        sources.push(user);
    }
    sources.extend(base);
    Ok((Lookup::new(sources), environment_info))
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
                lines.push(format!("double steel:profile:{name}Mm = {value}"));
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
                shadowed_library: "weak".into(),
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
        assert_eq!(library.entries.len(), 55);
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
        // and no HEA/HEB 250 (those designations do not exist; the sequence steps 240 to 260/270).
        assert_eq!(
            designations,
            [
                "HEA100", "HEA120", "HEA140", "HEA160", "HEA180", "HEA200", "HEA220", "HEA240",
                "HEA260", "HEA280", "HEA300", "HEA320", "HEA340", "HEA360", "HEA400", "HEA450",
                "HEA500", "HEA550", "HEA600", "HEB100", "HEB120", "HEB140", "HEB160", "HEB180",
                "HEB200", "HEB220", "HEB240", "HEB260", "HEB280", "HEB300", "HEB320", "HEB340",
                "HEB360", "HEB400", "HEB450", "HEB500", "HEB550", "HEB600", "IPE100", "IPE120",
                "IPE140", "IPE160", "IPE180", "IPE200", "IPE220", "IPE240", "IPE270", "IPE300",
                "IPE330", "IPE360", "IPE400", "IPE450", "IPE500", "IPE550", "IPE600",
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
        for absent in ["IPE250", "IPE280", "HEA250", "HEB250", "IPE650", "HEA700"] {
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
        assert_eq!(environment.sources[0].library.entries.len(), 55);
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
}
