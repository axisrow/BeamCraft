// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A bounded scanner of a *received* IFC STEP file (E04 study, contract
//! `docs/contracts/f12-external-reference-workflow-v0.1.md` XR-SOURCE-002).
//!
//! It answers one question: what exactly are these bytes, before anything converts them into
//! a reference? From the file alone it pins the original-byte SHA-256 and size, the declared
//! IFC schema, the descriptive header facts, an entity inventory, the GlobalId inventory with
//! duplicates, the declared project length unit and an explicit loss list of everything it did
//! not interpret. It is a study instrument, not an importer: nothing builds elements from what
//! it reads, and no conversion, mapping or admission happens here.
//!
//! Unlike [`crate::verify`], which checks the strict form this crate writes, this scanner
//! accepts the loose forms real exporters send: CRLF line ends, instance numbers with gaps,
//! several DATA sections, blank lines. Every deviation is counted or listed as a loss, never
//! guessed away. The supported reference subset (schema versions, entity families) and the
//! converter are owner decisions recorded in E04; this module only measures the input.

use crate::guid;
use beamcraft_model::digest::{ContentDigest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// Largest input the scanner accepts: the same ceiling as a stored foreign reference source,
/// because the received IFC bytes become exactly that source evidence when the workflow is
/// implemented.
pub const MAX_INPUT_BYTES: u64 = 32 * 1024 * 1024;

/// Largest number of distinct entity names kept in the inventory. Beyond it the rest is
/// counted as unclassified instead of growing the output without bound.
pub const MAX_DISTINCT_NAMES: usize = 2048;

/// Largest number of entities kept for reference resolution (the interpreted unit and
/// project entities). A real file holds a handful; a hostile one is refused resolution.
const MAX_INTERPRETED_ENTITIES: usize = 100_000;

/// Largest number of listed examples per diagnostic (duplicate GlobalIds, conversion-based
/// unit names, non-instance lines).
pub const MAX_LISTED: usize = 32;

/// Largest number of losses listed; further losses collapse into the truncation flag.
pub const MAX_LOSSES: usize = 64;

/// Largest header string kept as a fact. Anything longer is a loss, not a truncated value.
const MAX_HEADER_STRING_BYTES: usize = 256;

/// Largest header or instance line interpreted. Real exporters keep instances on one line;
/// a longer line is counted as uninterpreted instead of being parsed.
const MAX_LINE_BYTES: usize = 16 * 1024;

/// Why the bytes were refused before any inventory exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    /// The bytes do not start as an ISO 10303-21 physical file, or the DATA or final
    /// `END-ISO-10303-21;` framing is missing: nothing about them is pinned as IFC.
    NotAStepFile(&'static str),
    /// The input is larger than [`MAX_INPUT_BYTES`].
    TooLarge,
}

impl core::fmt::Display for ReadError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotAStepFile(why) => write!(f, "not an IFC STEP file: {why}"),
            Self::TooLarge => write!(f, "the file is larger than {MAX_INPUT_BYTES} bytes"),
        }
    }
}

impl core::error::Error for ReadError {}

/// One thing the scan did not interpret or could not resolve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loss {
    /// Stable code of the loss, for example `SCHEMA_OUTSIDE_STUDIED_SET`.
    pub code: &'static str,
    /// One-line detail with the observed facts.
    pub detail: String,
}

/// The descriptive header facts (XR-SOURCE-002: descriptive metadata, never version identity).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HeaderFacts {
    /// `FILE_NAME` attribute 1: the file name the sender declared.
    pub file_name: Option<String>,
    /// `FILE_NAME` attribute 2: the declared UTC time stamp.
    pub time_stamp: Option<String>,
    /// `FILE_NAME` attribute 5: the preprocessor version.
    pub preprocessor_version: Option<String>,
    /// `FILE_NAME` attribute 6: the originating system.
    pub originating_system: Option<String>,
}

/// A duplicate GlobalId with the instance number where it occurred.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalIdOccurrence {
    /// The instance number `#n`.
    pub instance: u64,
    /// The GlobalId text.
    pub global_id: String,
}

/// What the scan pinned about the input.
#[derive(Debug, Clone, PartialEq)]
pub struct Inspection {
    /// SHA-256 of the exact bytes scanned.
    pub digest: ContentDigest,
    /// Size of the scanned bytes.
    pub bytes: u64,
    /// The declared `FILE_SCHEMA` token, upper case, for example `IFC4`.
    pub schema_token: Option<String>,
    /// Whether the declared schema is the IFC4 subset this crate writes and studies.
    pub schema_recognized: bool,
    /// The descriptive header facts, when the header line was readable.
    pub header: Option<HeaderFacts>,
    /// Number of `#n = NAME(...)` instances found in the DATA sections.
    pub entity_count: u64,
    /// Instances per entity name, capped at [`MAX_DISTINCT_NAMES`] distinct names.
    pub entity_names: BTreeMap<String, u64>,
    /// Instances beyond the distinct-name cap, counted but not named.
    pub unclassified_instances: u64,
    /// DATA lines that are neither an instance nor empty: continuations of multi-line
    /// instances and anything else the line scan cannot classify.
    pub non_instance_lines: u64,
    /// Instances whose first attribute is a valid 22-character `IfcGloballyUniqueId`.
    pub global_id_count: u64,
    /// Distinct GlobalId values among them.
    pub distinct_global_ids: u64,
    /// Occurrences beyond the first for a repeated value.
    pub duplicate_global_ids: u64,
    /// Up to [`MAX_LISTED`] duplicate occurrences, for diagnosis.
    pub duplicate_samples: Vec<GlobalIdOccurrence>,
    /// Number of `IfcProject` instances.
    pub project_count: u64,
    /// True when an `IfcProject` exists whose first attribute is not a valid GlobalId.
    pub project_without_global_id: bool,
    /// The declared project length unit in metres per file unit, with where it was read
    /// from: the `IfcProject` units assignment (`project_units`) or, when there is no
    /// project or its chain is unreadable, the only SI length unit present
    /// (`si_units_only`).
    pub length_unit: Option<UnitsReading>,
    /// Non-SI (conversion-based) length unit names; their factors are not interpreted here.
    pub conversion_based_length_units: Vec<String>,
    /// The losses, capped at [`MAX_LOSSES`].
    pub losses: Vec<Loss>,
    /// True when losses beyond the cap were dropped.
    pub losses_truncated: bool,
}

