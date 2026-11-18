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
use beamcraft_host::{
    revision::{Revision, RevisionStore, Uuid7},
    storage::BlobStore,
};
use beamcraft_json::{Json, Limits};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::ElementId;
use session::Refusal;
use session::draft;
use std::collections::BTreeMap;
use std::ffi::OsString;

type Run<T> = Result<T, Refusal>;

fn invalid(message: &'static str) -> Refusal {
    Refusal {
        rule_id: None,
        code: ErrorCode::InvalidArgument,
        message,
    }
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
    let options: BTreeMap<&str, &str> =
        crate::bom_cmd::pairs(args, &["--store", "--element", "--offset", "--limit"], 8)?
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
    let revision = store.open()?;
    let inspection = inspect_revision(&revision)?;
    let inputs = inputs_of(&revision)?;
    let report = check::check_model(&inspection, &inputs, element, offset, limit)?;
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
    let json = Json::object([
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

/// The routes whose write commands operate on the model store: the modelling,
/// definition and construction routes, the recorded-input route and the
/// reference and numbering routes. A successful write on one of them carries
/// the `checks` summary. The other routes deliberately carry none: `Generic`
/// runs the store-level primitives (`put-blob`, `commit-revision`, recovery)
/// whose layers need not be model content; the export routes (`ExportView`,
/// `ScreenshotView`, `Export`) and `Bom`, `Schedule`, `Changeset`, `Drawing`
/// and `Ifc` render output files or their own documents; `Views` manages
/// display state and `Library` the named library file; `History`, `Coordinator`
/// and `Coordination` manage compaction, leases and coordination descriptors;
/// `Project` provisions a fresh store and changes no modelled revision; and
/// `Session`, `Batch` and `Commands` run no model write in their own process —
/// those return before `dispatch`, so the summary seam never sees them.
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

/// The window opt-out (contract section 11): `BEAMCRAFT_CHECKS_SUMMARY` set to
/// exactly `off` omits the summary from write replies for this process; unset
/// or any other value keeps summaries on.
pub(crate) fn checks_opted_out() -> bool {
    std::env::var("BEAMCRAFT_CHECKS_SUMMARY").as_deref() == Ok("off")
}

/// Attaches the `checks` summary to a successful write reply (contract
/// section 11): the summary checks the result revision named by the write
/// reply, including an earlier replay, and reports counts only. A refused write
/// keeps its rejected reply with no summary; the summary never changes the
/// write's exit code, because the write happened. A command running inside an
/// open draft batch reports the draft as not evaluated instead of checking
/// the saved head.
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
    if entry.access != registry::Access::Write || !MODEL_WRITE_ROUTES.contains(&entry.route) {
        return (json, exit);
    }
    let Some(root) = store_root(args) else {
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

/// The `--store` value of a command line: strict option pairs, the value right
/// after the key. A line that reached exit 0 already passed the pair grammar.
fn store_root(args: &[OsString]) -> Option<&str> {
    args.get(1..)?
        .as_chunks::<2>()
        .0
        .iter()
        .find_map(|pair| match pair {
            [key, value] if key.to_str() == Some("--store") => value.to_str(),
            _ => None,
        })
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
    let summary = (|| -> Run<Json> {
        let id = reply
            .get("revision")
            .and_then(|r| r.get("revision_id"))
            .and_then(Json::string)
            .map_err(|_| invalid("the write result does not identify its revision"))?;
        let id =
            Uuid7::parse(id).map_err(|_| invalid("the write result has an invalid revision"))?;
        let sequence = reply
            .get("revision")
            .and_then(|r| r.get("sequence"))
            .and_then(Json::string)
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .ok_or_else(|| invalid("the write result does not identify its revision sequence"))?;
        let revision = store.open_at(sequence)?;
        if revision.info().revision() != &id {
            return Err(invalid(
                "the write result revision does not match the opened snapshot",
            ));
        }
        let inspection = inspect_revision(&revision)?;
        let inputs = inputs_of(&revision)?;
        let report = check::check_model(&inspection, &inputs, None, 0, SUMMARY_PAGE_LIMIT)?;
        Ok(Json::object([
            ("revision_id", Json::text(report.revision_id.clone())),
            ("status", Json::text(report.status)),
            ("counts", counts_json(&report)),
            ("findings_total", Json::uint(report.findings_total as u64)),
        ]))
    })();
    match summary {
        Ok(json) => json,
        Err(refusal) => unavailable(&format!("{} ({})", refusal.message, refusal.code.as_str())),
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
    let mut entries = match &with_summary {
        Json::Object(entries) => entries.clone(),
        _ => return reply,
    };
    entries.insert(
        "checks".to_owned(),
        unavailable("the checks summary did not fit the reply budget"),
    );
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
