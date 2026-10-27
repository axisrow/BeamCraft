// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
// Contributor-original generated synthetic fixtures, MPL-2.0; covered by ATTRIBUTION.md's
// test-data row. Expectations follow F04 section 2/7/8 and vocabulary 3.1/5.3.

//! U03 UUID and relationship index regressions over the real bounded reader and the U02
//! authoring layer. No SDK or independent reader is involved.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use beamcraft_layer_engine::{
    authored::{Footprint, Group, *},
    index::*,
    *,
};
use beamcraft_model::{
    digest::{ContentDigest, Sha256},
    error::ErrorCode,
};
use std::{
    io::Cursor as IoCursor,
    time::{Duration, Instant},
};

const PLATE: &str = "0199e7a0-0000-7000-8000-000000000003";
const MEMBER: &str = "0199e7a0-0000-7000-8000-000000000004";
const FRESH: &str = "0199e7a0-0000-7000-8000-000000000009";
const ACTOR: &str = "0199e7a0-0000-7000-8000-000000000005";
const OP: &str = "0199e7a0-0000-7000-8000-000000000006";
const LAYERS: [&str; 4] = ["root.usda", "authored.usda", "derived.usda", "weak.usda"];

fn digest(bytes: &[u8]) -> ContentDigest {
    Sha256::digest(bytes).unwrap()
}
fn control() -> Control {
    Control {
        cancellation: Cancellation::default(),
        deadline: Instant::now() + Duration::from_secs(10),
    }
}
fn asset(name: &str, text: impl AsRef<[u8]>) -> Asset {
    Asset {
        name: name.into(),
        digest: digest(text.as_ref()),
        bytes: text.as_ref().to_vec(),
    }
}
fn layer(body: &str) -> String {
    format!("#usda 1.0\n(metersPerUnit=0.001\nupAxis=\"Z\")\n{body}\n")
}
fn request(root: &str, assets: Vec<Asset>, owned: &[&str], page_items: usize) -> OpenRequest {
    let mut request = OpenRequest {
        identity: Identity {
            project: "0199e7a0-0000-7000-8000-000000000001".into(),
            workspace: "0199e7a0-0000-7000-8000-000000000002".into(),
            revision: digest(b"pinned U03 base"),
            generation: 42,
        },
        root: root.into(),
        assets,
        owned_layers: owned.iter().map(|s| (*s).into()).collect(),
        manifest: digest(b"placeholder"),
        backend: BACKEND,
        profile: Profile::StaticLocal,
        budgets: Budgets {
            page_items,
            ..Budgets::FIRST_SLICE
        },
    };
    request.manifest = request.manifest_digest().unwrap();
    request
}
fn code<T>(result: Result<T, Diagnostic>) -> ErrorCode {
    result.err().unwrap().code
}
fn eid(text: &str) -> ElementId {
    ElementId::parse(text).unwrap()
}
fn hex(id: &str) -> String {
    id.replace('-', "")
}

// ---- hand-written reader fixtures -------------------------------------------------------------

fn world(elements: &str, definitions: &str, other: &str) -> String {
    layer(&format!(
        "def Xform \"World\" {{\n def Scope \"Steel\" {{\n def Scope \"Parts\" {{\n{elements}\n}}\n}}\n def Scope \"Definitions\" {{\n def Scope \"Profiles\" {{\n{definitions}\n}}\n}}\n{other}\n}}"
    ))
}
fn el(id: &str, extra: &str) -> String {
    format!(
        "def Xform \"PRT_{}\" {{\n custom string bim:id = \"{id}\"\n custom string bim:name = \"label\"\n{extra}\n}}\n",
        hex(id)
    )
}
fn seq(n: u32) -> String {
    format!("0199e7a0-0000-7000-8000-{n:012x}")
}
fn link(name: &str, id: &str) -> String {
    format!("custom rel {name} = </World/Steel/Parts/PRT_{}>\n", hex(id))
}
fn reader(assets: Vec<Asset>, owned: &[&str], page_items: usize) -> (Reader, Opened) {
    let root = assets[0].name.clone();
    let mut reader = Reader::default();
    let opened = reader
        .open(request(&root, assets, owned, page_items), &control())
        .unwrap();
    (reader, opened)
}
fn single(body: String) -> (Reader, Opened) {
    reader(vec![asset("root.usda", body)], &["root.usda"], 64)
}
fn built(body: String) -> Result<ElementIndex, Diagnostic> {
    let mut r = Reader::default();
    let o = r.open(
        request(
            "root.usda",
            vec![asset("root.usda", body)],
            &["root.usda"],
            64,
        ),
        &control(),
    )?;
    r.element_index(&o.context, &control())
}
fn reader_pages(r: &Reader, context: &Context) -> Vec<Page> {
    let mut pages = Vec::new();
    let mut cursor = None;
    loop {
        let page = r.read(context, cursor.as_ref(), &control()).unwrap();
        cursor = page.next.clone();
        pages.push(page);
        if cursor.is_none() {
            break;
        }
    }
    pages
}

#[test]
fn category_paths_are_checked_against_kinds_and_relationships_resolve_exact_paths() {
    let id = seq(0x139);
    let index = built(world(
        &el(&id, "custom token bim:elementKind = \"beam\""),
        "",
        "",
    ))
    .unwrap();
    let other_category = format!("/World/Steel/Assemblies/ASM_{}", hex(&id));
    assert_eq!(
        code(index.lookup_path(&other_category)),
        ErrorCode::NotFound
    );
    assert_eq!(
        code(built(world(
            &el(&id, "custom token bim:elementKind = \"assembly\""),
            "",
            ""
        ))),
        ErrorCode::InvalidId
    );
    assert_eq!(
        code(built(world(
            &el(&id, &format!("custom rel bim:test:to = <{other_category}>")),
            "",
            ""
        ))),
        ErrorCode::DanglingRelationship
    );
    assert_eq!(
        code(built(world(&el(&id, "").replace("PRT_", "E_"), "", ""))),
        ErrorCode::InvalidId
    );
    let digest = digest(b"synthetic material");
    let wrong_kind = format!(
        "def Scope \"PRF_{digest}\" {{\n custom token bim:definitionKind = \"material\"\n }}"
    );
    assert_eq!(
        code(built(world("", &wrong_kind, ""))),
        ErrorCode::InvalidId
    );
    for definition in [
        "def Scope \"PRF_abc\" {}".to_owned(),
        format!(
            "def Scope \"D_{digest}\" {{ custom string bim:contentDigest = \"{digest}\"\n custom token bim:definitionKind = \"rectangle\" }}"
        ),
    ] {
        assert_eq!(
            code(built(world("", &definition, ""))),
            ErrorCode::InvalidId
        );
    }
    // The fastener category answers the same three checks: a section kind at a fastener path,
    // a fastener kind at a profile path, and a malformed leaf under /Fasteners are refused.
    let fastener_world = |definitions: &str| {
        layer(&format!(
            "def Xform \"World\" {{\n def Scope \"Definitions\" {{\n def Scope \"Fasteners\" {{\n{definitions}\n}}\n}}\n}}"
        ))
    };
    let section_at_fastener = format!(
        "def Scope \"FST_{digest}\" {{\n custom token bim:definitionKind = \"rectangle\"\n }}"
    );
    assert_eq!(
        code(built(fastener_world(&section_at_fastener))),
        ErrorCode::InvalidId
    );
    let fastener_at_profile = format!(
        "def Scope \"PRF_{digest}\" {{\n custom token bim:definitionKind = \"fastener\"\n }}"
    );
    assert_eq!(
        code(built(world("", &fastener_at_profile, ""))),
        ErrorCode::InvalidId
    );
    assert_eq!(
        code(built(fastener_world("def Scope \"FST_abc\" {}"))),
        ErrorCode::InvalidId
    );
}
fn assemble(opened: &Opened, pages: &[Page]) -> Result<ElementIndex, Diagnostic> {
    let mut builder = IndexBuilder::new(opened)?;
    for page in pages {
        builder.push(page, &control())?;
    }
    builder.finish(&control())
}

