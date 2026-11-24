// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original synthetic firm libraries that follow no national standard, MPL-2.0. The
// projects are created from the shipped europe, russia and blank environments, whose provenance
// and licences are in `ATTRIBUTION.md`.

//! The namespaces of #628 end to end through the real `beamcraft` binary. A project's chain
//! carries three namespaces, profiles, fasteners and materials, and `--namespace` on
//! `list-library` and `show-library` reaches all of them. `all` pages the three resolvers as one
//! position space in that order; the resolvers are never merged, so the shadow report covers
//! every namespace without a record across two of them; `--filter` folds case in every script
//! and drops whitespace, while every exact lookup stays exact and inside its namespace; without
//! `--namespace` the replies keep their shape. The creation commands resolve a name through the
//! same three namespaces with one resolver, the model first and then the chain: their replies
//! name the layer and the library that answered, the shadowed entries and a lost environment;
//! two generations of one designation in the model refuse until a digest is given; and a repin
//! still follows the current library. Every run names its own `BEAMCRAFT_CONFIG_DIR`, so the
//! user's real application data is never read or written.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use crate::support::{self, at, list, text};

use beamcraft_json::{Json, Limits};
use std::fs;
use std::path::Path;
use std::process::Command;

const ACTOR: &str = "0199e7a0-0000-7000-8000-000000000426";

struct Fixture {
    shared: support::StoreFixture,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            shared: support::StoreFixture::new("catalog-namespaces-tests"),
        };
        fs::create_dir_all(fixture.shared.root.join("config/libraries")).unwrap();
        fs::create_dir(fixture.shared.root.join("models")).unwrap();
        fixture
    }
    fn path(&self, name: &str) -> String {
        self.shared.root.join(name).to_str().unwrap().to_owned()
    }
    fn raw(&self, args: &[&str]) -> (i32, Json) {
        let output = Command::new(env!("CARGO_BIN_EXE_beamcraft"))
            .args(args)
            .env("BEAMCRAFT_CONFIG_DIR", self.path("config"))
            .output()
            .unwrap();
        let body = beamcraft_json::decode(&output.stdout, &Limits::DEFAULT).unwrap();
        (output.status.code().unwrap(), body)
    }
    fn ok(&self, args: &[&str]) -> Json {
        let (code, body) = self.raw(args);
        assert_eq!(code, 0, "{args:?}: {body:?}");
        body
    }
    fn code(&self, args: &[&str]) -> String {
        let (code, body) = self.raw(args);
        assert_eq!(code, 2, "{args:?}: {body:?}");
        text(&body, "code")
    }
    /// The head revision of the store.
    fn head(&self, store: &str) -> String {
        text(
            &self.ok(&["open-revision", "--store", store]),
            "revision_id",
        )
    }
    /// A beam of `profile` and `material` between two points: the reply of `create-member`.
    fn member(&self, store: &str, profile: &str, material: &str) -> Json {
        self.ok(&[
            "create-member",
            "--store",
            store,
            "--kind",
            "beam",
            "--start",
            "0,0,0",
            "--end",
            "6000,0,0",
            "--profile",
            profile,
            "--material",
            material,
        ])
    }
    /// Two stacked plates of 200 x 200 x 20 mm, the parts of a bolt group: their identities.
    fn parts(&self, store: &str) -> [String; 2] {
        [None, Some("0,0,20")].map(|translation| {
            let mut args = vec![
                "create-plate",
                "--store",
                store,
                "--width",
                "200",
                "--length",
                "200",
                "--thickness",
                "20",
            ];
            args.extend(
                translation
                    .map(|t| vec!["--translation", t])
                    .unwrap_or_default(),
            );
            text(at(&self.ok(&args), &["plate"]), "element_id")
        })
    }
    /// `create-bolt-group` over `parts`: a row of two fasteners with no hole, named by
    /// `fastener` (`--designation TEXT` or `--fastener-digest SHA256`), with `extra` beside.
    /// The raw reply: its exit status and its body.
    fn group_raw(
        &self,
        store: &str,
        parts: &[String; 2],
        name: &str,
        fastener: &[&str],
        extra: &[&str],
    ) -> (i32, Json) {
        let joined = parts.join(",");
        let mut args = vec![
            "create-bolt-group",
            "--store",
            store,
            "--name",
            name,
            "--parts",
            &joined,
            "--point1",
            "50,50,40",
            "--point2",
            "50,150,40",
            "--plane-normal",
            "0,0,-1",
            "--offset-start",
            "25",
            "--count",
            "2",
            "--pitch",
            "50",
            "--hole-mode",
            "none",
        ];
        args.extend_from_slice(fastener);
        args.extend_from_slice(extra);
        self.raw(&args)
    }
    /// The reply of a `create-bolt-group` that must succeed ([`Self::group_raw`]).
    fn group(
        &self,
        store: &str,
        parts: &[String; 2],
        name: &str,
        fastener: &[&str],
        extra: &[&str],
    ) -> Json {
        let (code, body) = self.group_raw(store, parts, name, fastener, extra);
        assert_eq!(code, 0, "{name}: {body:?}");
        body
    }
    /// Installs the mixed firm file into the application data's `libraries/` and answers its path.
    fn firm(&self) -> String {
        let path = self.shared.root.join("config/libraries/mine.usda");
        fs::write(&path, firm_file()).unwrap();
        path.to_str().unwrap().to_owned()
    }
    /// Creates a project named `name` from the shipped environment `environment` and answers
    /// its store path.
    fn create(&self, name: &str, environment: &str) -> String {
        let models = self.path("models");
        text(
            at(
                &self.ok(&[
                    "create-project",
                    "--parent",
                    &models,
                    "--name",
                    name,
                    "--actor-id",
                    ACTOR,
                    "--environment",
                    environment,
                ]),
                &["project"],
            ),
            "path",
        )
    }
    /// `list-library --project STORE` with the extra options.
    fn listing(&self, store: &str, options: &[&str]) -> Json {
        let mut args = vec!["list-library", "--project", store];
        args.extend_from_slice(options);
        self.ok(&args)
    }
    /// `show-library --project STORE --designation DESIGNATION` with the extra options.
    fn shown(&self, store: &str, options: &[&str], designation: &str) -> Json {
        let mut args = vec!["show-library", "--project", store];
        args.extend_from_slice(options);
        args.extend(["--designation", designation]);
        self.ok(&args)
    }
    /// The refusal code of `show-library --project STORE --designation DESIGNATION`.
    fn not_found(&self, store: &str, options: &[&str], designation: &str) {
        let mut args = vec!["show-library", "--project", store];
        args.extend_from_slice(options);
        args.extend(["--designation", designation]);
        assert_eq!(self.code(&args), "NOT_FOUND", "{args:?}");
    }
}

