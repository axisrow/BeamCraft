// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Q03 headless command `check-model`: check one identified revision of a
//! model store and print the report. It calls
//! `beamcraft_commands::plate::inspect` and `beamcraft_commands::check` and
//! prints one JSON object; exit 0 for a green or unknown (not fully checked)
//! model, 2 for a red one. A refusal keeps the ordinary rejected-reply shape
//! with its own exit 2, so a scripted consumer branches on the reply, not on
//! the code alone.

use beamcraft_commands::check::{self, Bucket, CheckReport, Coverage};
use beamcraft_commands::plate::inspect_revision;
use beamcraft_commands::registry::{self, Route};
use beamcraft_commands::task_check::{MAX_SPEC_BYTES, TaskFinding, TaskSpec, VERSION};
use beamcraft_host::{
    revision::{Revision, RevisionStore, Uuid7},
    storage::BlobStore,
};
use beamcraft_json::{Json, Limits};
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::ElementId;
use session::Refusal;
use session::draft;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::Write;
use std::sync::OnceLock;

type Run<T> = Result<T, Refusal>;

fn invalid(message: &'static str) -> Refusal {
    Refusal {
        rule_id: None,
        code: ErrorCode::InvalidArgument,
        message,
    }
}

enum TaskBinding {
    Unconfigured,
    Approved(TaskSpec),
    Unavailable(Refusal),
}

fn task_binding() -> &'static TaskBinding {
    static TASK: OnceLock<TaskBinding> = OnceLock::new();
    TASK.get_or_init(|| {
        let path = std::env::var_os("BEAMCRAFT_TASK_SPEC");
        let expected = std::env::var_os("BEAMCRAFT_TASK_SPEC_SHA256");
        if path.is_none() && expected.is_none() {
            return TaskBinding::Unconfigured;
        }
        let load = || -> Run<TaskSpec> {
            let expected = expected
                .as_deref()
                .and_then(|text| text.to_str())
                .and_then(|text| text.parse::<ContentDigest>().ok())
                .ok_or_else(|| {
                    invalid("BEAMCRAFT_TASK_SPEC_SHA256 must identify the approved specification")
                })?;
            let path = path
                .as_deref()
                .and_then(|path| path.to_str())
                .ok_or_else(|| {
                    invalid("BEAMCRAFT_TASK_SPEC must name the approved specification file")
                })?;
            let bytes = beamcraft_commands::source_file::read_file(path, MAX_SPEC_BYTES).map_err(
                |code| {
                    Refusal::new(
                        code,
                        "the approved task specification file could not be read",
                    )
                },
            )?;
            TaskSpec::decode(&bytes, expected)
        };
        match load() {
            Ok(spec) => TaskBinding::Approved(spec),
            Err(error) => TaskBinding::Unavailable(error),
        }
    })
}

/// Whether this process is checking an independently supplied task.
pub(crate) fn task_configured() -> bool {
    !matches!(task_binding(), TaskBinding::Unconfigured)
}

fn task_finding_json(finding: TaskFinding) -> Json {
    Json::object([
        ("code", Json::text(finding.code)),
        ("status", Json::text(finding.status)),
        (
            "requirement",
            finding
                .requirement
                .map_or(Json::Null, |i| Json::uint(i as u64)),
        ),
        ("target", Json::text(finding.target)),
        ("message", Json::text(finding.message)),
        ("expected", finding.expected),
        ("actual", finding.actual),
    ])
}