#[test]
fn ids_are_full_uuidv7_and_same_millisecond_ids_are_never_shortened() {
    let good = "0199e7a0-1234-7abc-8def-0123456789ab";
    let id = eid(good);
    assert_eq!(id.to_string(), good);
    assert_eq!(
        id.path(beamcraft_model::prim::ElementCategory::Part),
        format!("/World/Steel/Parts/PRT_{}", hex(good))
    );
    assert_eq!(
        ElementId::from_path(&id.path(beamcraft_model::prim::ElementCategory::Part)).unwrap(),
        id
    );
    for bad in [
        "",
        "0199e7a0",
        "0199e7a0-1234",
        "0199e7a0-1234-7abc-8def",
        "0199e7a0-1234-7abc-8def-0123456789a",
        "0199e7a0-1234-7abc-8def-0123456789abc",
        "0199E7A0-1234-7ABC-8DEF-0123456789AB",
        "0199e7a0-1234-4abc-8def-0123456789ab",
        "0199e7a0-1234-7abc-cdef-0123456789ab",
        "0199e7a012347abc8def0123456789ab",
        " 0199e7a0-1234-7abc-8def-0123456789ab",
        "0199e7a0-1234-7abc-8def-0123456789ab\n",
        "0199e7a0-1234-7abc-8def-0123456789ªb",
    ] {
        assert_eq!(code(ElementId::parse(bad)), ErrorCode::InvalidId, "{bad:?}");
    }
    let path = id.path(beamcraft_model::prim::ElementCategory::Part);
    for bad in [
        "/World/Steel/Parts/PRT_0199e7a0",
        "/World/Steel/Parts/PRT_0199e7a012347abc8def0123456789a",
        "/World/Steel/Parts/PRT_0199E7A012347ABC8DEF0123456789AB",
        "/World/Steel/Parts/PRT_0199e7a0-1234-7abc-8def-0123456789ab",
        "/World/Other/E_0199e7a012347abc8def0123456789ab",
        "/World/Steel/0199e7a012347abc8def0123456789ab",
        &format!("{path}/Geometry"),
    ] {
        assert_eq!(
            code(ElementId::from_path(bad)),
            ErrorCode::InvalidId,
            "{bad}"
        );
    }

    // 200 elements share the same millisecond prefix and differ only in the low bits.
    let ids: Vec<String> = (0..200u32)
        .map(|i| {
            format!(
                "0199e7a0-1234-7{:03x}-8{:03x}-{:012x}",
                i,
                (i * 5) % 4096,
                i * 977 + 1
            )
        })
        .collect();
    assert!(ids.iter().all(|s| s.starts_with("0199e7a0-1234-7")));
    let body: String = ids.iter().map(|s| el(s, "")).collect();
    let index = built(world(&body, "", "")).unwrap();
    assert_eq!(index.len(), 200);
    for text in &ids {
        let entry = index.lookup(text).unwrap();
        assert_eq!(entry.id.to_string(), *text);
        assert_eq!(entry.path, format!("/World/Steel/Parts/PRT_{}", hex(text)));
        assert_eq!(index.lookup_path(&entry.path).unwrap().id, entry.id);
    }
    // A timestamp or any other prefix never resolves.
    for prefix in ["0199e7a0-1234", "0199e7a0-1234-7000", "0199e7a0123"] {
        assert_eq!(code(index.lookup(prefix)), ErrorCode::InvalidId);
    }
    assert_eq!(
        code(index.lookup_path("/World/Steel/Parts/PRT_0199e7a01234")),
        ErrorCode::InvalidId
    );
    assert_eq!(
        code(index.lookup("0199e7a0-1234-7fff-8fff-ffffffffffff")),
        ErrorCode::NotFound
    );
}

#[test]
fn reader_index_resolves_ids_provenance_and_typed_relationships() {
    let (a, b) = (seq(0x10), seq(0x11));
    let profile = Resource::new(Definition::RectangleProfile {
        width_mm: 100.0,
        height_mm: 50.0,
        section: SectionProperties::default(),
    })
    .unwrap()
    .digest;
    let definition = format!(
        "def Xform \"PRF_{profile}\" {{ custom string bim:contentDigest = \"{profile}\" }}"
    );
    let body = world(
        &(el(
            &a,
            &format!(
                "custom rel bim:profile = </World/Definitions/Profiles/PRF_{profile}>\n{}",
                link("bim:test:supports", &b)
            ),
        ) + &el(&b, "")),
        &definition,
        "",
    );
    let (r, o) = single(body.clone());
    let index = r.element_index(&o.context, &control()).unwrap();
    assert_eq!(index.completeness(), Completeness::Complete);
    assert_eq!(index.len(), 2);
    let entry = index.lookup(&a).unwrap();
    assert_eq!(entry.ownership, Ownership::Owned);
    assert_eq!(entry.lifecycle, Lifecycle::Active);
    assert!(entry.usd_active);
    assert_eq!(entry.label.as_deref(), Some("label"));
    assert_eq!(entry.sources.len(), 1);
    assert_eq!(entry.sources[0].layer, "root.usda");
    assert_eq!(entry.sources[0].digest, digest(body.as_bytes()));
    assert!(entry.sources[0].owned);

    let out = index.outgoing(&eid(&a)).unwrap();
    assert_eq!(
        out.iter()
            .map(|l| (l.name.as_str(), l.ordinal))
            .collect::<Vec<_>>(),
        [("bim:profile", 0), ("bim:test:supports", 0)]
    );
    assert_eq!(out[0].target, LinkTarget::Definition(profile));
    assert_eq!(out[1].target, LinkTarget::Element(eid(&b)));
    let incoming = index.incoming(&eid(&b)).unwrap();
    assert_eq!(incoming.len(), 1);
    assert_eq!(incoming[0].owner, Some(eid(&a)));
    assert!(incoming[0].source_owned);
    assert!(index.incoming(&eid(&a)).unwrap().is_empty());
    assert_eq!(index.incoming_definition(&profile).unwrap().len(), 1);
    assert_eq!(index.definition(&profile).unwrap().digest, profile);
    assert!(index.unresolved().is_empty());

    // Same pinned state: the rebuild is identical and the token commits the full identity.
    let again = r.element_index(&o.context, &control()).unwrap();
    assert_eq!(index, again);
    let token = index.token();
    assert_eq!(token.identity.generation, 42);
    assert_eq!(token.completeness, Completeness::Complete);
    assert_eq!(token.prim_count, o.prim_count);
    assert_eq!(token.structure_digest, again.structure_digest());
    r.verify_index(&index, &control()).unwrap();
}

#[test]
fn foreign_ancestors_descendants_and_dangling_foreign_links_stay_inert_and_read_only() {
    let (own, foreign, ghost, deep) = (seq(0x20), seq(0x21), seq(0x22), seq(0x23));
    let foreign_src = layer(
        "def Xform \"Source\" { custom double future:v = 1\n custom rel foreign:gone = </Source/Missing> }",
    );
    let refs = "(references = @foreign.usda@</Source>)";
    let elements = el(&own, &link("bim:test:to", &foreign))
        + &format!(
            "def Xform \"PRT_{}\" {refs} {{ custom string bim:id = \"{foreign}\" }}\n",
            hex(&foreign)
        )
        + &format!(
            "def Xform \"PRT_{}\" {{ custom string bim:id = \"{ghost}\"\n def Xform \"Child\" {refs} {{ }} }}\n",
            hex(&ghost)
        )
        + &el(&deep, "");
    let (r, o) = reader(
        vec![
            asset("root.usda", world(&elements, "", "")),
            asset("foreign.usda", foreign_src),
        ],
        &["root.usda"],
        64,
    );
    let index = r.element_index(&o.context, &control()).unwrap();
    assert_eq!(index.lookup(&own).unwrap().ownership, Ownership::Owned);
    assert_eq!(index.lookup(&deep).unwrap().ownership, Ownership::Owned);
    // A stronger owned opinion never makes a foreign prim owned.
    assert_eq!(
        index.lookup(&foreign).unwrap().ownership,
        Ownership::Foreign
    );
    assert!(
        index
            .lookup(&foreign)
            .unwrap()
            .sources
            .iter()
            .any(|s| !s.owned)
    );
    // A foreign descendant also taints the owned element that contains it.
    assert_eq!(index.lookup(&ghost).unwrap().ownership, Ownership::Foreign);
    // An owned link to a foreign element resolves and is an incoming dependency of it.
    assert_eq!(index.incoming(&eid(&foreign)).unwrap().len(), 1);
    // A dangling link carried by foreign content is preserved, not fatal, and reported.
    let unresolved = index.unresolved();
    assert!(!unresolved.is_empty());
    assert!(unresolved.iter().all(|l| !l.source_owned));
    assert!(
        unresolved
            .iter()
            .any(|l| matches!(&l.target, LinkTarget::Unresolved(p) if p.ends_with("/Missing")))
    );
}

