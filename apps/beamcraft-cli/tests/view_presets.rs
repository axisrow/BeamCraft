// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic model and presets, MPL-2.0.

//! V07 discipline view presets through the real binary: define, list, apply and remove over
//! real files. A preset is display state composed of existing primitives only, so applying one
//! rewrites the views file and nothing else: the model head stays byte-identical and the
//! provenance of the view stays exactly as it was captured.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use crate::support::{StoreFixture, at, list, text};
use beamcraft_json::{Json, Limits};

struct Presets {
    f: StoreFixture,
    file: String,
    views_file: String,
}

impl Presets {
    fn new() -> Self {
        let f = StoreFixture::new("v07-presets");
        let file = f.root.join("presets.json").to_str().unwrap().to_owned();
        let views_file = f.root.join("views.usda").to_str().unwrap().to_owned();
        Self {
            f,
            file,
            views_file,
        }
    }

    fn ok(&self, command: &str, args: &[&str]) -> Json {
        let (code, reply) = self.f.run(command, args);
        assert_eq!(code, 0, "{command} {args:?}: {reply:?}");
        reply
    }

    fn presets(&self, command: &str, args: &[&str]) -> Json {
        let mut all = vec!["--file", self.file.as_str()];
        all.extend_from_slice(args);
        self.ok(command, &all)
    }

    fn refuse(&self, command: &str, args: &[&str]) {
        let (code, reply) = self.f.run(command, args);
        assert_eq!(code, 2, "{command} {args:?}: {reply:?}");
        assert_eq!(text(&reply, "outcome"), "rejected");
    }
}

