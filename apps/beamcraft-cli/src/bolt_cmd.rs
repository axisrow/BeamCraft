// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A14.3 headless commands: `create-bolt-group` and `update-bolt-group` place a row of
//! identical fasteners of a pinned library definition between two or more parts and edit one
//! group as one atomic edit (`--repin` follows the current library; an A05 group is upgraded
//! by supplying the placement inputs and a fastener). With `--hole-mode generate` the group
//! also writes one group-owned hole record per crossing fastener on every connected plate
//! (A14.4); `--hole-mode none` (a screw's default) writes no part. The A05 `create-bolt` token
//! is retired: stored annotation groups stay readable and upgradable, `bolts` lists them. The
//! commands call the engine API through `beamcraft_commands::bolt` and print one JSON object with
//! the same error shape and exit codes as the weld commands.

use crate::command_context::{self, CONTEXT_OPTIONS};
use crate::plate_cmd::{footprint_json, id_error, revision_json};
use beamcraft_commands::bolt::{
    BoltContext, BoltGroupView, BoltView, GroupChange, bolt_groups, bolts, create_bolt_group,
    update_bolt_group,
};
use beamcraft_commands::definition::{DefinitionRecord, head_definitions};
use beamcraft_commands::library::{Lookup, Source, load_fastener};
use beamcraft_host::{
    ids::IdSource,
    revision::{NoFault, RevisionStore},
    storage::BlobStore,
};
use beamcraft_json::Json;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ElementId, OpId};
use session::Refusal;
use std::collections::BTreeMap;
use std::ffi::OsString;
use steel::model::command::{
    BoltGroupInputs, BoltLength, BoltSpacing, CreateBoltGroup, UpdateBoltGroup,
};
use steel::model::fastener::HoleMode;

type Run<T> = Result<T, Refusal>;

fn invalid(message: &'static str) -> Refusal {
    Refusal {
        rule_id: None,
        code: ErrorCode::InvalidArgument,
        message,
    }
}

/// Runs one bolt command and returns the JSON result and the process exit code.
pub fn execute(args: &[OsString]) -> (Json, u8) {
    crate::report::report(run(args))
}

/// Parses `X,Y,Z` as three finite numbers of millimetres.
fn vec3(text: &str) -> Run<[f64; 3]> {
    let parts: Option<Vec<f64>> = text
        .split(',')
        .map(|p| p.trim().parse::<f64>().ok())
        .collect();
    let Some(values) = parts else {
        return Err(invalid("the vector is three comma-separated numbers"));
    };
    let v: [f64; 3] = <[f64; 3]>::try_from(values.as_slice())
        .map_err(|_| invalid("the vector is three comma-separated numbers"))?;
    if v.iter().any(|n| !n.is_finite()) {
        return Err(Refusal {
            rule_id: None,
            code: ErrorCode::NonFiniteNumber,
            message: "the vector is three comma-separated numbers",
        });
    }
    Ok(v)
}

/// Parses `a,b,c` as a non-empty list of finite numbers.
fn f64_list(text: &str, what: &'static str) -> Run<Vec<f64>> {
    let values = text
        .split(',')
        .map(|p| p.trim().parse::<f64>().map_err(|_| invalid(what)))
        .collect::<Run<Vec<_>>>()?;
    if values.is_empty() || values.iter().any(|n| !n.is_finite()) {
        return Err(invalid(what));
    }
    Ok(values)
}

fn boolean(text: &str) -> Run<bool> {
    match text {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(invalid(
            "--washer-under-head and --washer-under-nut are true or false",
        )),
    }
}

/// Normalizes a plane normal: the model stores a unit vector, so the CLI normalizes its input
/// and refuses zero as degenerate.
fn unit_normal(text: &str) -> Run<[f64; 3]> {
    let normal = vec3(text)?;
    let squared = normal.iter().map(|v| v * v).sum::<f64>();
    if squared <= 0.0 {
        return Err(invalid("--plane-normal is a non-zero vector"));
    }
    let norm = squared.sqrt();
    Ok([normal[0] / norm, normal[1] / norm, normal[2] / norm])
}