/// One bolt entry of the suite's library files: a hex bolt of `diameter` with the thread
/// protrusion `protrusion`. Every value is invented for these tests.
fn bolt(prim: &str, designation: &str, diameter: f64, protrusion: f64) -> String {
    format!(
        "        def Scope \"{prim}\"\n        {{\nstring steel:fastener:designation = \"{designation}\"\ntoken steel:fastener:kind = \"bolt\"\ntoken steel:fastener:headKind = \"hex\"\ndouble steel:fastener:nominalDiameterMm = {diameter}\ndouble steel:fastener:headAcrossFlatsMm = {}\ndouble steel:fastener:headHeightMm = {}\ndouble steel:fastener:nutAcrossFlatsMm = {}\ndouble steel:fastener:nutHeightMm = {}\ndouble steel:fastener:holeClearanceMm = 2\ndouble[] steel:fastener:lengthsMm = [60, 70, 80]\ndouble steel:fastener:threadProtrusionMm = {protrusion}\nbool steel:fastener:washerUnderHead = false\nbool steel:fastener:washerUnderNut = false\ntoken steel:fastener:holeMode = \"generate\"\n        }}\n",
        diameter * 1.5,
        diameter * 0.625,
        diameter * 1.5,
        diameter * 0.9,
    )
}

/// A fastener library of one entry, the `M20` of the firm file with another thread protrusion: a
/// second generation of the designation, for a file named on the command line.
fn generation_file() -> String {
    format!(
        "#usda 1.0\n(\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n    customLayerData = {{\n        string \"bim:library:id\" = \"generation\"\n    }}\n)\ndef Scope \"Library\"\n{{\n    def Scope \"Fasteners\"\n    {{\n{}    }}\n}}\n",
        bolt("M20", "M20", 20.0, 5.0)
    )
}

/// The mixed firm file of the suite, `bim:library:id` `mine`: the profile `IPE300` at another
/// height than the shipped one, the bolts `M20` (another thread protrusion than the shipped
/// one) and `FIRM-M16`, the grades `S355J0` (another density than the shipped one) and
/// `FIRM-S355`. Every value is invented for these tests. The entry prim of a designation with
/// a hyphen carries an underscore: a prim name is an identifier.
fn firm_file() -> String {
    let profile = "        def Scope \"IPE300\"\n        {\nstring steel:profile:designation = \"IPE300\"\ntoken steel:profile:family = \"i_shape\"\ndouble steel:profile:hMm = 299\ndouble steel:profile:bMm = 150\ndouble steel:profile:twMm = 7.1\ndouble steel:profile:tfMm = 10.7\ndouble steel:profile:rMm = 15\n        }\n";
    let grade = |prim: &str, designation: &str, density: f64| {
        format!(
            "        def Scope \"{prim}\"\n        {{\nstring steel:material:grade = \"{designation}\"\ndouble steel:material:densityKgM3 = {density}\n        }}\n"
        )
    };
    let bolts = format!(
        "{}{}",
        bolt("M20", "M20", 20.0, 4.0),
        bolt("FIRM_M16", "FIRM-M16", 16.0, 4.0)
    );
    let grades = format!(
        "{}{}",
        grade("S355J0", "S355J0", 7800.0),
        grade("FIRM_S355", "FIRM-S355", 7850.0)
    );
    format!(
        "#usda 1.0\n(\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n    customLayerData = {{\n        string \"bim:library:id\" = \"mine\"\n    }}\n)\ndef Scope \"Library\"\n{{\n    def Scope \"Profiles\"\n    {{\n{profile}    }}\n    def Scope \"Fasteners\"\n    {{\n{bolts}    }}\n    def Scope \"Materials\"\n    {{\n{grades}    }}\n}}\n"
    )
}

/// The designations of a listing's entries, in page order.
fn designations(listed: &Json) -> Vec<String> {
    list(listed, "entries")
        .iter()
        .map(|e| text(e, "designation"))
        .collect()
}

/// The unsigned number at `key`; `None` when the key is null.
fn number(json: &Json, key: &str) -> Option<u64> {
    json.get(key).ok().and_then(|v| v.unsigned().ok())
}

fn count(json: &Json, key: &str) -> u64 {
    number(json, key).unwrap()
}

/// The entry of a listing that carries `designation`.
fn entry<'a>(listed: &'a Json, designation: &str) -> &'a Json {
    list(listed, "entries")
        .iter()
        .find(|e| text(e, "designation") == designation)
        .unwrap()
}

