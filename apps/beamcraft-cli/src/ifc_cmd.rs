// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The IFC input commands: `ifc-inspect` and `ifc-shapes`, both read-only over one
//! received IFC file. `ifc-inspect` prints one JSON object pinning what the bytes declare
//! (XR-SOURCE-002): the original-byte SHA-256 and size, the declared schema, the descriptive
//! header, the entity and GlobalId inventories, the declared length unit and the explicit
//! loss list. `ifc-shapes` reads the placed triangle bodies: products, placement chains,
//! tessellations and per-body counts and bounds in millimetres. Neither opens a store,
//! writes anything or converts anything into BeamCraft's own model; the same bytes answer
//! with the same JSON.

use crate::plate_cmd::{invalid, num};
use crate::report::report;
use beamcraft_commands::source_file::read_file;
use beamcraft_ifc::read::{self, Inspection, ReadError, UnitsReading};
use beamcraft_ifc::shapes::{self, Bounds, ShapeBody, Shapes};
use beamcraft_json::Json;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::reference::check_source_path;
use session::Refusal;
use std::ffi::OsString;

type Run<T> = Result<T, Refusal>;

/// Runs one ifc command and returns the JSON result and the process exit code.
pub fn execute(args: &[OsString]) -> (Json, u8) {
    report(run(args))
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
    match *command {
        "ifc-inspect" => with_source(tail, |bytes| match read::inspect(&bytes) {
            Ok(inspection) => inspection_json(&inspection),
            Err(ReadError::NotAStepFile(why)) => Err(Refusal {
                rule_id: None,
                code: ErrorCode::InvalidArgument,
                message: why,
            }),
            Err(ReadError::TooLarge) => Err(Refusal {
                rule_id: None,
                code: ErrorCode::LimitExceeded,
                message: "the file is larger than the 32 MiB input cap",
            }),
        }),
        "ifc-shapes" => with_source(tail, |bytes| match shapes::read(&bytes) {
            Ok(shapes) => shapes_json(&shapes),
            Err(ReadError::NotAStepFile(why)) => Err(Refusal {
                rule_id: None,
                code: ErrorCode::InvalidArgument,
                message: why,
            }),
            Err(ReadError::TooLarge) => Err(Refusal {
                rule_id: None,
                code: ErrorCode::LimitExceeded,
                message: "the file is larger than the 32 MiB input cap",
            }),
        }),
        _ => Err(invalid("unknown command")),
    }
}

/// Reads `--source FILE` from the command tail, applies the bounded reader and hands the
/// bytes to `read_bytes`.
fn with_source(tail: &[&str], read_bytes: impl FnOnce(Vec<u8>) -> Run<Json>) -> Run<Json> {
    let &[key, source] = tail else {
        return Err(invalid("the command takes exactly --source FILE"));
    };
    if key != "--source" {
        return Err(invalid("unknown option"));
    }
    // Before the file system is touched: a URL or a network path is refused on its text alone.
    check_source_path(source).map_err(Refusal::from)?;
    let cap = usize::try_from(read::MAX_INPUT_BYTES).map_err(|_| invalid("input cap"))?;
    let bytes = read_file(source, cap).map_err(|code| Refusal {
        rule_id: None,
        code,
        message: "the source file is not readable",
    })?;
    read_bytes(bytes)
}