#[test]
fn duplicate_malformed_misplaced_and_mismatched_identities_are_rejected() {
    let id = seq(0x30);
    // Same bim:id at two parents: duplicate, whichever is visited first.
    let other = format!(
        "def Scope \"Levels\" {{ {} }}",
        el(&id, "").replace("PRT_", "LVL_")
    );
    assert_eq!(
        code(built(world(&el(&id, ""), "", &other))),
        ErrorCode::DuplicateId
    );
    // An element away from /World/Steel/Parts/PRT_<32 hex> has no canonical path.
    assert_eq!(
        code(built(world(
            "",
            "",
            &format!("def Scope \"Other\" {{ {} }}", el(&id, ""))
        ))),
        ErrorCode::InvalidId
    );
    // Short, upper-case and mismatched identities fail even before an index exists.
    let short = "def Xform \"PRT_0199e7a0\" { custom string bim:id = \"0199e7a0\" }";
    let open = |elements: String| {
        let (mut r, _) = (Reader::default(), ());
        code(r.open(
            request(
                "root.usda",
                vec![asset("root.usda", world(&elements, "", ""))],
                &["root.usda"],
                64,
            ),
            &control(),
        ))
    };
    assert_eq!(open(short.into()), ErrorCode::InvalidId);
    assert_eq!(
        open(format!(
            "def Xform \"PRT_{}\" {{ custom string bim:id = \"{}\" }}",
            hex(&seq(0x31)),
            seq(0x32)
        )),
        ErrorCode::InvalidId
    );
    let upper = id.to_uppercase();
    assert_eq!(
        open(format!(
            "def Xform \"PRT_{}\" {{ custom string bim:id = \"{upper}\" }}",
            hex(&upper)
        )),
        ErrorCode::InvalidId
    );
    // Labels never become identity: empty and oversized owned labels are rejected as labels.
    let labelled = |label: &str| {
        built(world(
            &format!(
                "def Xform \"PRT_{}\" {{ custom string bim:id = \"{id}\"\n custom string bim:name = \"{label}\" }}",
                hex(&id)
            ),
            "",
            "",
        ))
    };
    assert_eq!(code(labelled("")), ErrorCode::InvalidArgument);
    assert_eq!(code(labelled(&"x".repeat(257))), ErrorCode::InvalidArgument);
    assert!(labelled(&"x".repeat(256)).is_ok());
}

#[test]
fn tampered_pages_cannot_forge_identity_provenance_or_bounds() {
    let (a, b) = (seq(0x40), seq(0x41));
    let (r, o) = single(world(&(el(&a, "") + &el(&b, "")), "", ""));
    let pages = reader_pages(&r, &o.context);
    assert_eq!(pages.len(), 1);
    let element = pages[0]
        .items
        .iter()
        .position(|p| p.element_id.as_deref() == Some(a.as_str()))
        .unwrap();
    let mutate = |f: &dyn Fn(&mut Prim)| {
        let mut page = pages[0].clone();
        f(&mut page.items[element]);
        assemble(&o, &[page])
    };
    assert!(assemble(&o, &pages).is_ok());
    assert_eq!(
        code(mutate(&|p| p.element_id = Some("0199e7a0".into()))),
        ErrorCode::InvalidId
    );
    // The snapshot identity names another valid UUID than bim:id and the path.
    assert_eq!(
        code(mutate(&|p| p.element_id = Some(seq(0x42)))),
        ErrorCode::InvalidId
    );
    assert_eq!(code(mutate(&|p| p.element_id = None)), ErrorCode::InvalidId);
    // The identity names the other element's UUID while bim:id and the path name this one.
    assert_eq!(
        code(mutate(&|p| p.element_id = Some(b.clone()))),
        ErrorCode::InvalidId
    );
    // A second prim carrying the same complete identity at another path is a duplicate.
    let mut page = pages[0].clone();
    let mut twin = page.items[element].clone();
    twin.path = format!("/World/Other/E_{}", hex(&a));
    let spare = page
        .items
        .iter()
        .position(|p| p.element_id.is_none())
        .unwrap();
    page.items[spare] = twin;
    assert_eq!(code(assemble(&o, &[page])), ErrorCode::DuplicateId);
    assert_eq!(
        code(mutate(&|p| p.sources.clear())),
        ErrorCode::DependencyUnavailable
    );
    // Cap plus one: more items than a page may carry.
    let mut page = pages[0].clone();
    let extra = page.items[0].clone();
    let limit = o.budgets.page_items;
    page.items.extend(std::iter::repeat_n(extra, limit));
    assert_eq!(code(assemble(&o, &[page])), ErrorCode::LimitExceeded);
    // A repeated prim path is an integrity fault.
    let mut page = pages[0].clone();
    page.items.push(page.items[0].clone());
    assert_eq!(code(assemble(&o, &[page])), ErrorCode::IntegrityError);
}

#[test]
fn dangling_short_and_unsupported_targets_of_owned_prims_are_rejected() {
    let (a, b) = (seq(0x50), seq(0x51));
    let missing = seq(0x52);
    let with = |link_text: &str, definitions: &str| {
        built(world(&(el(&a, link_text) + &el(&b, "")), definitions, ""))
    };
    assert_eq!(
        code(with(&link("bim:test:to", &missing), "")),
        ErrorCode::DanglingRelationship
    );
    let absent = "0".repeat(64);
    assert_eq!(
        code(with(
            &format!("custom rel bim:profile = </World/Definitions/Profiles/PRF_{absent}>"),
            ""
        )),
        ErrorCode::DanglingRelationship
    );
    assert_eq!(
        code(with(
            "custom rel bim:test:to = </World/Steel/Parts/PRT_0199e7a0>",
            ""
        )),
        ErrorCode::InvalidId
    );
    assert_eq!(
        code(with(
            &format!(
                "custom rel bim:test:to = </World/Steel/Parts/PRT_{}>",
                hex(&b).to_uppercase()
            ),
            ""
        )),
        ErrorCode::InvalidId
    );
    assert_eq!(
        code(with(
            "custom rel bim:profile = </World/Definitions/Profiles/PRF_abc>",
            ""
        )),
        ErrorCode::InvalidId
    );
    // A digest-less fastener target reports the digest diagnostic, not a dangling link.
    assert_eq!(
        code(with(
            "custom rel bim:test:to = </World/Definitions/Fasteners/FST_abc>",
            ""
        )),
        ErrorCode::InvalidId
    );
    assert_eq!(
        code(with("custom rel bim:test:to = </Nowhere>", "")),
        ErrorCode::DanglingRelationship
    );
    assert_eq!(
        code(with(
            &format!(
                "custom rel bim:test:to = </World/Steel/Parts/PRT_{}.bim:name>",
                hex(&b)
            ),
            ""
        )),
        ErrorCode::UnsupportedProperty
    );
    // Existing non-element targets resolve as plain prims; self links are recorded.
    let ok = with(
        &format!(
            "custom rel bim:test:to = [</World/Steel>, </World/Steel/Parts/PRT_{0}>]",
            hex(&a)
        ),
        "",
    )
    .unwrap();
    let out = ok.outgoing(&eid(&a)).unwrap();
    assert_eq!(out[0].target, LinkTarget::Prim("/World/Steel".into()));
    assert_eq!(out[1].target, LinkTarget::Element(eid(&a)));
    assert_eq!(out[1].ordinal, 1);
}