/// The stored record of one group: the row inputs, the authoritative spacings and the resolved
/// length. The bearing point of fastener `i` is `point1 + on_plane*y + depth*z + s_i*x`, with
/// `s_0` the offset start and `s_{k+1} = s_k + spacings[k]`.
fn spec_fields(view: &BoltGroupView) -> Vec<(&'static str, Json)> {
    let doubles = |name: &'static str, values: &[f64]| {
        (
            name,
            Json::Array(values.iter().map(|n| Json::text(n.to_string())).collect()),
        )
    };
    vec![
        doubles("point1_mm", &view.spec.point1_mm),
        doubles("point2_mm", &view.spec.point2_mm),
        doubles("plane_normal_mm", &view.spec.plane_normal_mm),
        (
            "rotation_rad",
            Json::text(view.spec.rotation_rad.to_string()),
        ),
        ("on_plane_mm", Json::text(view.spec.on_plane_mm.to_string())),
        ("depth_mm", Json::text(view.spec.depth_mm.to_string())),
        (
            "offset_start_mm",
            Json::text(view.spec.offset_start_mm.to_string()),
        ),
        ("spacing_mode", Json::text(view.spec.spacing_mode.clone())),
        (
            "spacing_value_mm",
            Json::text(view.spec.spacing_value_mm.to_string()),
        ),
        doubles("spacings_mm", &view.spec.spacings_mm),
        ("length_mode", Json::text(view.spec.length_mode.clone())),
        ("length_mm", Json::text(view.spec.length_mm.to_string())),
        (
            "washer_under_head",
            Json::text(view.spec.washer_under_head.to_string()),
        ),
        (
            "washer_under_nut",
            Json::text(view.spec.washer_under_nut.to_string()),
        ),
        ("hole_mode", Json::text(view.spec.hole_mode.clone())),
        (
            "hole_clearance_mm",
            match view.spec.hole_clearance_mm {
                Some(clearance) => Json::text(clearance.to_string()),
                None => Json::Null,
            },
        ),
    ]
}

/// The `--hole-mode` option: the amendment tokens, or the definition's preference when absent.
fn hole_mode(text: &str) -> Run<HoleMode> {
    match text {
        "generate" => Ok(HoleMode::Generate),
        "none" => Ok(HoleMode::None),
        _ => Err(invalid("--hole-mode is generate or none")),
    }
}

fn views_json(views: &[BoltView]) -> Json {
    Json::Array(
        views
            .iter()
            .map(|v| {
                Json::object([
                    ("id", Json::text(v.id.to_string())),
                    ("name", Json::text(v.name.clone())),
                    ("main", Json::text(v.main.to_string())),
                    ("secondary", Json::text(v.secondary.to_string())),
                    ("diameter_mm", Json::text(v.spec.diameter_mm.to_string())),
                    ("count", Json::text(v.spec.count.to_string())),
                    ("pitch_mm", Json::text(v.spec.pitch_mm.to_string())),
                    (
                        "origin_mm",
                        Json::Array(
                            v.spec
                                .origin_mm
                                .iter()
                                .map(|n| Json::text(n.to_string()))
                                .collect(),
                        ),
                    ),
                    (
                        "direction_mm",
                        Json::Array(
                            v.spec
                                .direction_mm
                                .iter()
                                .map(|n| Json::text(n.to_string()))
                                .collect(),
                        ),
                    ),
                ])
            })
            .collect(),
    )
}

fn group_views_json(views: &[BoltGroupView]) -> Vec<Json> {
    views
        .iter()
        .map(|v| {
            let mut fields = BTreeMap::new();
            fields.insert("id".to_owned(), Json::text(v.id.to_string()));
            fields.insert("name".to_owned(), Json::text(v.name.clone()));
            fields.insert(
                "parts".to_owned(),
                Json::Array(v.parts.iter().map(|p| Json::text(p.to_string())).collect()),
            );
            fields.insert(
                "fastener".to_owned(),
                Json::text(v.spec.fastener.to_string()),
            );
            fields.insert(
                "count".to_owned(),
                Json::text((v.spec.spacings_mm.len() + 1).to_string()),
            );
            for (name, value) in spec_fields(v) {
                fields.insert(name.to_owned(), value);
            }
            Json::Object(fields)
        })
        .collect()
}