fn check_revision(
    revision: &Revision,
    element: Option<ElementId>,
    offset: usize,
    limit: usize,
) -> Run<(CheckReport, Option<Json>)> {
    let inputs = inputs_of(revision)?;
    let binding = task_binding();
    if let TaskBinding::Approved(spec) = binding {
        let checked = spec.check_revision(revision, inputs, element, offset, limit)?;
        let task = Json::object([
            ("version", Json::uint(VERSION)),
            ("sha256", Json::text(checked.digest.to_string())),
            ("status", Json::text(checked.model.status)),
            (
                "reason",
                element.map_or(Json::Null, |_| Json::text("partial_geometry_scope")),
            ),
            ("requirements", Json::uint(checked.requirement_count as u64)),
            (
                "findings",
                Json::Array(
                    checked
                        .findings
                        .into_iter()
                        .map(task_finding_json)
                        .collect(),
                ),
            ),
        ]);
        return Ok((checked.model, Some(task)));
    }
    let inspection = inspect_revision(revision)?;
    let mut model = check::check_model(&inspection, &inputs, element, offset, limit)?;
    let task = if let TaskBinding::Unavailable(refusal) = binding {
        if model.status != "red" {
            model.status = "unknown";
        }
        Some(Json::object([
            ("status", Json::text("unknown")),
            ("code", Json::text(refusal.code.as_str())),
            ("reason", Json::text(refusal.message)),
        ]))
    } else {
        None
    };
    Ok((model, task))
}

/// The recorded inputs of the verified head: the declared joint intents
/// (a model may declare several; their scopes merge and their connections
/// concatenate), the recorded gap requirements, and the weld facts of the
/// A06 and A15 welds. A store with no records yields the default inputs and
/// the connectivity rules stay not applicable for every element.
fn inputs_of(revision: &Revision) -> Run<steel::engine::check::RecordedInputs> {
    let mut inputs = steel::engine::check::RecordedInputs::default();
    let mut scope = Vec::new();
    for record in beamcraft_commands::record::records_revision(revision)? {
        match record.payload {
            beamcraft_model::record::RecordPayload::JointIntent(mut intent) => {
                scope.append(&mut intent.scope);
                match &mut inputs.intent {
                    Some(joint) => joint.connections.append(&mut intent.connections),
                    None => inputs.intent = Some(intent),
                }
            }
            beamcraft_model::record::RecordPayload::GapRequirement(gap) => {
                inputs.gaps.push(gap);
            }
            beamcraft_model::record::RecordPayload::CornerCut(corner) => {
                inputs.corner_cuts.push(corner);
            }
        }
    }
    if let Some(joint) = &mut inputs.intent {
        scope.sort();
        scope.dedup();
        joint.scope = scope;
    }
    for weld in beamcraft_commands::weld::welds_revision(revision)? {
        inputs.welds.push(steel::engine::check::WeldFact {
            element: weld.id,
            main: weld.main,
            secondary: weld.secondary,
            size_mm: weld.spec.size_mm,
            path_mm: weld.solid.map(|solid| solid.geometry.path_mm.clone()),
        });
    }
    Ok(inputs)
}

/// Runs `check-model` and returns the JSON result and the process exit code:
/// 0 green or yellow (not fully checked), 2 red, and the shared rejection
/// envelope for an operational failure.
pub fn execute(args: &[OsString]) -> (Json, u8) {
    match run(args) {
        Ok((json, exit)) => (json, exit),
        Err(refusal) => crate::report::report(Err(refusal)),
    }
}