#[test]
fn lifecycle_is_indexed_deactivated_targets_are_reported_and_reactivation_keeps_identity() {
    let (a, b, c) = (seq(0x60), seq(0x61), seq(0x62));
    let fixture = |state: &str| {
        built(world(
            &(el(&a, &link("bim:test:to", &b))
                + &el(&b, &format!("custom bool bim:isDeactivated = {state}"))
                + &format!(
                    "def Xform \"PRT_{}\" (active = false) {{ custom string bim:id = \"{c}\" }}",
                    hex(&c)
                )),
            "",
            "",
        ))
        .unwrap()
    };
    let off = fixture("true");
    assert_eq!(off.lookup(&b).unwrap().lifecycle, Lifecycle::Deactivated);
    assert_eq!(off.lookup(&a).unwrap().lifecycle, Lifecycle::Active);
    assert!(!off.lookup(&c).unwrap().usd_active);
    // The link stays valid and is reported; the element and its incoming link are retained.
    assert_eq!(off.deactivated_targets().unwrap().len(), 1);
    assert_eq!(off.incoming(&eid(&b)).unwrap().len(), 1);
    let on = fixture("false");
    assert!(on.deactivated_targets().unwrap().is_empty());
    assert_eq!(on.lookup(&b).unwrap().lifecycle, Lifecycle::Active);
    let ids = |i: &ElementIndex| {
        i.elements()
            .map(|e| (e.id, e.path.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(&off), ids(&on));
    assert_ne!(off, on);
    assert_ne!(off.structure_digest(), on.structure_digest());
    for bad in ["archived", "Active", ""] {
        let result = built(world(
            &el(&a, &format!("custom token bim:isDeactivated = \"{bad}\"")),
            "",
            "",
        ));
        assert_eq!(code(result), ErrorCode::InvalidArgument, "{bad:?}");
    }
    assert_eq!(
        code(built(world(
            &el(&a, "custom token bim:isDeactivated = None"),
            "",
            ""
        ))),
        ErrorCode::InvalidArgument
    );
}

#[test]
fn partial_gapped_replayed_and_mixed_context_deliveries_are_never_complete() {
    let ids: Vec<String> = (0x70..0x73).map(seq).collect();
    let body = world(
        &(el(&ids[0], &link("bim:test:to", &ids[2])) + &el(&ids[1], "") + &el(&ids[2], "")),
        "",
        "",
    );
    let assets = || vec![asset("root.usda", body.clone())];
    let (r, o) = reader(assets(), &["root.usda"], 2);
    let pages = reader_pages(&r, &o.context);
    assert_eq!(pages.len(), 4);

    // Missing tail: strict finish refuses; the explicit partial result proves nothing.
    let mut builder = IndexBuilder::new(&o).unwrap();
    builder.push(&pages[0], &control()).unwrap();
    builder.push(&pages[1], &control()).unwrap();
    builder.push(&pages[2], &control()).unwrap();
    assert_eq!(
        code(IndexBuilder::new(&o).and_then(|mut b| {
            b.push(&pages[0], &control())?;
            b.finish(&control())
        })),
        ErrorCode::DependencyUnavailable
    );
    let partial = builder.finish_available(&control()).unwrap();
    assert_eq!(partial.completeness(), Completeness::Partial);
    assert_eq!(partial.token().completeness, Completeness::Partial);
    assert!(partial.lookup(&ids[0]).is_ok());
    // Absence in a partial index is not evidence: the last element has not been delivered.
    assert_eq!(
        code(partial.lookup(&ids[2])),
        ErrorCode::DependencyUnavailable
    );
    assert_eq!(
        code(partial.incoming(&eid(&ids[0]))),
        ErrorCode::DependencyUnavailable
    );
    assert_eq!(
        code(partial.outgoing(&eid(&ids[0]))),
        ErrorCode::DependencyUnavailable
    );
    assert_eq!(
        code(partial.delete_blockers(&eid(&ids[0]))),
        ErrorCode::DependencyUnavailable
    );
    assert_eq!(
        code(partial.deactivated_targets()),
        ErrorCode::DependencyUnavailable
    );
    assert_eq!(
        code(partial.copy_plan(&eid(&ids[0]), &eid(&seq(0x79)))),
        ErrorCode::DependencyUnavailable
    );
    assert_eq!(partial.unresolved().len(), 1);
    assert_eq!(
        code(r.verify_index(&partial, &control())),
        ErrorCode::DependencyUnavailable
    );
    let complete = assemble(&o, &pages).unwrap();
    assert_ne!(partial, complete);

    // A gap, a replay, a page after completion and an exceeded declared count.
    let mut gap = IndexBuilder::new(&o).unwrap();
    assert_eq!(
        code(gap.push(&pages[1], &control())),
        ErrorCode::DependencyUnavailable
    );
    assert_eq!(
        code(gap.finish_available(&control())),
        ErrorCode::IntegrityError
    );
    let mut replay = IndexBuilder::new(&o).unwrap();
    replay.push(&pages[0], &control()).unwrap();
    assert_eq!(
        code(replay.push(&pages[0], &control())),
        ErrorCode::IntegrityError
    );
    assert_eq!(code(replay.finish(&control())), ErrorCode::IntegrityError);
    let mut late = IndexBuilder::new(&o).unwrap();
    for page in &pages {
        late.push(page, &control()).unwrap();
    }
    assert_eq!(
        code(late.push(&pages[2], &control())),
        ErrorCode::InvalidArgument
    );

    // Pages of another reader, or of an earlier incarnation, are mixed-context input.
    let (r2, o2) = reader(assets(), &["root.usda"], 2);
    let foreign_pages = reader_pages(&r2, &o2.context);
    let mut mixed = IndexBuilder::new(&o).unwrap();
    assert_eq!(
        code(mixed.push(&foreign_pages[0], &control())),
        ErrorCode::StaleWorkspace
    );
    let mut r = r;
    let reopened = r
        .open(
            request("root.usda", assets(), &["root.usda"], 2),
            &control(),
        )
        .unwrap();
    let mut stale = IndexBuilder::new(&reopened).unwrap();
    assert_eq!(
        code(stale.push(&pages[0], &control())),
        ErrorCode::StaleWorkspace
    );
    // The old context and index are stale for every consumer after the reopen.
    assert_eq!(
        code(r.element_index(&o.context, &control())),
        ErrorCode::StaleWorkspace
    );
    assert_eq!(
        code(r.verify_index(&complete, &control())),
        ErrorCode::StaleWorkspace
    );
    let fresh = r.element_index(&reopened.context, &control()).unwrap();
    assert_eq!(fresh, complete);
}

// ---- U02 authored scenarios ---------------------------------------------------------------------

fn initial() -> Vec<Asset> {
    vec![
        asset(
            "root.usda",
            "#usda 1.0\n(metersPerUnit=0.001\nupAxis=\"Z\"\nsubLayers=[@authored.usda@,@derived.usda@,@weak.usda@])\n",
        ),
        asset("authored.usda", layer("")),
        asset("derived.usda", layer("")),
        asset("weak.usda", layer("")),
    ]
}
fn targets() -> EditTargets {
    EditTargets {
        authored: "authored.usda".into(),
        derived: "derived.usda".into(),
    }
}
fn open_author(assets: Vec<Asset>) -> (Author, Opened) {
    Author::open(
        request("root.usda", assets, &LAYERS, 2),
        targets(),
        schema_identity().unwrap(),
        &control(),
    )
    .unwrap()
}
fn resources() -> Vec<Resource> {
    vec![
        Resource::new(Definition::RectangleProfile {
            width_mm: 100.0,
            height_mm: 50.0,
            section: SectionProperties::default(),
        })
        .unwrap(),
        Resource::new(Definition::Material {
            density_kg_m3: Some(1000.0),
            grade: None,
            standard: None,
            display_name: None,
        })
        .unwrap(),
    ]
}
fn pose() -> Placement {
    Placement {
        translation_mm: [12.25, -42.125, 0.0],
        orientation_xyzw: [0.0, 0.0, 0.0, 1.0],
    }
}
fn plate() -> Change {
    Change::Create(Element {
        id: PLATE.into(),
        name: "Plate".into(),
        shape: Shape::Plate {
            contour_mm: vec![
                [0.0, 0.0, 0.0],
                [1000.0, 0.0, 0.0],
                [1000.0, 500.0, 0.0],
                [0.0, 500.0, 0.0],
            ],
            thickness_mm: 10.0,
            thickness_side: None,
            corners: Vec::new(),
            holes: Vec::new(),
            hole_records: Vec::new(),
            cuts: Vec::new(),
        },
        placement: pose(),
        material: Some(resources()[1].digest),
    })
}
fn member(id: &str, name: &str) -> Change {
    Change::Create(Element {
        id: id.into(),
        name: name.into(),
        shape: Shape::Member {
            kind: MemberKind::Beam,
            axis_mm: vec![[0.0, 0.0, 0.0], [0.0, 0.0, 2000.0]],
            profile: resources()[0].digest,
            roll_rad: Some(0.125),
            placement: None,
            cuts: Vec::new(),
        },
        placement: pose(),
        material: None,
    })
}
fn apply(author: &mut Author, context: &Context, changes: Vec<Change>) -> Opened {
    let prepared = author
        .prepare(
            Prepare {
                context: context.clone(),
                op_id: OP.into(),
                actor: ACTOR.into(),
                schema: schema_identity().unwrap(),
                resources: resources(),
                changes,
            },
            &control(),
        )
        .unwrap();
    author
        .install(
            context,
            &prepared.token,
            &prepared.op_id,
            Decision::Accepted,
            &control(),
        )
        .unwrap()
}
fn populated() -> (Author, Opened) {
    let (mut a, o) = open_author(initial());
    let o = apply(&mut a, &o.context, vec![plate(), member(MEMBER, "Member")]);
    (a, o)
}
fn over(element: &str, text: &str) -> String {
    format!(
        "over \"World\" {{ over \"Steel\" {{ over \"Parts\" {{ over \"PRT_{}\" {{ {text} }} }} }} }}\n",
        hex(element)
    )
}
fn relayer(author: &Author, context: &Context, weak: &str) -> (Author, Opened) {
    let mut assets = author.export(context, &control()).unwrap();
    *assets.iter_mut().find(|a| a.name == "weak.usda").unwrap() = asset("weak.usda", layer(weak));
    open_author(assets)
}
fn snapshot_of(author: &Author, context: &Context) -> Vec<Prim> {
    let mut all = Vec::new();
    let mut cursor = None;
    loop {
        let page = author.read(context, cursor.as_ref(), &control()).unwrap();
        all.extend(page.items);
        cursor = page.next;
        if cursor.is_none() {
            return all;
        }
    }
}

#[test]
fn rename_preserves_id_path_and_structure_and_invalidates_the_previous_index() {
    let (mut a, o) = populated();
    let before = a.element_index(&o.context, &control()).unwrap();
    assert_eq!(before.len(), 2);
    assert_eq!(
        before.lookup(PLATE).unwrap().label.as_deref(),
        Some("Plate")
    );
    let grant = a.authorize_write(&before, &eid(PLATE), &control()).unwrap();
    assert_eq!(grant.write_layer, "authored.usda");
    assert_eq!(
        grant.path,
        eid(PLATE).path(beamcraft_model::prim::ElementCategory::Part)
    );
    assert_eq!(grant.generation, 42 + 1);

    let renamed = apply(
        &mut a,
        &o.context,
        vec![Change::Label {
            id: PLATE.into(),
            name: "Renamed Ω".into(),
        }],
    );
    let after = a.element_index(&renamed.context, &control()).unwrap();
    let (old, new) = (before.lookup(PLATE).unwrap(), after.lookup(PLATE).unwrap());
    assert_eq!(old.id, new.id);
    assert_eq!(old.path, new.path);
    assert_eq!(new.label.as_deref(), Some("Renamed Ω"));
    assert_eq!(old.sources[0].layer, new.sources[0].layer);
    assert_eq!(before.structure_digest(), after.structure_digest());
    assert_ne!(before, after);
    assert_eq!(
        before.elements().map(|e| e.id).collect::<Vec<_>>(),
        after.elements().map(|e| e.id).collect::<Vec<_>>()
    );
    // The index of the previous incarnation can no longer authorize anything.
    assert_eq!(
        code(a.authorize_write(&before, &eid(PLATE), &control())),
        ErrorCode::StaleWorkspace
    );
    assert_eq!(
        code(a.verify_index(&before, &control())),
        ErrorCode::StaleWorkspace
    );
    assert_eq!(
        a.authorize_write(&after, &eid(PLATE), &control())
            .unwrap()
            .generation,
        renamed.context.identity().generation
    );
}

#[test]
fn authorized_writes_need_a_live_complete_owned_untampered_index() {
    let (a, o) = populated();
    let (a, o) = {
        // A foreign element composed through a reference, plus an owned element's foreign child.
        let mut assets = a.export(&o.context, &control()).unwrap();
        let foreign = asset(
            "foreign.usda",
            layer("def Xform \"Source\" { custom double x = 1 }"),
        );
        let fid = seq(0x80);
        assets.push(foreign);
        let weak = layer(&format!(
            "over \"World\" {{ over \"Steel\" {{ over \"Parts\" {{ def Xform \"PRT_{}\" (references = @foreign.usda@</Source>) {{ custom string bim:id = \"{fid}\" }} }} }} }}",
            hex(&fid)
        ));
        *assets.iter_mut().find(|x| x.name == "weak.usda").unwrap() = asset("weak.usda", weak);
        let (a, o) = Author::open(
            request("root.usda", assets, &[LAYERS.as_slice(), &[]].concat(), 2),
            targets(),
            schema_identity().unwrap(),
            &control(),
        )
        .unwrap();
        (a, o)
    };
    let index = a.element_index(&o.context, &control()).unwrap();
    let fid = eid(&seq(0x80));
    assert_eq!(index.element(&fid).unwrap().ownership, Ownership::Foreign);
    assert_eq!(
        code(a.authorize_write(&index, &fid, &control())),
        ErrorCode::ForeignReadOnly
    );
    assert_eq!(
        code(a.check_delete(&index, &fid, &control())),
        ErrorCode::ForeignReadOnly
    );
    assert!(a.authorize_write(&index, &eid(PLATE), &control()).is_ok());
    assert_eq!(
        code(a.authorize_write(&index, &eid(&seq(0x81)), &control())),
        ErrorCode::NotFound
    );

    // Partial: the identical first page set without its tail cannot authorize.
    let mut builder = IndexBuilder::new(&o).unwrap();
    let first = a.read(&o.context, None, &control()).unwrap();
    builder.push(&first, &control()).unwrap();
    let partial = builder.finish_available(&control()).unwrap();
    assert_eq!(partial.completeness(), Completeness::Partial);
    assert_eq!(
        code(a.authorize_write(&partial, &eid(PLATE), &control())),
        ErrorCode::DependencyUnavailable
    );

    // Forged: a foreign element whose page claims every site is owned still cannot authorize.
    let mut pages = Vec::new();
    let mut cursor = None;
    loop {
        let page = a.read(&o.context, cursor.as_ref(), &control()).unwrap();
        cursor = page.next.clone();
        pages.push(page);
        if cursor.is_none() {
            break;
        }
    }
    for page in &mut pages {
        for prim in &mut page.items {
            for source in &mut prim.sources {
                source.owned = true;
            }
        }
    }
    let forged = assemble(&o, &pages).unwrap();
    assert_eq!(forged.element(&fid).unwrap().ownership, Ownership::Owned);
    assert_eq!(
        code(a.authorize_write(&forged, &fid, &control())),
        ErrorCode::IntegrityError
    );
}

#[test]
fn delete_and_dependent_reads_collide_and_missing_link_evidence_is_unavailable() {
    let (a, o) = populated();
    let weak = over(PLATE, &link("bim:test:supports", MEMBER));
    let (a, o) = relayer(&a, &o.context, &weak);
    let index = a.element_index(&o.context, &control()).unwrap();
    let (plate_id, member_id) = (eid(PLATE), eid(MEMBER));
    let blockers = index.delete_blockers(&member_id).unwrap();
    assert_eq!(blockers.len(), 1);
    assert_eq!(blockers[0].owner, Some(plate_id));
    assert_eq!(index.dependents(&member_id).unwrap(), [plate_id].into());
    assert_eq!(
        code(a.check_delete(&index, &member_id, &control())),
        ErrorCode::DependencyConflict
    );
    // Nothing points to the plate (its own derived children do not count).
    a.check_delete(&index, &plate_id, &control()).unwrap();

    let fp = |id: &str, group| Footprint {
        identity: id.into(),
        group,
    };
    let certify = |a: &Author, i: &ElementIndex, w: &[Footprint], r: &[Footprint]| {
        a.certify_dependent_reads(
            i,
            &w.iter().cloned().collect(),
            &r.iter().cloned().collect(),
            &control(),
        )
    };
    let delete_member = [fp(MEMBER, Group::Lifecycle)];
    for read in [
        fp(PLATE, Group::Placement),
        fp(MEMBER, Group::Label),
        fp(PLATE, Group::Relationship),
    ] {
        assert_eq!(
            code(certify(&a, &index, &delete_member, &[read])),
            ErrorCode::DependencyConflict
        );
    }
    let unrelated = seq(0x90);
    let profile = resources()[0].digest.to_string();
    certify(
        &a,
        &index,
        &delete_member,
        &[fp(&unrelated, Group::Label), fp(&profile, Group::Material)],
    )
    .unwrap();
    // Direction matters: the plate depends on the member, not the other way round.
    certify(
        &a,
        &index,
        &[fp(PLATE, Group::Lifecycle)],
        &[fp(MEMBER, Group::ShapeInputs)],
    )
    .unwrap();
    // A non-lifecycle write is outside this check; short footprint identities are invalid.
    certify(
        &a,
        &index,
        &[fp(MEMBER, Group::Label)],
        &[fp(PLATE, Group::Label)],
    )
    .unwrap();
    assert_eq!(
        code(certify(
            &a,
            &index,
            &delete_member,
            &[fp("0199e7a0", Group::Label)]
        )),
        ErrorCode::InvalidId
    );
    // Creating an element has no dependents to collide with.
    certify(
        &a,
        &index,
        &[fp(&unrelated, Group::Lifecycle)],
        &[fp(PLATE, Group::Label)],
    )
    .unwrap();

    // Incomplete incoming-link evidence never means "no links".
    let mut builder = IndexBuilder::new(&o).unwrap();
    builder
        .push(&a.read(&o.context, None, &control()).unwrap(), &control())
        .unwrap();
    let partial = builder.finish_available(&control()).unwrap();
    for result in [
        a.check_delete(&partial, &member_id, &control()),
        certify(&a, &partial, &delete_member, &[]),
    ] {
        assert_eq!(code(result), ErrorCode::DependencyUnavailable);
    }
    // A forged index that omits the dependency cannot certify a delete either.
    let mut pages = Vec::new();
    let mut cursor = None;
    loop {
        let page = a.read(&o.context, cursor.as_ref(), &control()).unwrap();
        cursor = page.next.clone();
        pages.push(page);
        if cursor.is_none() {
            break;
        }
    }
    for prim in pages.iter_mut().flat_map(|p| p.items.iter_mut()) {
        prim.relationships.retain(|r| r.name != "bim:test:supports");
    }
    let forged = assemble(&o, &pages).unwrap();
    assert!(forged.delete_blockers(&member_id).unwrap().is_empty());
    assert_eq!(
        code(a.check_delete(&forged, &member_id, &control())),
        ErrorCode::IntegrityError
    );
    assert_eq!(
        code(certify(
            &a,
            &forged,
            &delete_member,
            &[fp(PLATE, Group::Label)]
        )),
        ErrorCode::IntegrityError
    );
}

#[test]
fn copy_gets_a_fresh_id_remaps_owned_links_and_leaves_the_source_unchanged() {
    let (a, o) = populated();
    let own = format!(
        "custom rel bim:copy:self = </World/Steel/Parts/PRT_{}>\n",
        hex(MEMBER)
    );
    let (mut a, o) = relayer(&a, &o.context, &over(MEMBER, &own));
    let index = a.element_index(&o.context, &control()).unwrap();
    let (source, fresh) = (eid(MEMBER), eid(FRESH));
    assert_eq!(
        code(index.copy_plan(&source, &source)),
        ErrorCode::DuplicateId
    );
    assert_eq!(
        code(index.copy_plan(&source, &eid(PLATE))),
        ErrorCode::DuplicateId
    );
    assert_eq!(
        code(index.copy_plan(&eid(&seq(0x91)), &fresh)),
        ErrorCode::NotFound
    );
    assert_eq!(
        code(ElementId::parse("0199e7a0-0000")),
        ErrorCode::InvalidId
    );
    let plan = index.copy_plan(&source, &fresh).unwrap();
    assert_eq!(plan.source, source);
    assert_eq!(
        plan.path,
        fresh.path(beamcraft_model::prim::ElementCategory::Part)
    );
    let profile = resources()[0].digest;
    assert_eq!(plan.links.len(), 2);
    assert!(
        plan.links
            .iter()
            .any(|l| l.name == "bim:copy:self" && l.target == LinkTarget::Element(fresh))
    );
    assert!(
        plan.links
            .iter()
            .any(|l| l.name == "bim:profile" && l.target == LinkTarget::Definition(profile))
    );
    // Planning is pure: the index and the pinned snapshot are untouched.
    a.verify_index(&index, &control()).unwrap();

    // Apply it with the real authoring layer: new element, then the remapped link.
    let source_before = snapshot_of(&a, &o.context)
        .into_iter()
        .find(|p| p.element_id.as_deref() == Some(MEMBER))
        .unwrap();
    let copied = apply(&mut a, &o.context, vec![member(FRESH, "Copy")]);
    let remap = format!(
        "custom rel bim:copy:self = </World/Steel/Parts/PRT_{}>\n",
        hex(FRESH)
    );
    let weak = over(MEMBER, &own) + &over(FRESH, &remap);
    let (a, o2) = relayer(&a, &copied.context, &weak);
    let after = a.element_index(&o2.context, &control()).unwrap();
    assert_eq!(after.len(), 3);
    let copy_links: Vec<_> = after
        .outgoing(&fresh)
        .unwrap()
        .into_iter()
        .map(|l| (l.name.clone(), l.target.clone()))
        .collect();
    let planned: Vec<_> = plan
        .links
        .iter()
        .map(|l| (l.name.clone(), l.target.clone()))
        .collect();
    assert_eq!(copy_links, planned);
    // The source keeps its identity, link and fields; nothing now points from it to the copy.
    let source_after = snapshot_of(&a, &o2.context)
        .into_iter()
        .find(|p| p.element_id.as_deref() == Some(MEMBER))
        .unwrap();
    assert_eq!(source_after.properties, source_before.properties);
    assert_eq!(source_after.relationships, source_before.relationships);
    assert_eq!(after.incoming(&source).unwrap().len(), 1);
    assert!(
        after
            .incoming(&fresh)
            .unwrap()
            .iter()
            .all(|l| l.owner == Some(fresh))
    );
    assert_eq!(after.incoming_definition(&profile).unwrap().len(), 2);
    assert_ne!(
        after.lookup(FRESH).unwrap().id,
        after.lookup(MEMBER).unwrap().id
    );

    // Relationships outside the copy set refuse the copy instead of copying incompletely.
    let weak = over(PLATE, &link("bim:test:supports", MEMBER));
    let (b, ob) = relayer(&a, &o2.context, &weak);
    let with_link = b.element_index(&ob.context, &control()).unwrap();
    assert_eq!(
        code(with_link.copy_plan(&eid(PLATE), &eid(&seq(0x92)))),
        ErrorCode::DependencyConflict
    );
}

#[test]
fn foreign_sources_cannot_be_copied() {
    let (r, o) = reader(
        vec![
            asset(
                "root.usda",
                world(
                    &format!(
                        "def Xform \"PRT_{}\" (references = @foreign.usda@</Source>) {{ custom string bim:id = \"{}\" }}",
                        hex(&seq(0xa0)),
                        seq(0xa0)
                    ),
                    "",
                    "",
                ),
            ),
            asset("foreign.usda", layer("def Xform \"Source\" { }")),
        ],
        &["root.usda"],
        64,
    );
    let index = r.element_index(&o.context, &control()).unwrap();
    assert_eq!(
        code(index.copy_plan(&eid(&seq(0xa0)), &eid(&seq(0xa1)))),
        ErrorCode::ForeignReadOnly
    );
}

// ---- real files: save, reopen, fresh readers ----------------------------------------------------

fn directory(name: &str) -> std::path::PathBuf {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("u03-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
fn save_and_load(name: &str, assets: &[Asset]) -> Vec<Asset> {
    let dir = directory(name);
    for a in assets {
        std::fs::write(dir.join(&a.name), &a.bytes).unwrap();
    }
    assets
        .iter()
        .map(|a| {
            let bytes = std::fs::read(dir.join(&a.name)).unwrap();
            assert_eq!(digest(&bytes), a.digest);
            asset(&a.name, bytes)
        })
        .collect()
}
fn usdc(text: &str) -> Vec<u8> {
    let data = openusd::usda::parse(text).unwrap();
    let mut out = IoCursor::new(Vec::new());
    openusd::usdc::CrateWriter::write(&data, &mut out).unwrap();
    out.into_inner()
}
type Elements = Vec<(ElementId, String, Option<String>, Option<String>, bool)>;
type Links = Vec<(String, String, usize, LinkTarget)>;
fn shape(index: &ElementIndex) -> (Elements, Links) {
    (
        index
            .elements()
            .map(|e| {
                (
                    e.id,
                    e.path.clone(),
                    e.kind.clone(),
                    e.label.clone(),
                    e.ownership == Ownership::Owned,
                )
            })
            .collect(),
        index
            .elements()
            .flat_map(|e| index.outgoing(&e.id).unwrap())
            .map(|l| {
                (
                    l.source_path.clone(),
                    l.name.clone(),
                    l.ordinal,
                    l.target.clone(),
                )
            })
            .collect(),
    )
}

#[test]
fn index_rebuilds_identically_from_saved_usda_and_usdc_files_and_fresh_readers() {
    let (a, o) = populated();
    let relink = over(PLATE, &link("bim:test:supports", MEMBER));
    let (a, o) = relayer(&a, &o.context, &relink);
    let original = a.element_index(&o.context, &control()).unwrap();
    assert_eq!(original.len(), 2);
    assert_eq!(original.incoming(&eid(MEMBER)).unwrap().len(), 1);

    // Save the owned layers as real files, then read them back with new readers/authors.
    let saved = a.export(&o.context, &control()).unwrap();
    let files = save_and_load("usda", &saved);
    let mut reader = Reader::default();
    let opened = reader
        .open(request("root.usda", files.clone(), &LAYERS, 64), &control())
        .unwrap();
    let from_reader = reader.element_index(&opened.context, &control()).unwrap();
    assert_eq!(from_reader, original);
    assert_eq!(from_reader.structure_digest(), original.structure_digest());
    let (author, opened) = open_author(files);
    let from_author = author.element_index(&opened.context, &control()).unwrap();
    assert_eq!(from_author, original);
    // Reopen again: the rebuild does not depend on the previous index or incarnation.
    reader
        .open(request("root.usda", saved.clone(), &LAYERS, 64), &control())
        .unwrap();
    let ctx = reader
        .open(request("root.usda", saved.clone(), &LAYERS, 3), &control())
        .unwrap()
        .context;
    assert_eq!(reader.element_index(&ctx, &control()).unwrap(), original);

    // Any changed link or identity changes the index; the digest is not a constant.
    let (mut b, ob) = relayer(&a, &o.context, &relink);
    let ob = apply(
        &mut b,
        &ob.context,
        vec![Change::Label {
            id: MEMBER.into(),
            name: "Other".into(),
        }],
    );
    let labelled = b.element_index(&ob.context, &control()).unwrap();
    assert_ne!(labelled, original);
    assert_eq!(labelled.structure_digest(), original.structure_digest());
    let (c, oc) = relayer(&a, &o.context, "");
    let without = c.element_index(&oc.context, &control()).unwrap();
    assert_ne!(without.structure_digest(), original.structure_digest());
}

#[test]
fn deactivated_and_reactivated_authored_elements_keep_identity_through_reopen() {
    let (a, o) = populated();
    let weak = |state: &str| {
        over(
            MEMBER,
            &format!("custom bool bim:isDeactivated = {state}\n"),
        ) + &over(PLATE, &link("bim:test:supports", MEMBER))
    };
    let (a1, o1) = relayer(&a, &o.context, &weak("true"));
    let off = a1.element_index(&o1.context, &control()).unwrap();
    assert_eq!(
        off.lookup(MEMBER).unwrap().lifecycle,
        Lifecycle::Deactivated
    );
    assert_eq!(off.deactivated_targets().unwrap().len(), 1);
    // Deactivation retains identity and the incoming link, so deleting is still blocked.
    assert_eq!(
        code(a1.check_delete(&off, &eid(MEMBER), &control())),
        ErrorCode::DependencyConflict
    );
    let (a2, o2) = relayer(&a1, &o1.context, &weak("false"));
    let on = a2.element_index(&o2.context, &control()).unwrap();
    assert_eq!(on.lookup(MEMBER).unwrap().lifecycle, Lifecycle::Active);
    assert!(on.deactivated_targets().unwrap().is_empty());
    assert_eq!(
        on.lookup(MEMBER).unwrap().id,
        off.lookup(MEMBER).unwrap().id
    );
    assert_eq!(
        on.lookup(MEMBER).unwrap().path,
        off.lookup(MEMBER).unwrap().path
    );
    assert_eq!(
        on.outgoing(&eid(PLATE)).unwrap(),
        off.outgoing(&eid(PLATE)).unwrap()
    );
}

#[test]
fn index_budgets_and_cancellation_fail_closed() {
    let ids: Vec<String> = (0xb0..0xb4).map(seq).collect();
    let body: String = ids.iter().map(|i| el(i, "")).collect();
    let (r, o) = reader(
        vec![asset("root.usda", world(&body, "", ""))],
        &["root.usda"],
        64,
    );
    let cancelled = Control {
        cancellation: Cancellation::default(),
        deadline: Instant::now() + Duration::from_secs(10),
    };
    cancelled.cancellation.cancel();
    assert_eq!(
        code(r.element_index(&o.context, &cancelled)),
        ErrorCode::Cancelled
    );
    let mut builder = IndexBuilder::new(&o).unwrap();
    let page = r.read(&o.context, None, &control()).unwrap();
    assert_eq!(code(builder.push(&page, &cancelled)), ErrorCode::Cancelled);
    assert_eq!(code(builder.finish(&control())), ErrorCode::IntegrityError);
    // The reader is unaffected and still produces the complete index.
    assert_eq!(
        r.element_index(&o.context, &control()).unwrap().len(),
        ids.len()
    );
    let empty = built(layer("")).unwrap();
    assert!(empty.is_empty());
    assert_eq!(empty.completeness(), Completeness::Complete);
    assert_eq!(code(empty.lookup(&seq(1))), ErrorCode::NotFound);
}

#[test]
fn binary_and_text_encodings_index_the_same_identities() {
    let (a, b) = (seq(0xc0), seq(0xc1));
    let text = world(&(el(&a, "") + &el(&b, "")), "", "");
    let (r, o) = single(text.clone());
    let from_text = r.element_index(&o.context, &control()).unwrap();
    let files = save_and_load("usdc", &[asset("root.usdc", usdc(&text))]);
    let (r, o) = reader(files, &["root.usdc"], 64);
    let from_binary = r.element_index(&o.context, &control()).unwrap();
    assert_eq!(shape(&from_binary), shape(&from_text));
    assert_eq!(from_binary.len(), 2);
    // The restricted binary profile refuses relationship specs, so only identities are compared.
    assert!(from_binary.incoming(&eid(&b)).unwrap().is_empty());
    assert_eq!(
        from_binary.lookup(&a).unwrap().sources[0].layer,
        "root.usdc"
    );
    // Same identities, different layer provenance: the indexes are not equal.
    assert_ne!(from_binary, from_text);
}

// ---- review regressions -------------------------------------------------------------------------

#[test]
fn links_to_descendant_prims_are_dependencies_of_the_containing_element() {
    let (a, b) = (seq(0xd0), seq(0xd1));
    let child = format!("</World/Steel/Parts/PRT_{}/Geometry>", hex(&b));
    let body = world(
        &(el(&a, &format!("custom rel bim:test:to = {child}"))
            + &el(
                &b,
                &format!(
                    "def Scope \"Geometry\" {{ custom rel bim:test:self = </World/Steel/Parts/PRT_{}/Geometry> }}",
                    hex(&b)
                ),
            )),
        "",
        "",
    );
    let index = built(body).unwrap();
    let (a_id, b_id) = (eid(&a), eid(&b));
    let incoming = index.incoming(&b_id).unwrap();
    assert_eq!(incoming.len(), 2);
    assert!(incoming.iter().any(|l| l.owner == Some(a_id)
        && matches!(&l.target, LinkTarget::Prim(p) if p.ends_with("/Geometry"))
        && l.target_owner == Some(b_id)));
    // The element's own descendant does not block its delete; the external link does.
    let blockers = index.delete_blockers(&b_id).unwrap();
    assert_eq!(blockers.len(), 1);
    assert_eq!(index.dependents(&b_id).unwrap(), [a_id].into());
    assert!(index.delete_blockers(&a_id).unwrap().is_empty());
}

#[test]
fn descendant_links_block_delete_and_collide_with_reads_through_the_author_guards() {
    let (a, o) = populated();
    let weak = over(MEMBER, "def Scope \"Geometry\" { }\n")
        + &over(
            PLATE,
            &format!(
                "custom rel bim:test:to = </World/Steel/Parts/PRT_{}/Geometry>\n",
                hex(MEMBER)
            ),
        );
    let (a, o) = relayer(&a, &o.context, &weak);
    let index = a.element_index(&o.context, &control()).unwrap();
    assert_eq!(
        code(a.check_delete(&index, &eid(MEMBER), &control())),
        ErrorCode::DependencyConflict
    );
    let write = [Footprint {
        identity: MEMBER.into(),
        group: Group::Lifecycle,
    }];
    let read = [Footprint {
        identity: PLATE.into(),
        group: Group::Placement,
    }];
    assert_eq!(
        code(a.certify_dependent_reads(
            &index,
            &write.iter().cloned().collect(),
            &read.iter().cloned().collect(),
            &control()
        )),
        ErrorCode::DependencyConflict
    );
}

#[test]
fn a_write_of_a_fresh_id_collides_with_a_read_of_its_absence() {
    let (a, o) = populated();
    let index = a.element_index(&o.context, &control()).unwrap();
    let fp = |id: &str| Footprint {
        identity: id.into(),
        group: Group::Lifecycle,
    };
    let certify = |w: &str, r: &str| {
        a.certify_dependent_reads(
            &index,
            &[fp(w)].into_iter().collect(),
            &[fp(r)].into_iter().collect(),
            &control(),
        )
    };
    assert_eq!(code(certify(FRESH, FRESH)), ErrorCode::DependencyConflict);
    certify(FRESH, &seq(0xe0)).unwrap();
}

#[test]
fn copy_plan_refuses_relationships_carried_by_descendant_prims() {
    let (a, o) = populated();
    let weak = over(
        MEMBER,
        &format!(
            "def Scope \"Child\" {{ custom rel bim:test:to = </World/Steel/Parts/PRT_{}> }}\n",
            hex(PLATE)
        ),
    );
    let (a, o) = relayer(&a, &o.context, &weak);
    let index = a.element_index(&o.context, &control()).unwrap();
    let outgoing = index.outgoing(&eid(MEMBER)).unwrap();
    assert!(outgoing.iter().any(|l| l.source_path.ends_with("/Child")));
    assert_eq!(
        code(index.copy_plan(&eid(MEMBER), &eid(FRESH))),
        ErrorCode::DependencyConflict
    );
}

#[test]
fn labels_that_authoring_accepts_are_always_indexable() {
    let (mut a, o) = populated();
    // U+0085 is outside the F04 control-character rule (U+0000-001F, U+007F).
    let name = "a\u{85}b";
    let o = apply(
        &mut a,
        &o.context,
        vec![Change::Label {
            id: PLATE.into(),
            name: name.into(),
        }],
    );
    let index = a.element_index(&o.context, &control()).unwrap();
    assert_eq!(index.lookup(PLATE).unwrap().label.as_deref(), Some(name));
    a.authorize_write(&index, &eid(PLATE), &control()).unwrap();
    for bad in ["", "\u{1f}", "\u{7f}"] {
        let result = built(world(
            &format!(
                "def Xform \"PRT_{0}\" {{ custom string bim:id = \"{1}\"\n custom string bim:name = \"x{bad}\" }}",
                hex(&seq(0xf0)),
                seq(0xf0)
            ),
            "",
            "",
        ));
        if bad.is_empty() {
            assert!(result.is_ok());
        } else {
            assert_eq!(code(result), ErrorCode::InvalidArgument);
        }
    }
}

#[test]
fn every_present_lifecycle_declaration_is_validated() {
    let id = seq(0xf1);
    for declaration in [
        "custom bool bim:isDeactivated",
        "custom bool bim:isDeactivated = None",
        "custom token bim:isDeactivated",
        "custom string bim:isDeactivated",
        "custom int bim:isDeactivated",
        "custom int bim:isDeactivated = 3",
        "custom string bim:isDeactivated = \"\"",
    ] {
        assert_eq!(
            code(built(world(&el(&id, declaration), "", ""))),
            ErrorCode::InvalidArgument,
            "{declaration}"
        );
    }
    assert!(
        built(world(
            &el(&id, "custom bool bim:isDeactivated = false"),
            "",
            ""
        ))
        .is_ok()
    );
}

#[test]
fn caller_held_pages_are_bounded_and_charged_before_they_are_copied() {
    let (a, b) = (seq(0xf2), seq(0xf3));
    let (r, o) = single(world(&(el(&a, "") + &el(&b, "")), "", ""));
    let pages = reader_pages(&r, &o.context);
    let element = pages[0]
        .items
        .iter()
        .position(|p| p.element_id.as_deref() == Some(a.as_str()))
        .unwrap();
    let mutate = |f: &dyn Fn(&mut Prim)| {
        let mut page = pages[0].clone();
        f(&mut page.items[element]);
        assemble(&o, &[page])
    };
    // Ignored properties are counted against the cumulative value budget.
    let ignored = Property {
        name: "future:x".into(),
        type_name: "double".into(),
        value: Some(Value::Number(1.0)),
        interpolation: None,
    };
    assert_eq!(
        code(mutate(&|p| p
            .properties
            .extend(std::iter::repeat_n(ignored.clone(), 5000)))),
        ErrorCode::LimitExceeded
    );
    // Excessive provenance, one relationship with oversized targets, and oversized names.
    let source = pages[0].items[element].sources[0].clone();
    assert_eq!(
        code(mutate(&|p| p
            .sources
            .extend(std::iter::repeat_n(source.clone(), 9000)))),
        ErrorCode::LimitExceeded
    );
    let long = format!("/World/{}", "x".repeat(1000));
    assert_eq!(
        code(mutate(&|p| p.relationships.push(Relationship {
            name: "bim:test:big".into(),
            targets: vec![long.clone(); 1024],
        }))),
        ErrorCode::LimitExceeded
    );
    // Ignored properties are never copied, so an oversized ignored name is harmless.
    assert!(
        mutate(&|p| p.properties.push(Property {
            name: "x".repeat(2000),
            type_name: "string".into(),
            value: None,
            interpolation: None,
        }))
        .is_ok()
    );
    // An oversized or non-text value of an index-relevant property is inert, never copied:
    // as a label it is rejected for an owned element, not retained.
    let huge = |value: Value| {
        mutate(&|p| {
            if let Some(n) = p.properties.iter_mut().find(|q| q.name == "bim:name") {
                n.value = Some(value.clone());
            }
        })
    };
    assert_eq!(
        code(huge(Value::Text("x".repeat(100_000)))),
        ErrorCode::InvalidArgument
    );
    assert_eq!(
        code(huge(Value::Array(vec![
            Value::Array(vec![Value::Number(
                1.0
            )]);
            8
        ]))),
        ErrorCode::InvalidArgument
    );
    assert!(assemble(&o, &pages).is_ok());
}