/// A resolved length unit and where it was read from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UnitsReading {
    /// Metres per file unit, for example 0.001 for millimetres.
    pub meters_per_unit: f64,
    /// `"project_units"` or `"si_units_only"`.
    pub origin: &'static str,
}

/// One interpreted instance (`IfcProject`, a unit or an assignment), kept for resolution.
struct Interpreted {
    name: String,
    /// The attribute text after the instance head.
    body: String,
}

/// The mutable state of one scan.
struct Scan {
    out: Inspection,
    interpreted: Vec<Interpreted>,
    /// Instance number to position in `interpreted`, when the numbers are unique.
    index: BTreeMap<u64, usize>,
    seen_global_ids: BTreeSet<[u8; 16]>,
    /// Reference resolution is disabled because instance numbers repeat, several DATA
    /// sections exist, or the interpreted-entity cap was hit.
    resolution_disabled: bool,
}

/// Scans one received IFC file. Linear in the input size; every allocation is bounded by
/// the constants above. The bytes must already fit [`MAX_INPUT_BYTES`]; the check is
/// repeated here so a caller cannot bypass it.
pub fn inspect(bytes: &[u8]) -> Result<Inspection, ReadError> {
    let bytes_u64 = bytes
        .len()
        .try_into()
        .map_err(|_| ReadError::TooLarge)
        .and_then(|n| {
            if n > MAX_INPUT_BYTES {
                Err(ReadError::TooLarge)
            } else {
                Ok(n)
            }
        })?;
    let digest = Sha256::digest(bytes).map_err(|_| ReadError::TooLarge)?;
    let bom: &[u8] = &[0xEF, 0xBB, 0xBF];
    let text = match core::str::from_utf8(bytes.strip_prefix(bom).unwrap_or(bytes)) {
        // ISO 10303-21 restricts the physical file to a subset of ASCII, so every accepted
        // file is UTF-8; the two checks agree. Bytes that are not text at all cannot be a
        // STEP file, so they are refused like any other framing break.
        Ok(text) => text,
        Err(_) => return Err(ReadError::NotAStepFile("the bytes are not ASCII text")),
    };
    let mut lines = text.split('\n').peekable();
    let first = lines.next().map(strip_cr);
    if first != Some("ISO-10303-21;") {
        return Err(ReadError::NotAStepFile(
            "the first line is not ISO-10303-21;",
        ));
    }
    if lines.next().map(strip_cr) != Some("HEADER;") {
        return Err(ReadError::NotAStepFile("the second line is not HEADER;"));
    }
    let mut scan = Scan {
        out: Inspection::new(digest, bytes_u64),
        interpreted: Vec::new(),
        index: BTreeMap::new(),
        seen_global_ids: BTreeSet::new(),
        resolution_disabled: false,
    };
    if bytes.starts_with(bom) {
        push_loss(
            &mut scan.out.losses,
            &mut scan.out.losses_truncated,
            Loss {
                code: "BYTE_ORDER_MARK",
                detail: "a UTF-8 byte order mark precedes ISO-10303-21;".to_owned(),
            },
        );
    }
    if bytes.iter().any(|b| *b >= 128) {
        push_loss(
            &mut scan.out.losses,
            &mut scan.out.losses_truncated,
            Loss {
                code: "NON_ASCII_BYTES",
                detail: "the file carries bytes outside 7-bit ASCII, which ISO 10303-21 forbids"
                    .to_owned(),
            },
        );
    }
    scan.header(&mut lines)?;
    scan.data(&mut lines)?;
    scan.resolve_units();
    Ok(scan.out)
}

fn strip_cr(line: &str) -> &str {
    line.strip_suffix('\r').unwrap_or(line)
}

fn push_loss(losses: &mut Vec<Loss>, truncated: &mut bool, loss: Loss) {
    if losses.len() < MAX_LOSSES {
        losses.push(loss);
    } else {
        *truncated = true;
    }
}

impl Inspection {
    fn new(digest: ContentDigest, bytes: u64) -> Self {
        Self {
            digest,
            bytes,
            schema_token: None,
            schema_recognized: false,
            header: None,
            entity_count: 0,
            entity_names: BTreeMap::new(),
            unclassified_instances: 0,
            non_instance_lines: 0,
            global_id_count: 0,
            distinct_global_ids: 0,
            duplicate_global_ids: 0,
            duplicate_samples: Vec::new(),
            project_count: 0,
            project_without_global_id: false,
            length_unit: None,
            conversion_based_length_units: Vec::new(),
            losses: Vec::new(),
            losses_truncated: false,
        }
    }
}

