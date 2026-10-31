// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The V12 named-view session format: the saved-view book between engine types and the JSON
//! document the CLI and the window persist beside a store. Mapping only — every admission
//! rule lives in `beamcraft_engine::views`.

use beamcraft_engine::views::{self, NamedView, PlaneSpec, ViewBook, ViewPose, WorkArea};
use beamcraft_json::Json;
use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;

fn bad(code: ErrorCode, field: &'static str, message: &'static str) -> CommandError {
    CommandError::new(code, field, message)
}

fn unread(field: &'static str) -> CommandError {
    bad(
        ErrorCode::InvalidArgument,
        field,
        "view file field is unreadable",
    )
}

/// Three finite numbers from a JSON array, or a refusal.
fn point3(json: &Json, key: &'static str) -> Result<[f64; 3], CommandError> {
    let array = json
        .get(key)
        .and_then(Json::array)
        .map_err(|_| unread(key))?;
    let values = array
        .iter()
        .map(|v| v.float().map_err(|_| unread(key)))
        .collect::<Result<Vec<_>, _>>()?;
    <[f64; 3]>::try_from(values).map_err(|_| unread(key))
}

/// The pose as session-file JSON.
pub fn pose_to_json(pose: &ViewPose) -> Result<Json, CommandError> {
    // Values are validated finite, so number() cannot refuse here.
    let point = |v: [f64; 3]| Json::Array(v.iter().filter_map(|v| Json::number(*v).ok()).collect());
    Ok(Json::object([
        ("target_mm", point(pose.target_mm)),
        (
            "yaw",
            Json::number(pose.yaw).map_err(|_| unread("views.pose"))?,
        ),
        (
            "pitch",
            Json::number(pose.pitch).map_err(|_| unread("views.pose"))?,
        ),
        (
            "distance_mm",
            Json::number(pose.distance_mm).map_err(|_| unread("views.pose"))?,
        ),
        ("orthographic", Json::Bool(pose.orthographic)),
    ]))
}

fn pose_from_json(json: &Json) -> Result<ViewPose, CommandError> {
    let number = |key: &'static str| json.get(key).and_then(Json::float).map_err(|_| unread(key));
    ViewPose::new(
        point3(json, "target_mm")?,
        number("yaw")?,
        number("pitch")?,
        number("distance_mm")?,
        json.get("orthographic")
            .and_then(Json::boolean)
            .map_err(|_| unread("orthographic"))?,
    )
}

/// The plane spec as session-file JSON.
pub fn plane_to_json(spec: &PlaneSpec) -> Json {
    let text = |v: &str| Json::text(v.to_owned());
    let point = |v: [f64; 3]| Json::Array(v.iter().filter_map(|v| Json::number(*v).ok()).collect());
    match spec {
        PlaneSpec::GridLine {
            grid,
            line,
            reverse,
        } => Json::object([(
            "grid_line",
            Json::object([
                ("grid", text(grid)),
                ("line", text(line)),
                ("reverse", Json::Bool(*reverse)),
            ]),
        )]),
        PlaneSpec::Level => Json::object([("level", Json::object([]))]),
        PlaneSpec::TwoPoints { from_mm, to_mm } => Json::object([(
            "two_points",
            Json::object([("from_mm", point(*from_mm)), ("to_mm", point(*to_mm))]),
        )]),
    }
}

fn plane_from_json(json: &Json) -> Result<PlaneSpec, CommandError> {
    if let Ok(inner) = json.get("grid_line") {
        let text = |key: &'static str| {
            inner
                .get(key)
                .and_then(Json::string)
                .map_err(|_| unread("views.plane"))
                .map(str::to_owned)
        };
        let reverse = inner
            .get("reverse")
            .and_then(Json::boolean)
            .map_err(|_| unread("views.plane"))?;
        return Ok(PlaneSpec::GridLine {
            grid: text("grid")?,
            line: text("line")?,
            reverse,
        });
    }
    if json.get("level").is_ok() {
        return Ok(PlaneSpec::Level);
    }
    if let Ok(inner) = json.get("two_points") {
        return Ok(PlaneSpec::TwoPoints {
            from_mm: point3(inner, "from_mm")?,
            to_mm: point3(inner, "to_mm")?,
        });
    }
    Err(bad(
        ErrorCode::InvalidArgument,
        "views.plane",
        "plane spec kind is unknown",
    ))
}

/// The box as session-file JSON.
pub fn work_area_to_json(area: &WorkArea) -> Result<Json, CommandError> {
    let point = |v: [f64; 3]| Json::Array(v.iter().filter_map(|v| Json::number(*v).ok()).collect());
    Ok(Json::object([
        ("min", point(area.min)),
        ("max", point(area.max)),
    ]))
}