/// Walks one namespace of the chain a listing entry per page and returns the listed designations
/// in page order.
fn walk(f: &Fixture, store: &str, namespace: &str) -> Vec<String> {
    let mut listed = Vec::new();
    let mut offset = 0u64;
    loop {
        let at = offset.to_string();
        let page = f.listing(
            store,
            &["--namespace", namespace, "--offset", &at, "--limit", "1"],
        );
        assert_eq!(count(&page, "offset"), offset, "{namespace}");
        assert_eq!(count(&page, "page_size"), 1, "{namespace}");
        listed.extend(designations(&page));
        match number(&page, "next_offset") {
            Some(next) => offset = next,
            None => return listed,
        }
    }
}

#[test]
fn every_namespace_lists_and_searches_through_one_command() {
    let f = Fixture::new();
    f.firm();
    let store = f.create("Europe", "europe");

    // Profiles: the search folds case and whitespace, and the hit names its namespace and layer.
    let profiles = f.listing(&store, &["--namespace", "profile", "--filter", "IPE 300"]);
    assert_eq!(designations(&profiles), ["IPE300"]);
    let hit = entry(&profiles, "IPE300");
    assert_eq!(text(hit, "namespace"), "profile");
    assert_eq!(text(hit, "layer"), "project");
    assert_eq!(text(hit, "library_id"), "europe-en-10365-profiles");

    // Fasteners: the shipped DIN set and the firm bolt, every entry a fastener definition, from
    // the project copy and the firm layer.
    let bolts = f.listing(&store, &["--namespace", "fastener"]);
    assert_eq!(
        designations(&bolts),
        ["FIRM-M16", "M12", "M16", "M20", "M24", "M27", "M30"]
    );
    for bolt in list(&bolts, "entries") {
        assert_eq!(text(bolt, "kind"), "fastener");
        assert_eq!(text(bolt, "namespace"), "fastener");
    }
    assert_eq!(text(entry(&bolts, "FIRM-M16"), "layer"), "firm");
    assert_eq!(text(entry(&bolts, "M16"), "layer"), "project");
    assert_eq!(
        text(entry(&bolts, "M16"), "library_id"),
        "beamcraft-din-fasteners"
    );

    // Materials: the eight S355 grades of EN 10025 and the firm grade.
    let grades = f.listing(&store, &["--namespace", "material", "--filter", "s355"]);
    assert_eq!(
        designations(&grades),
        [
            "FIRM-S355",
            "S355J0",
            "S355J2",
            "S355JR",
            "S355K2",
            "S355M",
            "S355ML",
            "S355N",
            "S355NL",
        ]
    );
    assert_eq!(text(entry(&grades, "FIRM-S355"), "kind"), "material");

    // Every namespace through one command: a needle reaches the namespace that holds it and
    // nothing else answers.
    let bolts = f.listing(&store, &["--namespace", "all", "--filter", "m16"]);
    assert_eq!(designations(&bolts), ["FIRM-M16", "M16"]);
    let grades = f.listing(&store, &["--namespace", "all", "--filter", "s355j0"]);
    assert_eq!(designations(&grades), ["S355J0"]);
}

#[test]
fn the_search_text_folds_case_script_and_whitespace() {
    let f = Fixture::new();
    f.firm();
    let store = f.create("Russia", "russia");

    // A lower-case Cyrillic needle finds the upper-case GOST 27772 grades, which the ASCII fold
    // it replaces never matched.
    let grades = f.listing(&store, &["--namespace", "material", "--filter", "с345"]);
    assert_eq!(designations(&grades), ["С345", "С345К"]);
    assert_eq!(
        entry(&grades, "С345").get("namespace").unwrap(),
        &Json::text("material")
    );
    // The Latin look-alike of the Cyrillic letter is another text, never a near match.
    let latin = f.listing(&store, &["--namespace", "material", "--filter", "c345"]);
    assert_eq!(count(&latin, "entry_count"), 0);
    assert!(list(&latin, "entries").is_empty());

    // Search proposes, the exact lookup disposes: the same lower-case text resolves nothing,
    // the grade's own spelling resolves from the project copy.
    f.not_found(&store, &["--namespace", "material"], "с345");
    let exact = f.shown(&store, &["--namespace", "material"], "С345");
    assert_eq!(text(&exact, "layer"), "project");
}

#[test]
fn all_namespaces_page_as_one_position_space() {
    let f = Fixture::new();
    f.firm();
    let store = f.create("Paged", "europe");

    // The three listings the `all` listing concatenates, and the `all` listing walked a page of
    // one entry at a time: the pages tile the three ranges exactly, in namespace order. The
    // profile range outgrew one page of 256 when #790 doubled the europe catalog, so each of
    // the three ranges is fetched full page by full page until it runs out.
    let expected: Vec<String> = ["profile", "fastener", "material"]
        .into_iter()
        .flat_map(|namespace| {
            let mut listed = Vec::new();
            let mut offset = 0u64;
            loop {
                let at = offset.to_string();
                let page = f.listing(
                    &store,
                    &["--namespace", namespace, "--offset", &at, "--limit", "256"],
                );
                let page_designations = designations(&page);
                let fetched = page_designations.len();
                listed.extend(page_designations);
                offset += fetched as u64;
                if number(&page, "next_offset").is_none() {
                    return listed;
                }
            }
        })
        .collect();
    assert_eq!(walk(&f, &store, "all"), expected);
    let first = f.listing(&store, &["--namespace", "all", "--limit", "1"]);
    assert_eq!(count(&first, "entry_count"), expected.len() as u64);

    // A page past the end is empty and last, and bad pages refuse as on every listing.
    let past = expected.len().to_string();
    let past = f.listing(
        &store,
        &["--namespace", "all", "--offset", &past, "--limit", "2"],
    );
    assert!(list(&past, "entries").is_empty());
    assert_eq!(count(&past, "page_size"), 0);
    assert!(number(&past, "next_offset").is_none());
    for bad in [["--limit", "0"], ["--limit", "257"], ["--offset", "-1"]] {
        assert_eq!(
            f.code(&[
                "list-library",
                "--project",
                &store,
                "--namespace",
                "all",
                bad[0],
                bad[1]
            ]),
            "INVALID_ARGUMENT",
            "{bad:?}"
        );
    }
}

