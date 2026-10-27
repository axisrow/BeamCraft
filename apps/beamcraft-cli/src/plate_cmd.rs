// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! P01 headless commands: `create-plate` and `inspect-elements`. They call the engine API through
//! the adapter of `beamcraft_cli::plate` and print one JSON object (same error shape as the other
//! commands; exit 0 success, 2 rejection, 3 backend unavailable, 4 outcome unknown).

use crate::report::report_with;
use beamcraft_cli::copy::{CopiedElement, CopyRequest, copy_element, copy_instance};
use beamcraft_cli::cut::{CutElement, CutRequest, cut_by_plane_record};
use beamcraft_cli::definition::{
    DefineRequest, Defined, DefinitionRecord, NameResolution, define, definition_path,
    head_definitions, resolve_name,
};
use beamcraft_cli::delete::{DeleteRequest, DeletedKind, delete_element};
use beamcraft_cli::edit::{UpdateRequest, UpdatedElement, update_element};
use beamcraft_cli::history::{Direction, Step, step, timeline};
use beamcraft_cli::hole::{AddHoleRequest, add_hole_command};
use beamcraft_cli::library::open_lookup;
use beamcraft_cli::member::{
    CreateMemberRequest, CreatePolybeamRequest, create_member, create_polybeam,
};
use beamcraft_cli::plate::{
    AcceptedBinding, CreateRequest, Created, Inspection, PlateError, ProjectClaim, QuantityReport,
    RevisionSummary, accepted_binding, claim_project, ensure_project, inspect,
};
use beamcraft_cli::plate_draft::{CreatedDraft, create_plate_draft};
use beamcraft_cli::workspace::{
    CreateWorkspace, WorkspaceStatus, create_workspace, publish_workspace,
    status as workspace_status,
};
use beamcraft_engine::command::{Quantities, origin_rectangle_contour};
use beamcraft_engine::footprint::{Entry, Footprint, Target};
use beamcraft_engine::plate_draft::UnsupportedPlate;
use beamcraft_engine::quantity::{Freshness, is_superseded_algorithm};
use beamcraft_engine::query::committed::MemberRecord;
use beamcraft_engine::query::committed::PlateRecord;
use beamcraft_host::{
    ids::IdSource,
    revision::{NoFault, RevisionStore, Uuid7},
    storage::BlobStore,
};
use beamcraft_json::{Json, Limits};
use beamcraft_model::command::{
    AddHole, CopyElement, CreatePlate, CreatePolybeam, CreateStraightMember, CutByPlane, CutRecord,
    DeleteElement, ElementPatch, HoleSpec, KeepSide, MAX_HOLES, MemberKind, Placement, PlateSide,
    UpdateElement,
};
use beamcraft_model::definition::SectionProperties;
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, OpId, ProjectId, RevisionId, WorkspaceId};
use beamcraft_model::limits::MAX_MEMBER_PATH_POINTS;
use beamcraft_model::section::{CustomSection, ParametricSection, SectionFamily, SectionShape};
use std::collections::BTreeMap;
use std::ffi::OsString;

pub(crate) type Run<T> = Result<T, PlateError>;

pub(crate) fn invalid(message: &'static str) -> PlateError {
    PlateError {
        rule_id: None,
        code: ErrorCode::InvalidArgument,
        message,
    }
}

/// Runs one plate command and returns the JSON result and the process exit code.
pub fn execute(args: &[OsString]) -> (Json, u8) {
    let mut candidates = Vec::new();
    let result = run(args, &mut candidates);
    report_with(result, candidates)
}

const CREATE_OPTIONS: &[&str] = &[
    "--class",
    "--plane",
    "--store",
    "--width",
    "--length",
    "--contour",
    "--thickness",
    "--thickness-side",
    "--name",
    "--translation",
    "--orientation-xyzw",
    "--material-digest",
    "--material",
    "--hole-rect",
    "--hole-round",
    "--hole-segments",
    "--new-id",
    "--op-id",
    "--revision-id",
    "--actor-id",
    "--project-id",
    "--workspace-id",
    "--z",
    "--level",
    "--offset",
    "--origin-at-grid",
];

const ADD_HOLE_OPTIONS: &[&str] = &[
    "--store",
    "--id",
    "--operation-id",
    "--polygon",
    "--hole-rect",
    "--hole-round",
    "--revision-id",
    "--actor-id",
    "--project-id",
    "--workspace-id",
];

const DEFINE_MATERIAL_OPTIONS: &[&str] = &[
    "--store",
    "--density",
    "--grade",
    "--standard",
    "--name",
    "--revision-id",
    "--actor-id",
    "--project-id",
    "--workspace-id",
];

const DEFINE_PROFILE_OPTIONS: &[&str] = &[
    "--store",
    "--width",
    "--height",
    "--family",
    "--h",
    "--b",
    "--tw",
    "--tf",
    "--t",
    "--r",
    "--c",
    "--d",
    "--outer",
    "--cavity",
    "--designation",
    "--standard",
    "--area-mm2",
    "--mass-per-meter-kg",
    "--surface-per-meter-m2",
    "--name",
    "--library",
    "--environment",
    "--user-library",
    "--revision-id",
    "--actor-id",
    "--project-id",
    "--workspace-id",
];

/// The `define-profile` options that describe a section family or a custom outline, never a
/// rectangle: without `--family` they would otherwise be ignored.
const SECTION_ONLY_OPTIONS: &[&str] = &[
    "--h", "--b", "--tw", "--tf", "--t", "--r", "--c", "--d", "--outer", "--cavity",
];

/// The `define-fastener` options (A14.1): the definition takes every value from one fastener
/// library entry; there is no explicit-value form and no environment yet.
const DEFINE_FASTENER_OPTIONS: &[&str] = &[
    "--store",
    "--fastener-library",
    "--designation",
    "--name",
    "--revision-id",
    "--actor-id",
    "--project-id",
    "--workspace-id",
];

const CREATE_MEMBER_OPTIONS: &[&str] = &[
    "--class",
    "--plane",
    "--store",
    "--kind",
    "--start",
    "--end",
    "--axis-lock",
    "--profile-digest",
    "--profile",
    "--roll",
    "--name",
    "--translation",
    "--orientation-xyzw",
    "--material-digest",
    "--material",
    "--new-id",
    "--op-id",
    "--revision-id",
    "--actor-id",
    "--project-id",
    "--workspace-id",
    "--z",
    "--level",
    "--offset",
    "--start-at-grid",
    "--end-at-grid",
];

const CREATE_POLYBEAM_OPTIONS: &[&str] = &[
    "--class",
    "--store",
    "--kind",
    "--points",
    "--profile-digest",
    "--profile",
    "--name",
    "--translation",
    "--orientation-xyzw",
    "--material-digest",
    "--material",
    "--new-id",
    "--op-id",
    "--revision-id",
    "--actor-id",
    "--project-id",
    "--workspace-id",
];

const CUT_OPTIONS: &[&str] = &[
    "--store",
    "--id",
    "--operation-id",
    "--point",
    "--normal",
    "--keep",
    "--op-id",
    "--revision-id",
    "--actor-id",
    "--project-id",
    "--workspace-id",
];

const UPDATE_OPTIONS: &[&str] = &[
    "--class",
    "--store",
    "--id",
    "--name",
    "--material-digest",
    "--thickness",
    "--thickness-side",
    "--axis-point",
    "--translation",
    "--orientation-xyzw",
    "--level",
    "--op-id",
    "--revision-id",
    "--actor-id",
    "--project-id",
    "--workspace-id",
];

const DELETE_OPTIONS: &[&str] = &[
    "--store",
    "--id",
    "--op-id",
    "--revision-id",
    "--actor-id",
    "--project-id",
    "--workspace-id",
];

const COPY_OPTIONS: &[&str] = &[
    "--new-axis-ids",
    "--store",
    "--id",
    "--name",
    "--offset",
    "--new-id",
    "--op-id",
    "--revision-id",
    "--actor-id",
    "--project-id",
    "--workspace-id",
];

const WORKSPACE_OPTIONS: &[&str] = &[
    "--store",
    "--workspace-dir",
    "--workspace-id",
    "--user-id",
    "--revision-id",
    "--op-id",
];

const HISTORY_OPTIONS: &[&str] = &["--workspace-dir", "--revision-id", "--op-id"];

fn run(args: &[OsString], candidates: &mut Vec<Json>) -> Run<Json> {
    let args: Vec<&str> = args
        .iter()
        .map(|a| {
            a.to_str()
                .filter(|s| s.len() <= 4096)
                .ok_or_else(|| invalid("argument is not bounded UTF-8"))
        })
        .collect::<Run<_>>()?;
    let (command, tail) = args
        .split_first()
        .ok_or_else(|| invalid("missing command"))?;
    let allowed: &[&str] = match *command {
        "create-plate" => CREATE_OPTIONS,
        "create-member" => CREATE_MEMBER_OPTIONS,
        "create-polybeam" => CREATE_POLYBEAM_OPTIONS,
        "add-hole" => ADD_HOLE_OPTIONS,
        "update-element" => UPDATE_OPTIONS,
        "delete-element" => DELETE_OPTIONS,
        "copy-element" | "copy-instance" => COPY_OPTIONS,
        "cut-by-plane" => CUT_OPTIONS,
        "create-workspace" | "inspect-workspace" | "publish-workspace" => WORKSPACE_OPTIONS,
        "undo-workspace" | "redo-workspace" => HISTORY_OPTIONS,
        "inspect-workspace-history" => &["--workspace-dir"],
        "define-material" => DEFINE_MATERIAL_OPTIONS,
        "define-profile" => DEFINE_PROFILE_OPTIONS,
        "define-fastener" => DEFINE_FASTENER_OPTIONS,
        "inspect-elements" => &["--store", "--element"],
        "definitions" => &["--store", "--name", "--offset", "--limit"],
        _ => return Err(invalid("unknown command")),
    };
    if !tail.len().is_multiple_of(2) || tail.len() > 40 {
        return Err(invalid("options are KEY VALUE pairs"));
    }
    let mut options = BTreeMap::new();
    for pair in tail.as_chunks::<2>().0 {
        let [key, value] = pair;
        if !allowed.contains(key) || options.insert(*key, *value).is_some() {
            return Err(invalid("unknown or repeated option"));
        }
    }
    if matches!(
        *command,
        "undo-workspace" | "redo-workspace" | "inspect-workspace-history"
    ) {
        return history_command(command, &options);
    }
    // Parsed before the store is opened, so a malformed id reads nothing.
    let element = match options.get("--element") {
        Some(text) => Some(ElementId::parse(text).map_err(id_error)?),
        None => None,
    };
    let store_root = options
        .get("--store")
        .ok_or_else(|| invalid("--store is required"))?;
    let blobs = BlobStore::new(store_root).map_err(|e| PlateError {
        rule_id: None,
        code: e.code(),
        message: "store root is not usable",
    })?;
    let store = RevisionStore::new(blobs);
    let _claim = check_project_option(&store, &options)?;
    match *command {
        "create-plate" => create(&store, &options, candidates),
        "create-member" => create_member_command(&store, &options, candidates),
        "create-polybeam" => create_polybeam_command(&store, &options, candidates),
        "add-hole" => add_hole_cli(&store, &options),
        "update-element" => update_command(&store, &options),
        "delete-element" => delete_command(&store, &options),
        "copy-element" => copy_command(&store, &options, false),
        "copy-instance" => copy_command(&store, &options, true),
        "cut-by-plane" => cut_command(&store, &options),
        "create-workspace" | "inspect-workspace" | "publish-workspace" => {
            workspace_command(&store, command, &options, candidates)
        }
        "define-material" | "define-profile" | "define-fastener" => {
            define_command(&store, command, &options)
        }
        "definitions" => definitions_json(&inspect(&store)?, &options),
        _ => Ok(inspection_json(&inspect(&store)?, element)?),
    }
}

fn float(options: &BTreeMap<&str, &str>, key: &'static str) -> Run<f64> {
    options
        .get(key)
        .ok_or_else(|| invalid("a required dimension is missing"))?
        .parse::<f64>()
        .map_err(|_| invalid("a number is malformed"))
}

pub(crate) fn translation(text: &str) -> Run<[f64; 3]> {
    let parts: Vec<f64> = text
        .split(',')
        .map(|p| {
            p.parse::<f64>()
                .map_err(|_| invalid("translation is not three numbers"))
        })
        .collect::<Run<_>>()?;
    parts
        .try_into()
        .map_err(|_| invalid("translation is not three numbers"))
}

pub(crate) fn quaternion(text: &str) -> Run<[f64; 4]> {
    let parts: Vec<f64> = text
        .split(',')
        .map(|p| {
            p.parse::<f64>()
                .map_err(|_| invalid("orientation is not four numbers"))
        })
        .collect::<Run<_>>()?;
    parts
        .try_into()
        .map_err(|_| invalid("orientation is not four numbers"))
}

pub(crate) fn id_error(error: beamcraft_model::id::IdError) -> PlateError {
    PlateError {
        rule_id: Some("DV-BASE-001"),
        code: error.code(),
        message: "identity is not a full canonical UUIDv7",
    }
}