fn change_json(subject: ElementId, op_id: OpId, change: &GroupChange) -> Json {
    let outcome = if change.replayed {
        "bolt_group_replayed"
    } else {
        "bolt_group_committed"
    };
    Json::object([
        ("outcome", Json::text(outcome)),
        ("op_id", Json::text(op_id.to_string())),
        ("element_id", Json::text(subject.to_string())),
        (
            "changed_layers",
            Json::Array(change.changed_layers.iter().map(Json::text).collect()),
        ),
        ("bolt_groups", Json::Array(group_views_json(&change.groups))),
        ("revision", revision_json(&change.revision)),
        ("footprint", footprint_json(change.footprint.as_ref())),
    ])
}

fn run(args: &[OsString]) -> Run<Json> {
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
    let mut create_options = vec![
        "--store",
        "--name",
        "--new-id",
        "--parts",
        "--fastener-library",
        "--designation",
        "--point1",
        "--point2",
        "--plane-normal",
        "--rotation-deg",
        "--on-plane",
        "--depth",
        "--offset-start",
        "--spacings",
        "--count",
        "--pitch",
        "--offset-end",
        "--washer-under-head",
        "--washer-under-nut",
        "--length-mode",
        "--length",
        "--hole-mode",
        "--hole-clearance",
    ];
    let own: &[&str] = match *command {
        "create-bolt-group" => create_options.leak(),
        "update-bolt-group" => {
            create_options.push("--target");
            create_options.push("--repin");
            create_options.leak()
        }
        "bolts" => &["--store"],
        _ => return Err(invalid("unknown command")),
    };
    if !tail.len().is_multiple_of(2) || tail.len() > 64 {
        return Err(invalid("options are KEY VALUE pairs"));
    }
    let mut options = BTreeMap::new();
    for pair in tail.as_chunks::<2>().0 {
        let [key, value] = pair;
        let allowed = own.contains(key) || CONTEXT_OPTIONS.contains(key);
        if !allowed || options.insert(*key, *value).is_some() {
            return Err(invalid("unknown or repeated option"));
        }
    }
    let store_root = options
        .get("--store")
        .ok_or_else(|| invalid("--store is required"))?;
    let blobs = BlobStore::new(store_root).map_err(|e| Refusal {
        rule_id: None,
        code: e.code(),
        message: "store root is not usable",
    })?;
    let store = RevisionStore::new(blobs);
    let _claim = crate::plate_cmd::check_project_option(&store, &options)?;
    if *command == "bolts" {
        return Ok(Json::object([
            ("outcome", Json::text("bolts")),
            ("bolts", views_json(&bolts(&store)?)),
            (
                "bolt_groups",
                Json::Array(group_views_json(&bolt_groups(&store)?)),
            ),
        ]));
    }
    let mut source = IdSource::system();
    let mut fresh = || {
        source.next_id().map_err(|_| Refusal {
            rule_id: None,
            code: ErrorCode::BackendUnavailable,
            message: "no identity could be generated",
        })
    };
    let given = |key: &str| options.get(key).copied();
    let required = |key: &str, missing: &'static str| given(key).ok_or_else(|| invalid(missing));
    let number = |key: &'static str, missing: &'static str| -> Run<f64> {
        required(key, missing)?
            .parse::<f64>()
            .map_err(|_| invalid("the value is a number"))
    };
    let optional_number = |key: &str| -> Run<Option<f64>> {
        given(key)
            .map(|v| {
                v.parse::<f64>()
                    .map_err(|_| invalid("the value is a number"))
            })
            .transpose()
    };
    let optional_flag = |key: &str| -> Run<Option<bool>> { given(key).map(boolean).transpose() };
    // The definition travels by designation through one library file; the entry carries every
    // value, so the model never needs the file again.
    let library_definition = |options: &BTreeMap<&str, &str>| -> Run<DefinitionRecord> {
        let path = options
            .get("--fastener-library")
            .ok_or_else(|| invalid("--fastener-library is required"))?;
        let designation = options
            .get("--designation")
            .ok_or_else(|| invalid("--designation is required"))?;
        let library = load_fastener(path)?;
        let lookup = Lookup::new(vec![Source {
            path: (*path).to_owned(),
            library,
        }]);
        let (_, entry) = lookup.find(designation)?;
        match &entry.record {
            record @ DefinitionRecord::Fastener { .. } => Ok(record.clone()),
            _ => Err(invalid("the designation does not name a fastener")),
        }
    };
    let element = |key: &str, missing: &'static str| -> Run<ElementId> {
        ElementId::parse(required(key, missing)?).map_err(id_error)
    };
    let parts = |options: &BTreeMap<&str, &str>| -> Run<Vec<ElementId>> {
        let text = options
            .get("--parts")
            .ok_or_else(|| invalid("--parts is two or more comma-separated part identities"))?;
        let parsed = text
            .split(',')
            .map(|p| ElementId::parse(p.trim()).map_err(id_error))
            .collect::<Run<Vec<_>>>()?;
        if parsed.len() < 2 {
            return Err(invalid(
                "--parts is two or more comma-separated part identities",
            ));
        }
        Ok(parsed)
    };
    // The spacing pattern: an explicit list, or one of the two generators. Exactly one form is
    // given; the generators need --count.
    let spacing = |options: &BTreeMap<&str, &str>| -> Run<BoltSpacing> {
        let count = |options: &BTreeMap<&str, &str>| -> Run<usize> {
            options
                .get("--count")
                .map(|v| {
                    v.parse::<usize>()
                        .map_err(|_| invalid("--count is a whole number"))
                })
                .transpose()?
                .ok_or_else(|| invalid("--count is required with a generator"))
        };
        match (
            options.get("--spacings"),
            options.get("--pitch"),
            options.get("--offset-end"),
        ) {
            (Some(text), None, None) => Ok(BoltSpacing::Spacings(f64_list(
                text,
                "--spacings is a comma-separated list of positive numbers",
            )?)),
            (None, Some(pitch), None) => Ok(BoltSpacing::Pitch {
                count: count(options)?,
                pitch_mm: pitch
                    .parse::<f64>()
                    .map_err(|_| invalid("--pitch is a number of millimetres"))?,
            }),
            (None, None, Some(end)) => Ok(BoltSpacing::OffsetEnd {
                count: count(options)?,
                offset_end_mm: end
                    .parse::<f64>()
                    .map_err(|_| invalid("--offset-end is a number of millimetres"))?,
            }),
            _ => Err(invalid(
                "give exactly one of --spacings, --count with --pitch, or --count with --offset-end",
            )),
        }
    };
    let length_choice = |options: &BTreeMap<&str, &str>| -> Run<BoltLength> {
        match options.get("--length-mode").copied() {
            None => Ok(BoltLength::Auto),
            Some("auto") => Ok(BoltLength::Auto),
            Some("fixed") => Ok(BoltLength::Fixed(number(
                "--length",
                "--length is required with --length-mode fixed",
            )?)),
            Some(_) => Err(invalid("--length-mode is auto or fixed")),
        }
    };
    let opt_hole_mode = |options: &BTreeMap<&str, &str>| -> Run<Option<HoleMode>> {
        options.get("--hole-mode").map(|v| hole_mode(v)).transpose()
    };
    let common = command_context::parse(&options, &mut fresh)?;
    let context = BoltContext {
        op_id: common.op_id,
        revision_id: common.revision_id,
        actor: common.actor,
        project: common.project,
        workspace: common.workspace,
    };
    if *command == "create-bolt-group" {
        let point1 = vec3(required("--point1", "--point1 is required")?)?;
        let point2 = vec3(required("--point2", "--point2 is required")?)?;
        let normal = unit_normal(required("--plane-normal", "--plane-normal is required")?)?;
        let rotation = optional_number("--rotation-deg")?
            .unwrap_or(0.0)
            .to_radians();
        let on_plane = optional_number("--on-plane")?.unwrap_or(0.0);
        let depth = optional_number("--depth")?.unwrap_or(0.0);
        let offset_start = number("--offset-start", "--offset-start is required")?;
        let name = required("--name", "--name is required")?.to_owned();
        let new_id = match given("--new-id") {
            Some(text) => ElementId::parse(text).map_err(id_error)?,
            None => ElementId::from_uuid(fresh()?),
        };
        let definition = library_definition(&options)?;
        let DefinitionRecord::Fastener { spec } = &definition else {
            return Err(invalid("the designation does not name a fastener"));
        };
        let digest = definition.resource()?.digest;
        let command = CreateBoltGroup {
            new_id,
            name,
            inputs: BoltGroupInputs {
                parts: parts(&options)?,
                fastener_digest: digest,
                fastener: spec.clone(),
                point1_mm: point1,
                point2_mm: point2,
                plane_normal_mm: normal,
                rotation_rad: rotation,
                on_plane_mm: on_plane,
                depth_mm: depth,
                offset_start_mm: offset_start,
                spacing: spacing(&options)?,
                washer_under_head: optional_flag("--washer-under-head")?,
                washer_under_nut: optional_flag("--washer-under-nut")?,
                length: length_choice(&options)?,
                hole_mode: opt_hole_mode(&options)?,
                hole_clearance_mm: optional_number("--hole-clearance")?,
            },
        };
        let subject = command.new_id;
        let change = create_bolt_group(&store, &context, &command, &definition, &mut NoFault)?;
        return Ok(change_json(subject, context.op_id, &change));
    }
    // update-bolt: the complete new inputs replace the stored ones; an absent input keeps the
    // stored value, and a stored A05 group is upgraded from its parts plus the given inputs.
    let target = element("--target", "--target is required")?;
    let current = bolt_groups(&store)?.into_iter().find(|g| g.id == target);
    let annotation = if current.is_none() {
        bolts(&store)?.into_iter().find(|b| b.id == target)
    } else {
        None
    };
    let repin = match given("--repin") {
        None => false,
        Some("true") => true,
        Some("false") => false,
        Some(_) => return Err(invalid("--repin is true or false")),
    };
    let from_library = given("--fastener-library").is_some() || given("--designation").is_some();
    // A repin always names the library entry it follows; a library without a repin on a stored
    // group would silently follow a changed definition, which only --repin may do. The upgrade
    // of an A05 annotation group is different: it has no definition yet, so naming the library
    // entry is the only way. A retry of an already accepted operation skips the guard and
    // reaches the adapter's replay comparison, whose verdict names the real state.
    if repin != from_library
        && current.is_some()
        && !beamcraft_commands::bolt::op_accepted(&store, &context.op_id)?
    {
        return Err(invalid(
            "--repin needs --fastener-library and --designation",
        ));
    }
    let definition = match (&current, repin || from_library) {
        (_, true) => library_definition(&options)?,
        (Some(group), false) => {
            // No repin: the group keeps its pinned definition, whatever the library says now.
            let pinned = head_definitions(&store)?;
            pinned.get(&group.spec.fastener).cloned().ok_or_else(|| {
                invalid("the pinned definition of the group is not in the head revision")
            })?
        }
        (None, false) => {
            return Err(invalid(
                "upgrading an A05 annotation group needs --fastener-library and --designation",
            ));
        }
    };
    let DefinitionRecord::Fastener { spec } = &definition else {
        return Err(invalid("the designation does not name a fastener"));
    };
    let digest = definition.resource()?.digest;
    // Each placement input is read on its own: an absent one keeps the stored value, and a
    // stored A05 group has no placement, so the upgrade must name every required one.
    let upgrade_needs = |key: &str| -> &'static str {
        match key {
            "--point1" => "upgrading an A05 annotation group needs --point1",
            "--point2" => "upgrading an A05 annotation group needs --point2",
            "--plane-normal" => "upgrading an A05 annotation group needs --plane-normal",
            "--offset-start" => "upgrading an A05 annotation group needs --offset-start",
            _ => "the value is required",
        }
    };
    let vec3_or = |key: &'static str, stored: Option<[f64; 3]>| -> Run<[f64; 3]> {
        match given(key) {
            Some(text) => vec3(text),
            None => stored.ok_or_else(|| invalid(upgrade_needs(key))),
        }
    };
    let number_or = |key: &'static str, stored: Option<f64>| -> Run<f64> {
        match given(key) {
            Some(text) => text
                .parse::<f64>()
                .map_err(|_| invalid("the value is a number")),
            None => stored.ok_or_else(|| invalid(upgrade_needs(key))),
        }
    };
    let stored = current.as_ref().map(|g| &g.spec);
    let point1 = vec3_or("--point1", stored.map(|s| s.point1_mm))?;
    let point2 = vec3_or("--point2", stored.map(|s| s.point2_mm))?;
    let normal = match given("--plane-normal") {
        Some(text) => unit_normal(text)?,
        None => stored
            .map(|s| s.plane_normal_mm)
            .ok_or_else(|| invalid("upgrading an A05 annotation group needs --plane-normal"))?,
    };
    let rotation = match given("--rotation-deg") {
        Some(text) => text
            .parse::<f64>()
            .map_err(|_| invalid("the value is a number"))?
            .to_radians(),
        // The stored radians stay; an upgrade takes the amendment's zero default.
        None => stored.map(|s| s.rotation_rad).unwrap_or(0.0),
    };
    // The position values default to the amendment's zero when absent and nothing is stored.
    let on_plane = match given("--on-plane") {
        Some(text) => text
            .parse::<f64>()
            .map_err(|_| invalid("the value is a number"))?,
        None => stored.map(|s| s.on_plane_mm).unwrap_or(0.0),
    };
    let depth = match given("--depth") {
        Some(text) => text
            .parse::<f64>()
            .map_err(|_| invalid("the value is a number"))?,
        None => stored.map(|s| s.depth_mm).unwrap_or(0.0),
    };
    let offset_start = number_or("--offset-start", stored.map(|s| s.offset_start_mm))?;
    let spacing = match given("--spacings")
        .or_else(|| given("--pitch"))
        .or_else(|| given("--offset-end"))
    {
        Some(_) => spacing(&options)?,
        None => match &current {
            Some(group) => match group.spec.spacing_mode.as_str() {
                "pitch" => BoltSpacing::Pitch {
                    count: group.spec.spacings_mm.len() + 1,
                    pitch_mm: group.spec.spacing_value_mm,
                },
                "offset_end" => BoltSpacing::OffsetEnd {
                    count: group.spec.spacings_mm.len() + 1,
                    offset_end_mm: group.spec.spacing_value_mm,
                },
                _ => BoltSpacing::Spacings(group.spec.spacings_mm.clone()),
            },
            None => {
                return Err(invalid(
                    "upgrading an A05 annotation group needs a spacing pattern",
                ));
            }
        },
    };
    let length = match given("--length-mode") {
        Some(_) => length_choice(&options)?,
        None => match &current {
            Some(group) if group.spec.length_mode == "fixed" => {
                BoltLength::Fixed(group.spec.length_mm)
            }
            _ => BoltLength::Auto,
        },
    };
    // The hole setting follows the same rule: an absent --hole-mode keeps the stored one (an
    // upgrade takes the definition's preference through the None default), and an absent
    // --hole-clearance keeps the stored value only when the group keeps generating.
    let stored_hole_mode = current
        .as_ref()
        .and_then(|g| steel::model::fastener::HoleMode::from_token(&g.spec.hole_mode));
    let hole_mode = match given("--hole-mode") {
        Some(text) => Some(hole_mode(text)?),
        None => stored_hole_mode,
    };
    let hole_clearance_mm = match given("--hole-clearance") {
        Some(text) => Some(
            text.parse::<f64>()
                .map_err(|_| invalid("the value is a number"))?,
        ),
        None => match hole_mode {
            Some(HoleMode::Generate) => current.as_ref().and_then(|g| g.spec.hole_clearance_mm),
            // A quiet group stores none; a mode that follows the definition takes its
            // clearance too.
            _ => None,
        },
    };
    let command = UpdateBoltGroup {
        target_id: target,
        inputs: BoltGroupInputs {
            parts: match given("--parts") {
                Some(_) => parts(&options)?,
                None => match (&current, &annotation) {
                    (Some(group), _) => group.parts.clone(),
                    (None, Some(bolt)) => vec![bolt.main, bolt.secondary],
                    (None, None) => {
                        return Err(invalid(
                            "no bolt group of this identity is in the head revision",
                        ));
                    }
                },
            },
            fastener_digest: digest,
            fastener: spec.clone(),
            point1_mm: point1,
            point2_mm: point2,
            plane_normal_mm: normal,
            rotation_rad: rotation,
            on_plane_mm: on_plane,
            depth_mm: depth,
            offset_start_mm: offset_start,
            spacing,
            washer_under_head: match optional_flag("--washer-under-head")? {
                Some(flag) => Some(flag),
                // A stored group keeps its switch; an upgrade takes the definition's own
                // preference.
                None => current.as_ref().map(|g| g.spec.washer_under_head),
            },
            washer_under_nut: match optional_flag("--washer-under-nut")? {
                Some(flag) => Some(flag),
                None => current.as_ref().map(|g| g.spec.washer_under_nut),
            },
            length,
            hole_mode,
            hole_clearance_mm,
        },
    };
    let change = update_bolt_group(&store, &context, &command, &definition, &mut NoFault)?;
    Ok(change_json(target, context.op_id, &change))
}