#[test]
fn the_shadow_report_covers_every_namespace() {
    let f = Fixture::new();
    f.firm();
    let store = f.create("Shadowed", "europe");

    // The firm file declares one entry of every namespace that the project copy declares too:
    // three shadow records, one per namespace, each answered inside its own resolver.
    let all = f.listing(&store, &["--namespace", "all"]);
    assert_eq!(count(&all, "shadowed_count"), 3);
    let shadowed = list(&all, "shadowed");
    let names: Vec<String> = shadowed.iter().map(|s| text(s, "designation")).collect();
    assert_eq!(names, ["IPE300", "M20", "S355J0"]);
    for record in shadowed {
        assert_eq!(text(record, "used_layer"), "project");
        assert_eq!(text(record, "shadowed_layer"), "firm");
        assert_eq!(text(record, "shadowed_library"), "mine");
    }
    // The libraries of the three resolvers in turn, strongest first inside each: the mixed firm
    // file appears once per namespace it feeds.
    let sources: Vec<(String, String)> = list(&all, "libraries")
        .iter()
        .map(|s| (text(s, "layer"), text(at(s, &["library"]), "id")))
        .collect();
    assert_eq!(
        sources,
        [
            ("project", "europe-en-10365-profiles"),
            ("project", "europe-en-10219-profiles"),
            ("firm", "mine"),
            ("neutral", "blank-profiles"),
            ("project", "beamcraft-din-fasteners"),
            ("firm", "mine"),
            ("project", "europe-en-10025-materials"),
            ("firm", "mine"),
        ]
        .map(|(layer, id)| (layer.to_owned(), id.to_owned()))
    );

    // One namespace reports its own shadows only, and the records page on their own.
    for (namespace, designation) in [
        ("profile", "IPE300"),
        ("fastener", "M20"),
        ("material", "S355J0"),
    ] {
        let listed = f.listing(&store, &["--namespace", namespace]);
        assert_eq!(count(&listed, "shadowed_count"), 1, "{namespace}");
        let own = list(&listed, "shadowed");
        assert_eq!(own.len(), 1);
        assert_eq!(text(&own[0], "designation"), designation);
        assert_eq!(
            list(&listed, "libraries").len(),
            if namespace == "profile" { 4 } else { 2 }
        );
    }
    let page = f.listing(
        &store,
        &["--namespace", "all", "--shadow-offset", "1", "--limit", "1"],
    );
    assert_eq!(count(&page, "shadowed_count"), 3);
    assert_eq!(list(&page, "shadowed").len(), 1);
    assert_eq!(text(&list(&page, "shadowed")[0], "designation"), "M20");
    assert_eq!(count(&page, "shadowed_next_offset"), 2);
    let last = f.listing(
        &store,
        &["--namespace", "all", "--shadow-offset", "3", "--limit", "1"],
    );
    assert!(list(&last, "shadowed").is_empty());
    assert!(number(&last, "shadowed_next_offset").is_none());
}

#[test]
fn an_exact_lookup_stays_exact_and_inside_its_namespace() {
    let f = Fixture::new();
    f.firm();
    let store = f.create("Exact", "europe");

    // Without a namespace the lookup answers from the profiles: no bolt there.
    f.not_found(&store, &[], "M20");

    // The named namespace answers with its strongest layer: the project copy, not the firm bolt
    // of the same designation. The reply and its entry name the namespace and the layer.
    let shown = f.shown(&store, &["--namespace", "fastener"], "M20");
    assert_eq!(text(&shown, "namespace"), "fastener");
    assert_eq!(text(&shown, "layer"), "project");
    assert_eq!(
        text(at(&shown, &["library"]), "id"),
        "beamcraft-din-fasteners"
    );
    assert!(text(&shown, "library_path").ends_with("din-fasteners.usda"));
    let hit = at(&shown, &["entry"]);
    assert_eq!(text(hit, "namespace"), "fastener");
    assert_eq!(text(hit, "layer"), "project");
    assert_eq!(text(hit, "kind"), "fastener");
    assert_eq!(at(hit, &["thread_protrusion_mm"]).float().unwrap(), 6.0);
    let shadows = list(&shown, "shadowed");
    assert_eq!(shadows.len(), 1);
    assert_eq!(text(&shadows[0], "shadowed_layer"), "firm");

    // The same for a grade and a profile designation.
    let grade = f.shown(&store, &["--namespace", "material"], "S355J0");
    assert_eq!(text(&grade, "namespace"), "material");
    assert_eq!(
        at(&grade, &["entry", "density_kg_m3"]).float().unwrap(),
        7850.0
    );
    let beam = f.shown(&store, &["--namespace", "profile"], "IPE300");
    assert_eq!(at(&beam, &["entry", "height_mm"]).float().unwrap(), 300.0);

    // The lookup stays exact in case and in spelling: `m20` is not `M20`, and a designation the
    // set does not publish is `NOT_FOUND`, never a near match.
    for designation in ["m20", "M25"] {
        f.not_found(&store, &["--namespace", "fastener"], designation);
    }
    // It never crosses into another namespace.
    f.not_found(&store, &["--namespace", "profile"], "IPE 300");
    f.not_found(&store, &["--namespace", "profile"], "S355J0");
    f.not_found(&store, &["--namespace", "material"], "IPE300");
}