/// D1: an explicit `--project-id` must be the project the store records (a store without a
/// revision takes it). The early check runs here; the returned claim binds the command's base
/// loading to that project until it is dropped, so a store initialised by another writer in
/// between is refused rather than adopted. Shared by every command that authors a revision.
pub(crate) fn check_project_option(
    store: &RevisionStore,
    options: &BTreeMap<&str, &str>,
) -> Run<Option<ProjectClaim>> {
    match options.get("--project-id") {
        Some(text) => {
            let project = ProjectId::parse(text).map_err(id_error)?;
            ensure_project(store, &project)?;
            Ok(Some(claim_project(project)))
        }
        None => Ok(None),
    }
}

fn digest_option(text: &str) -> Run<ContentDigest> {
    text.parse::<ContentDigest>()
        .map_err(|_| invalid("a digest is not 64 lower-case hexadecimal characters"))
}

/// Most candidates of an ambiguous name that an error response lists; the rest is counted and
/// listed by `definitions --name NAME --offset N`. Sized so the response stays far inside the
/// shared JSON encoder budgets whatever the number of definitions.
const MAX_LISTED_CANDIDATES: usize = 16;

/// The diagnostics of an ambiguous name: at most [`MAX_LISTED_CANDIDATES`] candidates and, when
/// there are more, a final object with the total and how to list the rest.
fn ambiguity_diagnostics(found: &[(ContentDigest, DefinitionRecord)]) -> Vec<Json> {
    let mut diagnostics: Vec<Json> = found
        .iter()
        .take(MAX_LISTED_CANDIDATES)
        .map(|(digest, record)| candidate_json(digest, record))
        .collect();
    if found.len() > MAX_LISTED_CANDIDATES {
        diagnostics.push(Json::object([
            ("candidate_count", Json::uint(found.len() as u64)),
            ("listed", Json::uint(MAX_LISTED_CANDIDATES as u64)),
            (
                "list_rest",
                Json::text(format!(
                    "definitions --store ROOT --name NAME --offset {MAX_LISTED_CANDIDATES}"
                )),
            ),
        ]));
    }
    diagnostics
}

/// A definition chosen by its full digest (`keys.0`) or by a readable name (`keys.1`), never
/// both. A name is resolved against the definitions of the head revision by display name or
/// catalogue label; no match is `NOT_FOUND` and a name that matches several definitions is
/// `INVALID_ARGUMENT` with the first candidates in `candidates`, never a guess. For an operation
/// the ledger already accepted, a name that still describes the definition the element was bound
/// to resolves to that definition (`replay`), so a retry is a replay even after another
/// definition with the same name was published; any other name resolves normally and the
/// engine's content fingerprint rejects the changed request.
fn selected(
    store: &RevisionStore,
    options: &BTreeMap<&str, &str>,
    keys: (&'static str, &'static str),
    profiles: bool,
    replay: Option<&AcceptedBinding>,
    candidates: &mut Vec<Json>,
) -> Run<Option<ContentDigest>> {
    let (digest_key, name_key) = keys;
    match (options.get(digest_key), options.get(name_key)) {
        (Some(_), Some(_)) => Err(invalid("give the digest or the name, not both")),
        (Some(text), None) => digest_option(text).map(Some),
        (None, None) => Ok(None),
        (None, Some(name)) => {
            if let Some(binding) = replay {
                let original = if profiles {
                    binding.profile
                } else {
                    binding.material
                };
                if let Some(digest) = original
                    && binding.definitions.get(&digest).is_some_and(|record| {
                        record.is_profile() == profiles && record.selectable_names().contains(name)
                    })
                {
                    return Ok(Some(digest));
                }
            }
            let definitions = head_definitions(store)?;
            match resolve_name(&definitions, name, profiles) {
                NameResolution::Found(digest) => Ok(Some(digest)),
                NameResolution::NotFound => Err(PlateError {
                    rule_id: None,
                    code: ErrorCode::NotFound,
                    message: if profiles {
                        "no profile definition in the head has this name"
                    } else {
                        "no material definition in the head has this name"
                    },
                }),
                NameResolution::Ambiguous(found) => {
                    candidates.extend(ambiguity_diagnostics(&found));
                    Err(invalid(
                        "the name matches more than one definition; use a unique name or the digest",
                    ))
                }
            }
        }
    }
}

/// One candidate of an ambiguous name.
fn candidate_json(digest: &ContentDigest, record: &DefinitionRecord) -> Json {
    Json::object([
        ("digest", Json::text(digest.to_string())),
        ("kind", Json::text(record.kind())),
        (
            "display_name",
            record.display_name().map_or(Json::Null, Json::text),
        ),
        (
            "names",
            Json::Array(
                record
                    .selectable_names()
                    .into_iter()
                    .map(Json::text)
                    .collect(),
            ),
        ),
    ])
}

/// The section shape of `define-profile --family`: a family token with one `--<parameter>` per
/// family parameter (millimetres), or `custom` with `--outer "X,Y;X,Y;..."` (counter-clockwise)
/// and optional `--cavity "X,Y;...|X,Y;..."` (each clockwise). The rectangle's `--width` and
/// `--height` do not combine with a family, and no parameter of another family is accepted.
fn section_shape(family: &str, options: &BTreeMap<&str, &str>) -> Run<SectionShape> {
    let parameters = ["--h", "--b", "--tw", "--tf", "--t", "--r", "--c", "--d"];
    if options.contains_key("--width") || options.contains_key("--height") {
        return Err(invalid(
            "--width and --height describe a rectangle, not a family",
        ));
    }
    if family == "custom" {
        if parameters.iter().any(|p| options.contains_key(p)) {
            return Err(invalid("a custom outline takes --outer and --cavity only"));
        }
        let polygon = |text: &str| -> Run<Vec<[f64; 2]>> {
            text.split(';')
                .map(|pair| {
                    let parts: Vec<f64> = pair
                        .split(',')
                        .map(|v| v.trim().parse::<f64>())
                        .collect::<Result<_, _>>()
                        .map_err(|_| invalid("an outline point is not two numbers"))?;
                    <[f64; 2]>::try_from(parts)
                        .map_err(|_| invalid("an outline point is not two numbers"))
                })
                .collect()
        };
        let outer = polygon(
            options
                .get("--outer")
                .ok_or_else(|| invalid("a custom outline needs --outer"))?,
        )?;
        let cavities = match options.get("--cavity") {
            Some(text) => text.split('|').map(polygon).collect::<Run<Vec<_>>>()?,
            None => Vec::new(),
        };
        return Ok(SectionShape::Custom(CustomSection { outer, cavities }));
    }
    if options.contains_key("--outer") || options.contains_key("--cavity") {
        return Err(invalid("--outer and --cavity belong to a custom outline"));
    }
    let family =
        SectionFamily::from_token(family).ok_or_else(|| invalid("unknown section family"))?;
    let names = family.parameters();
    if parameters
        .iter()
        .any(|p| options.contains_key(p) && !names.iter().any(|n| p.get(2..) == Some(*n)))
    {
        return Err(invalid(
            "a parameter does not belong to this section family",
        ));
    }
    let values = names
        .iter()
        .map(|name| {
            options
                .get(format!("--{name}").as_str())
                .ok_or_else(|| invalid("a required section parameter is missing"))?
                .parse::<f64>()
                .map_err(|_| invalid("a number is malformed"))
        })
        .collect::<Run<Vec<_>>>()?;
    Ok(SectionShape::Parametric(ParametricSection::new(
        family, values,
    )?))
}

/// The plate contour of `--contour "X,Y[,Z];..."`: two or three components, in the order
/// given. Winding, simplicity and the size floor are the kernel's to decide.
fn contour_points(text: &str) -> Run<Vec<[f64; 3]>> {
    if text.split(';').count() > 1024 {
        return Err(PlateError {
            rule_id: None,
            code: ErrorCode::LimitExceeded,
            message: "more than 1024 contour points",
        });
    }
    text.split(';')
        .map(|pair| {
            let bad = || invalid("a contour point is not two or three numbers");
            let mut parts = pair.split(',').map(|v| v.trim().parse::<f64>());
            let mut next = || parts.next().transpose().map_err(|_| bad());
            let (x, y, z) = (next()?, next()?, next()?);
            match (x, y, z, next()?) {
                (Some(x), Some(y), None, None) => Ok([x, y, 0.0]),
                (Some(x), Some(y), Some(z), None) => Ok([x, y, z]),
                _ => Err(bad()),
            }
        })
        .collect()
}

/// The profile definition that explicit `define-profile`-style options describe: a rectangle from
/// `--width` and `--height`, a family from `--family` and its parameters, or a custom outline,
/// with the optional `--designation`, `--standard` and declared section numbers. Shared by
/// `define-profile` and `add-library`; the display name is left to the caller.
pub(crate) fn profile_record(options: &BTreeMap<&str, &str>) -> Run<DefinitionRecord> {
    let given = |key: &str| options.get(key).copied();
    let optional_float = |key: &str| {
        given(key)
            .map(|text| {
                text.parse::<f64>()
                    .map_err(|_| invalid("a number is malformed"))
            })
            .transpose()
    };
    let section = SectionProperties {
        designation: given("--designation").map(str::to_owned),
        standard: given("--standard").map(str::to_owned),
        area_mm2: optional_float("--area-mm2")?,
        mass_per_meter_kg: optional_float("--mass-per-meter-kg")?,
        surface_per_meter_m2: optional_float("--surface-per-meter-m2")?,
        display_name: None,
    };
    Ok(match given("--family") {
        None if SECTION_ONLY_OPTIONS
            .iter()
            .any(|option| options.contains_key(option)) =>
        {
            return Err(invalid("section parameters need --family"));
        }
        None => DefinitionRecord::RectangleProfile {
            width_mm: float(options, "--width")?,
            height_mm: float(options, "--height")?,
            section,
        },
        Some(family) => DefinitionRecord::SectionProfile {
            shape: section_shape(family, options)?,
            section,
        },
    })
}

fn define_command(
    store: &RevisionStore,
    command: &str,
    options: &BTreeMap<&str, &str>,
) -> Run<Json> {
    let mut source = IdSource::system();
    let mut fresh = || {
        source.next_id().map_err(|_| PlateError {
            rule_id: None,
            code: ErrorCode::BackendUnavailable,
            message: "no identity could be generated",
        })
    };
    let given = |key: &str| options.get(key).copied();
    let optional_float = |key: &str| {
        given(key)
            .map(|text| {
                text.parse::<f64>()
                    .map_err(|_| invalid("a number is malformed"))
            })
            .transpose()
    };
    let definition = if command == "define-material" {
        DefinitionRecord::Material {
            density_kg_m3: optional_float("--density")?,
            grade: given("--grade").map(str::to_owned),
            standard: given("--standard").map(str::to_owned),
            display_name: None,
        }
        .named(given("--name"))
    } else if command == "define-fastener" {
        // A14.1: the definition takes every value from one fastener library entry, so it never
        // depends on the file afterwards. One library, no environment yet (O-B2).
        let path =
            given("--fastener-library").ok_or_else(|| invalid("--fastener-library is required"))?;
        let designation =
            given("--designation").ok_or_else(|| invalid("--designation is required"))?;
        let library = beamcraft_cli::library::load_fastener(path)?;
        library
            .entry(designation)?
            .record
            .clone()
            .named(given("--name"))
    } else if ["--library", "--environment", "--user-library"]
        .iter()
        .any(|option| options.contains_key(option))
    {
        // The definition takes every value from the library entry and stores them itself, so it
        // never depends on the file afterwards.
        const OWN_VALUES: &[&str] = &[
            "--width",
            "--height",
            "--family",
            "--standard",
            "--area-mm2",
            "--mass-per-meter-kg",
            "--surface-per-meter-m2",
        ];
        if OWN_VALUES
            .iter()
            .chain(SECTION_ONLY_OPTIONS)
            .any(|option| options.contains_key(option))
        {
            return Err(invalid(
                "--library and --environment take their values from the entry; only --designation, --user-library and --name combine with them",
            ));
        }
        let designation = given("--designation")
            .ok_or_else(|| invalid("a library or environment needs --designation"))?;
        let (lookup, _) = open_lookup(
            given("--library"),
            given("--environment"),
            given("--user-library"),
        )?;
        lookup
            .find(designation)?
            .1
            .record
            .clone()
            .named(given("--name"))
    } else {
        profile_record(options)?.named(given("--name"))
    };
    let revision_id = match given("--revision-id") {
        Some(text) => RevisionId::parse(text).map_err(id_error)?,
        None => RevisionId::from_uuid(fresh()?),
    };
    let actor = match given("--actor-id") {
        Some(text) => Uuid7::parse(text).map_err(PlateError::from)?,
        None => Uuid7::parse(&fresh()?.to_string()).map_err(PlateError::from)?,
    };
    let project = match given("--project-id") {
        Some(text) => ProjectId::parse(text).map_err(id_error)?,
        None => ProjectId::from_uuid(fresh()?),
    };
    let workspace = match given("--workspace-id") {
        Some(text) => WorkspaceId::parse(text).map_err(id_error)?,
        None => WorkspaceId::from_uuid(fresh()?),
    };
    let defined = define(
        store,
        &DefineRequest {
            revision_id,
            op_id: OpId::from_uuid(fresh()?),
            actor,
            project,
            workspace,
            definition,
        },
        &mut NoFault,
    )?;
    defined_json(&defined)
}

