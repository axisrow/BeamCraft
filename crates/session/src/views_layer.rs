// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The W01 views layer on disk: the mapping between the engine's view book and the layer
//! engine's camera-prim views layer, plus the file shape the CLI and the window share.
//! Mapping and format only — every admission rule lives in the engine's view book and in the
//! layer profile (`beamcraft_layer_engine::authored::views`).

use crate::Refusal;
use crate::pipeline::{control, digest_of, open_request};
use beamcraft_engine::solid::rotate_vector;
use beamcraft_engine::views::{
    FilterOverride, ModelView, Projection, ViewBook, ViewFilter, ViewProvenance, WorkArea,
};
use beamcraft_engine::work_plane::WorkPlane;
use beamcraft_engine::{ElementKind, KindFilter, RenderMode};
use beamcraft_json::Limits;
use beamcraft_layer_engine::authored::views as layer;
use beamcraft_layer_engine::authored::views::{CameraProjection, SavedView, ViewStamp, ViewsLayer};
use beamcraft_layer_engine::{Asset, Prim, Reader};
use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;
use std::collections::BTreeSet;

/// The bytes a views layer starts with; anything else is the legacy JSON sidecar.
const LAYER_MAGIC: &[u8] = b"#usda";

/// The virtual single-layer manifest the reader sees; a sidecar is the session's own content,
/// so the session builds and verifies the commitment itself. The identities are fixed
/// sentinels: the layer is standalone, so no store identity enters it.
const LAYER_NAME: &str = "views.usda";
const SIDE_CAR_PROJECT: &str = "0199e7a0-0000-7000-8000-0000000000fe";
const SIDE_CAR_WORKSPACE: &str = "0199e7a0-0000-7000-8000-0000000000ff";

fn refused(error: ErrorCode, message: &'static str) -> CommandError {
    CommandError::new(error, "views.layer", message)
}

fn from_layer(field: &'static str) -> impl Fn(beamcraft_layer_engine::Diagnostic) -> CommandError {
    move |error| CommandError::new(error.code, field, error.message)
}

fn from_pipeline(field: &'static str) -> impl Fn(Refusal) -> CommandError {
    move |error| CommandError::new(error.code, field, error.message)
}

/// Whether the bytes are a views layer (`true`) or a legacy JSON sidecar (`false`).
pub fn looks_like_layer(bytes: &[u8]) -> bool {
    bytes.starts_with(LAYER_MAGIC)
}

/// The composed prims of one views layer, read under the reader's budgets and manifest
/// commitment.
fn prims(bytes: &[u8]) -> Result<Vec<Prim>, CommandError> {
    let layer_digest = digest_of(bytes).map_err(from_pipeline("views.layer"))?;
    let asset = Asset {
        name: LAYER_NAME.into(),
        digest: layer_digest,
        bytes: bytes.to_vec(),
    };
    // The sidecar is the session's own content: the single layer is owned, so its prims compose.
    let mut request = open_request(
        LAYER_NAME,
        SIDE_CAR_PROJECT,
        SIDE_CAR_WORKSPACE,
        layer_digest,
        0,
        vec![asset],
        &BTreeSet::new(),
    )
    .map_err(from_pipeline("views.layer"))?;
    request.manifest = request
        .manifest_digest()
        .map_err(from_layer("views.layer"))?;
    let mut reader = Reader::default();
    let opened = reader
        .open(request, &control())
        .map_err(from_layer("views.layer"))?;
    let mut out = Vec::new();
    let mut cursor = None;
    loop {
        let page = reader
            .read(&opened.context, cursor.as_ref(), &control())
            .map_err(from_layer("views.layer"))?;
        out.extend(page.items);
        match page.next {
            Some(next) => cursor = Some(next),
            None => return Ok(out),
        }
    }
}