#[test]
fn a_bad_namespace_option_is_refused() {
    let f = Fixture::new();
    let lib = f.firm();
    let store = f.create("Refused", "europe");

    // A named file is one namespace already; `all` lists and never resolves; a token is exact.
    for (args, message) in [
        (
            vec!["list-library", "--library", &lib, "--namespace", "profile"],
            "--namespace needs --project",
        ),
        (
            vec![
                "show-library",
                "--fastener-library",
                &lib,
                "--designation",
                "M20",
                "--namespace",
                "fastener",
            ],
            "--namespace needs --project",
        ),
        (
            vec![
                "show-library",
                "--project",
                &store,
                "--designation",
                "M20",
                "--namespace",
                "all",
            ],
            "--namespace all lists every namespace; show-library takes one of profile, fastener or material",
        ),
        (
            vec![
                "show-library",
                "--project",
                &store,
                "--designation",
                "M20",
                "--namespace",
                "bolt",
            ],
            "--namespace is one of profile, fastener, material or all",
        ),
        (
            vec![
                "list-library",
                "--project",
                &store,
                "--namespace",
                "profiles",
            ],
            "--namespace is one of profile, fastener, material or all",
        ),
    ] {
        let (code, body) = f.raw(&args);
        assert_eq!(code, 2, "{args:?}: {body:?}");
        assert_eq!(text(&body, "code"), "INVALID_ARGUMENT", "{args:?}");
        let refusal = text(&body, "message");
        assert_eq!(refusal, message, "{args:?}");
        // The registry's probe passes a value that names nothing and reads this word as an
        // option that is not listed.
        assert!(!refusal.contains("unknown"), "{args:?}: {refusal}");
    }
}

#[test]
fn the_reply_without_namespace_is_unchanged() {
    let f = Fixture::new();
    f.firm();
    let store = f.create("Untagged", "europe");

    // The default listing is the profile listing: same entries from the same libraries, without
    // the namespace key.
    let plain = f.listing(&store, &[]);
    let profiles = f.listing(&store, &["--namespace", "profile"]);
    assert_eq!(
        count(&plain, "entry_count"),
        count(&profiles, "entry_count")
    );
    assert!(plain.get("namespace").is_err());
    let tagged = list(&profiles, "entries");
    assert_eq!(list(&plain, "entries").len(), tagged.len());
    for (plain, tagged) in list(&plain, "entries").iter().zip(tagged) {
        assert!(plain.get("namespace").is_err(), "{plain:?}");
        for key in ["designation", "layer", "library_id", "digest", "kind"] {
            assert_eq!(text(plain, key), text(tagged, key));
        }
        assert_eq!(tagged.get("namespace").unwrap(), &Json::text("profile"));
    }
    assert_eq!(list(&plain, "libraries"), list(&profiles, "libraries"));
    assert_eq!(list(&plain, "shadowed"), list(&profiles, "shadowed"));

    // The one difference is the widened search fold, which the default listing shares.
    assert_eq!(
        designations(&f.listing(&store, &["--filter", "IPE 300"])),
        ["IPE300"]
    );

    // `show-library` keeps its shape too.
    let plain = f.shown(&store, &[], "IPE300");
    assert!(plain.get("namespace").is_err() && plain.get("layer").is_err());
    let hit = at(&plain, &["entry"]);
    assert!(hit.get("namespace").is_err() && hit.get("layer").is_err());
    let profiles = f.shown(&store, &["--namespace", "profile"], "IPE300");
    assert_eq!(text(&profiles, "namespace"), "profile");
    assert_eq!(text(&profiles, "layer"), "project");
}

#[test]
fn a_lost_environment_copy_is_reported_in_every_namespace() {
    let f = Fixture::new();
    f.firm();
    let store = f.create("Lost", "europe");

    // The copy folder is gone: the chain resolves from the firm layer and the neutral
    // environment alone, and every namespace's reply carries the loss by name.
    fs::remove_dir_all(Path::new(&store).join("environment")).unwrap();
    let lost = "the project was created with the environment europe version 1, whose copy is no longer in the project";
    let bolts = f.listing(&store, &["--namespace", "fastener"]);
    assert_eq!(designations(&bolts), ["FIRM-M16", "M20"]);
    assert_eq!(
        count(&bolts, "shadowed_count"),
        0,
        "nothing shadows the firm bolts now"
    );
    // The firm file is the one fastener library of the chain and no environment remains: the
    // reply names it under --namespace too, as the reply of a single --library does.
    assert_eq!(text(at(&bolts, &["library"]), "id"), "mine");
    assert!(text(at(&bolts, &["environment_lost"]), "message").contains(lost));
    for namespace in ["profile", "material", "all"] {
        let listed = f.listing(&store, &["--namespace", namespace]);
        assert!(
            text(at(&listed, &["environment_lost"]), "message").contains(lost),
            "{namespace}"
        );
        assert!(listed.get("environment").is_err(), "{namespace}");
    }

    // The hazard the warning names: a designation the copy answered now resolves from the firm
    // layer, to another bolt and another density than the project was created with.
    let bolt = f.shown(&store, &["--namespace", "fastener"], "M20");
    assert_eq!(text(&bolt, "layer"), "firm");
    assert_eq!(
        at(&bolt, &["entry", "thread_protrusion_mm"])
            .float()
            .unwrap(),
        4.0
    );
    assert!(bolt.get("environment_lost").is_ok());
    let grade = f.shown(&store, &["--namespace", "material"], "S355J0");
    assert_eq!(
        at(&grade, &["entry", "density_kg_m3"]).float().unwrap(),
        7800.0
    );
    assert!(grade.get("environment_lost").is_ok());

    // A designation only the copy answered is absent now; nothing is substituted under it.
    f.not_found(&store, &["--namespace", "fastener"], "M16");
    f.not_found(&store, &["--namespace", "material"], "S355J2");

    // A plain project: a namespace no layer of the chain declares lists empty and resolves
    // nothing, without a warning.
    let bare = Fixture::new();
    let store = bare.create("Bare", "blank");
    for namespace in ["fastener", "material"] {
        let listed = bare.listing(&store, &["--namespace", namespace]);
        assert_eq!(count(&listed, "entry_count"), 0, "{namespace}");
        assert!(list(&listed, "libraries").is_empty(), "{namespace}");
        assert!(listed.get("environment_lost").is_err(), "{namespace}");
    }
    bare.not_found(&store, &["--namespace", "material"], "S355J0");
    let all = bare.listing(&store, &["--namespace", "all"]);
    let profiles = bare.listing(&store, &["--namespace", "profile"]);
    assert_eq!(count(&all, "entry_count"), count(&profiles, "entry_count"));
    assert_eq!(
        list(&all, "libraries").len(),
        list(&profiles, "libraries").len()
    );
}