fn create(
    store: &RevisionStore,
    options: &BTreeMap<&str, &str>,
    candidates: &mut Vec<Json>,
) -> Run<Json> {
    let mut source = IdSource::system();
    let mut fresh = || {
        source.next_id().map_err(|_| PlateError {
            rule_id: None,
            code: ErrorCode::BackendUnavailable,
            message: "no identity could be generated",
        })
    };
    let given = |key: &str| options.get(key).copied();
    let plane = command_plane(options)?;
    let new_id = match given("--new-id") {
        Some(text) => ElementId::parse(text).map_err(id_error)?,
        None => ElementId::from_uuid(fresh()?),
    };
    let op_id = match given("--op-id") {
        Some(text) => OpId::parse(text).map_err(id_error)?,
        None => OpId::from_uuid(fresh()?),
    };
    let revision_id = match given("--revision-id") {
        Some(text) => RevisionId::parse(text).map_err(id_error)?,
        None => RevisionId::from_uuid(fresh()?),
    };
    let actor = match given("--actor-id") {
        Some(text) => Uuid7::parse(text).map_err(PlateError::from)?,
        None => Uuid7::parse(&fresh()?.to_string()).map_err(PlateError::from)?,
    };
    let project = match given("--project-id") {
        Some(text) => ProjectId::parse(text).map_err(id_error)?,
        None => ProjectId::from_uuid(fresh()?),
    };
    let workspace = match given("--workspace-id") {
        Some(text) => WorkspaceId::parse(text).map_err(id_error)?,
        None => WorkspaceId::from_uuid(fresh()?),
    };
    let replay = if options.contains_key("--op-id") && options.contains_key("--material") {
        accepted_binding(store, &op_id, &new_id)?
    } else {
        None
    };
    let thickness = float(options, "--thickness")?;
    let thickness_side = thickness_side(options)?;
    let contour = match given("--contour") {
        Some(text) => {
            if options.contains_key("--width") || options.contains_key("--length") {
                return Err(invalid(
                    "--contour replaces --width and --length; give one or the other",
                ));
            }
            contour_points(text)?
        }
        None => origin_rectangle_contour(float(options, "--width")?, float(options, "--length")?)
            .map_err(PlateError::from)?,
    };
    let name = given("--name").unwrap_or("Plate").to_owned();
    let command = CreatePlate {
        class: given("--class")
            .map(|v| {
                v.parse::<i32>()
                    .map_err(|_| invalid("class must be an int32"))
            })
            .transpose()?
            .unwrap_or(0),
        new_id,
        name,
        contour_points_mm: contour,
        thickness_mm: thickness,
        thickness_side,
        placement: if let Some(plane) = plane {
            plane.placement()?
        } else {
            Placement {
                translation_mm: if options.contains_key("--origin-at-grid") {
                    grid_placement(store, options, &op_id, "--origin-at-grid")?
                } else {
                    if ["--z", "--level", "--offset"]
                        .iter()
                        .any(|k| options.contains_key(k))
                    {
                        return Err(invalid("height options require grid placement"));
                    }
                    given("--translation").map_or(Ok([0.0; 3]), translation)?
                },
                orientation_xyzw: given("--orientation-xyzw")
                    .map_or(Ok([0.0, 0.0, 0.0, 1.0]), quaternion)?,
            }
        },
        material_digest: selected(
            store,
            options,
            ("--material-digest", "--material"),
            false,
            replay.as_ref(),
            candidates,
        )?,
        holes_mm: holes(options)?,
    };
    let created = create_plate_draft(
        store,
        &CreateRequest {
            op_id,
            revision_id,
            actor,
            project,
            workspace,
            command,
        },
        &mut NoFault,
    )?;
    match created {
        CreatedDraft::Evaluated(created) => created_json(&created, op_id),
        CreatedDraft::Unsupported(created) => Ok(Json::object([
            (
                "outcome",
                Json::text(if created.replayed {
                    "plate_replayed"
                } else {
                    "plate_created"
                }),
            ),
            ("op_id", Json::text(op_id.to_string())),
            ("plate", unsupported_plate_json(&created.plate)?),
            ("revision", revision_json(&created.revision)),
            ("footprint", footprint_json(created.footprint.as_ref())),
        ])),
    }
}

/// The through holes of `create-plate`: `--hole-rect "X,Y,W,H;..."` (axis-aligned rectangles from
/// the corner `X,Y`) and `--hole-round "X,Y,D;..."` (a regular polygon of `--hole-segments`
/// sides, default 32, inscribed in the circle of centre `X,Y` and diameter `D`). Both are only
/// generators: the model stores the clockwise polygons, so a "round" hole is that polygon and its
/// volume is the polygon's, exactly. The polygon vertices use the platform sin and cos here, at
/// the input edge, and are authored as the doubles they round to.
fn holes(options: &BTreeMap<&str, &str>) -> Run<Vec<Vec<[f64; 2]>>> {
    let given = |key: &str| options.get(key).copied();
    let segments = match given("--hole-segments") {
        Some(text) => text
            .parse::<usize>()
            .ok()
            .filter(|n| (8..=256).contains(n))
            .ok_or_else(|| invalid("--hole-segments is 8 to 256"))?,
        None => 32,
    };
    // Non-finite values stay the kernel's `NON_FINITE_NUMBER`; only finite non-positive sizes are
    // refused here, because a reversed polygon would otherwise pass as a valid clockwise hole.
    let size_ok = |sizes: &[f64]| sizes.iter().all(|v| !v.is_finite() || *v > 0.0);
    let mut result = Vec::new();
    if let Some(list) = given("--hole-rect") {
        for item in list.split(';').take(MAX_HOLES.saturating_add(1)) {
            if let [x, y, w, h] = numbers_list(item, 4)?[..] {
                if !size_ok(&[w, h]) {
                    return Err(invalid("a hole width and height must be positive"));
                }
                result.push(vec![[x, y], [x, y + h], [x + w, y + h], [x + w, y]]);
            }
        }
    }
    if let Some(list) = given("--hole-round") {
        for item in list.split(';').take(MAX_HOLES.saturating_add(1)) {
            if let [x, y, d] = numbers_list(item, 3)?[..] {
                if !size_ok(&[d]) {
                    return Err(invalid("a hole diameter must be positive"));
                }
                let r = d / 2.0;
                result.push(
                    (0..segments)
                        .map(|k| {
                            let theta = -std::f64::consts::TAU * (k as f64) / (segments as f64);
                            [x + r * theta.cos(), y + r * theta.sin()]
                        })
                        .collect(),
                );
            }
        }
    }
    Ok(result)
}

/// The `--thickness-side` option (A12.1): `top`, `middle` or `bottom`; absent means the
/// engine's `top` default.
fn thickness_side(options: &BTreeMap<&str, &str>) -> Run<Option<PlateSide>> {
    options
        .get("--thickness-side")
        .map(|token| {
            PlateSide::parse(token)
                .ok_or_else(|| invalid("--thickness-side is top, middle or bottom"))
        })
        .transpose()
}

/// The cutting plane of `cut-by-plane`: `--point X,Y,Z --normal X,Y,Z --keep positive|negative`,
/// element-local millimetres; whether the plane crosses the element is the kernel's decision, and
/// a scaled normal describes the same plane.
fn keep_side(options: &BTreeMap<&str, &str>) -> Run<KeepSide> {
    options
        .get("--keep")
        .and_then(|token| KeepSide::parse(token))
        .ok_or_else(|| invalid("--keep is positive or negative"))
}

fn update_command(store: &RevisionStore, options: &BTreeMap<&str, &str>) -> Run<Json> {
    let mut source = IdSource::system();
    let mut fresh = || {
        source.next_id().map_err(|_| PlateError {
            rule_id: None,
            code: ErrorCode::BackendUnavailable,
            message: "no identity could be generated",
        })
    };
    let given = |key: &str| options.get(key).copied();
    let target_id = ElementId::parse(given("--id").ok_or_else(|| invalid("--id is required"))?)
        .map_err(id_error)?;
    let op_id = match given("--op-id") {
        Some(text) => OpId::parse(text).map_err(id_error)?,
        None => OpId::from_uuid(fresh()?),
    };
    let revision_id = match given("--revision-id") {
        Some(text) => RevisionId::parse(text).map_err(id_error)?,
        None => RevisionId::from_uuid(fresh()?),
    };
    let actor = match given("--actor-id") {
        Some(text) => Uuid7::parse(text).map_err(PlateError::from)?,
        None => Uuid7::parse(&fresh()?.to_string()).map_err(PlateError::from)?,
    };
    let project = match given("--project-id") {
        Some(text) => ProjectId::parse(text).map_err(id_error)?,
        None => ProjectId::from_uuid(fresh()?),
    };
    let workspace = match given("--workspace-id") {
        Some(text) => WorkspaceId::parse(text).map_err(id_error)?,
        None => WorkspaceId::from_uuid(fresh()?),
    };
    let patch = ElementPatch {
        class: given("--class")
            .map(|v| {
                v.parse::<i32>()
                    .map_err(|_| invalid("class must be an int32"))
            })
            .transpose()?,
        name: given("--name").map(str::to_owned),
        material: match given("--material-digest") {
            None => None,
            Some("none") => Some(None),
            Some(text) => Some(Some(digest_option(text)?)),
        },
        thickness_mm: given("--thickness")
            .map(|text| {
                text.parse::<f64>()
                    .map_err(|_| invalid("a number is malformed"))
            })
            .transpose()?,
        thickness_side: thickness_side(options)?,
        translation_mm: given("--translation").map(translation).transpose()?,
        orientation_xyzw: match given("--orientation-xyzw") {
            None => None,
            Some("none") => Some([0.0, 0.0, 0.0, 1.0]),
            Some(text) => Some(quaternion(text)?),
        },
        level: given("--level")
            .map(|text| match text {
                "none" => Ok(None),
                other => ElementId::parse(other).map(Some).map_err(id_error),
            })
            .transpose()?,
        axis_points_mm: given("--axis-point").map(path_points).transpose()?,
    };
    let updated = update_element(
        store,
        &UpdateRequest {
            op_id,
            revision_id,
            actor,
            project,
            workspace,
            command: UpdateElement { target_id, patch },
        },
        &mut NoFault,
    )?;
    let element = match &updated.element {
        UpdatedElement::Plate(record) => plate_json(record)?,
        UpdatedElement::Member(record) => member_json(record)?,
    };
    Ok(Json::object([
        (
            "outcome",
            Json::text(if updated.replayed {
                "element_update_replayed"
            } else {
                "element_updated"
            }),
        ),
        ("op_id", Json::text(op_id.to_string())),
        (
            "effect",
            Json::object([
                ("name", Json::Bool(updated.effect.name)),
                ("class", Json::Bool(updated.effect.class)),
                ("material", Json::Bool(updated.effect.material)),
                ("shape", Json::Bool(updated.effect.shape)),
                ("placement", Json::Bool(updated.effect.placement)),
                ("orientation", Json::Bool(updated.effect.orientation)),
                ("level", Json::Bool(updated.effect.level)),
            ]),
        ),
        (
            "changed_layers",
            Json::Array(updated.changed_layers.iter().map(Json::text).collect()),
        ),
        ("element", element),
        ("revision", revision_json(&updated.revision)),
        ("footprint", footprint_json(updated.footprint.as_ref())),
    ]))
}

/// The engine's `CutByPlane` (A03, amendment A-2) through the real adapter: appends one
/// `plane_cut` record, rebuilds the mesh and the quantities, and publishes the revision.
fn cut_command(store: &RevisionStore, options: &BTreeMap<&str, &str>) -> Run<Json> {
    let mut source = IdSource::system();
    let mut fresh = || {
        source.next_id().map_err(|_| PlateError {
            rule_id: None,
            code: ErrorCode::BackendUnavailable,
            message: "no identity could be generated",
        })
    };
    let given = |key: &str| options.get(key).copied();
    let target_id = ElementId::parse(given("--id").ok_or_else(|| invalid("--id is required"))?)
        .map_err(id_error)?;
    let operation_id =
        OpId::parse(given("--operation-id").ok_or_else(|| invalid("--operation-id is required"))?)
            .map_err(id_error)?;
    let op_id = match given("--op-id") {
        Some(text) => OpId::parse(text).map_err(id_error)?,
        None => OpId::from_uuid(fresh()?),
    };
    let revision_id = match given("--revision-id") {
        Some(text) => RevisionId::parse(text).map_err(id_error)?,
        None => RevisionId::from_uuid(fresh()?),
    };
    let actor = match given("--actor-id") {
        Some(text) => Uuid7::parse(text).map_err(PlateError::from)?,
        None => Uuid7::parse(&fresh()?.to_string()).map_err(PlateError::from)?,
    };
    let project = match given("--project-id") {
        Some(text) => ProjectId::parse(text).map_err(id_error)?,
        None => ProjectId::from_uuid(fresh()?),
    };
    let workspace = match given("--workspace-id") {
        Some(text) => WorkspaceId::parse(text).map_err(id_error)?,
        None => WorkspaceId::from_uuid(fresh()?),
    };
    let command = CutByPlane {
        target_id,
        operation_id,
        point_mm: translation(given("--point").ok_or_else(|| invalid("--point is required"))?)?,
        normal: translation(given("--normal").ok_or_else(|| invalid("--normal is required"))?)?,
        keep: keep_side(options)?,
    };
    let cut = cut_by_plane_record(
        store,
        &CutRequest {
            op_id,
            revision_id,
            actor,
            project,
            workspace,
            command,
        },
        &mut NoFault,
    )?;
    let element = match &cut.element {
        CutElement::Plate(record) => plate_json(record)?,
        CutElement::Member(record) => member_json(record)?,
    };
    Ok(Json::object([
        (
            "outcome",
            Json::text(if cut.replayed {
                "cut_replayed"
            } else {
                "cut_applied"
            }),
        ),
        ("op_id", Json::text(op_id.to_string())),
        (
            "record",
            Json::object([
                ("operation_id", Json::text(operation_id.to_string())),
                ("ordinal", Json::uint(u64::from(cut.ordinal))),
            ]),
        ),
        ("element", element),
        ("revision", revision_json(&cut.revision)),
        ("footprint", footprint_json(cut.footprint.as_ref())),
    ]))
}