impl Scan {
    fn loss(&mut self, code: &'static str, detail: String) {
        push_loss(
            &mut self.out.losses,
            &mut self.out.losses_truncated,
            Loss { code, detail },
        );
    }

    /// Reads the header section: the schema token and the descriptive facts.
    fn header(
        &mut self,
        lines: &mut std::iter::Peekable<std::str::Split<'_, char>>,
    ) -> Result<(), ReadError> {
        let mut omitted_facts = false;
        loop {
            let Some(raw) = lines.next() else {
                return Err(ReadError::NotAStepFile("the header section never ends"));
            };
            let line = strip_cr(raw);
            if line == "ENDSEC;" {
                break;
            }
            if line.len() > MAX_LINE_BYTES {
                omitted_facts = true;
                continue;
            }
            if let Some(token) = file_schema(line) {
                let recognized = token == "IFC4";
                self.out.schema_recognized = recognized;
                if !recognized {
                    self.loss(
                        "SCHEMA_OUTSIDE_STUDIED_SET",
                        format!(
                            "the file declares {token}; the studied writer subset is IFC4 and \
                             the supported reference subset is an owner decision (E04)"
                        ),
                    );
                }
                self.out.schema_token = Some(token);
            } else if line.starts_with("FILE_SCHEMA") {
                self.loss(
                    "SCHEMA_UNREADABLE",
                    format!(
                        "the FILE_SCHEMA header line does not have the declared form: {}",
                        clip(line)
                    ),
                );
            } else if line.starts_with("FILE_NAME(") && self.out.header.is_none() {
                let (facts, omitted) = file_name_facts(line);
                omitted_facts |= omitted;
                self.out.header = Some(facts);
            }
        }
        if self.out.schema_token.is_none() {
            self.loss(
                "SCHEMA_MISSING",
                "the header declares no FILE_SCHEMA".to_owned(),
            );
        }
        if omitted_facts {
            self.loss(
                "HEADER_FACTS_OMITTED",
                "a header line or string was too long or unreadable and was not kept".to_owned(),
            );
        }
        Ok(())
    }

    /// Walks the DATA sections. More than one section is legal ISO 10303-21 but unusual in
    /// IFC; instance numbers are then not unique, which is recorded instead of guessed.
    fn data(
        &mut self,
        lines: &mut std::iter::Peekable<std::str::Split<'_, char>>,
    ) -> Result<(), ReadError> {
        let mut sections = 0;
        loop {
            let Some(raw) = lines.next() else {
                return Err(ReadError::NotAStepFile(
                    "the file ends without END-ISO-10303-21;",
                ));
            };
            let line = strip_cr(raw);
            match line {
                "DATA;" => {
                    sections += 1;
                    if sections > 1 {
                        self.resolution_disabled = true;
                        self.loss(
                            "MULTIPLE_DATA_SECTIONS",
                            "instance numbers may repeat across sections; reference \
                             resolution is disabled"
                                .to_owned(),
                        );
                    }
                    loop {
                        let Some(raw) = lines.next() else {
                            return Err(ReadError::NotAStepFile(
                                "the file ends without END-ISO-10303-21;",
                            ));
                        };
                        let line = strip_cr(raw);
                        if line == "ENDSEC;" {
                            break;
                        }
                        if line == "END-ISO-10303-21;" {
                            return Err(ReadError::NotAStepFile(
                                "the DATA section ends without ENDSEC;",
                            ));
                        }
                        if !line.is_empty() {
                            self.instance_line(line);
                        }
                    }
                }
                "END-ISO-10303-21;" => {
                    // Only empty lines may follow; the peekable is consumed either way.
                    if lines.all(|rest| strip_cr(rest).is_empty()) {
                        return Ok(());
                    }
                    self.loss(
                        "TRAILING_CONTENT",
                        "content follows END-ISO-10303-21;".to_owned(),
                    );
                    return Ok(());
                }
                other => {
                    if !other.is_empty() {
                        self.loss(
                            "OUTSIDE_DATA_SECTION",
                            format!("a line outside a section was skipped: {:?}", clip(other)),
                        );
                    }
                }
            }
        }
    }

