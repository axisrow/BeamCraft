// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The V12 view-book session format: the named views and their folders between engine types and
//! the JSON document the CLI and the window persist beside a store. Mapping only — every
//! admission rule lives in `beamcraft_engine::views`. Temporary views are session state and are
//! never written.

use beamcraft_engine::views::{ModelView, Projection, ViewBook, WorkArea};
use beamcraft_engine::work_plane::WorkPlane;
use beamcraft_engine::{ElementKind, KindFilter, RenderMode};
use beamcraft_json::Json;
use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::Uuid7;

/// The format version this module writes and reads.
pub const VERSION: u64 = 1;

fn unread(field: &'static str) -> CommandError {
    CommandError::new(
        ErrorCode::InvalidArgument,
        field,
        "view file field is unreadable",
    )
}

fn numbers<const N: usize>(json: &Json, key: &'static str) -> Result<[f64; N], CommandError> {
    let values = json
        .get(key)
        .and_then(Json::array)
        .map_err(|_| unread(key))?
        .iter()
        .map(|v| v.float().map_err(|_| unread(key)))
        .collect::<Result<Vec<_>, _>>()?;
    <[f64; N]>::try_from(values).map_err(|_| unread(key))
}

fn optional<'a>(json: &'a Json, key: &str) -> Option<&'a Json> {
    json.get(key).ok().filter(|v| !matches!(v, Json::Null))
}

fn number(value: f64, field: &'static str) -> Result<Json, CommandError> {
    Json::number(value).map_err(|_| unread(field))
}

fn array(values: &[f64], field: &'static str) -> Result<Json, CommandError> {
    Ok(Json::Array(
        values
            .iter()
            .map(|v| number(*v, field))
            .collect::<Result<_, _>>()?,
    ))
}

/// A view plane as JSON: origin and the two in-plane axes.
pub fn plane_to_json(plane: &WorkPlane) -> Result<Json, CommandError> {
    Ok(Json::object([
        ("origin_mm", array(&plane.origin(), "views.plane")?),
        ("x", array(&plane.x(), "views.plane")?),
        ("y", array(&plane.y(), "views.plane")?),
    ]))
}

fn plane_from_json(json: &Json) -> Result<WorkPlane, CommandError> {
    WorkPlane::new(
        numbers(json, "origin_mm")?,
        numbers(json, "x")?,
        numbers(json, "y")?,
    )
}

/// A work area as JSON.
pub fn work_area_to_json(area: &WorkArea) -> Result<Json, CommandError> {
    Ok(Json::object([
        ("min", array(&area.min, "views.work_area")?),
        ("max", array(&area.max, "views.work_area")?),
    ]))
}

fn kinds_to_json(filter: KindFilter) -> Json {
    Json::Array(
        filter
            .hidden_kinds()
            .into_iter()
            .map(|k| Json::text(k.as_token()))
            .collect(),
    )
}

fn kinds_from_json(json: &Json) -> Result<KindFilter, CommandError> {
    let mut filter = KindFilter::SHOW_ALL;
    for token in json
        .get("hidden_kinds")
        .and_then(Json::array)
        .map_err(|_| unread("views.hidden_kinds"))?
    {
        let kind = token
            .string()
            .ok()
            .and_then(ElementKind::parse)
            .ok_or_else(|| unread("views.hidden_kinds"))?;
        filter = filter.hiding(kind);
    }
    Ok(filter)
}

/// One view as JSON, in the session file's element shape.
pub fn view_to_json(view: &ModelView) -> Result<Json, CommandError> {
    Ok(Json::object([
        ("id", Json::text(view.id.to_string())),
        ("name", Json::text(&view.name)),
        ("plane", plane_to_json(&view.plane)?),
        ("projection", Json::text(view.projection.as_str())),
        ("span_mm", number(view.span_mm, "views.span")?),
        (
            "depth_mm",
            match view.depth {
                Some(depth) => array(&depth, "views.depth")?,
                None => Json::Null,
            },
        ),
        ("render_mode", Json::text(view.render_mode.as_token())),
        ("grids_visible", Json::Bool(view.grids_visible)),
        ("folder", Json::text(&view.folder)),
        ("favorite", Json::Bool(view.favorite)),
        ("hidden_kinds", kinds_to_json(view.hidden_kinds)),
        (
            "work_area",
            match view.work_area.as_ref() {
                Some(area) => work_area_to_json(area)?,
                None => Json::Null,
            },
        ),
        ("rotation_rad", array(&view.rotation_rad, "views.rotation")?),
        (
            "color",
            match view.color {
                Some(color) => array(&color, "views.color")?,
                None => Json::Null,
            },
        ),
        ("opacity", number(view.opacity, "views.opacity")?),
    ]))
}