fn run(args: &[OsString]) -> Run<(Json, u8)> {
    let options: BTreeMap<&str, &str> = crate::bom_cmd::pairs(
        args,
        &["--store", "--revision", "--element", "--offset", "--limit"],
        10,
    )?
    .into_iter()
    .collect();
    let store = options
        .get("--store")
        .ok_or_else(|| invalid("--store is required"))?;
    let blobs = BlobStore::new(store).map_err(|e| Refusal {
        rule_id: None,
        code: e.code(),
        message: "store root is not usable",
    })?;
    let store = RevisionStore::new(blobs);
    let element = options
        .get("--element")
        .map(|text| {
            ElementId::parse(text)
                .map_err(|_| invalid("--element is a UUID of the checked revision"))
        })
        .transpose()?;
    let (offset, limit) = crate::plate_cmd::page_of(&options)?;
    let revision = match options.get("--revision") {
        Some(id) => revision_by_id(&store, id)?,
        None => store.open()?,
    };
    let (report, task) = check_revision(&revision, element, offset, limit)?;
    let buckets = report
        .coverage
        .iter()
        .filter(|row| matches!(row.bucket, Bucket::Checked))
        .count();
    let not_applicable = report
        .coverage
        .iter()
        .filter(|row| matches!(row.bucket, Bucket::NotApplicable(_)))
        .count();
    let unknown = report
        .coverage
        .iter()
        .filter(|row| matches!(row.bucket, Bucket::Unknown(_)))
        .count();
    let mut json = Json::object([
        ("outcome", Json::text("model_checked")),
        ("revision_id", Json::text(report.revision_id.clone())),
        (
            "scope",
            Json::object([
                ("mode", Json::text(report.scope_mode)),
                (
                    "element",
                    report
                        .scope_element
                        .map_or(Json::Null, |id| Json::text(id.to_string())),
                ),
                ("elements", Json::uint(report.scope_elements as u64)),
            ]),
        ),
        ("status", Json::text(report.status)),
        ("contact_tolerance_mm", number(report.tolerance_mm)),
        ("counts", counts_json(&report)),
        (
            "coverage",
            Json::object([
                ("checked", Json::uint(buckets as u64)),
                ("not_applicable", Json::uint(not_applicable as u64)),
                ("unknown", Json::uint(unknown as u64)),
                (
                    "elements",
                    Json::Array(report.coverage.iter().map(row_json).collect()),
                ),
                ("limitations", Json::text(report.limitations.clone())),
            ]),
        ),
        ("findings_total", Json::uint(report.findings_total as u64)),
        (
            "findings",
            Json::Array(report.findings.iter().map(finding_json).collect()),
        ),
    ]);
    if let Json::Object(entries) = &mut json {
        entries.insert(
            "task".to_owned(),
            task.unwrap_or_else(|| task_unknown("task_approval_missing")),
        );
        entries.insert(
            "manifest_digest".to_owned(),
            Json::text(revision.info().manifest_digest().to_string()),
        );
        entries.insert(
            "next_offset".to_owned(),
            next_offset(
                report.findings_total,
                offset.saturating_add(report.findings.len()),
            ),
        );
        if draft::is_open() && task_configured() {
            entries.insert(
                "draft_checks".to_owned(),
                task_unknown("draft_not_evaluated"),
            );
        }
    }
    let exit = if report.status == "red" { 2 } else { 0 };
    Ok((json, exit))
}

/// The per-rule counts of a report: one entry per supported rule, whole-scope,
/// with the `fail` and `unknown` finding counts of that rule.
fn counts_json(report: &CheckReport) -> Json {
    let counts = check::RULES
        .iter()
        .map(|rule| {
            let entry = report.counts.get(rule).cloned().unwrap_or_default();
            (
                (*rule).to_owned(),
                Json::object([
                    ("fail", Json::uint(entry.fail as u64)),
                    ("unknown", Json::uint(entry.unknown as u64)),
                ]),
            )
        })
        .collect::<BTreeMap<_, _>>();
    Json::Object(counts)
}

/// One coverage row of the reply.
fn row_json(row: &Coverage) -> Json {
    let (bucket, reason) = match &row.bucket {
        Bucket::Checked => ("checked", None),
        Bucket::NotApplicable(reason) => ("not_applicable", Some((*reason).to_owned())),
        Bucket::Unknown(reason) => ("unknown", Some((*reason).to_owned())),
    };
    Json::object([
        ("id", Json::text(row.id.to_string())),
        ("kind", Json::text(row.kind.clone())),
        ("bucket", Json::text(bucket)),
        ("reason", reason.map_or(Json::Null, Json::text)),
    ])
}