    /// Classifies one DATA line: an instance head, or an unclassifiable line.
    fn instance_line(&mut self, line: &str) {
        let bytes = line.as_bytes();
        if bytes.first() != Some(&b'#') {
            self.out.non_instance_lines += 1;
            if self.out.non_instance_lines <= MAX_LISTED as u64 {
                self.loss(
                    "NON_INSTANCE_LINE",
                    format!("a DATA line that does not start with '#': {:?}", clip(line)),
                );
            }
            return;
        }
        let Some(equals) = line.find('=') else {
            self.out.non_instance_lines += 1;
            return;
        };
        let Some(id) = line.get(1..equals).and_then(|d| d.trim().parse().ok()) else {
            self.out.non_instance_lines += 1;
            return;
        };
        let rest = line.get(equals + 1..).unwrap_or_default();
        let Some(open) = rest.find('(') else {
            self.out.non_instance_lines += 1;
            return;
        };
        let raw_name = rest.get(..open).unwrap_or_default().trim();
        let name: String = raw_name.chars().map(|c| c.to_ascii_uppercase()).collect();
        let name_ok = !name.is_empty()
            && name
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit());
        if !name_ok {
            self.out.non_instance_lines += 1;
            return;
        }
        self.out.entity_count += 1;
        if self.out.entity_names.len() < MAX_DISTINCT_NAMES {
            *self.out.entity_names.entry(name.clone()).or_default() += 1;
        } else {
            self.out.unclassified_instances += 1;
        }
        let attributes = rest.get(open + 1..).unwrap_or_default();
        // A well-formed instance line ends with `);`; without it the attributes are a
        // fragment of a multi-line instance and no argument split can succeed.
        let attributes = attributes
            .strip_suffix(';')
            .and_then(|body| body.strip_suffix(')'))
            .unwrap_or(attributes);
        self.global_id(id, attributes);
        // The units assignment may reference every member of the IfcUnit select type
        // (IfcSIUnit, IfcConversionBasedUnit, IfcContextDependentUnit, IfcDerivedUnit);
        // a member kept out of this list is reported as a missing instance even when it
        // exists.
        let interpreted = matches!(
            name.as_str(),
            "IFCSIUNIT"
                | "IFCPROJECT"
                | "IFCUNITASSIGNMENT"
                | "IFCCONVERSIONBASEDUNIT"
                | "IFCCONTEXTDEPENDENTUNIT"
                | "IFCDERIVEDUNIT"
        );
        if name == "IFCPROJECT" {
            self.out.project_count += 1;
            if !project_global_id_is_valid(attributes) {
                self.out.project_without_global_id = true;
                self.loss(
                    "PROJECT_WITHOUT_GLOBALID",
                    format!("#{id} is an IfcProject whose first attribute is not a valid GlobalId"),
                );
            }
        }
        if name == "IFCCONVERSIONBASEDUNIT"
            && let Some(args) = split_args(attributes)
            && args.get(1).map(|a| strip_dots(a)) == Some("LENGTHUNIT")
            && self.out.conversion_based_length_units.len() < MAX_LISTED
            && let Some(label) = args.get(2).and_then(|raw| raw_string(raw))
            && !self.out.conversion_based_length_units.contains(&label)
        {
            self.out.conversion_based_length_units.push(label);
        }
        if interpreted {
            if self.interpreted.len() >= MAX_INTERPRETED_ENTITIES {
                self.resolution_disabled = true;
                self.loss(
                    "INTERPRETED_ENTITY_CAP",
                    format!(
                        "more than {MAX_INTERPRETED_ENTITIES} project or unit instances exist; \
                         reference resolution is disabled"
                    ),
                );
                return;
            }
            if self.index.contains_key(&id) {
                self.resolution_disabled = true;
                self.loss(
                    "REPEATED_INSTANCE_NUMBER",
                    format!("#{id} occurs more than once; reference resolution is disabled"),
                );
            }
            self.index.insert(id, self.interpreted.len());
            self.interpreted.push(Interpreted {
                name,
                body: attributes.to_owned(),
            });
        }
    }

    /// The GlobalId candidate of an instance: a valid first-attribute string, deduplicated.
    fn global_id(&mut self, id: u64, attributes: &str) {
        let Some(text) = first_string(attributes) else {
            return;
        };
        let Ok(decoded) = guid::decode(text) else {
            return;
        };
        self.out.global_id_count += 1;
        if !self.seen_global_ids.insert(decoded) {
            self.out.duplicate_global_ids += 1;
            if self.out.duplicate_samples.len() < MAX_LISTED {
                self.out.duplicate_samples.push(GlobalIdOccurrence {
                    instance: id,
                    global_id: text.to_owned(),
                });
            }
        }
    }

    /// Resolves the declared project length unit, or falls back to the only SI length unit
    /// present when there is no project or its chain is unreadable. Nothing is guessed:
    /// every unresolvable case is a loss.
    fn resolve_units(&mut self) {
        self.out.distinct_global_ids = self.seen_global_ids.len() as u64;
        if self.resolution_disabled {
            self.loss(
                "REFERENCE_RESOLUTION_DISABLED",
                "instance numbers or sections are not unique enough to resolve references"
                    .to_owned(),
            );
            return;
        }
        let projects = self.interpreted_of("IFCPROJECT");
        if projects.len() > 1 {
            self.loss(
                "MULTIPLE_PROJECTS",
                format!(
                    "{} IfcProject instances exist; which one carries the file units is not \
                     decided here",
                    projects.len()
                ),
            );
            return;
        }
        if let Some(project) = projects.first() {
            let assignment = self.attribute_ref(project, 8).filter(|id| {
                self.interpreted_by_id(*id)
                    .is_some_and(|e| e.name == "IFCUNITASSIGNMENT")
            });
            match assignment {
                Some(id) => {
                    // The declaration is authoritative: when it holds no SI length unit
                    // there is no fallback to whatever else the file contains.
                    if let Some(reading) = self.units_of_assignment(id) {
                        self.out.length_unit = Some(reading);
                    }
                    return;
                }
                None => self.loss(
                    "PROJECT_UNITS_UNRESOLVED",
                    "the IfcProject units assignment is missing or is not an IfcUnitAssignment"
                        .to_owned(),
                ),
            }
        }
        // Fallback: the distinct SI length units present anywhere in the file.
        let mut si: Vec<f64> = Vec::new();
        for entity in &self.interpreted {
            if entity.name == "IFCSIUNIT"
                && let Some(reading) = si_length_of(&entity.body)
                && !si.contains(&reading)
            {
                si.push(reading);
            }
        }
        match si.as_slice() {
            [one] => {
                self.out.length_unit = Some(UnitsReading {
                    meters_per_unit: *one,
                    origin: "si_units_only",
                });
            }
            [] => self.loss(
                "NO_LENGTH_UNIT",
                "no SI length unit was found, so metres per unit is unknown".to_owned(),
            ),
            many => self.loss(
                "AMBIGUOUS_SI_LENGTH_UNITS",
                format!(
                    "{} distinct SI length units exist; which applies to the geometry is \
                     decided per representation context, which is not interpreted here",
                    many.len()
                ),
            ),
        }
    }

    fn interpreted_of(&self, name: &str) -> Vec<&Interpreted> {
        self.interpreted.iter().filter(|e| e.name == name).collect()
    }

    fn interpreted_by_id(&self, id: u64) -> Option<&Interpreted> {
        self.index.get(&id).and_then(|at| self.interpreted.get(*at))
    }

    /// The `#n` of one attribute of an entity, when it is a reference.
    fn attribute_ref(&self, entity: &Interpreted, position: usize) -> Option<u64> {
        let args = split_args(&entity.body)?;
        let text = args.get(position)?.trim();
        let digits = text.strip_prefix('#')?;
        if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
            digits.parse().ok()
        } else {
            None
        }
    }

    /// The length reading of the `IfcUnitAssignment` at `#id`: its SI length units. The
    /// conversion-based ones are counted by the instance scan; their factors are losses.
    /// A derived or context-dependent unit cannot change the length unit and is no loss.
    /// Every shape the scan cannot split is a loss, never a silent `None`.
    fn units_of_assignment(&mut self, id: u64) -> Option<UnitsReading> {
        let body = self.interpreted_by_id(id).map(|e| e.body.clone())?;
        let Some(list) = split_args(&body).and_then(|args| args.into_iter().next()) else {
            self.loss(
                "PROJECT_UNITS_UNREADABLE",
                format!(
                    "the attributes of the units assignment #{id} cannot be split into \
                     arguments"
                ),
            );
            return None;
        };
        let Some(list) = list
            .trim()
            .strip_prefix('(')
            .and_then(|rest| rest.strip_suffix(')'))
        else {
            self.loss(
                "PROJECT_UNITS_UNREADABLE",
                format!("the units assignment #{id} does not hold a units list"),
            );
            return None;
        };
        let mut si: Vec<f64> = Vec::new();
        for token in list.split(',') {
            let Some(id) = token.trim().strip_prefix('#').and_then(|d| d.parse().ok()) else {
                continue;
            };
            let Some(entity) = self.interpreted_by_id(id) else {
                self.loss(
                    "DANGLING_UNIT_REFERENCE",
                    format!("#{id} in the units assignment refers to a missing instance"),
                );
                continue;
            };
            match entity.name.as_str() {
                "IFCSIUNIT" => {
                    if let Some(reading) = si_length_of(&entity.body)
                        && !si.contains(&reading)
                    {
                        si.push(reading);
                    }
                }
                // A conversion-based unit is counted by the instance scan; its factor is
                // not interpreted here. A derived or context-dependent unit cannot change
                // the length unit, so neither one is a loss.
                "IFCCONVERSIONBASEDUNIT" | "IFCCONTEXTDEPENDENTUNIT" | "IFCDERIVEDUNIT" => {}
                other => self.loss(
                    "UNKNOWN_UNIT_KIND",
                    format!(
                        "#{id} is a {other} in the units assignment, which is not interpreted \
                         here"
                    ),
                ),
            }
        }
        match si.as_slice() {
            [one] => Some(UnitsReading {
                meters_per_unit: *one,
                origin: "project_units",
            }),
            [] => {
                self.loss(
                    "PROJECT_UNITS_NO_SI_LENGTH",
                    "the project units assignment holds no SI length unit, so metres per \
                     unit is unknown here; a conversion-based unit's factor is not \
                     interpreted"
                        .to_owned(),
                );
                None
            }
            many => {
                self.loss(
                    "AMBIGUOUS_SI_LENGTH_UNITS",
                    format!(
                        "the project units assignment holds {} distinct SI length units",
                        many.len()
                    ),
                );
                None
            }
        }
    }
}