/// The layer record of one view: the camera prim the views layer stores.
fn to_saved(view: &ModelView) -> Result<SavedView, CommandError> {
    let pose = view.plane.placement()?;
    let mut saved = SavedView::new(
        view.id,
        &view.name,
        beamcraft_layer_engine::authored::Placement {
            translation_mm: pose.translation_mm,
            orientation_xyzw: pose.orientation_xyzw,
        },
        view.span_mm,
    )
    .map_err(from_layer("views.camera"))?;
    saved.projection = match view.projection {
        Projection::Orthographic => CameraProjection::Orthographic,
        Projection::Perspective => CameraProjection::Perspective,
    };
    saved.depth_mm = view.depth;
    saved.render_mode = view.render_mode.as_token().to_owned();
    saved.grids_visible = view.grids_visible;
    saved.hidden_kinds = view
        .hidden_kinds
        .hidden_kinds()
        .into_iter()
        .map(|kind| kind.as_token().to_owned())
        .collect();
    saved.folder = view.folder.clone();
    saved.favorite = view.favorite;
    saved.default = view.is_default;
    saved.work_area_mm = view.work_area.as_ref().map(|area| [area.min, area.max]);
    saved.rotation_rad = view.rotation_rad;
    saved.color_rgb = view.color;
    saved.opacity = view.opacity;
    saved.representation = view.representation.as_token().to_owned();
    saved.beyond_depth = view.beyond.as_str().to_owned();
    saved.colour_set = view.colour_set.clone();
    saved.filter_overrides = view.overrides.iter().map(FilterOverride::to_spec).collect();
    if let Some(provenance) = &view.provenance {
        saved.stamp = Some(
            ViewStamp::new(
                &provenance.author,
                &provenance.comment,
                provenance.model_revision,
                provenance.created_at_ms,
            )
            .map_err(from_layer("views.provenance"))?,
        );
    }
    Ok(saved)
}

/// The engine view of one stored camera prim.
fn to_view(saved: SavedView) -> Result<ModelView, CommandError> {
    let orientation = saved.placement.orientation_xyzw;
    let plane = WorkPlane::new(
        saved.placement.translation_mm,
        rotate_vector([1.0, 0.0, 0.0], orientation),
        rotate_vector([0.0, 1.0, 0.0], orientation),
    )?;
    let mut view = ModelView::new(saved.id, &saved.name, plane, saved.span_mm);
    view.projection = Projection::parse(saved.projection.as_str())?;
    view.depth = saved.depth_mm;
    view.render_mode = RenderMode::parse(&saved.render_mode).ok_or_else(|| {
        refused(
            ErrorCode::UnsupportedVersion,
            "the stored display mode is not one of the engine's modes",
        )
    })?;
    view.grids_visible = saved.grids_visible;
    let mut hidden = KindFilter::SHOW_ALL;
    for kind in &saved.hidden_kinds {
        let kind = ElementKind::parse(kind).ok_or_else(|| {
            refused(
                ErrorCode::UnsupportedVersion,
                "the stored hidden kind is not one of the engine's kinds",
            )
        })?;
        hidden = hidden.hiding(kind);
    }
    view.hidden_kinds = hidden;
    view.folder = saved.folder;
    view.favorite = saved.favorite;
    view.is_default = saved.default;
    view.work_area = match saved.work_area_mm {
        Some([min, max]) => Some(WorkArea::new(min, max)?),
        None => None,
    };
    view.rotation_rad = saved.rotation_rad;
    view.color = saved.color_rgb;
    view.opacity = saved.opacity;
    view.representation = beamcraft_engine::Representation::parse(&saved.representation)
        .ok_or_else(|| {
            refused(
                ErrorCode::UnsupportedVersion,
                "the stored representation is not one of the engine's levels",
            )
        })?;
    view.beyond =
        beamcraft_engine::views::DepthBeyond::parse(&saved.beyond_depth).map_err(|_| {
            refused(
                ErrorCode::UnsupportedVersion,
                "the stored beyond-depth setting is not cut or transparent",
            )
        })?;
    view.colour_set = saved.colour_set;
    view.overrides = saved
        .filter_overrides
        .iter()
        .map(|spec| FilterOverride::parse_spec(spec))
        .collect::<Result<Vec<_>, _>>()?;
    if let Some(stamp) = saved.stamp {
        view.provenance = Some(
            ViewProvenance::new(
                &stamp.author,
                &stamp.comment,
                stamp.model_revision,
                stamp.created_at_ms,
            )
            .map_err(|_| {
                refused(
                    ErrorCode::InvalidArgument,
                    "the stored provenance is refused",
                )
            })?,
        );
    }
    view.validate()?;
    Ok(view)
}

