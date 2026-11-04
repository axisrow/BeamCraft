// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The W01 views layer: canonical camera-prim bytes, the reader round trip and the bounds.
//! Original fixtures; no SDK/independent-reader claim.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use beamcraft_layer_engine::authored::Placement;
use beamcraft_layer_engine::authored::views::{
    self, CameraProjection, SavedView, VIEWS_FOLDER, ViewStamp, views_schema_identity,
};
use beamcraft_layer_engine::*;
use beamcraft_model::digest::{ContentDigest, Sha256};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::Uuid7;
use std::time::{Duration, Instant};

fn control() -> Control {
    Control {
        cancellation: Cancellation::default(),
        deadline: Instant::now() + Duration::from_secs(90),
    }
}

fn digest(bytes: &[u8]) -> ContentDigest {
    Sha256::digest(bytes).unwrap()
}

fn asset(name: &str, bytes: impl Into<Vec<u8>>) -> Asset {
    let bytes = bytes.into();
    Asset {
        name: name.into(),
        digest: digest(&bytes),
        bytes,
    }
}

/// Opens one views layer through the reader and returns its composed prims.
fn prims_of(bytes: &[u8]) -> Vec<Prim> {
    let mut request = OpenRequest {
        identity: Identity {
            project: "0199e7a0-0000-7000-8000-000000000001".into(),
            workspace: "0199e7a0-0000-7000-8000-000000000002".into(),
            revision: digest(b"a views revision"),
            generation: 7,
        },
        root: "views.usda".into(),
        owned_layers: ["views.usda".to_owned()].into(),
        assets: vec![asset("views.usda", bytes)],
        manifest: digest(b"pending"),
        backend: BACKEND,
        profile: Profile::StaticLocal,
        budgets: Budgets::FIRST_SLICE,
    };
    request.manifest = request.manifest_digest().unwrap();
    let mut reader = Reader::default();
    let opened = reader.open(request, &control()).unwrap();
    let mut prims = Vec::new();
    let mut cursor = None;
    loop {
        let page = reader
            .read(&opened.context, cursor.as_ref(), &control())
            .unwrap();
        prims.extend(page.items);
        match page.next {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    prims
}

fn placement(origin: [f64; 3], yaw_quarter_turns: f64) -> Placement {
    // A unit quaternion for a rotation about Z; exact for quarter turns.
    let half = yaw_quarter_turns / 2.0;
    Placement {
        translation_mm: origin,
        orientation_xyzw: [0.0, 0.0, half.sin(), half.cos()],
    }
}

fn view(id_text: &str, name: &str) -> SavedView {
    let id = Uuid7::parse(id_text).unwrap();
    SavedView::new(id, name, placement([1000.0, 2000.0, 3000.0], 1.0), 4321.0).unwrap()
}

/// A layer of exactly these views and no empty folders.
fn layer_of(views: &[SavedView]) -> views::ViewsLayer {
    views::ViewsLayer {
        views: views.to_vec(),
        empty_folders: Vec::new(),
    }
}

const ID_A: &str = "0199e7a0-0000-7000-8000-0000000000aa";
const ID_B: &str = "0199e7a0-0000-7000-8000-0000000000bb";
const REVISION: &str = "0199e7a0-0000-7000-8000-0000000000cc";

/// One fully populated view: every optional field set and the provenance stamped.
fn full_view(id_text: &str, name: &str) -> SavedView {
    let mut view = view(id_text, name);
    view.projection = CameraProjection::Perspective;
    view.depth_mm = Some([8.0, 9.0]);
    view.render_mode = "wireframe".to_owned();
    view.grids_visible = false;
    view.hidden_kinds = vec!["grid".to_owned(), "level".to_owned()];
    view.folder = "Review/North".to_owned();
    view.favorite = true;
    view.default = true;
    view.work_area_mm = Some([[-500.0, -600.0, -700.0], [500.0, 600.0, 700.0]]);
    view.rotation_rad = [0.25, -0.5];
    view.color_rgb = Some([0.1, 0.2, 0.3]);
    view.opacity = 0.75;
    view.representation = "simplified".to_owned();
    view.colour_set = Some("Part classes".to_owned());
    view.stamp = Some(
        ViewStamp::new(
            "  author  ",
            "checked the brace",
            Uuid7::parse(REVISION).unwrap(),
            1_762_000_000_000,
        )
        .unwrap(),
    );
    view
}

#[test]
fn canonical_bytes_round_trip_through_the_reader() {
    let saved = [full_view(ID_A, "Front"), view(ID_B, "Top")];
    let bytes = views::bytes(&layer_of(&saved)).unwrap();
    assert_eq!(
        bytes,
        views::bytes(&layer_of(&saved)).unwrap(),
        "the layer text must be canonical"
    );
    assert_eq!(views::layer_digest(&bytes).unwrap(), digest(&bytes));
    let digest_text = format!("{}", views_schema_identity().unwrap().digest);
    let text = std::str::from_utf8(&bytes).unwrap();
    assert!(
        text.contains("bimViewsSchemaDigest"),
        "the layer pins its mapping identity"
    );
    assert!(
        text.contains(&digest_text),
        "the pinned digest is the views schema digest"
    );
    assert!(
        text.contains("def Camera"),
        "a saved view is a standard USD camera prim"
    );
    let read = views::read_layer(&prims_of(&bytes)).unwrap();
    assert_eq!(read.views.len(), 2);
    assert_eq!(
        read.views,
        saved.to_vec(),
        "the reader restores the exact saved views"
    );
    let composed = prims_of(&bytes);
    let cameras = composed
        .iter()
        .filter(|p| p.type_name.as_deref() == Some("Camera"))
        .map(|p| p.path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(cameras.len(), 2);
    assert!(cameras.iter().all(|p| p.starts_with(VIEWS_FOLDER)));
}

#[test]
fn representation_and_colour_set_round_trip_and_default_when_absent() {
    // The full view carries both attributes; the plain view carries neither, and the prim
    // reads with the display defaults, so layer files written before V07 still read.
    let saved = [full_view(ID_A, "Front"), view(ID_B, "Top")];
    let bytes = views::bytes(&layer_of(&saved)).unwrap();
    let text = std::str::from_utf8(&bytes).unwrap();
    assert!(
        text.contains("bim:view:representation"),
        "the view authors its representation"
    );
    assert!(
        text.contains("bim:view:colourSet"),
        "the view authors its colour set name"
    );
    let read = views::read_layer(&prims_of(&bytes)).unwrap();
    assert_eq!(read.views, saved.to_vec());
    let plain = &read.views[1];
    assert_eq!(plain.representation, "full");
    assert_eq!(plain.colour_set, None);
    // A present attribute of another USD type refuses the prim, never silently absent.
    let mut wrong = full_view(ID_A, "Front");
    wrong.representation = String::new();
    assert!(views::bytes(&layer_of(&[wrong])).is_err());
    let mut bad_set = full_view(ID_A, "Front");
    bad_set.colour_set = Some("a\tb".to_owned());
    assert!(views::bytes(&layer_of(&[bad_set])).is_err());
}

#[test]
fn canonical_order_is_identity_order() {
    let bytes = views::bytes(&layer_of(&[view(ID_B, "Top"), view(ID_A, "Front")])).unwrap();
    let read = views::read_layer(&prims_of(&bytes)).unwrap();
    assert_eq!(read.views[0].id.to_string(), ID_A);
    assert_eq!(read.views[1].id.to_string(), ID_B);
}

#[test]
fn a_camera_outside_the_views_folder_is_inert() {
    // A sibling scope's name shares the folder's prefix but not its namespace; an owned camera
    // there is foreign content and stays uninterpreted, never a refusal of the whole read.
    let stranger = "#usda 1.0\n(metersPerUnit = 0.001\nupAxis = \"Z\")\ndef Xform \"World\" { def Scope \"ViewsFoo\" { def Camera \"Stranger\" { double height = 1 } } }\n";
    let read = views::read_layer(&prims_of(stranger.as_bytes())).unwrap();
    assert!(read.views.is_empty());
}

#[test]
fn a_wrong_typed_optional_attribute_is_refused() {
    let bytes = views::bytes(&layer_of(&[full_view(ID_A, "Front")])).unwrap();
    let text = std::str::from_utf8(&bytes).unwrap();
    // The depth record redeclared with another USD type (and a value valid for that type) must
    // refuse the prim, never read the record as silently absent.
    let broken = text.replacen(
        "double[] bim:view:depthMm = [8.0, 9.0]",
        "double3 bim:view:depthMm = (8.0, 9.0, 1.0)",
        1,
    );
    assert!(broken != text, "the depth record text form was expected");
    assert!(views::read_layer(&prims_of(broken.as_bytes())).is_err());
    // The same presence rule for the default mark: a wrong USD type refuses, never a silent
    // unmarked view.
    let broken = text.replacen(
        "bool bim:view:default = true",
        "double bim:view:default = 1",
        1,
    );
    assert!(broken != text, "the default mark text form was expected");
    assert!(views::read_layer(&prims_of(broken.as_bytes())).is_err());
}

#[test]
fn the_stamp_survives_the_round_trip_exactly() {
    let bytes = views::bytes(&layer_of(&[full_view(ID_A, "Front")])).unwrap();
    let read = views::read_layer(&prims_of(&bytes)).unwrap();
    let stamp = read.views[0].stamp.as_ref().unwrap();
    assert_eq!(
        stamp.author, "author",
        "the author is trimmed once at capture"
    );
    assert_eq!(stamp.comment, "checked the brace");
    assert_eq!(stamp.created_at_ms, 1_762_000_000_000);
    assert_eq!(stamp.model_revision.to_string(), REVISION);
    // An unstamped view reads without provenance.
    let plain = views::bytes(&layer_of(&[view(ID_B, "Top")])).unwrap();
    assert!(
        views::read_layer(&prims_of(&plain)).unwrap().views[0]
            .stamp
            .is_none()
    );
}

#[test]
fn a_half_stamped_prim_is_refused() {
    let bytes = views::bytes(&layer_of(&[full_view(ID_A, "Front")])).unwrap();
    let text = std::str::from_utf8(&bytes).unwrap();
    // Rename exactly one provenance attribute: the prim then carries three of four.
    let broken = text.replacen("bim:view:comment", "bim:view:note", 1);
    assert!(views::read_layer(&prims_of(broken.as_bytes())).is_err());
}

#[test]
fn empty_folders_survive_the_round_trip_sorted_and_unique() {
    let layer = views::ViewsLayer {
        views: vec![full_view(ID_A, "Front")],
        empty_folders: vec![
            "South".to_owned(),
            "Review/East".to_owned(),
            "South".to_owned(),
        ],
    };
    let bytes = views::bytes(&layer).unwrap();
    let read = views::read_layer(&prims_of(&bytes)).unwrap();
    // Sorted, deduplicated, and kept apart from the folder a view names.
    assert_eq!(
        read.empty_folders,
        vec!["Review/East".to_owned(), "South".to_owned()]
    );
    assert_eq!(read.views[0].folder, "Review/North");
    // The canonical text stores them sorted, so the same set commits one digest.
    let same = views::bytes(&views::ViewsLayer {
        views: layer.views.clone(),
        empty_folders: vec!["Review/East".to_owned(), "South".to_owned()],
    })
    .unwrap();
    assert_eq!(bytes, same);
    // A folders record of another USD type refuses the scope: presence is by name alone, so a
    // record redeclared as a parsable `token[]` of the same values is a refusal, never a
    // silent absence.
    let text = std::str::from_utf8(&bytes).unwrap();
    let broken = text.replacen(
        "string[] bim:view:folders = [\"Review/East\", \"South\"]",
        "token[] bim:view:folders = [\"Review/East\", \"South\"]",
        1,
    );
    assert!(broken != text, "the folders record text form was expected");
    assert!(views::read_layer(&prims_of(broken.as_bytes())).is_err());
    // A folder that breaks the path rules is refused.
    let bad = views::ViewsLayer {
        views: Vec::new(),
        empty_folders: vec!["  spaced".to_owned()],
    };
    assert!(views::bytes(&bad).is_err());
    let dot = views::ViewsLayer {
        views: Vec::new(),
        empty_folders: vec!["a/./b".to_owned()],
    };
    assert!(views::bytes(&dot).is_err());
}

#[test]
fn bounds_are_refused() {
    let mut bad = view(ID_A, "Front");
    bad.span_mm = 0.0;
    assert!(views::bytes(&layer_of(&[bad])).is_err());
    let mut bad = view(ID_A, "Front");
    bad.span_mm = views::MAX_SPAN_MM * 2.0;
    assert!(views::bytes(&layer_of(&[bad])).is_err());
    // A non-unit orientation is not a camera placement.
    let mut bad = view(ID_A, "Front");
    bad.placement.orientation_xyzw = [0.0, 0.0, 0.1, 0.1];
    assert!(views::bytes(&layer_of(&[bad])).is_err());
    // Provenance bounds: length, controls and a non-empty author.
    let too_long_author = "a".repeat(129);
    assert!(ViewStamp::new(&too_long_author, "", Uuid7::parse(REVISION).unwrap(), 1).is_err());
    let mut embedded = "a".to_owned();
    embedded.push('\u{7}');
    embedded.push('b');
    assert!(ViewStamp::new(&embedded, "", Uuid7::parse(REVISION).unwrap(), 1).is_err());
    assert!(
        ViewStamp::new(
            "author",
            &"c".repeat(513),
            Uuid7::parse(REVISION).unwrap(),
            1
        )
        .is_err()
    );
    // Display bounds.
    let mut bad = view(ID_A, "Front");
    bad.stamp = Some(ViewStamp::new("author", "", Uuid7::parse(REVISION).unwrap(), 1).unwrap());
    bad.opacity = 1.5;
    assert!(views::bytes(&layer_of(&[bad])).is_err());
    // Duplicate identities are refused, never merged.
    let duplicate = [view(ID_A, "One"), view(ID_A, "Two")];
    assert_eq!(
        views::bytes(&layer_of(&duplicate)).unwrap_err().code,
        ErrorCode::InvalidId
    );
    // The count cap binds.
    let over = (0..views::MAX_VIEWS + 1)
        .map(|i| view(&format!("0199e7a0-0000-7000-8000-{:012x}", i), "many"))
        .collect::<Vec<_>>();
    assert_eq!(
        views::bytes(&layer_of(&over)).unwrap_err().code,
        ErrorCode::LimitExceeded
    );
}

#[test]
fn names_are_trimmed_and_validated() {
    assert!(
        SavedView::new(
            Uuid7::parse(ID_A).unwrap(),
            "  ",
            placement([0.0; 3], 0.0),
            100.0
        )
        .is_err()
    );
    assert!(
        SavedView::new(
            Uuid7::parse(ID_A).unwrap(),
            &"n".repeat(129),
            placement([0.0; 3], 0.0),
            100.0
        )
        .is_err()
    );
    let view = SavedView::new(
        Uuid7::parse(ID_A).unwrap(),
        "  Front  ",
        placement([0.0; 3], 0.0),
        100.0,
    )
    .unwrap();
    assert_eq!(view.name, "Front");
}