/// The declared schema of a header line `FILE_SCHEMA(('IFC4'));`, upper case.
fn file_schema(line: &str) -> Option<String> {
    let inner = line.strip_prefix("FILE_SCHEMA((")?.strip_suffix("));")?;
    let token = raw_string(inner)?;
    if token.is_empty() || token.len() > MAX_HEADER_STRING_BYTES {
        return None;
    }
    Some(token.to_ascii_uppercase())
}

/// The descriptive facts of a header line `FILE_NAME('n','t',(...),(...),'p','o','a');`.
/// The author, organization and authorization attributes are descriptive and are not kept.
/// Returns whether any fact had to be omitted.
fn file_name_facts(line: &str) -> (HeaderFacts, bool) {
    let inner = line
        .strip_prefix("FILE_NAME(")
        .and_then(|rest| rest.strip_suffix(");"))
        .unwrap_or_default();
    let args = split_args(inner).unwrap_or_default();
    let mut omitted = false;
    let fact = |at: usize, omitted: &mut bool| -> Option<String> {
        let raw = args.get(at)?;
        if raw.len() > MAX_HEADER_STRING_BYTES {
            *omitted = true;
            return None;
        }
        match raw_string(raw) {
            Some(text) if text.len() <= MAX_HEADER_STRING_BYTES => Some(text),
            Some(_) => {
                *omitted = true;
                None
            }
            None => None,
        }
    };
    let facts = HeaderFacts {
        file_name: fact(0, &mut omitted),
        time_stamp: fact(1, &mut omitted),
        preprocessor_version: fact(4, &mut omitted),
        originating_system: fact(5, &mut omitted),
    };
    (facts, omitted)
}

