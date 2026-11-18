// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The V12 view-book session format: the named views and their folders between engine types and
//! the JSON document the CLI and the window persist beside a store. Mapping only — every
//! admission rule lives in `beamcraft_engine::views`. Temporary views are session state and are
//! never written.

use beamcraft_engine::filter::Operator;
use beamcraft_engine::view_filter::{MAX_CONDITIONS, MAX_FILTERS, MAX_OVERRIDES};
use beamcraft_engine::views::{
    FilterCondition, FilterField, FilterMatch, FilterOverride, ModelView, Projection, ViewBook,
    ViewFilter, ViewProvenance, WorkArea,
};
use beamcraft_engine::work_plane::WorkPlane;
use beamcraft_engine::{ElementKind, KindFilter, RenderMode, Representation};
use beamcraft_json::Json;
use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::Uuid7;

/// The format version this module writes. Version 2 added the W01 provenance stamp; the view
/// filters and the views' filter overrides are optional keys of version 2, so files written
/// before them still read.
pub const VERSION: u64 = 2;
/// Version 1 predates the W01 provenance stamp; its files still read, without provenance.
const READ_VERSIONS: [u64; 2] = [1, VERSION];

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

/// A provenance stamp as JSON (W01).
pub fn provenance_to_json(provenance: &ViewProvenance) -> Result<Json, CommandError> {
    Ok(Json::object([
        ("author", Json::text(&provenance.author)),
        ("created_at_ms", Json::uint(provenance.created_at_ms)),
        ("comment", Json::text(&provenance.comment)),
        (
            "model_revision",
            Json::text(provenance.model_revision.to_string()),
        ),
    ]))
}

fn provenance_from_json(json: &Json) -> Result<ViewProvenance, CommandError> {
    let text = |key: &'static str| {
        json.get(key)
            .and_then(Json::string)
            .map(str::to_owned)
            .map_err(|_| unread(key))
    };
    let revision = Uuid7::parse(&text("model_revision")?).map_err(|_| unread("model_revision"))?;
    let created = json
        .get("created_at_ms")
        .and_then(Json::unsigned)
        .map_err(|_| unread("created_at_ms"))?;
    ViewProvenance::new(&text("author")?, &text("comment")?, revision, created)
        .map_err(|_| unread("views.provenance"))
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
        ("beyond", Json::text(view.beyond.as_str())),
        ("render_mode", Json::text(view.render_mode.as_token())),
        ("grids_visible", Json::Bool(view.grids_visible)),
        ("folder", Json::text(&view.folder)),
        ("favorite", Json::Bool(view.favorite)),
        ("default", Json::Bool(view.is_default)),
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
        ("overrides", overrides_to_json(&view.overrides)?),
        ("representation", Json::text(view.representation.as_token())),
        (
            "colour_set",
            match &view.colour_set {
                Some(set) => Json::text(set),
                None => Json::Null,
            },
        ),
        (
            "provenance",
            match &view.provenance {
                Some(provenance) => provenance_to_json(provenance)?,
                None => Json::Null,
            },
        ),
    ]))
}

fn color_to_json(color: Option<[f64; 3]>, field: &'static str) -> Result<Json, CommandError> {
    match color {
        Some(color) => array(&color, field),
        None => Ok(Json::Null),
    }
}

/// A view's or preset's filter overrides as JSON, in order.
pub fn overrides_to_json(overrides: &[FilterOverride]) -> Result<Json, CommandError> {
    Ok(Json::Array(
        overrides
            .iter()
            .map(|o| {
                Ok(Json::object([
                    ("filter", Json::text(&o.filter)),
                    ("visible", Json::Bool(o.visible)),
                    ("color", color_to_json(o.color, "views.override.color")?),
                    (
                        "transparency",
                        number(o.transparency, "views.override.transparency")?,
                    ),
                ]))
            })
            .collect::<Result<_, CommandError>>()?,
    ))
}