/// The one definition `definitions --name NAME` lists in the store.
fn definition(f: &Fixture, store: &str, name: &str) -> Json {
    let defined = f.ok(&["definitions", "--store", store, "--name", name]);
    let found = list(&defined, "definitions");
    assert_eq!(found.len(), 1, "{name}: {found:?}");
    found[0].clone()
}

/// A number of the one definition `definitions --name NAME` lists, at `path`.
fn number_of(f: &Fixture, store: &str, name: &str, path: &[&str]) -> f64 {
    at(&definition(f, store, name), path).float().unwrap()
}

/// The identity of the one bolt group named `name`, and the digest of its fastener.
fn group_of(f: &Fixture, store: &str, name: &str) -> (String, String) {
    let listed = f.ok(&["bolts", "--store", store]);
    let groups: Vec<&Json> = list(&listed, "bolt_groups")
        .iter()
        .filter(|g| text(g, "name") == name)
        .collect();
    assert_eq!(groups.len(), 1, "{name}: {groups:?}");
    (text(groups[0], "id"), text(groups[0], "fastener"))
}

/// The lookup record of `namespace` in a creation reply.
fn lookup<'a>(reply: &'a Json, namespace: &str) -> &'a Json {
    let found = list(reply, "lookups")
        .iter()
        .find(|l| text(l, "namespace") == namespace);
    assert!(found.is_some(), "no {namespace} lookup: {reply:?}");
    found.unwrap()
}

/// Asserts that `namespace` of the creation `reply` was answered by `layer`: the model (no
/// library is named), or the library `show-library` names for the designation in the project's
/// chain.
fn answered(f: &Fixture, reply: &Json, store: &str, namespace: &str, layer: &str) {
    let found = lookup(reply, namespace);
    assert_eq!(text(found, "layer"), layer, "{namespace}: {reply:?}");
    if layer == "model" {
        assert_eq!(at(found, &["library"]), &Json::Null, "{reply:?}");
        return;
    }
    let designation = text(found, "name");
    let shown = f.shown(store, &["--namespace", namespace], &designation);
    assert_eq!(
        text(found, "library"),
        text(at(&shown, &["library"]), "id"),
        "{namespace}: {reply:?}"
    );
}