/// Comma-separated numbers, exactly `count` of them (the shared shape of every hole option).
fn numbers_list(text: &str, count: usize) -> Run<Vec<f64>> {
    let values: Vec<f64> = text
        .split(',')
        .map(|p| {
            p.trim()
                .parse::<f64>()
                .map_err(|_| invalid("a hole value is not a number"))
        })
        .collect::<Run<_>>()?;
    if values.len() == count {
        Ok(values)
    } else {
        Err(invalid("a hole has the wrong number of values"))
    }
}

/// The one hole of `add-hole` (A04): `--polygon "X,Y;..."` (the clockwise polygon itself) or a
/// generator, `--hole-rect "X,Y,W,H"` or `--hole-round "X,Y,D"`, in plate-local millimetres.
/// Exactly one is accepted, and only the shape is parsed here: finiteness, positivity and the
/// caps are the model's admission (`hole` field), and a round hole is the regular inscribed
/// `N`-gon of F06 section 2 (A-2), whose `N` follows the contract's own tolerance, so
/// `--hole-segments` does not apply.
fn hole_spec(options: &BTreeMap<&str, &str>) -> Run<HoleSpec> {
    let given = |key: &str| options.get(key).copied();
    let given_count = ["--polygon", "--hole-rect", "--hole-round"]
        .iter()
        .filter(|key| given(key).is_some())
        .count();
    if given_count != 1 {
        return Err(invalid(
            "exactly one of --polygon, --hole-rect or --hole-round is required",
        ));
    }
    if let Some(text) = given("--polygon") {
        let points = text
            .split(';')
            .map(|item| match numbers_list(item, 2)?[..] {
                [x, y] => Ok([x, y]),
                _ => Err(invalid("a hole has the wrong number of values")),
            })
            .collect::<Run<Vec<_>>>()?;
        return Ok(HoleSpec::Polygon(points));
    }
    if let Some(text) = given("--hole-rect") {
        return match numbers_list(text, 4)?[..] {
            [x, y, w, h] => Ok(HoleSpec::Rectangle {
                corner_mm: [x, y],
                width_mm: w,
                height_mm: h,
            }),
            _ => Err(invalid("a hole has the wrong number of values")),
        };
    }
    let Some(text) = given("--hole-round") else {
        return Err(invalid(
            "exactly one of --polygon, --hole-rect or --hole-round is required",
        ));
    };
    match numbers_list(text, 3)?[..] {
        [x, y, d] => Ok(HoleSpec::Round {
            centre_mm: [x, y],
            diameter_mm: d,
        }),
        _ => Err(invalid("a hole has the wrong number of values")),
    }
}

/// The `add-hole` command (A04): one through-hole operation record on a plate, committed as
/// the next revision. The `--operation-id` is the record's identity and the logical operation
/// of the command; absent, a fresh id is generated.
fn add_hole_cli(store: &RevisionStore, options: &BTreeMap<&str, &str>) -> Run<Json> {
    let mut source = IdSource::system();
    let mut fresh = || {
        source.next_id().map_err(|_| PlateError {
            rule_id: None,
            code: ErrorCode::BackendUnavailable,
            message: "no identity could be generated",
        })
    };
    let given = |key: &str| options.get(key).copied();
    let target_id = ElementId::parse(given("--id").ok_or_else(|| invalid("--id is required"))?)
        .map_err(id_error)?;
    let operation_id = match given("--operation-id") {
        Some(text) => OpId::parse(text).map_err(id_error)?,
        None => OpId::from_uuid(fresh()?),
    };
    let revision_id = match given("--revision-id") {
        Some(text) => RevisionId::parse(text).map_err(id_error)?,
        None => RevisionId::from_uuid(fresh()?),
    };
    let actor = match given("--actor-id") {
        Some(text) => Uuid7::parse(text).map_err(PlateError::from)?,
        None => Uuid7::parse(&fresh()?.to_string()).map_err(PlateError::from)?,
    };
    let project = match given("--project-id") {
        Some(text) => ProjectId::parse(text).map_err(id_error)?,
        None => ProjectId::from_uuid(fresh()?),
    };
    let workspace = match given("--workspace-id") {
        Some(text) => WorkspaceId::parse(text).map_err(id_error)?,
        None => WorkspaceId::from_uuid(fresh()?),
    };
    let added = add_hole_command(
        store,
        &AddHoleRequest {
            op_id: operation_id,
            revision_id,
            actor,
            project,
            workspace,
            command: AddHole {
                target_id,
                operation_id,
                hole: hole_spec(options)?,
            },
        },
        &mut NoFault,
    )?;
    let element = plate_json(&added.plate)?;
    Ok(Json::object([
        (
            "outcome",
            Json::text(if added.replayed {
                "hole_add_replayed"
            } else {
                "hole_added"
            }),
        ),
        ("op_id", Json::text(operation_id.to_string())),
        (
            "changed_layers",
            Json::Array(added.changed_layers.iter().map(Json::text).collect()),
        ),
        ("element", element),
        ("revision", revision_json(&added.revision)),
        ("footprint", footprint_json(added.footprint.as_ref())),
    ]))
}

fn delete_command(store: &RevisionStore, options: &BTreeMap<&str, &str>) -> Run<Json> {
    let mut source = IdSource::system();
    let mut fresh = || {
        source.next_id().map_err(|_| PlateError {
            rule_id: None,
            code: ErrorCode::BackendUnavailable,
            message: "no identity could be generated",
        })
    };
    let given = |key: &str| options.get(key).copied();
    let target_id = ElementId::parse(given("--id").ok_or_else(|| invalid("--id is required"))?)
        .map_err(id_error)?;
    let op_id = match given("--op-id") {
        Some(text) => OpId::parse(text).map_err(id_error)?,
        None => OpId::from_uuid(fresh()?),
    };
    let revision_id = match given("--revision-id") {
        Some(text) => RevisionId::parse(text).map_err(id_error)?,
        None => RevisionId::from_uuid(fresh()?),
    };
    let actor = match given("--actor-id") {
        Some(text) => Uuid7::parse(text).map_err(PlateError::from)?,
        None => Uuid7::parse(&fresh()?.to_string()).map_err(PlateError::from)?,
    };
    let project = match given("--project-id") {
        Some(text) => ProjectId::parse(text).map_err(id_error)?,
        None => ProjectId::from_uuid(fresh()?),
    };
    let workspace = match given("--workspace-id") {
        Some(text) => WorkspaceId::parse(text).map_err(id_error)?,
        None => WorkspaceId::from_uuid(fresh()?),
    };
    let deleted = delete_element(
        store,
        &DeleteRequest {
            op_id,
            revision_id,
            actor,
            project,
            workspace,
            command: DeleteElement { target_id },
        },
        &mut NoFault,
    )?;
    Ok(Json::object([
        (
            "outcome",
            Json::text(if deleted.replayed {
                "element_delete_replayed"
            } else {
                "element_deleted"
            }),
        ),
        ("op_id", Json::text(op_id.to_string())),
        ("element_id", Json::text(target_id.to_string())),
        (
            "kind",
            deleted.kind.map_or(Json::Null, |k| {
                Json::text(match k {
                    DeletedKind::Plate => "plate",
                    DeletedKind::Member => "member",
                    DeletedKind::Assembly => "assembly",
                    DeletedKind::Weld => "weld",
                    DeletedKind::Bolt => "bolt",
                    DeletedKind::Level => "level",
                    DeletedKind::Grid => "grid",
                })
            }),
        ),
        (
            "changed_layers",
            Json::Array(deleted.changed_layers.iter().map(Json::text).collect()),
        ),
        ("revision", revision_json(&deleted.revision)),
        ("footprint", footprint_json(deleted.footprint.as_ref())),
    ]))
}

fn copy_command(
    store: &RevisionStore,
    options: &BTreeMap<&str, &str>,
    instance: bool,
) -> Run<Json> {
    let source_id = ElementId::parse(
        options
            .get("--id")
            .ok_or_else(|| invalid("--id is required"))?,
    )
    .map_err(id_error)?;
    if beamcraft_cli::grid::grids(store)?
        .iter()
        .any(|g| g.id == source_id)
        || options.contains_key("--new-axis-ids")
    {
        if instance {
            return Err(invalid("a grid cannot be a part instance"));
        }
        return crate::grid_cmd::copy_command(store, options);
    }

    let mut source = IdSource::system();
    let mut fresh = || {
        source.next_id().map_err(|_| PlateError {
            rule_id: None,
            code: ErrorCode::BackendUnavailable,
            message: "no identity could be generated",
        })
    };
    let given = |key: &str| options.get(key).copied();
    let source_id = ElementId::parse(given("--id").ok_or_else(|| invalid("--id is required"))?)
        .map_err(id_error)?;
    let new_id = match given("--new-id") {
        Some(text) => ElementId::parse(text).map_err(id_error)?,
        None => ElementId::from_uuid(fresh()?),
    };
    let op_id = match given("--op-id") {
        Some(text) => OpId::parse(text).map_err(id_error)?,
        None => OpId::from_uuid(fresh()?),
    };
    let revision_id = match given("--revision-id") {
        Some(text) => RevisionId::parse(text).map_err(id_error)?,
        None => RevisionId::from_uuid(fresh()?),
    };
    let actor = match given("--actor-id") {
        Some(text) => Uuid7::parse(text).map_err(PlateError::from)?,
        None => Uuid7::parse(&fresh()?.to_string()).map_err(PlateError::from)?,
    };
    let project = match given("--project-id") {
        Some(text) => ProjectId::parse(text).map_err(id_error)?,
        None => ProjectId::from_uuid(fresh()?),
    };
    let workspace = match given("--workspace-id") {
        Some(text) => WorkspaceId::parse(text).map_err(id_error)?,
        None => WorkspaceId::from_uuid(fresh()?),
    };
    let copy = if instance {
        copy_instance
    } else {
        copy_element
    };
    let copied = copy(
        store,
        &CopyRequest {
            op_id,
            revision_id,
            actor,
            project,
            workspace,
            command: CopyElement {
                source_id,
                new_id,
                name: given("--name").map(str::to_owned),
                offset_mm: given("--offset").map_or(Ok([0.0; 3]), translation)?,
            },
        },
        &mut NoFault,
    )?;
    let element = match &copied.element {
        CopiedElement::Plate(record) => plate_json(record)?,
        CopiedElement::Member(record) => member_json(record)?,
    };
    Ok(Json::object([
        (
            "outcome",
            Json::text(if copied.replayed {
                "element_copy_replayed"
            } else {
                "element_copied"
            }),
        ),
        ("op_id", Json::text(op_id.to_string())),
        ("source_id", Json::text(source_id.to_string())),
        (
            "changed_layers",
            Json::Array(copied.changed_layers.iter().map(Json::text).collect()),
        ),
        ("element", element),
        ("revision", revision_json(&copied.revision)),
        ("footprint", footprint_json(copied.footprint.as_ref())),
    ]))
}

fn workspace_json(status: &WorkspaceStatus) -> Json {
    Json::object([
        (
            "workspace_id",
            Json::text(status.meta.workspace_id.as_str()),
        ),
        ("user_id", Json::text(status.meta.user_id.as_str())),
        (
            "base_revision",
            Json::text(status.meta.base_revision.as_str()),
        ),
        ("base_is_current", Json::Bool(status.base_is_current)),
        (
            "rebase_sequence",
            Json::text(status.meta.rebase_sequence.to_string()),
        ),
        (
            "unsent_revisions",
            Json::text(status.unsent_revisions.to_string()),
        ),
        ("draft_head", revision_json(&status.draft_head)),
        ("published_head", revision_json(&status.published_head)),
        (
            "pending_conflicts",
            Json::Array(
                status
                    .pending_conflicts
                    .iter()
                    .map(beamcraft_cli::workspace::conflicts::conflict_json)
                    .collect(),
            ),
        ),
    ])
}