fn work_area_from_json(json: &Json) -> Result<WorkArea, CommandError> {
    WorkArea::new(point3(json, "min")?, point3(json, "max")?)
}

/// One saved view as JSON, in the session file's element shape.
pub fn view_to_json(view: &NamedView) -> Result<Json, CommandError> {
    Ok(Json::object([
        ("name", Json::text(&view.name)),
        (
            "plane",
            view.plane.as_ref().map_or(Json::Null, plane_to_json),
        ),
        ("pose", pose_to_json(&view.pose)?),
        (
            "work_area",
            match view.work_area.as_ref() {
                Some(area) => work_area_to_json(area)?,
                None => Json::Null,
            },
        ),
    ]))
}

/// The session file as JSON, one bounded object.
pub fn book_to_json(book: &ViewBook) -> Result<Json, CommandError> {
    let views = book
        .iter()
        .map(view_to_json)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json::object([
        ("version", Json::uint(1)),
        ("views", Json::Array(views)),
    ]))
}

/// Reads a book from the session file's JSON. Unknown object keys are ignored, so a newer
/// writer stays readable; anything malformed refuses whole.
pub fn book_from_json(json: &Json) -> Result<ViewBook, CommandError> {
    let views = json.get("views").and_then(Json::array).map_err(|_| {
        bad(
            ErrorCode::InvalidArgument,
            "views.file",
            "view file has no views array",
        )
    })?;
    if views.len() > views::MAX_VIEWS {
        return Err(bad(
            ErrorCode::LimitExceeded,
            "views.count",
            "view file exceeds the saved-view limit",
        ));
    }
    let mut book = ViewBook::new();
    for view in views {
        let name = view
            .get("name")
            .and_then(Json::string)
            .map_err(|_| unread("views.name"))?;
        let plane = match view.get("plane") {
            Ok(plane) if !matches!(plane, Json::Null) => Some(plane_from_json(plane)?),
            _ => None,
        };
        let work_area = match view.get("work_area") {
            Ok(area) if !matches!(area, Json::Null) => Some(work_area_from_json(area)?),
            _ => None,
        };
        let pose = pose_from_json(view.get("pose").map_err(|_| unread("views.pose"))?)?;
        book.save(NamedView::new(name, plane, pose, work_area)?)?;
    }
    Ok(book)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn pose() -> ViewPose {
        ViewPose::new([0.0; 3], 0.0, 0.1, 100.0, false).unwrap()
    }

    #[test]
    fn the_book_round_trips_through_json_and_refuses_malformed_files() {
        let mut book = ViewBook::new();
        book.save(
            NamedView::new(
                "Plan",
                Some(PlaneSpec::Level),
                pose(),
                Some(WorkArea::new([0.0; 3], [100.0; 3]).unwrap()),
            )
            .unwrap(),
        )
        .unwrap();
        book.save(NamedView::new("3D", None, pose(), None).unwrap())
            .unwrap();
        let json = book_to_json(&book).unwrap();
        let text = beamcraft_json::decode(
            &json.encode(&beamcraft_json::Limits::DEFAULT).unwrap(),
            &beamcraft_json::Limits::DEFAULT,
        )
        .unwrap();
        assert_eq!(book_from_json(&text).unwrap(), book);
        // A file without a views array and an overfull file refuse whole.
        assert!(book_from_json(&Json::object([("version", Json::uint(1))])).is_err());
        let mut full = ViewBook::new();
        for i in 0..views::MAX_VIEWS {
            full.save(NamedView::new(&format!("v{i}"), None, pose(), None).unwrap())
                .unwrap();
        }
        assert!(
            full.save(NamedView::new("over", None, pose(), None).unwrap())
                .is_err()
        );
        // Every plane kind keeps its shape.
        for spec in [
            PlaneSpec::Level,
            PlaneSpec::GridLine {
                grid: "G".into(),
                line: "A".into(),
                reverse: true,
            },
            PlaneSpec::TwoPoints {
                from_mm: [0.0; 3],
                to_mm: [1.0; 3],
            },
        ] {
            let view = NamedView::new("v", Some(spec.clone()), pose(), None).unwrap();
            let json = view_to_json(&view).unwrap();
            let read = book_from_json(&Json::object([
                ("version", Json::uint(1)),
                ("views", Json::Array(vec![json])),
            ]))
            .unwrap();
            assert_eq!(read.iter().next().unwrap().plane, Some(spec));
        }
    }
}