/// One finding of the reply. The `fix` is present only when the intended repair
/// and all its inputs are unambiguous (contract section 9): the corner-cut rule
/// offers `set-plate-corner` with the required chamfer at the authored corner
/// vertex, and every other finding stays null.
fn finding_json(finding: &check::Finding) -> Json {
    Json::object([
        ("rule", Json::text(finding.rule)),
        ("status", Json::text(finding.status)),
        (
            "elements",
            Json::Array(
                finding
                    .elements
                    .iter()
                    .map(|(id, name)| {
                        Json::object([
                            ("id", Json::text(id.to_string())),
                            ("name", Json::text(name.clone())),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("message", Json::text(finding.message.clone())),
        ("basis", Json::text(finding.basis.clone())),
        ("measured_mm", number_opt(finding.measured_mm)),
        ("required_mm", number_opt(finding.required_mm)),
        ("tolerance_mm", number_opt(finding.tolerance_mm)),
        ("mesh_bound_mm", number_opt(finding.mesh_bound_mm)),
        (
            "effective_tolerance_mm",
            number_opt(finding.effective_tolerance_mm),
        ),
        (
            "bounds",
            Json::Array(
                finding
                    .bounds
                    .iter()
                    .map(|bound| {
                        Json::object([
                            ("name", Json::text(bound.name)),
                            ("bound_mm", number(bound.bound_mm)),
                            ("tolerance_mm", number(bound.tolerance_mm)),
                            ("source", Json::text(bound.source)),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "missing_inputs",
            Json::Array(
                finding
                    .missing_inputs
                    .iter()
                    .map(|input| Json::text((*input).to_owned()))
                    .collect(),
            ),
        ),
        (
            "fix",
            match &finding.fix {
                None => Json::Null,
                Some(fix) => Json::object([
                    ("command", Json::text(fix.command)),
                    (
                        "arguments",
                        Json::Array(
                            fix.arguments
                                .iter()
                                .map(|argument| Json::text(argument.clone()))
                                .collect(),
                        ),
                    ),
                    ("effect", Json::text(fix.effect.clone())),
                ]),
            },
        ),
    ])
}

fn number_opt(value: Option<f64>) -> Json {
    value.map_or(Json::Null, number)
}

/// A finite measurement; an unfittable value stays null rather than failing
/// the whole reply.
fn number(value: f64) -> Json {
    Json::number(value).unwrap_or(Json::Null)
}

/// Standalone modelling routes that publish a revision and carry the slice 5 summary.
/// Task-bound commands also classify the remaining write routes from the registry.
const MODEL_WRITE_ROUTES: &[Route] = &[
    Route::Plate,
    Route::Weld,
    Route::Bolt,
    Route::Attribute,
    Route::Annotation,
    Route::Assembly,
    Route::Construction,
    Route::WorkPlane,
    Route::Grid,
    Route::Level,
    Route::Record,
    Route::Reference,
    Route::Numbering,
];

/// The findings page the summary materializes: the status, counts and total
/// are whole-scope regardless of the page, so one finding is enough.
const SUMMARY_PAGE_LIMIT: usize = 1;
const TASK_PAGE_LIMIT: usize = 32;
const MODEL_ROOT_OPTIONS: &[&str] = &["--store", "--workspace-dir", "--published", "--model-store"];

/// The window opt-out (contract section 11): `BEAMCRAFT_CHECKS_SUMMARY` set to
/// exactly `off` omits the summary from write replies for this process; unset
/// or any other value keeps summaries on.
pub(crate) fn checks_opted_out() -> bool {
    !task_configured() && std::env::var("BEAMCRAFT_CHECKS_SUMMARY").as_deref() == Ok("off")
}

/// Attaches checks to a successful write. The result revision can be an
/// earlier replay. Task-bound replies include findings and classify writes
/// without a verifiable revision as unknown. Drafted writes are unevaluated.
pub(crate) fn with_summary(args: &[OsString], json: Json, exit: u8) -> (Json, u8) {
    if exit != 0 || checks_opted_out() {
        return (json, exit);
    }
    let Some(word) = args.first().and_then(|word| word.to_str()) else {
        return (json, exit);
    };
    let Some(entry) = registry::COMMANDS.iter().find(|entry| entry.name == word) else {
        return (json, exit);
    };
    if entry.access != registry::Access::Write {
        return (json, exit);
    }
    if task_configured() {
        let mut roots = MODEL_ROOT_OPTIONS
            .iter()
            .filter_map(|key| store_option(entry, args, key))
            .collect::<Vec<_>>();
        if let Ok(root) = json
            .get("project")
            .and_then(|project| project.get("path"))
            .and_then(Json::string)
        {
            roots.push(root);
        }
        let checks = if draft::is_open() {
            task_unknown("draft_not_evaluated")
        } else {
            task_summary_for_roots(&roots, &json)
        };
        return (insert_checks(json, checks), exit);
    }
    if !MODEL_WRITE_ROUTES.contains(&entry.route) {
        return (json, exit);
    }
    let Some(root) = store_option(entry, args, "--store") else {
        return (json, exit);
    };
    let checks = if draft::is_open() {
        // The drafted command has published no revision; the saved head is not
        // what the command changed (contract section 10).
        Json::object([
            ("status", Json::text("unknown")),
            ("reason", Json::text("draft_not_evaluated")),
        ])
    } else {
        summary_for_root(root, &json)
    };
    (insert_checks(json, checks), exit)
}

/// Finds the supplied root without interpreting a command option's value as an option.
fn store_option<'a>(
    entry: &registry::Command,
    args: &'a [OsString],
    name: &str,
) -> Option<&'a str> {
    let mut tokens = args.iter().skip(1);
    while let Some(key) = tokens.next().and_then(|key| key.to_str()) {
        if entry
            .params
            .iter()
            .any(|param| param.name == key && param.value.is_empty())
        {
            continue;
        }
        let value = tokens.next()?;
        if key == name {
            return value.to_str();
        }
    }
    None
}

/// The summary for a store named by root: an operational failure of the
/// follow-up check reports the checks as unavailable, never green.
fn summary_for_root(root: &str, reply: &Json) -> Json {
    match BlobStore::new(root) {
        Ok(blobs) => summary_for_store(&RevisionStore::new(blobs), reply),
        Err(e) => unavailable(&format!(
            "the store could not be opened ({})",
            e.code().as_str()
        )),
    }
}

/// Checks one store's resulting revision and builds the counts-only summary.
/// A refusal of the check (an unreadable store, a head that cannot be
/// inspected) becomes `unavailable` with the refusal's message and code, so
/// the successful write result travels with an honest "not checked".
pub(crate) fn summary_for_store(store: &RevisionStore, reply: &Json) -> Json {
    match result_revision(store, reply) {
        Ok(revision) => summary_of_revision(&revision),
        Err(refusal) if task_configured() => {
            task_unknown(&format!("{} ({})", refusal.message, refusal.code.as_str(),))
        }
        Err(refusal) => unavailable(&format!("{} ({})", refusal.message, refusal.code.as_str())),
    }
}

fn result_revision(store: &RevisionStore, reply: &Json) -> Run<Revision> {
    let summary = reply
        .get("revision")
        .or_else(|_| reply.get("project").and_then(|p| p.get("revision")))
        .or_else(|_| reply.get("draft_head"))
        .or_else(|_| reply.get("workspace").and_then(|w| w.get("draft_head")))
        .or_else(|_| reply.get("changeset"))
        .unwrap_or(reply);
    let id = summary
        .get("revision_id")
        .and_then(Json::string)
        .map_err(|_| invalid("the command result does not identify its revision"))?;
    let Ok(sequence) = summary.get("sequence") else {
        return revision_by_id(store, id);
    };
    let sequence = sequence
        .unsigned()
        .ok()
        .or_else(|| sequence.string().ok().and_then(|s| s.parse::<u64>().ok()))
        .ok_or_else(|| invalid("the command result has an invalid revision sequence"))?;
    let revision = store.open_at(sequence)?;
    if revision.info().revision().as_str() != id {
        return Err(invalid(
            "the command result revision does not match the opened snapshot",
        ));
    }
    if let Ok(digest) = summary.get("manifest_digest").and_then(Json::string)
        && revision.info().manifest_digest().to_string() != digest
    {
        return Err(invalid(
            "the command result manifest does not match the opened snapshot",
        ));
    }
    Ok(revision)
}

fn revision_by_id(store: &RevisionStore, id: &str) -> Run<Revision> {
    let id = Uuid7::parse(id).map_err(|_| invalid("expected a UUIDv7 revision id"))?;
    let sequence = store.sequence_of(&id)?.ok_or_else(|| {
        Refusal::new(
            ErrorCode::NotFound,
            "the result revision is unavailable in this store",
        )
    })?;
    let revision = store.open_at(sequence)?;
    if revision.info().revision() != &id {
        return Err(invalid(
            "the command result revision does not match the opened snapshot",
        ));
    }
    Ok(revision)
}

fn summary_of_revision(revision: &Revision) -> Json {
    match revision_summary(revision) {
        Ok(report) => report,
        Err(error) if task_configured() => Json::object([
            ("status", Json::text("unknown")),
            ("code", Json::text(error.code.as_str())),
            ("reason", Json::text(error.message)),
            (
                "revision_id",
                Json::text(revision.info().revision().as_str()),
            ),
            (
                "manifest_digest",
                Json::text(revision.info().manifest_digest().to_string()),
            ),
        ]),
        Err(error) => unavailable(&format!("{} ({})", error.message, error.code.as_str())),
    }
}

fn revision_summary(revision: &Revision) -> Run<Json> {
    let limit = if task_configured() {
        usize::MAX
    } else {
        SUMMARY_PAGE_LIMIT
    };
    let (report, task) = check_revision(revision, None, 0, limit)?;
    let mut result = Json::object([
        ("revision_id", Json::text(report.revision_id.clone())),
        ("status", Json::text(report.status)),
        ("counts", counts_json(&report)),
        ("findings_total", Json::uint(report.findings_total as u64)),
    ]);
    if let Json::Object(entries) = &mut result {
        let Some(task) = task else {
            entries.insert("task".to_owned(), task_unknown("task_approval_missing"));
            return Ok(result);
        };
        let unchecked = report
            .coverage
            .iter()
            .filter(|row| matches!(row.bucket, Bucket::Unknown(_)))
            .collect::<Vec<_>>();
        let total = report.findings_total.max(unchecked.len());
        let manifest = revision.info().manifest_digest().to_string();
        let finding_rows = |offset| {
            Json::Array(
                report
                    .findings
                    .iter()
                    .skip(offset)
                    .take(TASK_PAGE_LIMIT)
                    .map(finding_json)
                    .collect(),
            )
        };
        let unchecked_rows = |offset| {
            Json::Array(
                unchecked
                    .iter()
                    .skip(offset)
                    .take(TASK_PAGE_LIMIT)
                    .map(|row| row_json(row))
                    .collect(),
            )
        };
        for offset in (TASK_PAGE_LIMIT..total).step_by(TASK_PAGE_LIMIT) {
            let checks = Json::object([
                ("revision_id", Json::text(&report.revision_id)),
                ("manifest_digest", Json::text(&manifest)),
                ("status", Json::text(report.status)),
                (
                    "task_sha256",
                    task.get("sha256").cloned().unwrap_or(Json::Null),
                ),
                ("offset", Json::uint(offset as u64)),
                (
                    "next_offset",
                    next_offset(total, offset.saturating_add(TASK_PAGE_LIMIT)),
                ),
                ("findings_total", Json::uint(report.findings_total as u64)),
                ("findings", finding_rows(offset)),
                ("unchecked_total", Json::uint(unchecked.len() as u64)),
                ("unchecked", unchecked_rows(offset)),
            ]);
            write_task_report(checks);
        }
        entries.insert("task".to_owned(), task);
        entries.insert("manifest_digest".to_owned(), Json::text(manifest));
        entries.insert("findings".to_owned(), finding_rows(0));
        entries.insert(
            "unchecked_total".to_owned(),
            Json::uint(unchecked.len() as u64),
        );
        entries.insert("unchecked".to_owned(), unchecked_rows(0));
        entries.insert(
            "next_offset".to_owned(),
            next_offset(total, TASK_PAGE_LIMIT),
        );
        entries.insert("scope".to_owned(), Json::text("model"));
    }
    Ok(result)
}

fn task_summary_for_roots(roots: &[&str], reply: &Json) -> Json {
    let mut reason = "the command did not identify a verifiable model store";
    for store in roots
        .iter()
        .filter(|root| std::path::Path::new(root).is_dir())
        .filter_map(|root| BlobStore::new(root).ok())
        .map(RevisionStore::new)
    {
        match result_revision(&store, reply) {
            Ok(revision) => return summary_of_revision(&revision),
            Err(error) => reason = error.message,
        }
    }
    task_unknown(reason)
}

fn next_offset(total: usize, seen: usize) -> Json {
    if seen < total {
        Json::uint(seen as u64)
    } else {
        Json::Null
    }
}

pub(crate) fn task_unknown(reason: &str) -> Json {
    Json::object([
        ("status", Json::text("unknown")),
        ("reason", Json::text(reason)),
    ])
}

/// Sends a task report beside a binary result or a reply whose JSON budget is already full.
pub(crate) fn write_task_report(checks: Json) {
    let reply = Json::object([("outcome", Json::text("task_checks")), ("checks", checks)]);
    let mut err = std::io::stderr().lock();
    if let Ok(bytes) = reply.encode(&Limits::DEFAULT) {
        let _ = err.write_all(&bytes).and_then(|()| err.write_all(b"\n"));
    } else {
        let _ = err.write_all(b"{\"outcome\":\"task_checks\",\"checks\":{\"status\":\"unknown\",\"reason\":\"task report exceeded the reply budget\"}}\n");
    }
}

/// The reply shape when the follow-up check did not run: neither green nor a
/// check status, with the reason stated.
fn unavailable(reason: &str) -> Json {
    Json::object([
        ("status", Json::text("unavailable")),
        ("reason", Json::text(reason.to_owned())),
    ])
}

/// Inserts the `checks` summary into a reply object without poisoning it
/// (contract section 11): if the reply with the full summary exceeds the reply
/// budgets, the summary shrinks to an unavailable marker; if even that does
/// not fit, the reply is returned unchanged — the write result survives.
pub(crate) fn insert_checks(reply: Json, checks: Json) -> Json {
    let mut entries = match &reply {
        Json::Object(entries) => entries.clone(),
        _ => return reply,
    };
    entries.insert("checks".to_owned(), checks);
    let with_summary = Json::Object(entries);
    if with_summary.encode(&Limits::DEFAULT).is_ok() {
        return with_summary;
    }
    let mut entries = match with_summary {
        Json::Object(entries) => entries,
        _ => return reply,
    };
    let compact = if task_configured() {
        let Some(checks) = entries.remove("checks") else {
            return reply;
        };
        let mut marker = BTreeMap::from([("report_on".to_owned(), Json::text("stderr"))]);
        for field in ["status", "reason", "revision_id", "manifest_digest"] {
            if let Ok(value) = checks.get(field) {
                marker.insert(field.to_owned(), value.clone());
            }
        }
        write_task_report(checks);
        Json::Object(marker)
    } else {
        unavailable("the checks summary did not fit the reply budget")
    };
    entries.insert("checks".to_owned(), compact);
    let shrunk = Json::Object(entries);
    if shrunk.encode(&Limits::DEFAULT).is_ok() {
        shrunk
    } else {
        reply
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

    use super::*;

    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

    /// One-entry filler strings stay inside the per-string budget, so a
    /// near-budget reply is many entries of them. Each entry's encoded cost is
    /// its filler plus a small constant, which a probe measures once; the
    /// three-digit key keeps every entry the same shape.
    const FILLER: usize = 1900;

    fn entry_constant() -> usize {
        let probe = Json::object([("f000", Json::text("x".repeat(FILLER)))]);
        probe.encode(&Limits::DEFAULT).unwrap().len() - FILLER
    }

    /// A reply object whose encoding sits `gap` bytes under the reply byte
    /// budget: whole entries first, one final entry sized to land the total.
    /// The separator accounting is off by the first entry's missing comma, so
    /// the last entry's filler is shaved until the reply alone fits exactly.
    fn reply_at(gap: usize) -> Json {
        let budget = Limits::DEFAULT.bytes - gap;
        let constant = entry_constant();
        // A middle entry costs its filler plus `constant - 1`: the probe's
        // braces are replaced by a leading comma.
        let step = FILLER + constant - 1;
        let mut pairs = BTreeMap::new();
        let mut remaining = budget;
        let mut n = 0usize;
        while remaining > step + 64 {
            pairs.insert(format!("f{n:03}"), Json::text("x".repeat(FILLER)));
            remaining -= step;
            n += 1;
        }
        pairs.insert(
            format!("f{n:03}"),
            Json::text("x".repeat(remaining - (constant - 1))),
        );
        let mut reply = Json::Object(pairs);
        while let Err(_) = reply.encode(&Limits::DEFAULT)
            && let Json::Object(entries) = &mut reply
            && let Some((_, last)) = entries.iter_mut().next_back()
            && let Json::String(text) = last
            && text.len() > 1
        {
            text.truncate(text.len() - 1);
        }
        reply
    }

    /// A summary-sized object: the real counts summary is of this order.
    fn large_checks() -> Json {
        let pairs = (0..9)
            .map(|n| {
                (
                    format!("RULE_NUMBER_{n}_OF_THE_SUPPORTED_SET"),
                    Json::object([("fail", Json::uint(12)), ("unknown", Json::uint(3))]),
                )
            })
            .collect::<BTreeMap<_, _>>();
        Json::Object(pairs)
    }

    #[test]
    fn the_summary_rides_a_reply_that_fits() {
        let reply = Json::object([("outcome", Json::text("created"))]);
        let checks = Json::object([("status", Json::text("red"))]);
        let combined = insert_checks(reply, checks);
        assert_eq!(text_of(&combined, "outcome"), "created");
        let checks = combined.get("checks").expect("checks present");
        assert_eq!(text_of(checks, "status"), "red");
    }

    #[test]
    fn a_reply_with_room_for_the_marker_but_not_the_summary_shrinks_it() {
        // ~150 bytes under the budget: a summary-sized object no longer fits,
        // the unavailable marker does, so the reply discloses unavailable.
        let reply = reply_at(150);
        assert!(reply.encode(&Limits::DEFAULT).is_ok());
        let combined = insert_checks(reply, large_checks());
        match &combined {
            Json::Object(entries) => {
                let checks = entries.get("checks").expect("checks disclosed");
                let status = checks
                    .get("status")
                    .ok()
                    .and_then(|s| s.string().ok())
                    .unwrap_or_default();
                assert_eq!(status, "unavailable");
            }
            other => panic!("unexpected reply shape {other:?}"),
        }
        assert!(combined.encode(&Limits::DEFAULT).is_ok());
    }

    #[test]
    fn a_reply_that_cannot_carry_even_the_marker_stays_whole() {
        // One byte under the budget: neither the summary nor the marker fits,
        // so the original reply is returned unchanged with its write result.
        let reply = reply_at(1);
        assert!(reply.encode(&Limits::DEFAULT).is_ok());
        let combined = insert_checks(reply, large_checks());
        match &combined {
            Json::Object(entries) => {
                assert!(
                    !entries.contains_key("checks"),
                    "an unfittable marker leaves the reply unchanged"
                );
            }
            other => panic!("unexpected reply shape {other:?}"),
        }
    }

    #[test]
    fn a_store_without_a_revision_reports_unavailable_never_green() {
        let root = std::env::temp_dir().join(format!(
            "beamcraft-check-summary-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let blobs = BlobStore::new(root.to_str().unwrap()).unwrap();
        let summary = summary_for_store(&RevisionStore::new(blobs), &Json::Null);
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(text_of(&summary, "status"), "unavailable");
        let reason = summary
            .get("reason")
            .ok()
            .and_then(|r| r.string().ok())
            .expect("reason");
        assert!(!reason.is_empty());
        assert!(summary.get("revision_id").is_err(), "nothing was checked");
    }

    fn text_of(json: &Json, key: &str) -> String {
        json.get(key)
            .ok()
            .and_then(|value| value.string().ok())
            .unwrap_or_else(|| panic!("{key} missing from {json:?}"))
            .to_owned()
    }
}