#[test]
fn creation_names_resolve_through_the_model_then_the_chain_and_the_reply_says_which() {
    let f = Fixture::new();
    f.firm();
    let store = f.create("Created", "europe");

    // First use: the project's copy answers both namespaces. For each name the reply names the
    // layer and the library that answered (the ones show-library names) and the weaker entry of
    // the same designation.
    let made = f.member(&store, "IPE300", "S355J0");
    assert_eq!(text(&made, "outcome"), "member_created");
    assert_eq!(list(&made, "lookups").len(), 2);
    for namespace in ["profile", "material"] {
        answered(&f, &made, &store, namespace, "project");
        let name = text(lookup(&made, namespace), "name");
        let shadows = list(lookup(&made, namespace), "shadowed");
        assert_eq!(shadows.len(), 1, "{namespace}: {made:?}");
        assert_eq!(text(&shadows[0], "designation"), name);
        assert_eq!(text(&shadows[0], "used_layer"), "project");
        assert_eq!(text(&shadows[0], "shadowed_library"), "mine");
        assert_eq!(text(&shadows[0], "shadowed_layer"), "firm");
    }
    assert!(made.get("environment_lost").is_err(), "{made:?}");
    assert_eq!(
        number_of(&f, &store, "IPE300", &["parameters_mm", "h"]),
        300.0
    );
    assert_eq!(number_of(&f, &store, "S355J0", &["density_kg_m3"]), 7850.0);

    // Second use: the model answers and the chain is never consulted, so nothing is shadowed
    // and no library is named.
    let again = f.member(&store, "IPE300", "S355J0");
    assert_eq!(list(&again, "lookups").len(), 2);
    for namespace in ["profile", "material"] {
        answered(&f, &again, &store, namespace, "model");
        assert!(list(lookup(&again, namespace), "shadowed").is_empty());
    }

    // The layers under the copy: the firm grade for a plate and the neutral fallback for a
    // designation no other layer declares; a seeded constant answers from the model.
    let plate = f.ok(&[
        "create-plate",
        "--store",
        &store,
        "--width",
        "200",
        "--length",
        "200",
        "--thickness",
        "20",
        "--material",
        "FIRM-S355",
    ]);
    assert_eq!(list(&plate, "lookups").len(), 1);
    answered(&f, &plate, &store, "material", "firm");
    assert!(list(lookup(&plate, "material"), "shadowed").is_empty());
    let neutral = f.member(&store, "I-200x100", "Steel");
    answered(&f, &neutral, &store, "profile", "neutral");
    answered(&f, &neutral, &store, "material", "model");

    // Fasteners join the walk, though they skipped the model before: the copy's M20 answers over
    // the firm bolt of the same designation, the firm's own bolt from the firm layer, and a
    // second group of the same designation from the model, with the digest of the first.
    let parts = f.parts(&store);
    let first = f.group(&store, &parts, "G1", &["--designation", "M20"], &[]);
    answered(&f, &first, &store, "fastener", "project");
    let shadows = list(lookup(&first, "fastener"), "shadowed");
    assert_eq!(shadows.len(), 1);
    assert_eq!(text(&shadows[0], "designation"), "M20");
    assert_eq!(text(&shadows[0], "shadowed_layer"), "firm");
    assert_eq!(number_of(&f, &store, "M20", &["thread_protrusion_mm"]), 6.0);
    let firm = f.group(&store, &parts, "G2", &["--designation", "FIRM-M16"], &[]);
    answered(&f, &firm, &store, "fastener", "firm");
    let again = f.group(&store, &parts, "G3", &["--designation", "M20"], &[]);
    answered(&f, &again, &store, "fastener", "model");
    assert!(list(lookup(&again, "fastener"), "shadowed").is_empty());
    assert_eq!(group_of(&f, &store, "G3").1, group_of(&f, &store, "G1").1);

    // The namespaces stay apart: the fastener the head defines never answers a profile lookup,
    // and a profile designation never names a fastener.
    assert_eq!(
        f.code(&[
            "create-member",
            "--store",
            &store,
            "--kind",
            "beam",
            "--start",
            "0,0,0",
            "--end",
            "6000,0,0",
            "--profile",
            "M20",
        ]),
        "NOT_FOUND"
    );
    assert_eq!(
        text(
            &f.group_raw(&store, &parts, "G4", &["--designation", "IPE300"], &[])
                .1,
            "code"
        ),
        "NOT_FOUND"
    );

    // A member's new section goes through the same resolver (a designation the model does not
    // hold yet is defined from the chain first), and a command that gives digests, or no name,
    // keeps its reply as it was.
    let member = text(at(&made, &["member"]), "element_id");
    let updated = f.ok(&[
        "update-element",
        "--store",
        &store,
        "--id",
        &member,
        "--profile",
        "IPE200",
    ]);
    assert_eq!(list(&updated, "lookups").len(), 1);
    answered(&f, &updated, &store, "profile", "project");
    let bare = f.ok(&[
        "create-plate",
        "--store",
        &store,
        "--width",
        "200",
        "--length",
        "200",
        "--thickness",
        "20",
    ]);
    assert!(
        bare.get("lookups").is_err() && bare.get("environment_lost").is_err(),
        "{bare:?}"
    );
}

#[test]
fn the_russia_pack_creates_through_the_same_resolver() {
    let f = Fixture::new();
    let store = f.create("Russia", "russia");

    // The GOST beam, the Cyrillic grade and the GOST bolt all answer from the project's copy,
    // and each reply names the library show-library names for its designation.
    let made = f.member(&store, "30B1", "С245");
    answered(&f, &made, &store, "profile", "project");
    answered(&f, &made, &store, "material", "project");
    assert_eq!(
        number_of(&f, &store, "С245", &["density_kg_m3"]),
        7850.0,
        "the pinned grade is the catalogue entry"
    );
    let parts = f.parts(&store);
    let group = f.group(&store, &parts, "R1", &["--designation", "M16"], &[]);
    answered(&f, &group, &store, "fastener", "project");
    assert_eq!(
        text(lookup(&group, "fastener"), "library"),
        "beamcraft-gost-fasteners"
    );
    assert!(list(lookup(&group, "fastener"), "shadowed").is_empty());
}

#[test]
fn a_lost_environment_is_reported_by_the_creation_replies() {
    let f = Fixture::new();
    f.firm();
    let store = f.create("Lost", "europe");
    let parts = f.parts(&store);
    // A fastener the model pins before the loss.
    f.group(&store, &parts, "G1", &["--designation", "M16"], &[]);

    fs::remove_dir_all(Path::new(&store).join("environment")).unwrap();
    let lost = "the project was created with the environment europe version 1, whose copy is no longer in the project";

    // The hazard the warning names: a name the copy answered now resolves from the firm layer,
    // to another bolt, section and density than the project was created with. The reply says
    // so beside its result instead of substituting silently.
    let group = f.group(&store, &parts, "G2", &["--designation", "M20"], &[]);
    answered(&f, &group, &store, "fastener", "firm");
    assert!(
        text(at(&group, &["environment_lost"]), "message").contains(lost),
        "{group:?}"
    );
    assert_eq!(number_of(&f, &store, "M20", &["thread_protrusion_mm"]), 4.0);
    let member = f.member(&store, "IPE300", "S355J0");
    for namespace in ["profile", "material"] {
        answered(&f, &member, &store, namespace, "firm");
    }
    assert!(
        text(at(&member, &["environment_lost"]), "message").contains(lost),
        "{member:?}"
    );
    assert_eq!(
        number_of(&f, &store, "IPE300", &["parameters_mm", "h"]),
        299.0
    );
    assert_eq!(number_of(&f, &store, "S355J0", &["density_kg_m3"]), 7800.0);

    // A definition the model pinned before the loss answers from the model, before the chain is
    // consulted: no warning, because nothing was substituted.
    let pinned = f.group(&store, &parts, "G3", &["--designation", "M16"], &[]);
    answered(&f, &pinned, &store, "fastener", "model");
    assert!(pinned.get("environment_lost").is_err(), "{pinned:?}");
    assert_eq!(group_of(&f, &store, "G3").1, group_of(&f, &store, "G1").1);

    // A name only the copy answered resolves nothing: NOT_FOUND, never a substitution.
    assert_eq!(
        text(
            &f.group_raw(&store, &parts, "G4", &["--designation", "M24"], &[])
                .1,
            "code"
        ),
        "NOT_FOUND"
    );
    assert_eq!(
        f.code(&[
            "create-member",
            "--store",
            &store,
            "--kind",
            "beam",
            "--start",
            "0,0,0",
            "--end",
            "6000,0,0",
            "--profile",
            "IPE200",
        ]),
        "NOT_FOUND"
    );
}