/// Splits a top-level argument list on its commas, honouring strings and parentheses.
/// The raw text of each argument is returned; nothing beyond the structure is parsed.
fn split_args(body: &str) -> Option<Vec<String>> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut depth: usize = 0;
    let mut chars = body.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            current.push(c);
            if c == '\'' {
                if chars.peek() == Some(&'\'') {
                    chars.next();
                    current.push('\'');
                } else {
                    in_string = false;
                }
            }
        } else {
            match c {
                '\'' => {
                    in_string = true;
                    current.push(c);
                }
                '(' => {
                    depth = depth.checked_add(1)?;
                    current.push(c);
                }
                ')' => {
                    depth = depth.checked_sub(1)?;
                    current.push(c);
                }
                ',' if depth == 0 => args.push(core::mem::take(&mut current)),
                _ => current.push(c),
            }
        }
        if current.len() > MAX_LINE_BYTES {
            return None;
        }
    }
    if in_string || depth != 0 {
        return None;
    }
    args.push(current);
    Some(args)
}

/// The decoded text of a raw STEP string argument `'...'`, or `None` for any other shape.
fn raw_string(raw: &str) -> Option<String> {
    let text = raw.trim();
    let inner = text.strip_prefix('\'')?.strip_suffix('\'')?;
    if inner.contains('\'') {
        // A string with an escaped apostrophe does not split into a clean argument.
        return None;
    }
    crate::verify::unescape(inner).ok()
}

/// Whether the first attribute of an `IfcProject` line is a valid GlobalId: an `IfcProject`
/// always has one in the schema, so its absence is a diagnosed loss, not a guess.
fn project_global_id_is_valid(attributes: &str) -> bool {
    match first_string(attributes) {
        Some(text) => guid::decode(text).is_ok(),
        None => false,
    }
}

/// The text when the first attribute of an instance is a string: its 22 characters are a
/// GlobalId candidate when they decode. Only entities under `IfcRoot` carry such an
/// attribute in the schema; a property name of that shape would also count here, which the
/// inventory discloses rather than resolves.
fn first_string(attributes: &str) -> Option<&str> {
    let rest = attributes.trim_start();
    if !rest.starts_with('\'') {
        return None;
    }
    let bytes = rest.as_bytes();
    let mut at = 1;
    while bytes.get(at).is_some() {
        match bytes.get(at) {
            Some(b'\'') if bytes.get(at.wrapping_add(1)) == Some(&b'\'') => at += 2,
            Some(b'\'') => break,
            Some(_) => at = at.wrapping_add(1),
            None => return None,
        }
    }
    if bytes.get(at) != Some(&b'\'') {
        return None;
    }
    let inner = rest.get(1..at)?;
    if inner.len() == 22 && !inner.contains('\'') {
        Some(inner)
    } else {
        None
    }
}

/// The metres-per-unit of an `IFCSIUNIT` body when it is a length unit with a known name
/// and prefix, for example `*,.LENGTHUNIT.,.MILLI.,.METRE.`. An absent prefix (`$`) is one;
/// any other name or prefix is unreadable here.
fn si_length_of(body: &str) -> Option<f64> {
    let args = split_args(body)?;
    if args.get(1).map(|a| strip_dots(a)) != Some("LENGTHUNIT") {
        return None;
    }
    let name = match args.get(3).map(|a| strip_dots(a)) {
        Some("METRE") => 1.0_f64,
        _ => return None,
    };
    let prefix = match args.get(2).map(|a| strip_dots(a)) {
        None | Some("") | Some("$") => 1.0,
        Some(prefix) => si_prefix_factor(prefix)?,
    };
    Some(name * prefix)
}

fn strip_dots(raw: &str) -> &str {
    let text = raw.trim();
    let body = text.strip_suffix('.').unwrap_or(text);
    body.strip_prefix('.').unwrap_or(body)
}

/// The factor of an `IfcSIPrefix` literal.
fn si_prefix_factor(prefix: &str) -> Option<f64> {
    let factor = match prefix {
        "EXA" => 1e18,
        "PETA" => 1e15,
        "TERA" => 1e12,
        "GIGA" => 1e9,
        "MEGA" => 1e6,
        "KILO" => 1e3,
        "HECTO" => 1e2,
        "DECA" => 1e1,
        "DECI" => 1e-1,
        "CENTI" => 1e-2,
        "MILLI" => 1e-3,
        "MICRO" => 1e-6,
        "NANO" => 1e-9,
        "PICO" => 1e-12,
        "FEMTO" => 1e-15,
        "ATTO" => 1e-18,
        _ => return None,
    };
    Some(factor)
}