fn history_command(command: &str, options: &BTreeMap<&str, &str>) -> Run<Json> {
    let dir = std::path::Path::new(
        options
            .get("--workspace-dir")
            .ok_or_else(|| invalid("--workspace-dir is required"))?,
    );
    let blobs = BlobStore::new(dir).map_err(|e| PlateError {
        rule_id: None,
        code: e.code(),
        message: "workspace directory is not usable",
    })?;
    let workspace = RevisionStore::new(blobs);
    let units = |sequences: &[u64]| {
        Json::Array(
            sequences
                .iter()
                .map(|s| Json::text(s.to_string()))
                .collect(),
        )
    };
    if command == "inspect-workspace-history" {
        let line = timeline(&workspace, dir)?;
        return Ok(Json::object([
            ("outcome", Json::text("workspace_history")),
            ("draft_head", revision_json(&line.head_summary())),
            ("undo_units", Json::text(line.undo.len().to_string())),
            ("redo_units", Json::text(line.redo.len().to_string())),
            ("undo_restores_sequences", units(&line.undo)),
            ("redo_restores_sequences", units(&line.redo)),
        ]));
    }
    let mut source = IdSource::system();
    let mut uuid = |key: &str| -> Run<Uuid7> {
        match options.get(key) {
            Some(text) => Uuid7::parse(text).map_err(PlateError::from),
            None => source
                .next_id()
                .map_err(|_| PlateError {
                    rule_id: None,
                    code: ErrorCode::BackendUnavailable,
                    message: "no identity could be generated",
                })
                .and_then(|id| Uuid7::parse(&id.to_string()).map_err(PlateError::from)),
        }
    };
    let (direction, done, replayed) = if command == "undo-workspace" {
        (
            Direction::Undo,
            "workspace_undone",
            "workspace_undo_replayed",
        )
    } else {
        (
            Direction::Redo,
            "workspace_redone",
            "workspace_redo_replayed",
        )
    };
    let stepped = step(
        &workspace,
        dir,
        direction,
        &Step {
            revision_id: uuid("--revision-id")?,
            op_id: uuid("--op-id")?,
        },
        &mut NoFault,
    )?;
    Ok(Json::object([
        (
            "outcome",
            Json::text(if stepped.replayed { replayed } else { done }),
        ),
        ("revision", revision_json(&stepped.revision)),
        (
            "restored_sequence",
            stepped
                .restored_sequence
                .map_or(Json::Null, |s| Json::text(s.to_string())),
        ),
        ("undo_units", Json::text(stepped.undo_units.to_string())),
        ("redo_units", Json::text(stepped.redo_units.to_string())),
    ]))
}

fn workspace_command(
    store: &RevisionStore,
    command: &str,
    options: &BTreeMap<&str, &str>,
    candidates: &mut Vec<Json>,
) -> Run<Json> {
    let mut source = IdSource::system();
    let mut fresh = || -> Run<String> {
        source
            .next_id()
            .map(|id| id.to_string())
            .map_err(|_| PlateError {
                rule_id: None,
                code: ErrorCode::BackendUnavailable,
                message: "no identity could be generated",
            })
    };
    let given = |key: &str| options.get(key).copied();
    let dir = std::path::Path::new(
        given("--workspace-dir").ok_or_else(|| invalid("--workspace-dir is required"))?,
    );
    if command == "publish-workspace" && given("--revision-id").is_some() {
        return Err(invalid(
            "publish-workspace takes its revision id from the draft head",
        ));
    }
    if command == "create-workspace" && !dir.exists() {
        std::fs::create_dir(dir).map_err(|_| PlateError {
            rule_id: None,
            code: ErrorCode::BackendUnavailable,
            message: "workspace directory could not be created",
        })?;
    }
    // Compared after resolving aliases such as `PUBLISHED/.` or a link to the same directory.
    let same_store = |a: &std::path::Path, b: &std::path::Path| matches!((std::fs::canonicalize(a), std::fs::canonicalize(b)), (Ok(a), Ok(b)) if a == b);
    let published_root = options.get("--store").copied().unwrap_or_default();
    if same_store(dir, std::path::Path::new(published_root)) {
        return Err(invalid("the workspace directory is the published store"));
    }
    let blobs = BlobStore::new(dir).map_err(|e| PlateError {
        rule_id: None,
        code: e.code(),
        message: "workspace directory is not usable",
    })?;
    let workspace = RevisionStore::new(blobs);
    let uuid = |key: &str, fresh: &mut dyn FnMut() -> Run<String>| -> Run<Uuid7> {
        match given(key) {
            Some(text) => Uuid7::parse(text).map_err(PlateError::from),
            None => Uuid7::parse(&fresh()?).map_err(PlateError::from),
        }
    };
    match command {
        "create-workspace" => {
            let created = create_workspace(
                store,
                &workspace,
                dir,
                &CreateWorkspace {
                    workspace_id: uuid("--workspace-id", &mut fresh)?,
                    user_id: uuid("--user-id", &mut fresh)?,
                    revision_id: uuid("--revision-id", &mut fresh)?,
                    op_id: uuid("--op-id", &mut fresh)?,
                },
                &mut NoFault,
            )?;
            Ok(Json::object([
                ("outcome", Json::text("workspace_created")),
                ("workspace", workspace_json(&created)),
            ]))
        }
        "inspect-workspace" => Ok(Json::object([
            ("outcome", Json::text("workspace_status")),
            (
                "workspace",
                workspace_json(&workspace_status(store, &workspace, dir)?),
            ),
        ])),
        _ => {
            let published = match publish_workspace(store, &workspace, dir, &mut NoFault) {
                Ok(published) => published,
                Err(error) if error.code == ErrorCode::DependencyConflict => {
                    // C04: the refusal names every pending conflict in its diagnostics.
                    candidates.extend(
                        beamcraft_cli::workspace::pending_conflicts(&workspace, dir)?
                            .iter()
                            .map(beamcraft_cli::workspace::conflicts::conflict_json),
                    );
                    return Err(error);
                }
                Err(error) => return Err(error),
            };
            Ok(Json::object([
                (
                    "outcome",
                    Json::text(if published.replayed {
                        "workspace_publish_replayed"
                    } else {
                        "workspace_published"
                    }),
                ),
                ("revision", revision_json(&published.revision)),
                ("workspace", workspace_json(&published.status)),
            ]))
        }
    }
}

fn create_member_command(
    store: &RevisionStore,
    options: &BTreeMap<&str, &str>,
    candidates: &mut Vec<Json>,
) -> Run<Json> {
    let mut source = IdSource::system();
    let mut fresh = || {
        source.next_id().map_err(|_| PlateError {
            rule_id: None,
            code: ErrorCode::BackendUnavailable,
            message: "no identity could be generated",
        })
    };
    let given = |key: &str| options.get(key).copied();
    let plane = command_plane(options)?;
    let new_id = match given("--new-id") {
        Some(text) => ElementId::parse(text).map_err(id_error)?,
        None => ElementId::from_uuid(fresh()?),
    };
    let op_id = match given("--op-id") {
        Some(text) => OpId::parse(text).map_err(id_error)?,
        None => OpId::from_uuid(fresh()?),
    };
    let revision_id = match given("--revision-id") {
        Some(text) => RevisionId::parse(text).map_err(id_error)?,
        None => RevisionId::from_uuid(fresh()?),
    };
    let actor = match given("--actor-id") {
        Some(text) => Uuid7::parse(text).map_err(PlateError::from)?,
        None => Uuid7::parse(&fresh()?.to_string()).map_err(PlateError::from)?,
    };
    let project = match given("--project-id") {
        Some(text) => ProjectId::parse(text).map_err(id_error)?,
        None => ProjectId::from_uuid(fresh()?),
    };
    let workspace = match given("--workspace-id") {
        Some(text) => WorkspaceId::parse(text).map_err(id_error)?,
        None => WorkspaceId::from_uuid(fresh()?),
    };
    let kind = match given("--kind") {
        Some("beam") | None => MemberKind::Beam,
        Some("column") => MemberKind::Column,
        Some(_) => return Err(invalid("--kind is beam or column")),
    };
    use beamcraft_engine::coordinate::AxisLock;
    if !options.contains_key("--start-at-grid")
        && !options.contains_key("--end-at-grid")
        && ["--z", "--level", "--offset"]
            .iter()
            .any(|k| options.contains_key(k))
    {
        return Err(invalid("height options require grid placement"));
    }
    let coordinate = |key: &'static str, grid, origin, lock: AxisLock| -> Run<[f64; 3]> {
        if options.contains_key(grid) {
            if options.contains_key(key) {
                return Err(invalid("point and grid address are mutually exclusive"));
            }
            return lock
                .constrain(origin, grid_placement(store, options, &op_id, grid)?)
                .map_err(PlateError::from);
        }
        let text = options
            .get(key)
            .ok_or_else(|| invalid("--start and --end are required"))?;
        plane
            .map_or_else(
                || beamcraft_engine::coordinate::resolve(text, origin, lock),
                |p| p.resolve_coordinate(text, origin, lock),
            )
            .map_err(PlateError::from)
    };
    let lock = AxisLock::parse(given("--axis-lock").unwrap_or("none")).map_err(PlateError::from)?;
    let start = coordinate("--start", "--start-at-grid", [0.0; 3], AxisLock::None)?;
    let end = coordinate("--end", "--end-at-grid", start, lock)?;
    let replay = if options.contains_key("--op-id")
        && (options.contains_key("--profile") || options.contains_key("--material"))
    {
        accepted_binding(store, &op_id, &new_id)?
    } else {
        None
    };
    let profile_digest = selected(
        store,
        options,
        ("--profile-digest", "--profile"),
        true,
        replay.as_ref(),
        candidates,
    )?
    .ok_or_else(|| invalid("--profile-digest or --profile is required"))?;
    let material_digest = selected(
        store,
        options,
        ("--material-digest", "--material"),
        false,
        replay.as_ref(),
        candidates,
    )?;
    let command = CreateStraightMember {
        new_id,
        kind,
        name: given("--name").unwrap_or(kind.as_str()).to_owned(),
        class: given("--class")
            .map(|v| {
                v.parse::<i32>()
                    .map_err(|_| invalid("class must be an int32"))
            })
            .transpose()?
            .unwrap_or(0),
        axis_points_mm: match plane {
            Some(p) => [p.to_world(start)?, p.to_world(end)?],
            None => [start, end],
        },
        profile_digest,
        placement: Placement {
            translation_mm: given("--translation").map_or(Ok([0.0; 3]), translation)?,
            orientation_xyzw: given("--orientation-xyzw")
                .map_or(Ok([0.0, 0.0, 0.0, 1.0]), quaternion)?,
        },
        roll_rad: given("--roll")
            .map(|text| {
                text.parse::<f64>()
                    .map_err(|_| invalid("a number is malformed"))
            })
            .transpose()?,
        material_digest,
    };
    let created = create_member(
        store,
        &CreateMemberRequest {
            op_id,
            revision_id,
            actor,
            project,
            workspace,
            command,
        },
        &mut NoFault,
    )?;
    Ok(Json::object([
        (
            "outcome",
            Json::text(if created.replayed {
                "member_replayed"
            } else {
                "member_created"
            }),
        ),
        ("op_id", Json::text(op_id.to_string())),
        ("member", member_json(&created.member)?),
        ("revision", revision_json(&created.revision)),
        ("footprint", footprint_json(created.footprint.as_ref())),
    ]))
}

/// The member path of `--points "X,Y,Z;X,Y,Z;..."`: at least three three-component points in
/// the order given. Chord floors, mitres and the mesh budget are the kernel's to decide.
fn path_points(text: &str) -> Run<Vec<[f64; 3]>> {
    if text.split(';').count() > MAX_MEMBER_PATH_POINTS {
        return Err(PlateError {
            rule_id: None,
            code: ErrorCode::LimitExceeded,
            message: "more than 1024 path points",
        });
    }
    text.split(';')
        .map(|point| {
            let bad = || invalid("a path point is not three numbers");
            let mut parts = point.split(',').map(|v| v.trim().parse::<f64>());
            let mut next = || parts.next().transpose().map_err(|_| bad());
            let (x, y, z) = (next()?, next()?, next()?);
            match (x, y, z, next()?) {
                (Some(x), Some(y), Some(z), None) => Ok([x, y, z]),
                _ => Err(bad()),
            }
        })
        .collect()
}