fn inspection_json(out: &Inspection) -> Run<Json> {
    let header = match &out.header {
        None => Json::Null,
        Some(facts) => Json::object([
            ("file_name", string_or_null(facts.file_name.as_deref())),
            ("time_stamp", string_or_null(facts.time_stamp.as_deref())),
            (
                "preprocessor_version",
                string_or_null(facts.preprocessor_version.as_deref()),
            ),
            (
                "originating_system",
                string_or_null(facts.originating_system.as_deref()),
            ),
        ]),
    };
    let names = out
        .entity_names
        .iter()
        .map(|(name, count)| (name.clone(), Json::uint(*count)))
        .collect();
    let samples = out
        .duplicate_samples
        .iter()
        .map(|sample| {
            Json::object([
                ("instance", Json::uint(sample.instance)),
                ("global_id", Json::text(sample.global_id.clone())),
            ])
        })
        .collect();
    let length_unit = match out.length_unit {
        None => Json::Null,
        Some(UnitsReading {
            meters_per_unit,
            origin,
        }) => Json::object([
            ("meters_per_unit", num(meters_per_unit)?),
            ("origin", Json::text(origin)),
        ]),
    };
    let losses = out
        .losses
        .iter()
        .map(|loss| {
            Json::object([
                ("code", Json::text(loss.code)),
                ("detail", Json::text(loss.detail.clone())),
            ])
        })
        .collect();
    Ok(Json::object([
        ("outcome", Json::text("ifc_inspected")),
        (
            "source",
            Json::object([
                ("bytes", Json::uint(out.bytes)),
                ("sha256", Json::text(out.digest.to_string())),
            ]),
        ),
        (
            "declared_schema",
            Json::object([
                ("token", string_or_null(out.schema_token.as_deref())),
                ("recognized", Json::Bool(out.schema_recognized)),
            ]),
        ),
        ("header", header),
        (
            "entities",
            Json::object([
                ("count", Json::uint(out.entity_count)),
                ("names", Json::Object(names)),
                ("unclassified", Json::uint(out.unclassified_instances)),
                ("non_instance_lines", Json::uint(out.non_instance_lines)),
            ]),
        ),
        (
            "global_ids",
            Json::object([
                ("count", Json::uint(out.global_id_count)),
                ("distinct", Json::uint(out.distinct_global_ids)),
                ("duplicates", Json::uint(out.duplicate_global_ids)),
                ("duplicate_samples", Json::Array(samples)),
                (
                    "project_without_global_id",
                    Json::Bool(out.project_without_global_id),
                ),
            ]),
        ),
        ("length_unit", length_unit),
        (
            "conversion_based_length_units",
            Json::Array(
                out.conversion_based_length_units
                    .iter()
                    .map(|name| Json::text(name.clone()))
                    .collect(),
            ),
        ),
        ("losses", Json::Array(losses)),
        ("losses_truncated", Json::Bool(out.losses_truncated)),
    ]))
}

fn string_or_null(text: Option<&str>) -> Json {
    match text {
        Some(text) => Json::text(text),
        None => Json::Null,
    }
}

/// One listed shape: identity facts, counts and the placed bounds. The vertices themselves
/// stay out of the reply; the bounds and counts are the checkable facts.
fn shape_json(shape: &ShapeBody) -> Run<Json> {
    let bounds = |bounds: &Bounds| -> Run<Json> {
        let axis = |values: &[f32; 3]| -> Run<Json> {
            Ok(Json::Array(
                values
                    .iter()
                    .map(|v| num(f64::from(*v)))
                    .collect::<Run<Vec<_>>>()?,
            ))
        };
        Ok(Json::object([
            ("min", axis(&bounds.min)?),
            ("max", axis(&bounds.max)?),
        ]))
    };
    Ok(Json::object([
        ("instance", Json::uint(shape.instance)),
        ("entity", Json::text(shape.entity.clone())),
        ("global_id", string_or_null(shape.global_id.as_deref())),
        ("name", string_or_null(shape.name.as_deref())),
        ("closed", Json::Bool(shape.closed)),
        ("vertices", Json::uint(shape.vertices_mm.len() as u64)),
        ("triangles", Json::uint(shape.triangles.len() as u64)),
        ("bounds", bounds(&shape.bounds_mm)?),
    ]))
}

