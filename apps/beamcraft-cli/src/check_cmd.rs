// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Q03 headless command `check-model`: check one identified revision of a
//! model store and print the report. It calls
//! `beamcraft_commands::plate::inspect` and `beamcraft_commands::check` and
//! prints one JSON object; exit 0 for a green or unknown (not fully checked)
//! model, 2 for a red one. A refusal keeps the ordinary rejected-reply shape
//! with its own exit 2, so a scripted consumer branches on the reply, not on
//! the code alone.

use beamcraft_commands::check::{self, Bucket, Coverage};
use beamcraft_commands::plate::inspect;
use beamcraft_host::{revision::RevisionStore, storage::BlobStore};
use beamcraft_json::Json;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::ElementId;
use session::Refusal;
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
    let inspection = inspect(&store)?;
    let report = check::check_model(&inspection, element, offset, limit)?;
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
        ("counts", Json::Object(counts)),
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
                ("limitations", Json::text(report.limitations)),
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

/// One finding of the reply. The `fix` stays null in this slice: a fix is
/// present only when the intended repair and all its inputs are unambiguous
/// (contract section 9), and no repair of this slice's rules is.
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
        ("fix", Json::Null),
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