/// Clips a line for a loss detail: enough characters to identify it.
fn clip(line: &str) -> String {
    const LIMIT: usize = 80;
    let mut out: String = line.chars().take(LIMIT).collect();
    if line.chars().count() > LIMIT {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inspect_str(text: &str) -> Inspection {
        inspect(text.as_bytes()).ok().unwrap()
    }

    /// A contributor-original synthetic IFC4 file in the strict form this crate writes.
    fn minimal_ifc4() -> String {
        [
            "ISO-10303-21;",
            "HEADER;",
            "FILE_DESCRIPTION(('synthetic study file'),'2;1');",
            "FILE_NAME('wall.ifc','2026-11-05T00:00:00',('study'),('apibim'),'none',\
             'beamcraft read tests','');",
            "FILE_SCHEMA(('IFC4'));",
            "ENDSEC;",
            "DATA;",
            "#1=IFCPROJECT('0YvucVMOX9kw8jhtQ9abcd',*,$,$,$,$,$,(#2),#3);",
            "#2=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#4,$);",
            "#3=IFCUNITASSIGNMENT((#5));",
            "#4=IFCAXIS2PLACEMENT3D(#6,$,$);",
            "#5=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);",
            "#6=IFCCARTESIANPOINT((0.,0.,0.));",
            "#7=IFCWALL('1Anr5VMOX0PO8jvHqZabcd',*,'wall',$,$,$,$,$,$);",
            "#8=IFCSLAB('2Bnr5VMOX0PO8jvHqZabcd',*,'slab',$,$,$,$,$,$);",
            "ENDSEC;",
            "END-ISO-10303-21;",
            "",
        ]
        .join("\n")
    }

    #[test]
    fn a_minimal_ifc4_file_is_pinned() {
        let file = minimal_ifc4();
        let out = inspect_str(&file);
        assert_eq!(out.schema_token.as_deref(), Some("IFC4"));
        assert!(out.schema_recognized);
        let header = out.header.unwrap_or_default();
        assert_eq!(header.file_name.as_deref(), Some("wall.ifc"));
        assert_eq!(header.time_stamp.as_deref(), Some("2026-11-05T00:00:00"));
        assert_eq!(
            header.originating_system.as_deref(),
            Some("beamcraft read tests")
        );
        assert_eq!(out.entity_count, 8);
        assert_eq!(out.entity_names.get("IFCWALL"), Some(&1));
        assert_eq!(out.global_id_count, 3);
        assert_eq!(out.distinct_global_ids, 3);
        assert_eq!(out.duplicate_global_ids, 0);
        assert_eq!(out.project_count, 1);
        assert!(!out.project_without_global_id);
        assert_eq!(
            out.length_unit,
            Some(UnitsReading {
                meters_per_unit: 0.001,
                origin: "project_units",
            })
        );
        assert!(out.losses.is_empty(), "{:?}", out.losses);
        assert_eq!(out.digest, Sha256::digest(file.as_bytes()).ok().unwrap());
        assert_eq!(out.bytes, file.len() as u64);
    }

    #[test]
    fn crlf_and_blank_lines_are_accepted() {
        let file = minimal_ifc4().replace('\n', "\r\n");
        let out = inspect_str(&file);
        assert_eq!(out.entity_count, 8);
        assert!(out.losses.is_empty(), "{:?}", out.losses);
    }

    #[test]
    fn a_duplicate_global_id_is_listed() {
        let file = minimal_ifc4().replace("'2Bnr5VMOX0PO8jvHqZabcd'", "'1Anr5VMOX0PO8jvHqZabcd'");
        let out = inspect_str(&file);
        assert_eq!(out.global_id_count, 3);
        assert_eq!(out.distinct_global_ids, 2);
        assert_eq!(out.duplicate_global_ids, 1);
        assert_eq!(
            out.duplicate_samples,
            vec![GlobalIdOccurrence {
                instance: 8,
                global_id: "1Anr5VMOX0PO8jvHqZabcd".to_owned(),
            }]
        );
    }

    #[test]
    fn bytes_that_are_not_text_are_refused() {
        let mut not_text = minimal_ifc4().into_bytes();
        let at = not_text.len() - 8;
        not_text[at] = 0xFF;
        assert_eq!(
            inspect(&not_text),
            Err(ReadError::NotAStepFile("the bytes are not ASCII text"))
        );
    }

    #[test]
    fn an_unreadable_units_assignment_is_a_loss_not_a_silent_null() {
        let file =
            minimal_ifc4().replace("#3=IFCUNITASSIGNMENT((#5));", "#3=IFCUNITASSIGNMENT((#5);");
        let out = inspect_str(&file);
        assert_eq!(out.length_unit, None);
        assert!(
            out.losses
                .iter()
                .any(|l| l.code == "PROJECT_UNITS_UNREADABLE"),
            "{:?}",
            out.losses
        );
    }

    #[test]
    fn a_declared_non_ifc4_schema_is_a_loss_not_a_refusal() {
        let file = minimal_ifc4().replace("FILE_SCHEMA(('IFC4'));", "FILE_SCHEMA(('IFC2X3'));");
        let out = inspect_str(&file);
        assert_eq!(out.schema_token.as_deref(), Some("IFC2X3"));
        assert!(!out.schema_recognized);
        assert!(
            out.losses
                .iter()
                .any(|l| l.code == "SCHEMA_OUTSIDE_STUDIED_SET"),
            "{:?}",
            out.losses
        );
    }

    #[test]
    fn an_inch_only_file_reports_the_conversion_based_unit_and_no_reading() {
        let file = minimal_ifc4()
            .replace("#5=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);\n", "")
            .replace(
                "#3=IFCUNITASSIGNMENT((#5));",
                "#3=IFCUNITASSIGNMENT((#9));\n\
                 #9=IFCCONVERSIONBASEDUNIT(*,.LENGTHUNIT.,'INCH',#10);\n\
                 #10=IFCMEASUREWITHUNIT(IFCLENGTHMEASURE(0.0254),#11);\n\
                 #11=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);",
            );
        let out = inspect_str(&file);
        assert_eq!(out.length_unit, None);
        assert_eq!(out.conversion_based_length_units, vec!["INCH"]);
        assert!(
            out.losses
                .iter()
                .any(|l| l.code == "PROJECT_UNITS_NO_SI_LENGTH"),
            "{:?}",
            out.losses
        );
    }

    #[test]
    fn a_broken_project_chain_falls_back_to_the_si_units_present() {
        let file = minimal_ifc4().replace("(#2),#3);", "(#2),#99);");
        let out = inspect_str(&file);
        assert_eq!(
            out.length_unit,
            Some(UnitsReading {
                meters_per_unit: 0.001,
                origin: "si_units_only",
            })
        );
        assert!(
            out.losses
                .iter()
                .any(|l| l.code == "PROJECT_UNITS_UNRESOLVED"),
            "{:?}",
            out.losses
        );
    }

    #[test]
    fn a_dangling_unit_inside_the_assignment_is_a_loss_but_the_reading_survives() {
        let file = minimal_ifc4().replace(
            "#3=IFCUNITASSIGNMENT((#5));",
            "#3=IFCUNITASSIGNMENT((#5,#99));",
        );
        let out = inspect_str(&file);
        assert_eq!(
            out.length_unit,
            Some(UnitsReading {
                meters_per_unit: 0.001,
                origin: "project_units",
            })
        );
        assert!(
            out.losses
                .iter()
                .any(|l| l.code == "DANGLING_UNIT_REFERENCE"),
            "{:?}",
            out.losses
        );
    }

    #[test]
    fn framing_breaks_are_refusals() {
        assert_eq!(
            inspect(b""),
            Err(ReadError::NotAStepFile(
                "the first line is not ISO-10303-21;"
            ))
        );
        assert_eq!(
            inspect(b"not a step file at all\n"),
            Err(ReadError::NotAStepFile(
                "the first line is not ISO-10303-21;"
            ))
        );
        let file = minimal_ifc4();
        assert_eq!(
            inspect(file.replace("END-ISO-10303-21;", "").as_bytes()),
            Err(ReadError::NotAStepFile(
                "the file ends without END-ISO-10303-21;"
            ))
        );
        assert_eq!(
            inspect(file.replace("HEADER;", "HEADERS;").as_bytes()),
            Err(ReadError::NotAStepFile("the second line is not HEADER;"))
        );
    }

    #[test]
    fn a_non_ascii_file_is_a_counted_loss_not_a_crash() {
        let file = minimal_ifc4().replace("'wall'", "'стена'");
        let out = inspect_str(&file);
        assert!(
            out.losses.iter().any(|l| l.code == "NON_ASCII_BYTES"),
            "{:?}",
            out.losses
        );
        assert_eq!(out.entity_count, 8);
    }

    #[test]
    fn lines_that_are_not_instances_are_counted() {
        let file = minimal_ifc4().replace("END-ISO-10303-21;\n", "END-ISO-10303-21;\njunk\n");
        let out = inspect_str(&file);
        assert_eq!(out.non_instance_lines, 0);
        assert!(out.losses.iter().any(|l| l.code == "TRAILING_CONTENT"));
        let broken = minimal_ifc4().replace("#7=IFCWALL(", "#7=ifcwall!(");
        let out = inspect_str(&broken);
        assert_eq!(out.non_instance_lines, 1);
        assert_eq!(out.entity_count, 7);
    }

    #[test]
    fn a_line_inside_data_that_never_starts_an_instance_is_counted() {
        let file = minimal_ifc4().replace("#7=IFCWALL(", "junk\n#7=IFCWALL(");
        let out = inspect_str(&file);
        assert_eq!(out.non_instance_lines, 1);
        assert_eq!(out.entity_count, 8);
        assert!(out.losses.iter().any(|l| l.code == "NON_INSTANCE_LINE"));
    }

    #[test]
    fn an_unknown_prefix_or_name_is_not_a_reading() {
        let file = minimal_ifc4().replace(".MILLI.,.METRE.", ".FOOT.,.METRE.");
        let out = inspect_str(&file);
        // .FOOT. is not an IfcSIPrefix literal, so the project chain resolves to nothing
        // and the declaration is authoritative: no fallback.
        assert_eq!(out.length_unit, None);
        assert!(
            out.losses
                .iter()
                .any(|l| l.code == "PROJECT_UNITS_NO_SI_LENGTH"),
            "{:?}",
            out.losses
        );
    }

    #[test]
    fn a_project_whose_global_id_is_not_valid_is_a_loss() {
        let file = minimal_ifc4().replace("'0YvucVMOX9kw8jhtQ9abcd'", "'the site project'");
        let out = inspect_str(&file);
        assert!(out.project_without_global_id);
        assert_eq!(out.global_id_count, 2);
        assert!(
            out.losses
                .iter()
                .any(|l| l.code == "PROJECT_WITHOUT_GLOBALID")
        );
    }

    #[test]
    fn oversized_input_is_refused_before_any_parsing() {
        let big = vec![b'a'; (MAX_INPUT_BYTES + 1) as usize];
        assert_eq!(inspect(&big), Err(ReadError::TooLarge));
    }
}