fn create_polybeam_command(
    store: &RevisionStore,
    options: &BTreeMap<&str, &str>,
    candidates: &mut Vec<Json>,
) -> Run<Json> {
    let mut source = IdSource::system();
    let mut fresh = || {
        source.next_id().map_err(|_| PlateError {
            rule_id: None,
            code: ErrorCode::BackendUnavailable,
            message: "no identity could be generated",
        })
    };
    let given = |key: &str| options.get(key).copied();
    let new_id = match given("--new-id") {
        Some(text) => ElementId::parse(text).map_err(id_error)?,
        None => ElementId::from_uuid(fresh()?),
    };
    let op_id = match given("--op-id") {
        Some(text) => OpId::parse(text).map_err(id_error)?,
        None => OpId::from_uuid(fresh()?),
    };
    let revision_id = match given("--revision-id") {
        Some(text) => RevisionId::parse(text).map_err(id_error)?,
        None => RevisionId::from_uuid(fresh()?),
    };
    let actor = match given("--actor-id") {
        Some(text) => Uuid7::parse(text).map_err(PlateError::from)?,
        None => Uuid7::parse(&fresh()?.to_string()).map_err(PlateError::from)?,
    };
    let project = match given("--project-id") {
        Some(text) => ProjectId::parse(text).map_err(id_error)?,
        None => ProjectId::from_uuid(fresh()?),
    };
    let workspace = match given("--workspace-id") {
        Some(text) => WorkspaceId::parse(text).map_err(id_error)?,
        None => WorkspaceId::from_uuid(fresh()?),
    };
    let kind = match given("--kind") {
        Some("beam") | None => MemberKind::Beam,
        Some("column") => MemberKind::Column,
        Some(_) => return Err(invalid("--kind is beam or column")),
    };
    let points = path_points(given("--points").ok_or_else(|| invalid("--points is required"))?)?;
    let replay = if options.contains_key("--op-id")
        && (options.contains_key("--profile") || options.contains_key("--material"))
    {
        accepted_binding(store, &op_id, &new_id)?
    } else {
        None
    };
    let profile_digest = selected(
        store,
        options,
        ("--profile-digest", "--profile"),
        true,
        replay.as_ref(),
        candidates,
    )?
    .ok_or_else(|| invalid("--profile-digest or --profile is required"))?;
    let material_digest = selected(
        store,
        options,
        ("--material-digest", "--material"),
        false,
        replay.as_ref(),
        candidates,
    )?;
    let command = CreatePolybeam {
        new_id,
        kind,
        name: given("--name").unwrap_or(kind.as_str()).to_owned(),
        class: given("--class")
            .map(|v| {
                v.parse::<i32>()
                    .map_err(|_| invalid("class must be an int32"))
            })
            .transpose()?
            .unwrap_or(0),
        path_points_mm: points,
        profile_digest,
        placement: Placement {
            translation_mm: given("--translation").map_or(Ok([0.0; 3]), translation)?,
            orientation_xyzw: given("--orientation-xyzw")
                .map_or(Ok([0.0, 0.0, 0.0, 1.0]), quaternion)?,
        },
        material_digest,
    };
    let created = create_polybeam(
        store,
        &CreatePolybeamRequest {
            op_id,
            revision_id,
            actor,
            project,
            workspace,
            command,
        },
        &mut NoFault,
    )?;
    Ok(Json::object([
        (
            "outcome",
            Json::text(if created.replayed {
                "polybeam_replayed"
            } else {
                "polybeam_created"
            }),
        ),
        ("op_id", Json::text(op_id.to_string())),
        ("member", member_json(&created.member)?),
        ("revision", revision_json(&created.revision)),
        ("footprint", footprint_json(created.footprint.as_ref())),
    ]))
}

fn command_plane(
    options: &BTreeMap<&str, &str>,
) -> Run<Option<beamcraft_engine::work_plane::WorkPlane>> {
    let Some(text) = options.get("--plane") else {
        return Ok(None);
    };
    if [
        "--translation",
        "--orientation-xyzw",
        "--origin-at-grid",
        "--start-at-grid",
        "--end-at-grid",
        "--z",
        "--level",
        "--offset",
    ]
    .iter()
    .any(|key| options.contains_key(key))
    {
        return Err(invalid(
            "--plane cannot be combined with explicit placement or grid placement",
        ));
    }
    beamcraft_engine::work_plane::WorkPlane::parse(text)
        .map(Some)
        .map_err(PlateError::from)
}

fn grid_placement(
    store: &RevisionStore,
    options: &BTreeMap<&str, &str>,
    op: &OpId,
    key: &str,
) -> Run<[f64; 3]> {
    let address = options
        .get(key)
        .ok_or_else(|| invalid("missing grid address"))?;
    let z = options
        .get("--z")
        .map(|s| s.parse::<f64>().map_err(|_| invalid("invalid height")))
        .transpose()?;
    let level = options
        .get("--level")
        .map(|s| ElementId::parse(s).map_err(id_error))
        .transpose()?;
    let offset = options
        .get("--offset")
        .map(|s| s.parse::<f64>().map_err(|_| invalid("invalid offset")))
        .transpose()?
        .unwrap_or(0.0);
    if options.contains_key("--translation")
        || options.contains_key("--orientation-xyzw")
        || (options.contains_key("--offset") && level.is_none())
    {
        return Err(invalid(
            "grid placement conflicts with translation, orientation or an offset without level",
        ));
    }
    beamcraft_cli::grid::resolve_placement(store, op, address, z, level, offset)
}

fn member_json(record: &MemberRecord) -> Run<Json> {
    let mesh = &record.body.solid;
    let a = &record.authored;
    Ok(Json::object([
        ("element_id", Json::text(a.id.to_string())),
        (
            "prim_path",
            Json::text(a.id.prim_path(beamcraft_model::prim::ElementCategory::Part)),
        ),
        ("kind", Json::text(a.kind.as_str())),
        ("name", Json::text(a.name.clone())),
        ("class", num(f64::from(a.class))?),
        ("profile_digest", Json::text(a.profile.to_string())),
        (
            "material_digest",
            a.material.map_or(Json::Null, |d| Json::text(d.to_string())),
        ),
        (
            "section_mm",
            Json::object([
                ("width", num(record.body.width_mm)?),
                ("height", num(record.body.height_mm)?),
            ]),
        ),
        (
            "axis_points_mm",
            Json::Array(
                a.axis_points_mm
                    .iter()
                    .map(|point| triple(*point))
                    .collect::<Run<Vec<_>>>()?,
            ),
        ),
        ("roll_rad", a.roll_rad.map_or(Ok(Json::Null), num)?),
        (
            "placement",
            Json::object([
                ("translation_mm", triple(a.placement.translation_mm)?),
                (
                    "orientation_xyzw",
                    Json::Array(
                        a.placement
                            .orientation_xyzw
                            .into_iter()
                            .map(num)
                            .collect::<Run<Vec<_>>>()?,
                    ),
                ),
            ]),
        ),
        ("quantities", quantities_json(&record.quantities)?),
        ("mesh_digest", Json::text(mesh.digest().to_string())),
        ("vertices", Json::uint(mesh.vertices().len() as u64)),
        ("triangles", Json::uint(mesh.triangles().len() as u64)),
        ("cuts", cuts_json(&record.authored.cuts)?),
    ]))
}

/// The owned plane-cut records of an element as JSON; empty means an uncut element.
fn cuts_json(cuts: &[CutRecord]) -> Run<Json> {
    let records = cuts
        .iter()
        .map(|cut| {
            Ok(Json::object([
                ("operation_id", Json::text(cut.operation_id.to_string())),
                ("ordinal", Json::uint(u64::from(cut.ordinal))),
                ("point_mm", triple(cut.point_mm)?),
                ("normal", triple(cut.normal)?),
                ("keep", Json::text(cut.keep.as_str())),
            ]))
        })
        .collect::<Run<Vec<_>>>()?;
    Ok(Json::Array(records))
}

pub(crate) fn num(value: f64) -> Run<Json> {
    Json::number(value).map_err(|_| PlateError {
        rule_id: None,
        code: ErrorCode::NonFiniteNumber,
        message: "a result is not finite",
    })
}

pub(crate) fn triple(values: [f64; 3]) -> Run<Json> {
    Ok(Json::Array(
        values.into_iter().map(num).collect::<Run<Vec<_>>>()?,
    ))
}

/// A mass rounded to 0.1 kg for display. The stored and exact values are never rounded.
fn display_kg(kg: f64) -> f64 {
    (kg * 10.0).round() / 10.0
}

/// The quantities of a plate or a member.
///
/// `volume.mm3`, `volume.m3` and `mass.kg` are the NET values and keep the names of the first
/// quantity slices, so existing readers do not change. The explicit gross and net figures are
/// next to them at full precision, and `mass.kg_display` and friends are the same masses rounded
/// to 0.1 kg for display. `section_area_mm2` and `area_source` describe a member's effective
/// area.
fn quantities_json(q: &Quantities) -> Run<Json> {
    let optional = |value: Option<f64>| value.map_or(Ok(Json::Null), num);
    let display = |value: Option<f64>| value.map(display_kg).map_or(Ok(Json::Null), num);
    Ok(Json::object([
        (
            "volume",
            Json::object([
                ("mm3", num(q.volume_mm3)?),
                ("m3", num(q.volume_m3)?),
                ("status", Json::text(q.volume_status.as_str())),
                ("algorithm", Json::text(q.algorithm)),
            ]),
        ),
        (
            "mass",
            Json::object([
                ("kg", optional(q.mass_kg)?),
                ("kg_display", display(q.mass_kg)?),
                ("density_kg_m3", optional(q.density_kg_m3)?),
                ("status", Json::text(q.mass_status.as_str())),
            ]),
        ),
        ("volume_gross_mm3", num(q.volume_gross_mm3)?),
        ("volume_net_mm3", num(q.volume_mm3)?),
        ("removed_volume_mm3", num(q.removed_volume_mm3)?),
        ("mass_gross_kg", optional(q.mass_gross_kg)?),
        ("mass_net_kg", optional(q.mass_kg)?),
        ("mass_gross_kg_display", display(q.mass_gross_kg)?),
        ("mass_net_kg_display", display(q.mass_kg)?),
        ("section_area_mm2", optional(q.section_area_mm2)?),
        (
            "area_source",
            q.area_source.map_or(Json::Null, |s| Json::text(s.as_str())),
        ),
    ]))
}

pub(crate) fn definition_json(digest: &ContentDigest, record: &DefinitionRecord) -> Run<Json> {
    let mut fields = vec![
        ("digest", Json::text(digest.to_string())),
        (
            "prim_path",
            Json::text(definition_path(digest, record.kind())),
        ),
        ("kind", Json::text(record.kind())),
        ("units", Json::text("mm")),
        ("schema_version", Json::text("0.1.0")),
        (
            "display_name",
            record.display_name().map_or(Json::Null, Json::text),
        ),
    ];
    match record {
        DefinitionRecord::Material {
            density_kg_m3,
            grade,
            standard,
            ..
        } => {
            fields.push(("density_kg_m3", density_kg_m3.map_or(Ok(Json::Null), num)?));
            fields.push(("grade", grade.as_ref().map_or(Json::Null, Json::text)));
            fields.push(("standard", standard.as_ref().map_or(Json::Null, Json::text)));
        }
        DefinitionRecord::RectangleProfile {
            width_mm,
            height_mm,
            section,
        } => {
            fields.push(("width_mm", num(*width_mm)?));
            fields.push(("height_mm", num(*height_mm)?));
            section_fields(&mut fields, section)?;
        }
        DefinitionRecord::SectionProfile { shape, section } => {
            let (width, height) = shape.extent_mm();
            fields.push(("width_mm", num(width)?));
            fields.push(("height_mm", num(height)?));
            match shape {
                SectionShape::Parametric(p) => {
                    let names = p.family().parameters();
                    let values = names
                        .iter()
                        .zip(p.values())
                        .map(|(n, v)| Ok(((*n).to_owned(), num(*v)?)))
                        .collect::<Run<BTreeMap<_, _>>>()?;
                    fields.push(("parameters_mm", Json::Object(values)));
                }
                SectionShape::Custom(c) => {
                    let polygon = |points: &[[f64; 2]]| -> Run<Json> {
                        Ok(Json::Array(
                            points
                                .iter()
                                .map(|[x, y]| Ok(Json::Array(vec![num(*x)?, num(*y)?])))
                                .collect::<Run<_>>()?,
                        ))
                    };
                    fields.push(("outer_mm", polygon(&c.outer)?));
                    fields.push((
                        "cavities_mm",
                        Json::Array(c.cavities.iter().map(|v| polygon(v)).collect::<Run<_>>()?),
                    ));
                }
            }
            section_fields(&mut fields, section)?;
            fields.push((
                "declared_area_warning",
                match record.declared_area_warning() {
                    Some(w) => Json::object([
                        ("declared_mm2", num(w.declared_mm2)?),
                        ("kernel_mm2", num(w.kernel_mm2)?),
                        ("exact_mm2", num(w.kernel_mm2)?),
                        ("relative_difference", num(w.relative_difference)?),
                        ("threshold", num(w.threshold)?),
                        ("threshold_percent", num(w.threshold * 100.0)?),
                    ]),
                    None => Json::Null,
                },
            ));
        }
        DefinitionRecord::Fastener { spec } => {
            let optional = |value: Option<f64>| -> Run<Json> { value.map_or(Ok(Json::Null), num) };
            fields.push(("fastener_kind", Json::text(spec.kind.token())));
            fields.push(("head_kind", Json::text(spec.head_kind.token())));
            fields.push(("nominal_diameter_mm", num(spec.nominal_diameter_mm)?));
            fields.push(("head_across_flats_mm", num(spec.head_across_flats_mm)?));
            fields.push(("head_height_mm", num(spec.head_height_mm)?));
            fields.push(("nut_across_flats_mm", optional(spec.nut_across_flats_mm)?));
            fields.push(("nut_height_mm", optional(spec.nut_height_mm)?));
            match &spec.washer {
                Some(w) => {
                    fields.push(("washer_outer_diameter_mm", num(w.outer_diameter_mm)?));
                    fields.push(("washer_inner_diameter_mm", num(w.inner_diameter_mm)?));
                    fields.push(("washer_thickness_mm", num(w.thickness_mm)?));
                }
                None => fields.push(("washer", Json::Null)),
            }
            fields.push(("hole_clearance_mm", num(spec.hole_clearance_mm)?));
            fields.push((
                "lengths_mm",
                Json::Array(
                    spec.lengths_mm
                        .iter()
                        .map(|v| num(*v))
                        .collect::<Run<Vec<_>>>()?,
                ),
            ));
            fields.push(("thread_protrusion_mm", num(spec.thread_protrusion_mm)?));
            fields.push(("mass_each_kg", optional(spec.mass_each_kg)?));
            fields.push(("washer_under_head", Json::Bool(spec.washer_under_head)));
            fields.push(("washer_under_nut", Json::Bool(spec.washer_under_nut)));
            fields.push(("hole_mode", Json::text(spec.hole_mode.token())));
            fields.push((
                "designation",
                spec.designation.as_ref().map_or(Json::Null, Json::text),
            ));
            fields.push((
                "standard",
                spec.standard.as_ref().map_or(Json::Null, Json::text),
            ));
            fields.push(("grade", spec.grade.as_ref().map_or(Json::Null, Json::text)));
        }
    }
    Ok(Json::Object(
        fields
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    ))
}