fn shapes_json(out: &Shapes) -> Run<Json> {
    let listed = out.shapes.iter().map(shape_json).collect::<Run<Vec<_>>>()?;
    let losses = out
        .losses
        .iter()
        .map(|loss| {
            Json::object([
                ("code", Json::text(loss.code)),
                ("detail", Json::text(loss.detail.clone())),
            ])
        })
        .collect();
    let length_unit = match out.length_unit {
        None => Json::Null,
        Some(UnitsReading {
            meters_per_unit,
            origin,
        }) => Json::object([
            ("meters_per_unit", num(meters_per_unit)?),
            ("origin", Json::text(origin)),
        ]),
    };
    Ok(Json::object([
        ("outcome", Json::text("ifc_shapes_read")),
        (
            "source",
            Json::object([
                ("bytes", Json::uint(out.bytes)),
                ("sha256", Json::text(out.digest.to_string())),
            ]),
        ),
        (
            "declared_schema",
            Json::object([
                ("token", string_or_null(out.schema_token.as_deref())),
                ("recognized", Json::Bool(out.schema_recognized)),
            ]),
        ),
        ("length_unit", length_unit),
        (
            "products",
            Json::object([("count", Json::uint(out.product_count))]),
        ),
        (
            "shapes",
            Json::object([
                ("count", Json::uint(out.shape_count)),
                ("vertices", Json::uint(out.vertex_total)),
                ("triangles", Json::uint(out.triangle_total)),
                ("listed", Json::Array(listed)),
                ("listed_count", Json::uint(out.shapes.len() as u64)),
                ("truncated", Json::Bool(out.shapes_truncated)),
            ]),
        ),
        ("losses", Json::Array(losses)),
        ("losses_truncated", Json::Bool(out.losses_truncated)),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_valid_file_is_pinned_and_a_missing_file_is_refused() {
        let file = [
            "ISO-10303-21;",
            "HEADER;",
            "FILE_NAME('w.ifc','2026-11-05T00:00:00',('a'),('b'),'c','d','e');",
            "FILE_SCHEMA(('IFC4'));",
            "ENDSEC;",
            "DATA;",
            "#1=IFCPROJECT('0YvucVMOX9kw8jhtQ9abcd',*,$,$,$,$,$,$,#2);",
            "#2=IFCUNITASSIGNMENT((#3));",
            "#3=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);",
            "ENDSEC;",
            "END-ISO-10303-21;",
            "",
        ]
        .join("\n");
        let out = read::inspect(file.as_bytes()).ok().unwrap();
        let json = inspection_json(&out).ok().unwrap();
        assert_eq!(json.get("outcome").ok(), Some(&Json::text("ifc_inspected")));
        let refusal = run(&[
            "ifc-inspect".into(),
            "--source".into(),
            "/definitely/not/a/step/file.ifc".into(),
        ]);
        assert_eq!(refusal.unwrap_err().code, ErrorCode::NotFound);
    }

    #[test]
    fn a_shapes_file_answers_with_placed_bodies() {
        let file = [
            "ISO-10303-21;",
            "HEADER;",
            "FILE_SCHEMA(('IFC4'));",
            "ENDSEC;",
            "DATA;",
            "#1=IFCPROJECT('0YvucVMOX9kw8jhtQ9abcd',*,$,$,$,$,$,$,#2);",
            "#2=IFCUNITASSIGNMENT((#3));",
            "#3=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);",
            "#4=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#5,$);",
            "#5=IFCAXIS2PLACEMENT3D(#6,$,$);",
            "#6=IFCCARTESIANPOINT((0.,0.,0.));",
            "#10=IFCPLATE('1Anr5VMOX0PO8jvHqZabcd',*,'PL-A',$,$,#11,#12,$,.NOTDEFINED.);",
            "#11=IFCLOCALPLACEMENT($,#5);",
            "#12=IFCPRODUCTDEFINITIONSHAPE($,$,(#13));",
            "#13=IFCSHAPEREPRESENTATION(#4,'Body','Tessellation',(#14));",
            "#14=IFCTRIANGULATEDFACESET(#15,$,.T.,((1,2,3)),$);",
            "#15=IFCCARTESIANPOINTLIST3D(((0.,0.,0.),(100.,0.,0.),(0.,80.,0.)));",
            "ENDSEC;",
            "END-ISO-10303-21;",
            "",
        ]
        .join("\n");
        let out = shapes::read(file.as_bytes()).ok().unwrap();
        let json = shapes_json(&out).ok().unwrap();
        assert_eq!(
            json.get("outcome").ok(),
            Some(&Json::text("ifc_shapes_read"))
        );
        let listed = json
            .get("shapes")
            .and_then(|shapes| shapes.get("listed"))
            .and_then(|listed| listed.array())
            .ok()
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(
            json.get("shapes")
                .and_then(|shapes| shapes.get("count"))
                .and_then(|count| count.unsigned())
                .ok(),
            Some(1)
        );
        let refusal = run(&[
            "ifc-shapes".into(),
            "--source".into(),
            "/definitely/not/a/step/file.ifc".into(),
        ]);
        assert_eq!(refusal.unwrap_err().code, ErrorCode::NotFound);
    }
}