/// Reads a view's or preset's overrides; files written before them have none.
pub fn overrides_from_json(json: &Json) -> Result<Vec<FilterOverride>, CommandError> {
    let Some(list) = optional(json, "overrides") else {
        return Ok(Vec::new());
    };
    let list = list.array().map_err(|_| unread("overrides"))?;
    if list.len() > MAX_OVERRIDES {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "views.overrides",
            "a view has at most 16 filter overrides",
        ));
    }
    list.iter()
        .map(|o| {
            Ok(FilterOverride {
                filter: o
                    .get("filter")
                    .and_then(Json::string)
                    .map_err(|_| unread("overrides.filter"))?
                    .to_owned(),
                visible: o
                    .get("visible")
                    .and_then(Json::boolean)
                    .map_err(|_| unread("overrides.visible"))?,
                color: match optional(o, "color") {
                    Some(_) => Some(numbers(o, "color")?),
                    None => None,
                },
                transparency: o
                    .get("transparency")
                    .and_then(Json::float)
                    .map_err(|_| unread("overrides.transparency"))?,
            })
        })
        .collect()
}

/// One filter as JSON: its name, how its conditions combine and the rows.
pub fn filter_to_json(filter: &ViewFilter) -> Json {
    Json::object([
        ("name", Json::text(&filter.name)),
        ("match", Json::text(filter.matching.token())),
        (
            "conditions",
            Json::Array(
                filter
                    .conditions
                    .iter()
                    .map(|c| {
                        Json::object([
                            ("field", Json::text(c.field.token())),
                            ("op", Json::text(c.op.token())),
                            ("value", Json::text(&c.value)),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

/// Reads one filter written by [`filter_to_json`].
pub fn filter_from_json(json: &Json) -> Result<ViewFilter, CommandError> {
    let text = |j: &Json, key: &'static str| {
        j.get(key)
            .and_then(Json::string)
            .map(str::to_owned)
            .map_err(|_| unread(key))
    };
    let rows = json
        .get("conditions")
        .and_then(Json::array)
        .map_err(|_| unread("conditions"))?;
    if rows.len() > MAX_CONDITIONS {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "views.filter.conditions",
            "a filter has at most 16 conditions",
        ));
    }
    let conditions = rows
        .iter()
        .map(|row| {
            let op = text(row, "op")?;
            Ok(FilterCondition::new(
                FilterField::parse(&text(row, "field")?)?,
                Operator::parse(&op).ok_or_else(|| unread("op"))?,
                &text(row, "value")?,
            ))
        })
        .collect::<Result<Vec<_>, CommandError>>()?;
    let filter = ViewFilter {
        name: text(json, "name")?,
        matching: FilterMatch::parse(&text(json, "match")?)?,
        conditions,
    };
    filter.validate()?;
    Ok(filter)
}

/// the evaluator refuses one that does not fit the attribute's definition.
pub fn filter_rule(filter: &ViewFilter) -> Json {
    let comparisons = filter
        .conditions
        .iter()
        .map(|c| {
            let value = match &c.field {
                FilterField::Class => c
                    .value
                    .parse::<i32>()
                    .map_or_else(|_| Json::text(&c.value), |v| Json::Number(v.to_string())),
                FilterField::Attribute(_) => attribute_literal(&c.value),
                FilterField::Type | FilterField::Name => Json::text(&c.value),
            };
            let mut pairs = vec![
                (
                    "field",
                    Json::text(match c.field {
                        FilterField::Type => "type",
                        FilterField::Name => "name",
                        FilterField::Class => "class",
                        FilterField::Attribute(_) => "attribute",
                    }),
                ),
                ("op", Json::text(c.op.token())),
                ("value", value),
            ];
            if let FilterField::Attribute(name) = &c.field {
                pairs.push(("attribute", Json::text(name)));
            }
            Json::Object(pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
        })
        .collect();
    Json::object([(filter.matching.token(), Json::Array(comparisons))])
}

fn attribute_literal(value: &str) -> Json {
    match value {
        "true" => return Json::Bool(true),
        "false" => return Json::Bool(false),
        _ => {}
    }
    let plain = |text: &str| {
        text.parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .and_then(|v| Json::number(v).ok())
    };
    if let Some(number) = plain(value) {
        return number;
    }
    if let Some((amount, unit)) = value.rsplit_once(' ')
        && !unit.is_empty()
        && unit
            .chars()
            .all(|c| c.is_alphabetic() || c == '/' || c == '²')
        && let Some(number) = plain(amount.trim())
    {
        return Json::object([("value", number), ("unit", Json::text(unit))]);
    }
    Json::text(value)
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
    // Files written before the setting existed cut, as views did then.
    view.beyond = match optional(json, "beyond") {
        Some(_) => beamcraft_engine::views::DepthBeyond::parse(text("beyond")?)
            .map_err(|_| unread("beyond"))?,
        None => beamcraft_engine::views::DepthBeyond::Cut,
    };
    view.render_mode =
        RenderMode::parse(text("render_mode")?).ok_or_else(|| unread("render_mode"))?;
    view.grids_visible = flag("grids_visible")?;
    text("folder")?.clone_into(&mut view.folder);
    view.favorite = flag("favorite")?;
    // Files written before the default view existed have no such key.
    view.is_default = match optional(json, "default") {
        Some(_) => flag("default")?,
        None => false,
    };
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
    view.overrides = overrides_from_json(json)?;
    // The representation and the colour set postdate the provenance stamp, so files written
    // earlier read with the display defaults; a present key of another type still refuses.
    view.representation = match optional(json, "representation") {
        Some(_) => Representation::parse(text("representation")?)
            .ok_or_else(|| unread("representation"))?,
        None => Representation::Full,
    };
    view.colour_set = match optional(json, "colour_set") {
        Some(_) => Some(text("colour_set")?.to_owned()),
        None => None,
    };
    view.provenance = match optional(json, "provenance") {
        Some(stamp) => Some(provenance_from_json(stamp)?),
        None => None,
    };
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
        (
            "filters",
            Json::Array(book.filters().iter().map(filter_to_json).collect()),
        ),
    ]))
}

/// Reads a book from the session file's JSON; anything malformed, another version, or past a
/// limit refuses whole.
pub fn book_from_json(json: &Json) -> Result<ViewBook, CommandError> {
    let version = json
        .get("version")
        .and_then(Json::unsigned)
        .map_err(|_| unread("version"))?;
    if !READ_VERSIONS.contains(&version) {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "views.version",
            "the view file has another format version",
        ));
    }
    let list = |key: &'static str| json.get(key).and_then(Json::array).map_err(|_| unread(key));
    let (views, folders) = (list("views")?, list("folders")?);
    // The filters postdate the provenance stamp: an absent list is none.
    let filters: &[Json] = match optional(json, "filters") {
        Some(_) => list("filters")?,
        None => &[],
    };
    if views.len() > beamcraft_engine::views::MAX_VIEWS
        || folders.len() > beamcraft_engine::views::MAX_FOLDERS
        || filters.len() > MAX_FILTERS
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
    let filters = filters
        .iter()
        .map(filter_from_json)
        .collect::<Result<Vec<_>, _>>()?;
    ViewBook::from_all(views, folders, filters)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn id(n: u64) -> Uuid7 {
        Uuid7::parse(&format!("019a7bf0-0000-7000-8000-{n:012}")).unwrap()
    }

    fn view(n: u64, name: &str) -> ModelView {
        ModelView::new(id(n), name, WorkPlane::default(), 4000.0)
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
        full.is_default = true;
        full.hidden_kinds = KindFilter::SHOW_ALL.hiding(ElementKind::Grid);
        full.work_area = Some(WorkArea::new([0.0; 3], [100.0, 200.0, 300.0]).unwrap());
        full.rotation_rad = [0.5, -0.25];
        full.color = Some([0.25, 0.5, 0.75]);
        full.opacity = 0.4;
        full.representation = Representation::Simplified;
        full.colour_set = Some("Part classes".into());
        let stamp_revision = Uuid7::parse("019a7bf0-0000-7000-8000-000000000009").unwrap();
        full.stamp_provenance("Ana", "frame front", stamp_revision, 1_760_000_000_000)
            .unwrap();
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

    fn beams() -> ViewFilter {
        let mut filter = ViewFilter::new(
            "Beams",
            vec![
                FilterCondition::new(FilterField::Type, Operator::Eq, "beam"),
                FilterCondition::new(FilterField::Class, Operator::Ge, "2"),
                FilterCondition::new(
                    FilterField::Attribute("Length".into()),
                    Operator::Gt,
                    "250 mm",
                ),
            ],
        );
        filter.matching = FilterMatch::Any;
        filter
    }

    #[test]
    fn filters_and_overrides_round_trip() {
        let mut book = ViewBook::new();
        book.save_filter(beams(), None).unwrap();
        let mut plan = view(1, "Plan");
        plan.overrides.push(FilterOverride {
            filter: "Beams".into(),
            visible: false,
            color: Some([1.0, 0.5, 0.0]),
            transparency: 0.25,
        });
        book.insert(plan.clone()).unwrap();
        let back = book_from_json(&book_to_json(&book).unwrap()).unwrap();
        assert_eq!(back, book);
    }

    #[test]
    fn a_version_one_file_reads_without_filters() {
        let mut v1 = view_to_json(&view(1, "A")).unwrap();
        if let Json::Object(map) = &mut v1 {
            map.remove("overrides");
        }
        let file = Json::object([
            ("version", Json::uint(1)),
            ("folders", Json::Array(vec![])),
            ("views", Json::Array(vec![v1])),
        ]);
        let book = book_from_json(&file).unwrap();
        assert_eq!(book.len(), 1);
        assert!(book.filters().is_empty());
    }

    #[test]
    fn a_filter_becomes_the_v14_rule() {
        let rule = filter_rule(&beams());
        let encoded =
            String::from_utf8(rule.encode(&beamcraft_json::Limits::DEFAULT).unwrap()).unwrap();
        assert_eq!(
            encoded,
            r#"{"any":[{"field":"type","op":"eq","value":"beam"},{"field":"class","op":"ge","value":2},{"attribute":"Length","field":"attribute","op":"gt","value":{"unit":"mm","value":250}}]}"#
        );
        assert_eq!(attribute_literal("true"), Json::Bool(true));
        assert_eq!(attribute_literal("S355"), Json::text("S355"));
        assert_eq!(attribute_literal("Shop 2"), Json::text("Shop 2"));
    }

    #[test]
    fn malformed_files_refuse_whole() {
        let good = book_to_json(&ViewBook::new()).unwrap();
        assert!(book_from_json(&good).unwrap().is_empty());
        let wrong_version = Json::object([
            ("version", Json::uint(3)),
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

    #[test]
    fn provenance_stamps_round_trip_and_version_one_files_read() {
        let mut stamped = view(1, "Provenance");
        stamped
            .stamp_provenance("Ana", "frame front", id(9), 1_760_000_000_000)
            .unwrap();
        let mut book = ViewBook::new();
        book.insert(stamped.clone()).unwrap();
        book.insert(view(2, "Plain")).unwrap();
        let back = book_from_json(&book_to_json(&book).unwrap()).unwrap();
        assert_eq!(
            back.get(&stamped.id).unwrap().provenance,
            stamped.provenance
        );
        assert_eq!(back.get(&id(2)).unwrap().provenance, None);
        // A version 1 file has no provenance keys at all and reads as before.
        let mut plain = view_to_json(&stamped).unwrap();
        if let Json::Object(map) = &mut plain {
            map.remove("provenance");
        }
        let legacy = Json::object([
            ("version", Json::uint(1)),
            ("folders", Json::Array(vec![])),
            ("views", Json::Array(vec![plain])),
        ]);
        let read = book_from_json(&legacy).unwrap();
        assert_eq!(read.get(&stamped.id).unwrap().provenance, None);
        // A file from before the representation and colour set existed reads with the display
        // defaults; the round trip above proves a present key still carries the values.
        let mut plain = view_to_json(&stamped).unwrap();
        if let Json::Object(map) = &mut plain {
            map.remove("representation");
            map.remove("colour_set");
        }
        let older = Json::object([
            ("version", Json::uint(2)),
            ("folders", Json::Array(vec![])),
            ("views", Json::Array(vec![plain])),
        ]);
        let read = book_from_json(&older).unwrap();
        let plain_view = read.get(&stamped.id).unwrap();
        assert_eq!(plain_view.representation, Representation::Full);
        assert_eq!(plain_view.colour_set, None);
        // A present key of another type refuses the read.
        let mut wrong = view_to_json(&stamped).unwrap();
        if let Json::Object(map) = &mut wrong {
            map.insert("representation".into(), Json::uint(3));
        }
        let wrong_file = Json::object([
            ("version", Json::uint(2)),
            ("folders", Json::Array(vec![])),
            ("views", Json::Array(vec![wrong])),
        ]);
        assert!(book_from_json(&wrong_file).is_err());
        // A stamp that breaks the admission rules refuses its read, so the file refuses whole.
        let empty_author = Json::object([
            ("author", Json::text("")),
            ("created_at_ms", Json::uint(1)),
            ("comment", Json::text("")),
            ("model_revision", Json::text(stamped.id.to_string())),
        ]);
        assert!(provenance_from_json(&empty_author).is_err());
    }
}