#[test]
fn presets_define_list_apply_and_the_model_stays_unchanged() {
    let p = Presets::new();
    p.ok(
        "create-plate",
        &["--width", "600", "--length", "300", "--thickness", "10"],
    );
    // A provenance-bearing capture: the stamp rides the view and must survive a preset apply.
    let head = p.ok("open-revision", &[]);
    let saved = p.ok(
        "save-view",
        &[
            "--file",
            p.views_file.as_str(),
            "--name",
            "Frame front",
            "--parallel",
            "xz",
            "--author",
            "Ana",
            "--comment",
            "front capture",
        ],
    );
    let view_id = text(&list(&saved, "views")[0], "id");
    let captured = at(&list(&saved, "views")[0], &["provenance"]);

    // The preset bundles exactly the display components: the kind filter (whose reference kind
    // is the reference visibility), the underlay slab, the representation, the colour set and
    // the grid toggle.
    let defined = p.presets(
        "define-preset",
        &[
            "--name",
            "Fabrication",
            "--hidden-kinds",
            "reference,grid",
            "--depth",
            "0,3000",
            "--representation",
            "simplified",
            "--colour-set",
            "Part classes",
            "--grids",
            "false",
        ],
    );
    assert_eq!(text(&defined, "outcome"), "preset_defined");
    assert_eq!(text(at(&defined, &["preset"]), "name"), "Fabrication"); // Names are unique case-insensitively, and a broken depth is refused.
    p.refuse(
        "define-preset",
        &["--file", p.file.as_str(), "--name", "FABRICATION"],
    );
    p.refuse(
        "define-preset",
        &["--file", p.file.as_str(), "--name", "Bad", "--depth", "0,0"],
    );
    p.refuse(
        "define-preset",
        &[
            "--file",
            p.file.as_str(),
            "--name",
            "Bad",
            "--representation",
            "axis",
        ],
    );

    let listed = p.presets("list-presets", &[]);
    assert_eq!(text(&listed, "outcome"), "presets");
    assert_eq!(list(&listed, "presets").len(), 1);

    // Applying writes the bundle onto the saved view and nothing else.
    let applied = p.ok(
        "apply-preset",
        &[
            "--file",
            p.file.as_str(),
            "--views-file",
            p.views_file.as_str(),
            "--preset",
            "fabrication",
            "--view",
            "frame front",
        ],
    );
    assert_eq!(text(&applied, "outcome"), "preset_applied");
    let view = at(&applied, &["view"]);
    assert_eq!(text(view, "representation"), "simplified");
    assert_eq!(text(view, "colour_set"), "Part classes");
    assert!(!at(view, &["grids_visible"]).boolean().unwrap());
    let kinds = list(view, "hidden_kinds")
        .iter()
        .map(|k| k.string().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert!(kinds.contains(&"reference".to_owned()), "{kinds:?}");
    assert!(kinds.contains(&"grid".to_owned()), "{kinds:?}");
    let depth = list(view, "depth_mm");
    assert_eq!(depth.len(), 2);
    assert_eq!(depth[0].float().unwrap(), 0.0);
    assert_eq!(depth[1].float().unwrap(), 3000.0);
    // The camera and the provenance stay exactly as captured.
    assert_eq!(at(view, &["provenance"]), captured);
    assert_eq!(text(view, "name"), "Frame front");

    // The acceptance: switching the preset never changes the model or its snapshot token; the
    // head reply (revision id and manifest digest) is byte-identical, and the views file is
    // now the canonical views layer.
    assert_eq!(p.ok("open-revision", &[]), head);
    let bytes = std::fs::read(&p.views_file).unwrap();
    assert!(bytes.starts_with(b"#usda"), "the views file is a layer");

    // The round trip through the layer keeps the bundle.
    let listed = p.ok("list-views", &["--file", p.views_file.as_str()]);
    let view = &list(&listed, "views")[0];
    assert_eq!(text(view, "representation"), "simplified");
    assert_eq!(text(view, "colour_set"), "Part classes");
    assert_eq!(at(view, &["provenance"]), captured);

    // The display fields are also settable on their own.
    let updated = p.ok(
        "update-view",
        &[
            "--file",
            p.views_file.as_str(),
            "--view",
            "Frame front",
            "--representation",
            "full",
            "--colour-set",
            "none",
        ],
    );
    let updated_view = &list(&updated, "views")[0];
    assert_eq!(text(updated_view, "representation"), "full");
    assert!(matches!(updated_view.get("colour_set"), Ok(Json::Null)));

    // Removal, and applying a removed or unknown preset refuses.
    let removed = p.presets("remove-preset", &["--name", "Fabrication"]);
    assert_eq!(text(&removed, "outcome"), "preset_removed");
    p.refuse(
        "apply-preset",
        &[
            "--file",
            p.file.as_str(),
            "--views-file",
            p.views_file.as_str(),
            "--preset",
            "Fabrication",
            "--view",
            &view_id,
        ],
    );
    p.refuse(
        "apply-preset",
        &[
            "--file",
            p.file.as_str(),
            "--views-file",
            p.views_file.as_str(),
            "--preset",
            "Ghost",
            "--view",
            &view_id,
        ],
    );
    assert_eq!(p.ok("open-revision", &[]), head);
}

#[test]
fn a_preset_applies_onto_a_legacy_json_views_file() {
    let p = Presets::new();
    // A version-2 sidecar written before the representation and colour set existed: the view
    // reads with the display defaults, the preset applies, and the next save rewrites the
    // file in the layer form.
    let number = |v: f64| Json::number(v).unwrap();
    let plane = Json::object([
        (
            "origin_mm",
            Json::Array(vec![number(0.0), number(0.0), number(0.0)]),
        ),
        (
            "x",
            Json::Array(vec![number(1.0), number(0.0), number(0.0)]),
        ),
        (
            "y",
            Json::Array(vec![number(0.0), number(1.0), number(0.0)]),
        ),
    ]);
    let view = Json::object([
        ("id", Json::text("019a7bf0-0000-7000-8000-000000000321")),
        ("name", Json::text("Legacy")),
        ("plane", plane),
        ("projection", Json::text("orthographic")),
        ("span_mm", number(5000.0)),
        ("depth_mm", Json::Null),
        ("render_mode", Json::text("shaded")),
        ("grids_visible", Json::Bool(true)),
        ("folder", Json::text("")),
        ("favorite", Json::Bool(false)),
        ("default", Json::Bool(false)),
        ("hidden_kinds", Json::Array(vec![])),
        ("work_area", Json::Null),
        ("rotation_rad", Json::Array(vec![number(0.0), number(0.0)])),
        ("color", Json::Null),
        ("opacity", number(1.0)),
    ]);
    let legacy = Json::object([
        ("version", Json::uint(2)),
        ("folders", Json::Array(vec![])),
        ("views", Json::Array(vec![view])),
    ]);
    std::fs::write(&p.views_file, legacy.encode(&Limits::DEFAULT).unwrap()).unwrap();
    p.presets("define-preset", &["--name", "Erection", "--grids", "false"]);
    let applied = p.ok(
        "apply-preset",
        &[
            "--file",
            p.file.as_str(),
            "--views-file",
            p.views_file.as_str(),
            "--preset",
            "Erection",
            "--view",
            "Legacy",
        ],
    );
    let view = at(&applied, &["view"]);
    assert_eq!(text(view, "representation"), "full", "the display default");
    assert!(matches!(view.get("colour_set"), Ok(Json::Null) | Err(_)));
    assert!(!at(view, &["grids_visible"]).boolean().unwrap());
    let bytes = std::fs::read(&p.views_file).unwrap();
    assert!(bytes.starts_with(b"#usda"), "the legacy file is rewritten");
}