/// The canonical views layer bytes of the book: one camera prim per saved view (temporary
/// views are session state and are never written), plus the folders no view uses.
pub fn layer_bytes(book: &ViewBook) -> Result<Vec<u8>, CommandError> {
    let saved = book
        .views()
        .iter()
        .filter(|view| !view.temporary)
        .map(to_saved)
        .collect::<Result<Vec<_>, _>>()?;
    let used = saved
        .iter()
        .map(|view| view.folder.as_str())
        .collect::<BTreeSet<_>>();
    let empty = book
        .folders()
        .filter(|folder| !used.contains(folder))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let filters = book
        .filters()
        .iter()
        .map(filter_text)
        .collect::<Result<Vec<_>, _>>()?;
    layer::bytes(&ViewsLayer {
        views: saved,
        empty_folders: empty,
        filters,
    })
    .map_err(from_layer("views.layer"))
}

/// One view filter as the compact JSON text the layer stores
/// (`beamcraft_host::views::filter_to_json`).
fn filter_text(filter: &ViewFilter) -> Result<String, CommandError> {
    let bytes = beamcraft_host::views::filter_to_json(filter)
        .encode(&Limits::DEFAULT)
        .map_err(|_| refused(ErrorCode::InvalidArgument, "a view filter does not encode"))?;
    String::from_utf8(bytes)
        .map_err(|_| refused(ErrorCode::InvalidArgument, "a view filter does not encode"))
}

/// One view filter back from its stored JSON text.
fn filter_from_text(text: &str) -> Result<ViewFilter, CommandError> {
    let json = beamcraft_json::decode(text.as_bytes(), &Limits::DEFAULT).map_err(|_| {
        refused(
            ErrorCode::InvalidArgument,
            "a stored view filter is not valid JSON",
        )
    })?;
    beamcraft_host::views::filter_from_json(&json)
}

/// The book of one views layer's bytes: camera prims back into engine views, with the view
/// filters their overrides name.
pub fn book_from_layer(bytes: &[u8]) -> Result<ViewBook, CommandError> {
    let content = layer::read_layer(&prims(bytes)?).map_err(from_layer("views.layer"))?;
    let views = content
        .views
        .into_iter()
        .map(to_view)
        .collect::<Result<Vec<_>, _>>()?;
    let filters = content
        .filters
        .iter()
        .map(|text| filter_from_text(text))
        .collect::<Result<Vec<_>, _>>()?;
    ViewBook::from_all(views, content.empty_folders, filters)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use beamcraft_engine::filter::Operator;
    use beamcraft_engine::views::{FilterCondition, FilterField, FilterMatch};
    use beamcraft_model::id::Uuid7;

    #[test]
    fn filters_and_overrides_ride_through_the_layer() {
        let mut book = ViewBook::new();
        let mut beams = ViewFilter::new(
            "Beams",
            vec![
                FilterCondition::new(FilterField::Type, Operator::Eq, "beam"),
                FilterCondition::new(
                    FilterField::Attribute("Fire rating".into()),
                    Operator::Eq,
                    "R30",
                ),
            ],
        );
        beams.matching = FilterMatch::Any;
        book.save_filter(beams, None).unwrap();
        let mut plan = ModelView::new(
            Uuid7::parse("0199e7a0-0000-7000-8000-0000000000a1").unwrap(),
            "Plan",
            WorkPlane::default(),
            5000.0,
        );
        plan.overrides = vec![FilterOverride::parse_spec("Beams|hide|1,0.5,0|0.25").unwrap()];
        book.insert(plan).unwrap();
        let back = book_from_layer(&layer_bytes(&book).unwrap()).unwrap();
        assert_eq!(back, book);
    }
}
