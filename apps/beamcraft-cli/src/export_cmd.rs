// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! E02 headless commands: `export-ifc` and `ifc-guid`. They parse, call the engine-side adapter
//! `beamcraft_cli::export` and print one JSON object with sorted keys; they compute nothing
//! else (`IFCX-SURF-001`). A refused export prints the common rejection shape, with the `IFC_`
//! token and the status (`failed`, or `incomplete` for an unavailable input) in `diagnostics`.

use crate::plate_cmd::id_error;
use crate::report::report_with;
use beamcraft_cli::export::{
    CONTRACT_VERSION, ExportReport, ExportRequest, check_destinations, ifc_code_of,
    ifc_guid_decode, ifc_guid_of, prepare_export, validate_request,
};
use beamcraft_cli::plate::PlateError;
use beamcraft_host::{revision::RevisionStore, storage::BlobStore};
use beamcraft_ifc::EXPORTER_VERSION;
use beamcraft_json::Json;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::Uuid7;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::Path;

type Run<T> = Result<T, PlateError>;

fn invalid(message: &'static str) -> PlateError {
    PlateError {
        rule_id: None,
        code: ErrorCode::InvalidArgument,
        message,
    }
}

/// Runs one export command and prints its JSON result; returns the process exit code.
pub fn run_and_print(args: &[OsString]) -> std::process::ExitCode {
    use std::io::Write;
    let (json, exit) = execute(args);
    let mut text = render(&json);
    text.push('\n');
    if std::io::stdout().lock().write_all(text.as_bytes()).is_err() {
        return std::process::ExitCode::from(3);
    }
    std::process::ExitCode::from(exit)
}

/// The JSON text of a value built by this module: sorted keys (the object is a `BTreeMap`),
/// no whitespace. The tree is shallow by construction; anything deeper than 16 is `null`.
fn render(json: &Json) -> String {
    fn write(json: &Json, out: &mut String, depth: usize) {
        use std::fmt::Write;
        if depth > 16 {
            out.push_str("null");
            return;
        }
        match json {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Number(n) => out.push_str(n),
            Json::String(s) => {
                out.push('"');
                for c in s.chars() {
                    match c {
                        '"' => out.push_str("\\\""),
                        '\\' => out.push_str("\\\\"),
                        c if c < ' ' => {
                            let _ = write!(out, "\\u{:04x}", c as u32);
                        }
                        c => out.push(c),
                    }
                }
                out.push('"');
            }
            Json::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write(item, out, depth + 1);
                }
                out.push(']');
            }
            Json::Object(map) => {
                out.push('{');
                for (i, (key, value)) in map.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write(&Json::String(key.clone()), out, depth + 1);
                    out.push(':');
                    write(value, out, depth + 1);
                }
                out.push('}');
            }
        }
    }
    let mut out = String::new();
    write(json, &mut out, 0);
    out
}

/// Runs one export command and returns the JSON result and the process exit code.
fn execute(args: &[OsString]) -> (Json, u8) {
    match run(args) {
        Ok(json) => (json, 0),
        Err(e) => {
            let status = if e.code == ErrorCode::DependencyUnavailable {
                "incomplete"
            } else {
                "failed"
            };
            report_with(
                Err(e),
                vec![Json::object([
                    ("ifc_code", Json::text(ifc_code_of(e.code))),
                    ("status", Json::text(status)),
                ])],
            )
        }
    }
}

const FLAGS: [&str; 2] = ["--strict", "--overwrite"];

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
    let own: &[&str] = match *command {
        "export-ifc" => &[
            "--store",
            "--out",
            "--revision",
            "--project-name",
            "--schema",
            "--format",
            "--report",
            "--strict",
            "--overwrite",
        ],
        "ifc-guid" => &["--id", "--decode"],
        _ => return Err(invalid("unknown command")),
    };
    let mut options: BTreeMap<&str, &str> = BTreeMap::new();
    let mut rest = tail.iter();
    while let Some(&key) = rest.next() {
        if !own.contains(&key) {
            return Err(invalid("unknown option"));
        }
        // A flag may stand alone or take `true` or `false`.
        let value = if FLAGS.contains(&key) {
            let mut peek = rest.clone();
            match peek.next() {
                Some(&v) if v == "true" || v == "false" => {
                    rest.next();
                    v
                }
                _ => "true",
            }
        } else {
            rest.next()
                .copied()
                .ok_or_else(|| invalid("an option needs a value"))?
        };
        if options.insert(key, value).is_some() {
            return Err(invalid("repeated option"));
        }
    }
    if *command == "ifc-guid" {
        return guid(&options);
    }
    export(&options)
}