/// Reads one view; every key is required, `depth_mm`, `work_area` and `color` may be null.
pub fn view_from_json(json: &Json) -> Result<ModelView, CommandError> {
    let text = |key: &'static str| {
        json.get(key)
            .and_then(Json::string)
            .map_err(|_| unread(key))
    };
    let flag = |key: &'static str| {
        json.get(key)
            .and_then(Json::boolean)
            .map_err(|_| unread(key))
    };
    let id = Uuid7::parse(text("id")?).map_err(|_| unread("id"))?;
    let plane = plane_from_json(json.get("plane").map_err(|_| unread("plane"))?)?;
    let span = json
        .get("span_mm")
        .and_then(Json::float)
        .map_err(|_| unread("span_mm"))?;
    let mut view = ModelView::new(id, text("name")?, plane, span);
    view.projection = Projection::parse(text("projection")?)?;
    view.depth = match optional(json, "depth_mm") {
        Some(_) => Some(numbers(json, "depth_mm")?),
        None => None,
    };
    view.render_mode =
        RenderMode::parse(text("render_mode")?).ok_or_else(|| unread("render_mode"))?;
    view.grids_visible = flag("grids_visible")?;
    text("folder")?.clone_into(&mut view.folder);
    view.favorite = flag("favorite")?;
    view.hidden_kinds = kinds_from_json(json)?;
    view.work_area = match optional(json, "work_area") {
        Some(area) => Some(WorkArea::new(numbers(area, "min")?, numbers(area, "max")?)?),
        None => None,
    };
    view.rotation_rad = numbers(json, "rotation_rad")?;
    view.color = match optional(json, "color") {
        Some(_) => Some(numbers(json, "color")?),
        None => None,
    };
    view.opacity = json
        .get("opacity")
        .and_then(Json::float)
        .map_err(|_| unread("opacity"))?;
    view.validate()?;
    Ok(view)
}

/// The session file as JSON: the folders and every saved (not temporary) view.
pub fn book_to_json(book: &ViewBook) -> Result<Json, CommandError> {
    let views = book
        .views()
        .iter()
        .filter(|v| !v.temporary)
        .map(view_to_json)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json::object([
        ("version", Json::uint(VERSION)),
        (
            "folders",
            Json::Array(book.folders().map(Json::text).collect()),
        ),
        ("views", Json::Array(views)),
    ]))
}

/// Reads a book from the session file's JSON; anything malformed, another version, or past a
/// limit refuses whole.
pub fn book_from_json(json: &Json) -> Result<ViewBook, CommandError> {
    let version = json
        .get("version")
        .and_then(Json::unsigned)
        .map_err(|_| unread("version"))?;
    if version != VERSION {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "views.version",
            "the view file has another format version",
        ));
    }
    let list = |key: &'static str| json.get(key).and_then(Json::array).map_err(|_| unread(key));
    let (views, folders) = (list("views")?, list("folders")?);
    if views.len() > beamcraft_engine::views::MAX_VIEWS
        || folders.len() > beamcraft_engine::views::MAX_FOLDERS
    {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "views.count",
            "the view file exceeds the view or folder limit",
        ));
    }
    let folders = folders
        .iter()
        .map(|f| f.string().map(str::to_owned).map_err(|_| unread("folders")))
        .collect::<Result<Vec<_>, _>>()?;
    let views = views
        .iter()
        .map(view_from_json)
        .collect::<Result<Vec<_>, _>>()?;
    ViewBook::from_parts(views, folders)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn view(n: u64, name: &str) -> ModelView {
        let id = Uuid7::parse(&format!("019a7bf0-0000-7000-8000-{n:012}")).unwrap();
        ModelView::new(id, name, WorkPlane::default(), 4000.0)
    }

    #[test]
    fn the_book_round_trips_through_json() {
        let mut full = view(1, "Section A");
        full.plane =
            WorkPlane::three_points([10.0, 0.0, 0.0], [10.0, 5.0, 0.0], [10.0, 0.0, 3.0]).unwrap();
        full.projection = Projection::Perspective;
        full.depth = Some([500.0, 250.0]);
        full.render_mode = RenderMode::Wireframe;
        full.grids_visible = false;
        full.folder = "Sections/Main".into();
        full.favorite = true;
        full.hidden_kinds = KindFilter::SHOW_ALL.hiding(ElementKind::Grid);
        full.work_area = Some(WorkArea::new([0.0; 3], [100.0, 200.0, 300.0]).unwrap());
        full.rotation_rad = [0.5, -0.25];
        full.color = Some([0.25, 0.5, 0.75]);
        full.opacity = 0.4;
        let mut temp = view(2, "Scratch");
        temp.temporary = true;
        let mut book = ViewBook::new();
        book.insert(full.clone()).unwrap();
        book.insert(view(3, "Plan")).unwrap();
        book.insert(temp).unwrap();
        book.create_folder("Empty").unwrap();
        let back = book_from_json(&book_to_json(&book).unwrap()).unwrap();
        assert_eq!(back.len(), 2, "the temporary view is not written");
        assert_eq!(back.get(&full.id).unwrap(), &full);
        assert_eq!(
            back.folders().collect::<Vec<_>>(),
            ["Empty", "Sections", "Sections/Main"]
        );
    }

    #[test]
    fn malformed_files_refuse_whole() {
        let good = book_to_json(&ViewBook::new()).unwrap();
        assert!(book_from_json(&good).unwrap().is_empty());
        let wrong_version = Json::object([
            ("version", Json::uint(2)),
            ("folders", Json::Array(vec![])),
            ("views", Json::Array(vec![])),
        ]);
        assert!(book_from_json(&wrong_version).is_err());
        let mut broken = view_to_json(&view(1, "A")).unwrap();
        if let Json::Object(map) = &mut broken {
            map.insert("opacity".into(), Json::text("high"));
        }
        let file = Json::object([
            ("version", Json::uint(1)),
            ("folders", Json::Array(vec![])),
            ("views", Json::Array(vec![broken])),
        ]);
        assert!(book_from_json(&file).is_err());
        let twice = Json::object([
            ("version", Json::uint(1)),
            ("folders", Json::Array(vec![])),
            (
                "views",
                Json::Array(vec![
                    view_to_json(&view(1, "A")).unwrap(),
                    view_to_json(&view(2, "a")).unwrap(),
                ]),
            ),
        ]);
        assert!(book_from_json(&twice).is_err(), "names stay unique");
    }
}