/// The declared designation, standard and section numbers of any profile definition.
fn section_fields(fields: &mut Vec<(&'static str, Json)>, section: &SectionProperties) -> Run<()> {
    fields.push((
        "designation",
        section.designation.as_ref().map_or(Json::Null, Json::text),
    ));
    fields.push((
        "standard",
        section.standard.as_ref().map_or(Json::Null, Json::text),
    ));
    fields.push(("area_mm2", section.area_mm2.map_or(Ok(Json::Null), num)?));
    fields.push((
        "mass_per_meter_kg",
        section.mass_per_meter_kg.map_or(Ok(Json::Null), num)?,
    ));
    fields.push((
        "surface_per_meter_m2",
        section.surface_per_meter_m2.map_or(Ok(Json::Null), num)?,
    ));
    Ok(())
}

fn defined_json(defined: &Defined) -> Run<Json> {
    Ok(Json::object([
        (
            "outcome",
            Json::text(if defined.already_defined {
                "definition_exists"
            } else {
                "definition_created"
            }),
        ),
        (
            "definition",
            definition_json(&defined.digest, &defined.record)?,
        ),
        ("revision", revision_json(&defined.revision)),
    ]))
}

/// Items per page of a listing when `--limit` is not given.
pub(crate) const DEFAULT_PAGE: usize = 64;
/// Largest `--limit` of a listing.
pub(crate) const MAX_PAGE: usize = 256;

/// `--offset` (default 0) and `--limit` (default 64, at most 256, at least 1) of a listing.
pub(crate) fn page_of(options: &BTreeMap<&str, &str>) -> Run<(usize, usize)> {
    let number = |key: &str, default: usize| {
        options.get(key).map_or(Ok(default), |text| {
            text.parse::<usize>()
                .map_err(|_| invalid("--offset and --limit are unsigned integers"))
        })
    };
    let (offset, limit) = (number("--offset", 0)?, number("--limit", DEFAULT_PAGE)?);
    if limit == 0 || limit > MAX_PAGE {
        return Err(invalid("--limit is between 1 and 256"));
    }
    Ok((offset, limit))
}

/// The response of a listing page that fits the shared JSON encoder budgets (1 MiB of text and
/// the token scratch of the parser): `build(count)` makes the complete response for the first
/// `count` items of the page, and `count` is halved until the response encodes. A page is never
/// empty while items remain; one item that alone exceeds the budgets is `LIMIT_EXCEEDED`.
pub(crate) fn fit_page(
    available: usize,
    limit: usize,
    build: impl Fn(usize) -> Run<Json>,
) -> Run<Json> {
    let mut count = available.min(limit);
    loop {
        let json = build(count)?;
        if json.encode(&Limits::DEFAULT).is_ok() {
            return Ok(json);
        }
        if count <= 1 {
            return Err(PlateError {
                rule_id: None,
                code: ErrorCode::LimitExceeded,
                message: "one listing entry exceeds the output budget",
            });
        }
        count = count.div_ceil(2);
    }
}

/// The plates that pin each material, in the order of `plates` (element-id order): the
/// `used_by` list of `definitions`, built once instead of scanning every plate for every listed
/// definition. Members are not listed, as before.
fn plate_users(plates: &[PlateRecord]) -> BTreeMap<ContentDigest, Vec<ElementId>> {
    let mut users: BTreeMap<ContentDigest, Vec<ElementId>> = BTreeMap::new();
    for plate in plates {
        if let Some(material) = plate.authored.material {
            users.entry(material).or_default().push(plate.authored.id);
        }
    }
    users
}

/// The `definitions` reply for `inspection` (also framed by `export-view --with-definitions`).
pub(crate) fn definitions_json(
    inspection: &Inspection,
    options: &BTreeMap<&str, &str>,
) -> Run<Json> {
    let (offset, limit) = page_of(options)?;
    let name = options.get("--name");
    let matching: Vec<_> = inspection
        .definitions
        .iter()
        .filter(|(_, record)| name.is_none_or(|n| record.selectable_names().contains(n)))
        .collect();
    let total = matching.len();
    let available = total.saturating_sub(offset);
    let users = plate_users(&inspection.plates);
    fit_page(available, limit, |count| {
        let definitions = matching
            .iter()
            .skip(offset)
            .take(count)
            .map(|(digest, record)| {
                let used_by: Vec<Json> = users
                    .get(*digest)
                    .map_or(&[][..], Vec::as_slice)
                    .iter()
                    .map(|id| Json::text(id.to_string()))
                    .collect();
                let Json::Object(mut object) = definition_json(digest, record)? else {
                    return Err(invalid("definition result is not an object"));
                };
                object.insert("used_by".into(), Json::Array(used_by));
                Ok(Json::Object(object))
            })
            .collect::<Run<Vec<_>>>()?;
        let next = offset.saturating_add(count);
        Ok(Json::object([
            ("outcome", Json::text("definitions")),
            ("revision", revision_json(&inspection.revision)),
            ("definition_count", Json::uint(total as u64)),
            ("offset", Json::uint(offset as u64)),
            ("page_size", Json::uint(definitions.len() as u64)),
            (
                "next_offset",
                if next < total {
                    Json::uint(next as u64)
                } else {
                    Json::Null
                },
            ),
            ("definitions", Json::Array(definitions)),
        ]))
    })
}

fn unsupported_plate_json(record: &UnsupportedPlate) -> Run<Json> {
    let authored = &record.authored;
    let quantity = |unit| {
        Json::object([
            (unit, Json::Null),
            ("status", Json::text("unsupported")),
            ("reason", Json::text(record.reason)),
            ("input_signature", Json::Null),
            ("algorithm", Json::Null),
        ])
    };
    Ok(Json::object([
        ("element_id", Json::text(authored.id.to_string())),
        (
            "prim_path",
            Json::text(
                authored
                    .id
                    .prim_path(beamcraft_model::prim::ElementCategory::Part),
            ),
        ),
        ("kind", Json::text("plate")),
        ("name", Json::text(&authored.name)),
        ("validity", Json::text("valid")),
        ("geometry_status", Json::text("unsupported")),
        ("geometry_code", Json::text("UNSUPPORTED_GEOMETRY")),
        ("geometry_reason", Json::text(record.reason)),
        (
            "material_digest",
            authored
                .material
                .map_or(Json::Null, |d| Json::text(d.to_string())),
        ),
        (
            "contour_points_mm",
            Json::Array(
                authored
                    .contour_points_mm
                    .iter()
                    .map(|p| triple(*p))
                    .collect::<Run<Vec<_>>>()?,
            ),
        ),
        ("thickness_mm", num(authored.thickness_mm)?),
        (
            "thickness_side",
            Json::text(authored.thickness_side.unwrap_or(PlateSide::Top).as_str()),
        ),
        (
            "placement",
            Json::object([
                ("translation_mm", triple(authored.placement.translation_mm)?),
                (
                    "orientation_xyzw",
                    Json::Array(
                        authored
                            .placement
                            .orientation_xyzw
                            .into_iter()
                            .map(num)
                            .collect::<Run<Vec<_>>>()?,
                    ),
                ),
            ]),
        ),
        (
            "quantities",
            Json::object([("volume", quantity("mm3")), ("mass", quantity("kg"))]),
        ),
        ("mesh_digest", Json::Null),
        ("shape_key", Json::Null),
        ("world_bounds_mm", Json::Null),
    ]))
}

fn plate_json(record: &PlateRecord) -> Run<Json> {
    let mesh = &record.body.solid;
    Ok(Json::object([
        ("element_id", Json::text(record.authored.id.to_string())),
        (
            "prim_path",
            Json::text(
                record
                    .authored
                    .id
                    .prim_path(beamcraft_model::prim::ElementCategory::Part),
            ),
        ),
        ("kind", Json::text("plate")),
        ("name", Json::text(record.authored.name.clone())),
        ("class", num(f64::from(record.authored.class))?),
        (
            "material_digest",
            record
                .authored
                .material
                .map_or(Json::Null, |d| Json::text(d.to_string())),
        ),
        (
            "dimensions_mm",
            Json::object([
                ("width", num(record.body.width_mm)?),
                ("length", num(record.body.length_mm)?),
                ("thickness", num(record.body.thickness_mm)?),
            ]),
        ),
        (
            "thickness_side",
            Json::text(
                record
                    .authored
                    .thickness_side
                    .unwrap_or(PlateSide::Top)
                    .as_str(),
            ),
        ),
        (
            "contour_points_mm",
            Json::Array(
                record
                    .authored
                    .contour_points_mm
                    .iter()
                    .map(|p| triple(*p))
                    .collect::<Run<Vec<_>>>()?,
            ),
        ),
        (
            "outline",
            Json::text(if record.body.is_rectangular_box() {
                "rectangle"
            } else {
                "contour"
            }),
        ),
        (
            "placement",
            Json::object([
                (
                    "translation_mm",
                    triple(record.authored.placement.translation_mm)?,
                ),
                (
                    "orientation_xyzw",
                    Json::Array(
                        record
                            .authored
                            .placement
                            .orientation_xyzw
                            .into_iter()
                            .map(num)
                            .collect::<Run<Vec<_>>>()?,
                    ),
                ),
            ]),
        ),
        ("quantities", quantities_json(&record.quantities)?),
        ("mesh_digest", Json::text(mesh.digest().to_string())),
        ("vertices", Json::uint(mesh.vertices().len() as u64)),
        ("triangles", Json::uint(mesh.triangles().len() as u64)),
        (
            "hole_count",
            Json::uint(
                (record.authored.holes_mm.len() + record.authored.hole_records.len()) as u64,
            ),
        ),
        ("cuts", cuts_json(&record.authored.cuts)?),
        (
            "holes_mm",
            Json::Array(
                record
                    .authored
                    .hole_polygons_in_replay_order()
                    .iter()
                    .map(|hole| {
                        Ok(Json::Array(
                            hole.iter()
                                .map(|[x, y]| Ok(Json::Array(vec![num(*x)?, num(*y)?])))
                                .collect::<Run<Vec<_>>>()?,
                        ))
                    })
                    .collect::<Run<Vec<_>>>()?,
            ),
        ),
        (
            "hole_records",
            Json::Array(
                record
                    .authored
                    .hole_records
                    .iter()
                    .map(|r| {
                        Json::object([
                            ("operation_id", Json::text(r.operation_id.to_string())),
                            ("ordinal", Json::uint(r.ordinal as u64)),
                        ])
                    })
                    .collect(),
            ),
        ),
    ]))
}

pub(crate) fn revision_json(revision: &RevisionSummary) -> Json {
    let details = revision.details.as_ref().map_or(Json::Null, |d| {
        Json::object([
            ("project_id", Json::text(d.project.clone())),
            ("workspace_id", Json::text(d.workspace.clone())),
            ("op_id", Json::text(d.op.clone())),
            ("workspace_generation", Json::text(d.generation.to_string())),
            (
                "index",
                d.index.map_or(Json::Null, |(structure, prims)| {
                    Json::object([
                        ("structure_digest", Json::text(structure.to_string())),
                        ("prim_count", Json::text(prims.to_string())),
                    ])
                }),
            ),
            (
                "layers",
                Json::Array(
                    d.layers
                        .iter()
                        .map(|l| {
                            Json::object([
                                ("role", Json::text(l.role)),
                                ("name", Json::text(l.name.clone())),
                                ("digest", Json::text(l.digest.to_string())),
                                ("size", Json::text(l.size.to_string())),
                            ])
                        })
                        .collect(),
                ),
            ),
        ])
    });
    Json::object([
        ("revision_id", Json::text(revision.revision_id.clone())),
        ("sequence", Json::text(revision.sequence.to_string())),
        (
            "manifest_digest",
            Json::text(revision.manifest_digest.to_string()),
        ),
        ("manifest", details),
    ])
}

pub(crate) fn footprint_json(footprint: Option<&Footprint>) -> Json {
    let entries = |set: &std::collections::BTreeSet<Entry>| {
        Json::Array(
            set.iter()
                .map(|e| {
                    let (kind, id) = match e.target {
                        Target::Element(id) => ("element", id.to_string()),
                        Target::Definition(digest) => ("definition", digest.to_string()),
                    };
                    Json::object([
                        ("target_kind", Json::text(kind)),
                        ("target", Json::text(id)),
                        ("group", Json::text(e.group.as_str())),
                    ])
                })
                .collect(),
        )
    };
    footprint.map_or(Json::Null, |f| {
        Json::object([
            ("reads", entries(f.reads())),
            ("writes", entries(f.writes())),
        ])
    })
}

fn created_json(created: &Created, op_id: OpId) -> Run<Json> {
    Ok(Json::object([
        (
            "outcome",
            Json::text(if created.replayed {
                "plate_replayed"
            } else {
                "plate_created"
            }),
        ),
        ("op_id", Json::text(op_id.to_string())),
        ("plate", plate_json(&created.plate)?),
        ("revision", revision_json(&created.revision)),
        ("footprint", footprint_json(created.footprint.as_ref())),
    ]))
}

fn freshness_json(
    freshness: Freshness,
    stored_value: Option<f64>,
    stored_status: Option<&str>,
) -> Run<Json> {
    Ok(Json::object([
        ("freshness", Json::text(freshness.as_str())),
        ("stored_value", stored_value.map_or(Ok(Json::Null), num)?),
        (
            "stored_status",
            stored_status.map_or(Json::Null, Json::text),
        ),
    ]))
}

/// The stored quantity record of one element against what the engine computes now. A `stale`
/// stored value is shown as a previous value, never as the current one.
fn stored_json(report: &QuantityReport) -> Run<Json> {
    let stored = report.stored.as_ref();
    Ok(Json::object([
        (
            "volume",
            freshness_json(
                report.assessment.volume,
                stored.map(|s| s.volume_mm3),
                stored.map(|s| s.volume_status.as_str()),
            )?,
        ),
        (
            "mass",
            freshness_json(
                report.assessment.mass,
                stored.and_then(|s| s.mass_kg),
                stored.map(|s| s.mass_status.as_str()),
            )?,
        ),
        (
            "expected_shape_signature",
            Json::text(report.expected.shape_signature.to_string()),
        ),
        (
            "expected_gross_signature",
            Json::text(report.expected.gross_signature.to_string()),
        ),
        (
            "expected_mass_signature",
            Json::text(report.expected.mass_signature.to_string()),
        ),
        (
            "stored_algorithm",
            stored.map_or(Json::Null, |s| Json::text(s.algorithm.as_str())),
        ),
        (
            "superseded_algorithm",
            Json::Bool(stored.is_some_and(|s| is_superseded_algorithm(&s.algorithm))),
        ),
        (
            "stored_shape_signature",
            stored.map_or(Json::Null, |s| Json::text(s.shape_signature.to_string())),
        ),
        (
            "stored_mass_signature",
            stored.map_or(Json::Null, |s| Json::text(s.mass_signature.to_string())),
        ),
    ]))
}

/// The element JSON with its stored-quantity report and its level (D2) attached.
fn with_stored(element: Json, id: ElementId, inspection: &Inspection) -> Run<Json> {
    let Json::Object(mut object) = element else {
        return Ok(element);
    };
    let count = match inspection.instances.group_size(id) {
        Ok(count) => count,
        Err(error) if error.code == ErrorCode::NotFound => 1,
        Err(error) => return Err(error.into()),
    };
    object.insert(
        "shared_definition".into(),
        if count > 1 {
            Json::object([
                (
                    "id",
                    Json::text(inspection.instances.definition(id)?.to_string()),
                ),
                ("part_count", Json::uint(count as u64)),
            ])
        } else {
            Json::Null
        },
    );
    if let Some(report) = inspection.quantities.get(&id) {
        object.insert("stored_quantities".into(), stored_json(report)?);
    }
    object.insert(
        "level".into(),
        inspection
            .levels
            .level_of(&id)
            .map_or(Json::Null, |l| Json::text(l.to_string())),
    );
    Ok(Json::Object(object))
}

/// Adds the readable names of the definitions an element pins (`profile_name` for a member,
/// `material_name`), or null when the definition has no name (published before A07.2).
fn with_names(
    element: Json,
    profile: Option<ContentDigest>,
    material: Option<ContentDigest>,
    inspection: &Inspection,
) -> Json {
    let Json::Object(mut object) = element else {
        return element;
    };
    let name = |digest: Option<ContentDigest>| {
        digest
            .and_then(|d| inspection.definitions.get(&d))
            .and_then(|record| record.display_name())
            .map_or(Json::Null, Json::text)
    };
    if profile.is_some() {
        object.insert("profile_name".into(), name(profile));
    }
    object.insert("material_name".into(), name(material));
    Json::Object(object)
}

/// The `inspect-elements` reply for `inspection` (also framed by `export-view --with-element`).
/// `only` keeps one element in the lists (plates, members, the snapshot's elements and their
/// meshes), none when the revision does not hold it, and names it as `element_filter`; the
/// counts still describe the whole revision. Without it the reply lists everything.
pub(crate) fn inspection_json(inspection: &Inspection, only: Option<ElementId>) -> Run<Json> {
    let snapshot = &inspection.snapshot;
    let wanted = |id: ElementId| only.is_none_or(|o| o == id);
    let shown: Vec<_> = snapshot
        .elements()
        .iter()
        .filter(|e| wanted(e.id))
        .collect();
    let mut plates = inspection
        .plates
        .iter()
        .filter(|p| wanted(p.authored.id))
        .map(|p| {
            Ok(with_names(
                with_stored(plate_json(p)?, p.authored.id, inspection)?,
                None,
                p.authored.material,
                inspection,
            ))
        })
        .collect::<Run<Vec<_>>>()?;
    for p in &inspection.unsupported_plates {
        if wanted(p.authored.id) {
            plates.push(with_names(
                with_stored(unsupported_plate_json(p)?, p.authored.id, inspection)?,
                None,
                p.authored.material,
                inspection,
            ));
        }
    }
    plates.sort_by_key(|p| {
        p.get("element_id")
            .ok()
            .and_then(|id| id.string().ok())
            .unwrap_or("")
            .to_owned()
    });
    let members = inspection
        .members
        .iter()
        .filter(|m| wanted(m.authored.id))
        .map(|m| {
            Ok(with_names(
                with_stored(member_json(m)?, m.authored.id, inspection)?,
                Some(m.authored.profile),
                m.authored.material,
                inspection,
            ))
        })
        .collect::<Run<Vec<_>>>()?;
    let meshes = snapshot
        .meshes()
        .iter()
        .filter(|m| {
            only.is_none()
                || shown.iter().any(|e| {
                    e.full.mesh_digest == m.digest() || e.simplified.mesh_digest == m.digest()
                })
        })
        .map(|m| {
            Json::object([
                ("digest", Json::text(m.digest().to_string())),
                ("bytes", Json::uint(m.bytes().len() as u64)),
                ("vertices", Json::uint(u64::from(m.vertex_count()))),
                ("triangles", Json::uint(u64::from(m.triangle_count()))),
            ])
        })
        .collect();
    let mut entries = shown
        .iter()
        .map(|e| {
            Ok(Json::object([
                ("id", Json::text(e.id.to_string())),
                ("instance", Json::uint(u64::from(e.index))),
                ("mesh_digest", Json::text(e.full.mesh_digest.to_string())),
                ("placement_mm", triple(e.placement_mm)?),
                (
                    "orientation_xyzw",
                    Json::Array(
                        e.orientation_xyzw
                            .into_iter()
                            .map(num)
                            .collect::<Run<Vec<_>>>()?,
                    ),
                ),
                ("world_min_mm", triple(e.full.world_bounds.min)?),
                ("world_max_mm", triple(e.full.world_bounds.max)?),
            ]))
        })
        .collect::<Run<Vec<_>>>()?;
    for p in &inspection.unsupported_plates {
        if wanted(p.authored.id) {
            entries.push(Json::object([
                ("id", Json::text(p.authored.id.to_string())),
                ("instance", Json::Null),
                ("mesh_digest", Json::Null),
                ("shape_key", Json::Null),
                ("geometry_status", Json::text("unsupported")),
                ("placement_mm", triple(p.authored.placement.translation_mm)?),
                ("world_min_mm", Json::Null),
                ("world_max_mm", Json::Null),
            ]));
        }
    }
    entries.sort_by_key(|p| {
        p.get("id")
            .ok()
            .and_then(|id| id.string().ok())
            .unwrap_or("")
            .to_owned()
    });
    let mut reply = Json::object([
        ("outcome", Json::text("plate_inspection")),
        ("revision", revision_json(&inspection.revision)),
        (
            "plate_count",
            Json::uint(
                inspection
                    .plates
                    .len()
                    .saturating_add(inspection.unsupported_plates.len()) as u64,
            ),
        ),
        ("member_count", Json::uint(inspection.members.len() as u64)),
        (
            "grids",
            crate::grid_cmd::views_json(
                &inspection
                    .grids
                    .iter()
                    .filter(|g| only.is_none_or(|id| id == g.id))
                    .cloned()
                    .collect::<Vec<_>>(),
            )?,
        ),
        (
            "other_element_count",
            Json::uint(inspection.other_elements as u64),
        ),
        ("plates", Json::Array(plates)),
        ("members", Json::Array(members)),
        (
            "project_id_record",
            inspection
                .project_record
                .as_deref()
                .map_or(Json::Null, Json::text),
        ),
        (
            "levels",
            crate::level_cmd::views_json(&beamcraft_cli::level::views(&inspection.levels))?,
        ),
        (
            "snapshot",
            Json::object([
                ("synthetic", Json::Bool(false)),
                ("token", Json::text(snapshot.token().as_str())),
                (
                    "pin_revision",
                    Json::text(snapshot.pin().revision().to_string()),
                ),
                (
                    "pin_generation",
                    Json::text(snapshot.pin().generation().to_string()),
                ),
                ("elements", Json::Array(entries)),
                ("meshes", Json::Array(meshes)),
            ]),
        ),
    ]);
    if let (Some(id), Json::Object(fields)) = (only, &mut reply) {
        fields.insert("element_filter".into(), Json::text(id.to_string()));
    }
    Ok(reply)
}

/// Domain failures retain the exact F10 rule through CLI adapters.
pub(crate) fn domain_diagnostics(error: &PlateError) -> Vec<Json> {
    error
        .rule_id
        .into_iter()
        .map(|rule| {
            Json::object([
                ("rule_id", Json::text(rule)),
                ("severity", Json::text("error")),
                ("code", Json::text(error.code.as_str())),
                ("message", Json::text(error.message)),
            ])
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steel(density: f64) -> (ContentDigest, DefinitionRecord) {
        let record = DefinitionRecord::Material {
            density_kg_m3: Some(density),
            grade: None,
            standard: None,
            display_name: Some("Steel".into()),
        };
        (record.resource().unwrap().digest, record)
    }

    #[test]
    fn a_large_ambiguous_set_is_capped_counted_and_still_encodes_with_its_code() {
        let found: Vec<_> = (1..=4096).map(|i| steel(f64::from(i))).collect();
        let diagnostics = ambiguity_diagnostics(&found);
        assert_eq!(diagnostics.len(), MAX_LISTED_CANDIDATES + 1);
        let summary = diagnostics.last().unwrap();
        assert_eq!(summary.get("candidate_count").unwrap().float(), Ok(4096.0));
        assert_eq!(summary.get("listed").unwrap().float(), Ok(16.0));
        let (json, exit) = report_with(Err(invalid("ambiguous")), diagnostics);
        assert_eq!(exit, 2);
        // The shared encoder accepts it and keeps the error code.
        let bytes = json.encode(&Limits::DEFAULT).unwrap();
        let back = beamcraft_json::decode(&bytes, &Limits::DEFAULT).unwrap();
        assert_eq!(back.get("code").unwrap().string(), Ok("INVALID_ARGUMENT"));
        // A small set lists everything and adds no summary.
        assert_eq!(ambiguity_diagnostics(&found[..3]).len(), 3);
    }

    #[test]
    fn a_page_shrinks_until_its_response_fits_the_encoder_budgets() {
        // 256 strings of 4096 bytes are just over the 1 MiB output limit.
        let page = |count: usize| {
            Ok(Json::object([(
                "items",
                Json::Array(vec![Json::text("a".repeat(4096)); count]),
            )]))
        };
        let json = fit_page(256, 256, page).unwrap();
        let shown = json.get("items").unwrap().array().unwrap().len();
        assert!(shown > 0 && shown < 256, "{shown}");
        assert!(json.encode(&Limits::DEFAULT).is_ok());
        let one = fit_page(1, 256, page).unwrap();
        assert_eq!(one.get("items").unwrap().array().unwrap().len(), 1);
        let none = fit_page(0, 256, page).unwrap();
        assert!(none.get("items").unwrap().array().unwrap().is_empty());
    }

    #[test]
    fn page_options_are_bounded() {
        let options = |pairs: &[(&'static str, &'static str)]| pairs.iter().copied().collect();
        assert_eq!(page_of(&options(&[])).unwrap(), (0, DEFAULT_PAGE));
        assert_eq!(
            page_of(&options(&[("--offset", "5"), ("--limit", "256")])).unwrap(),
            (5, 256)
        );
        for bad in [("--limit", "0"), ("--limit", "257"), ("--offset", "-1")] {
            assert!(page_of(&options(&[bad])).is_err(), "{bad:?}");
        }
    }
}