fn guid(options: &BTreeMap<&str, &str>) -> Run<Json> {
    match (options.get("--id"), options.get("--decode")) {
        (Some(id), None) => Ok(Json::object([
            ("outcome", Json::text("ifc_guid")),
            ("element_id", Json::text(*id)),
            ("ifc_global_id", Json::text(ifc_guid_of(id)?)),
        ])),
        (None, Some(global)) => Ok(Json::object([
            ("outcome", Json::text("ifc_guid")),
            ("ifc_global_id", Json::text(*global)),
            ("uuid", Json::text(ifc_guid_decode(global)?)),
        ])),
        _ => Err(invalid("exactly one of --id and --decode is required")),
    }
}

fn flag(options: &BTreeMap<&str, &str>, key: &str) -> bool {
    options.get(key).is_some_and(|v| *v == "true")
}

fn export(options: &BTreeMap<&str, &str>) -> Run<Json> {
    let store_root = options
        .get("--store")
        .ok_or_else(|| invalid("--store is required"))?;
    let out = options
        .get("--out")
        .ok_or_else(|| invalid("--out is required"))?;
    let request = ExportRequest {
        contract_version: CONTRACT_VERSION.into(),
        schema: options.get("--schema").copied().unwrap_or("IFC4").into(),
        format: options.get("--format").copied().unwrap_or("step").into(),
        revision: options
            .get("--revision")
            .map(|text| Uuid7::parse(text).map_err(|_| id_error(beamcraft_model::id::IdError)))
            .transpose()?,
        project_name: options.get("--project-name").map(|s| (*s).to_owned()),
        strict: flag(options, "--strict"),
        overwrite: flag(options, "--overwrite"),
    };
    validate_request(&request)?;
    let blobs = BlobStore::new(store_root).map_err(|e| PlateError {
        rule_id: None,
        code: e.code(),
        message: "store root is not usable",
    })?;
    let store = RevisionStore::new(blobs);
    let report_path = options.get("--report").map(Path::new);
    check_destinations(&store, Path::new(out), report_path)?;
    // Both files are written and synced before either is installed, so a refused report leaves
    // the destinations as they were.
    let prepared = prepare_export(&store, &request, Path::new(out))?;
    let json = report_json(&prepared.report)?;
    let staged_report = report_path
        .map(|path| {
            let encoded = render(&json).into_bytes();
            beamcraft_host::output::stage_file(path, request.overwrite, |file| {
                use std::io::Write;
                file.write_all(&encoded)
                    .map_err(|_| beamcraft_host::output::OutputError {
                        code: ErrorCode::BackendUnavailable,
                        message: "the report could not be written",
                    })
            })
            .map_err(|e| PlateError {
                rule_id: None,
                code: e.code,
                message: e.message,
            })
        })
        .transpose()?;
    let installed = prepared.install()?;
    if let Some(staged) = staged_report
        && let Err(e) = staged.install()
    {
        installed.undo();
        return Err(PlateError {
            rule_id: None,
            code: e.code,
            message: e.message,
        });
    }
    Ok(json)
}

fn opt(text: Option<&str>) -> Json {
    text.map_or(Json::Null, Json::text)
}