#[test]
fn two_generations_of_one_designation_refuse_until_a_digest_names_one() {
    let f = Fixture::new();
    f.firm();
    let store = f.create("Generations", "europe");
    let parts = f.parts(&store);
    // Generation one: the copy's M20. Generation two: a file named on the command line answers
    // alone and pins its own M20 beside it.
    f.group(&store, &parts, "G1", &["--designation", "M20"], &[]);
    let file = f.path("generation.usda");
    fs::write(&file, generation_file()).unwrap();
    let second = f.group(
        &store,
        &parts,
        "G2",
        &["--designation", "M20"],
        &["--fastener-library", &file],
    );
    let found = lookup(&second, "fastener");
    assert_eq!(
        (
            text(found, "layer").as_str(),
            text(found, "library").as_str()
        ),
        ("library", "generation"),
        "{second:?}"
    );
    assert!(list(found, "shadowed").is_empty());
    let (one, two) = (group_of(&f, &store, "G1").1, group_of(&f, &store, "G2").1);
    assert_ne!(one, two);

    // The model holds two generations of M20: a third group by designation through the chain
    // refuses, names both digests in its diagnostics, and writes nothing.
    let head = f.head(&store);
    let (code, refused) = f.group_raw(&store, &parts, "G3", &["--designation", "M20"], &[]);
    assert_eq!(code, 2, "{refused:?}");
    assert_eq!(text(&refused, "code"), "INVALID_ARGUMENT");
    let diagnostics = list(&refused, "diagnostics");
    let mut digests: Vec<String> = diagnostics.iter().map(|d| text(d, "digest")).collect();
    digests.sort();
    let mut generations = vec![one.clone(), two.clone()];
    generations.sort();
    assert_eq!(digests, generations, "{refused:?}");
    for candidate in diagnostics {
        assert_eq!(text(candidate, "kind"), "fastener");
        assert!(list(candidate, "names").contains(&Json::text("M20")));
    }
    assert_eq!(f.head(&store), head, "the refusal writes nothing");

    // The digest names one generation exactly, and the reply keeps its shape.
    let chosen = f.group(&store, &parts, "G3", &["--fastener-digest", &one], &[]);
    assert_eq!(group_of(&f, &store, "G3").1, one);
    assert!(chosen.get("lookups").is_err(), "{chosen:?}");

    // The digest option stands alone and names a fastener the head holds: never beside a name
    // or a library file, never a profile, never a digest the head does not hold.
    f.member(&store, "IPE300", "Steel");
    let profile = text(&definition(&f, &store, "IPE300"), "digest");
    let absent = "0".repeat(64);
    for (fastener, expected) in [
        (
            vec!["--fastener-digest", one.as_str(), "--designation", "M20"],
            "INVALID_ARGUMENT",
        ),
        (
            vec![
                "--fastener-digest",
                one.as_str(),
                "--fastener-library",
                file.as_str(),
            ],
            "INVALID_ARGUMENT",
        ),
        (
            vec!["--fastener-digest", profile.as_str()],
            "INVALID_ARGUMENT",
        ),
        (vec!["--fastener-digest", absent.as_str()], "NOT_FOUND"),
    ] {
        let (code, body) = f.group_raw(&store, &parts, "G4", &fastener, &[]);
        assert_eq!(
            (code, text(&body, "code")),
            (2, expected.to_owned()),
            "{fastener:?}: {body:?}"
        );
    }
    assert_eq!(
        group_of(&f, &store, "G3").1,
        one,
        "nothing else was written"
    );

    // A repin follows the current library entry, never the model: with the copy gone the firm
    // layer answers, and the two generations the head holds refuse nothing. The firm bolt
    // carries no washer, so the repin replaces the stored washer switches.
    fs::remove_dir_all(Path::new(&store).join("environment")).unwrap();
    let (target, _) = group_of(&f, &store, "G3");
    let repinned = f.ok(&[
        "update-bolt-group",
        "--store",
        &store,
        "--target",
        &target,
        "--repin",
        "true",
        "--designation",
        "M20",
        "--washer-under-head",
        "false",
        "--washer-under-nut",
        "false",
    ]);
    answered(&f, &repinned, &store, "fastener", "firm");
    assert!(repinned.get("environment_lost").is_ok(), "{repinned:?}");
    let followed = group_of(&f, &store, "G3").1;
    assert_ne!(followed, one);
    assert_ne!(followed, two);
}