fn report_json(report: &ExportReport) -> Run<Json> {
    let findings = report
        .findings
        .iter()
        .map(|f| {
            Json::object([
                ("code", Json::text(f.code)),
                ("f04_code", opt(f.f04_code.map(ErrorCode::as_str))),
                ("severity", Json::text(f.severity.as_str())),
                (
                    "element_id",
                    opt(f.element_id.map(|e| e.to_string()).as_deref()),
                ),
                ("field", Json::text(f.field)),
                ("message", Json::text(f.message)),
            ])
        })
        .collect();
    let levels = report
        .levels
        .iter()
        .map(|(id, global)| {
            Json::object([
                ("level_id", Json::text(id.to_string())),
                ("ifc_global_id", Json::text(global.clone())),
            ])
        })
        .collect();
    let elements = report
        .elements
        .iter()
        .map(|e| {
            Json::object([
                ("element_id", Json::text(e.element_id.to_string())),
                ("ifc_global_id", opt(e.ifc_global_id.as_deref())),
                ("kind", Json::text(e.kind.clone())),
                ("exported", Json::Bool(e.exported)),
                ("entity", opt(e.entity)),
                ("representation", opt(e.representation)),
                ("volume", opt(e.volume)),
                ("mass", opt(e.mass)),
                (
                    "omissions",
                    Json::Array(e.omissions.iter().map(|o| Json::text(*o)).collect()),
                ),
            ])
        })
        .collect();
    let count = |n: usize| Json::uint(n as u64);
    let output = match &report.output {
        Some(o) => Json::object([
            ("bytes", Json::uint(o.bytes)),
            ("sha256", Json::text(o.sha256.to_string())),
        ]),
        None => Json::Null,
    };
    Ok(Json::object([
        ("outcome", Json::text("exported")),
        ("contract_version", Json::text(CONTRACT_VERSION)),
        ("schema", Json::text("IFC4")),
        ("exporter_version", Json::text(EXPORTER_VERSION)),
        ("project_id", Json::text(report.project_id.to_string())),
        ("revision_id", Json::text(report.revision_id.to_string())),
        ("status", Json::text(report.status.as_str())),
        ("primary_code", Json::Null),
        ("incomplete", Json::Bool(false)),
        ("total_findings", count(report.total_findings)),
        ("truncated", Json::Bool(report.truncated)),
        ("findings", Json::Array(findings)),
        (
            "counts",
            Json::object([
                ("elements_total", count(report.counts.elements_total)),
                ("exported", count(report.counts.exported)),
                ("not_exported", count(report.counts.not_exported)),
                ("proxies", count(report.counts.proxies)),
                (
                    "without_representation",
                    count(report.counts.without_representation),
                ),
            ]),
        ),
        ("levels", Json::Array(levels)),
        ("elements", Json::Array(elements)),
        ("output", output),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_beyond_the_control_message_bounds_is_still_rendered() {
        // 20,000 rows of ten values are far beyond the 65,536 tokens of the control messages;
        // the report of a large model must not turn into LIMIT_EXCEEDED after its file exists.
        let rows: Vec<Json> = (0..20_000)
            .map(|i| {
                Json::object([
                    ("element_id", Json::text(format!("id-{i}"))),
                    ("exported", Json::Bool(true)),
                    ("omissions", Json::Array(vec![Json::text("mass")])),
                    ("volume", Json::Null),
                    ("a", Json::uint(1)),
                    ("b", Json::uint(2)),
                ])
            })
            .collect();
        let report = Json::object([("elements", Json::Array(rows))]);
        assert!(report.encode(&beamcraft_json::Limits::DEFAULT).is_err());
        let text = render(&report);
        assert!(text.starts_with("{\"elements\":[{\"a\":1,\"b\":2,\"element_id\":\"id-0\""));
        assert!(text.len() > 1_000_000);
    }

    #[test]
    fn strings_are_escaped_and_keys_sorted() {
        let json = Json::object([
            ("b", Json::text("quote \" slash \\ tab \t é")),
            ("a", Json::Null),
        ]);
        assert_eq!(
            render(&json),
            "{\"a\":null,\"b\":\"quote \\\" slash \\\\ tab \\u0009 é\"}"
        );
    }

    #[test]
    fn a_structure_deeper_than_the_cap_renders_null_instead_of_recursing() {
        let mut json = Json::uint(1);
        for _ in 0..40 {
            json = Json::Array(vec![json]);
        }
        let text = render(&json);
        assert!(text.contains("null") && text.len() < 100);
    }
}
