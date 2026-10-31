// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! U22 coordination-record regressions with original in-memory fixtures: exact round-trips
//! through a real save/reopen cycle, and refusals of unknown, oversized and malformed records
//! with F04 codes (F10 0.2 sections 2 to 4, F12 sections 2 to 4 and 9). 0.1 stores must keep
//! their current contracts, so one case validates an ordinary 0.1 layer that carries inert
//! record content and one case proves the 0.1 line never accepts the 0.2 pin.

#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use beamcraft_layer_engine::authored::coordination;
use beamcraft_layer_engine::authored::{schema_identity, validate_layers};
use beamcraft_layer_engine::*;
use beamcraft_model::coordination::{
    AuthoredScope, DecisionReason, DecisionVerdict, MAX_SECTIONS, OpeningDecision, ProjectMetadata,
    SectionRecord, ZoneRecord,
};
use beamcraft_model::digest::{ContentDigest, Sha256};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::Uuid7;
use openusd::sdf::{self, AbstractData};
use openusd::usda::TextWriter;
use std::time::{Duration, Instant};

const SECTION: &str = "0199e7a0-0000-7000-8000-000000000010";
const SECTION_B: &str = "0199e7a0-0000-7000-8000-000000000011";
const ZONE: &str = "0199e7a0-0000-7000-8000-000000000012";
const DECISION_A: &str = "0199e7a0-0000-7000-8000-000000000013";
const DECISION_B: &str = "0199e7a0-0000-7000-8000-000000000014";
const PLATE: &str = "0199e7a0-0000-7000-8000-000000000016";
const ACTOR: &str = "0199e7a0-0000-7000-8000-000000000017";

fn control() -> Control {
    Control {
        cancellation: Cancellation::default(),
        deadline: Instant::now() + Duration::from_secs(10),
    }
}
fn digest(bytes: &[u8]) -> ContentDigest {
    Sha256::digest(bytes).unwrap()
}
fn asset(name: &str, bytes: Vec<u8>) -> Asset {
    Asset {
        name: name.into(),
        digest: digest(&bytes),
        bytes,
    }
}
fn hex(id: &str) -> String {
    Uuid7::parse(id).unwrap().hex32()
}
fn pin() -> String {
    coordination::schema_identity_v02()
        .unwrap()
        .digest
        .to_string()
}
/// Serializes an authored in-memory layer: the exact bytes a save would store.
fn serialize(data: &sdf::Data) -> Vec<u8> {
    let mut bytes = Vec::new();
    TextWriter::write(data, &mut bytes).unwrap();
    bytes
}
fn root_pin(data: &mut sdf::Data, pin: Option<&str>) {
    let mut dictionary = sdf::Value::Dictionary(Default::default());
    if let Some(pin) = pin {
        dictionary = sdf::Value::Dictionary(
            [(
                "bimSchemaDigest".to_owned(),
                sdf::Value::String(pin.to_owned()),
            )]
            .into_iter()
            .collect(),
        );
    }
    data.set_field(&sdf::Path::abs_root(), "customLayerData", dictionary);
}
/// An empty 0.2-mapped root layer, ready for record authoring.
fn v02_layer() -> sdf::Data {
    let mut data = sdf::Data::new();
    data.create_spec(sdf::Path::abs_root(), sdf::SpecType::PseudoRoot);
    data.set_field(
        &sdf::Path::abs_root(),
        "metersPerUnit",
        sdf::Value::Double(0.001),
    );
    data.set_field(&sdf::Path::abs_root(), "upAxis", sdf::Value::token("Z"));
    root_pin(&mut data, Some(&pin()));
    data
}
fn request(assets: Vec<Asset>) -> OpenRequest {
    let mut request = OpenRequest {
        identity: Identity {
            project: "0199e7a0-0000-7000-8000-000000000001".into(),
            workspace: "0199e7a0-0000-7000-8000-000000000002".into(),
            revision: digest(b"coordination base"),
            generation: 7,
        },
        root: assets.first().unwrap().name.clone(),
        owned_layers: [assets.first().unwrap().name.clone()].into(),
        assets,
        manifest: digest(b"pending"),
        backend: BACKEND,
        profile: Profile::StaticLocal,
        budgets: Budgets::default(),
    };
    request.manifest = request.manifest_digest().unwrap();
    request
}
/// Saves and reopens one layer through the real backend, returning every composed prim.
fn reopen(bytes: Vec<u8>) -> (Reader, Opened, Vec<Prim>) {
    let mut reader = Reader::default();
    let opened = reader
        .open(request(vec![asset("authored.usda", bytes)]), &control())
        .unwrap();
    let mut items = Vec::new();
    let mut cursor = None;
    loop {
        let page = reader
            .read(&opened.context, cursor.as_ref(), &control())
            .unwrap();
        items.extend(page.items);
        match page.next {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    (reader, opened, items)
}
fn section(id: &str, code: &str) -> SectionRecord {
    SectionRecord {
        id: id.into(),
        code: code.into(),
        label: format!("Section {code}"),
        modules: vec!["steel".into()],
    }
}
fn metadata() -> ProjectMetadata {
    ProjectMetadata {
        code: "P-2026".into(),
        name: "Bridge".into(),
        organization: Some("Apibim".into()),
    }
}
fn accepted(proposal: &str) -> OpeningDecision {
    OpeningDecision {
        id: DECISION_A.into(),
        proposal: proposal.into(),
        proposal_digest: digest(b"proposal bytes"),
        host_revision: "0199e7a0-0000-7000-8000-000000000018".into(),
        verdict: DecisionVerdict::Accepted,
        reason: None,
    }
}
fn rejected(proposal: &str) -> OpeningDecision {
    OpeningDecision {
        id: DECISION_B.into(),
        proposal: proposal.into(),
        proposal_digest: digest(b"other proposal bytes"),
        host_revision: "0199e7a0-0000-7000-8000-000000000018".into(),
        verdict: DecisionVerdict::Rejected,
        reason: Some(DecisionReason::Size),
    }
}
/// The complete authored record set this suite round-trips.
fn populate(data: &mut sdf::Data) {
    coordination::author_metadata(data, &metadata()).unwrap();
    coordination::author_section(data, &section(SECTION, "KMD")).unwrap();
    coordination::author_section(
        data,
        &SectionRecord {
            modules: Vec::new(),
            ..section(SECTION_B, "KM")
        },
    )
    .unwrap();
    coordination::author_zone(
        data,
        &ZoneRecord {
            id: ZONE.into(),
            code: "K1".into(),
            label: "Zone K1".into(),
        },
    )
    .unwrap();
    coordination::author_decision(data, &accepted(SECTION)).unwrap();
    coordination::author_decision(data, &rejected(SECTION_B)).unwrap();
    coordination::author_scope(
        data,
        "/World/Scratch/Holder",
        Some(&AuthoredScope {
            section: SECTION.into(),
            zone: Some(ZONE.into()),
        }),
    )
    .unwrap();
}

#[test]
fn records_round_trip_through_save_and_reopen() {
    let mut data = v02_layer();
    populate(&mut data);
    let saved = serialize(&data);
    let (_, opened, prims) = reopen(saved.clone());
    let view = coordination::read(&opened.metadata, &prims).unwrap();
    assert_eq!(view.metadata, Some(metadata()));
    let mut sections = view.sections.clone();
    sections.sort_by(|a, b| a.id.cmp(&b.id));
    let mut expected = vec![
        section(SECTION, "KMD"),
        SectionRecord {
            modules: Vec::new(),
            ..section(SECTION_B, "KM")
        },
    ];
    expected.sort_by(|a, b| a.id.cmp(&b.id));
    assert_eq!(sections, expected);
    assert_eq!(
        view.zones,
        vec![ZoneRecord {
            id: ZONE.into(),
            code: "K1".into(),
            label: "Zone K1".into()
        }]
    );
    assert_eq!(view.decisions, vec![accepted(SECTION), rejected(SECTION_B)]);
    let element = prims
        .iter()
        .find(|p| p.path == "/World/Scratch/Holder")
        .unwrap();
    assert_eq!(
        coordination::authored_scope(element).unwrap(),
        Some(AuthoredScope {
            section: SECTION.into(),
            zone: Some(ZONE.into()),
        })
    );
    // A second reopen of the same saved bytes is identical: the save is reproducible.
    let (_, reopened, prims) = reopen(saved);
    assert_eq!(
        coordination::read(&reopened.metadata, &prims).unwrap(),
        view
    );
}

#[test]
fn scope_removal_round_trips_as_unknown_provenance() {
    let mut data = v02_layer();
    let path = "/World/Scratch/Holder";
    let scope = AuthoredScope {
        section: SECTION.into(),
        zone: Some(ZONE.into()),
    };
    coordination::author_scope(&mut data, path, Some(&scope)).unwrap();
    let saved = serialize(&data);
    let (_, _, prims) = reopen(saved);
    let element = prims.iter().find(|p| p.path == path).unwrap();
    assert_eq!(coordination::authored_scope(element).unwrap(), Some(scope));
    // Removing the record authors neither attribute; the element reads as unknown provenance.
    coordination::author_scope(&mut data, path, None).unwrap();
    let saved = serialize(&data);
    let (_, _, prims) = reopen(saved);
    let element = prims.iter().find(|p| p.path == path).unwrap();
    assert!(coordination::authored_scope(element).unwrap().is_none());
    assert!(
        element
            .properties
            .iter()
            .all(|p| p.name != "bim:sectionId" && p.name != "bim:zoneId")
    );
}

#[test]
fn pin_is_exact_and_distinguishes_the_compatibility_lines() {
    let identity = coordination::schema_identity_v02().unwrap();
    assert_eq!(identity.model, "0.2.0");
    assert_eq!(coordination::schema_identity_v02().unwrap(), identity);
    assert_ne!(identity, schema_identity().unwrap());
    assert!(coordination::pin_accepted_v02(&identity.digest.to_string()).unwrap());
    for refused in [
        schema_identity().unwrap().digest.to_string(),
        String::new(),
        "unknown".into(),
    ] {
        assert!(!coordination::pin_accepted_v02(&refused).unwrap());
    }
    // A layer without the 0.2 pin is never interpreted on this line.
    let mut unpinned = v02_layer();
    root_pin(&mut unpinned, None);
    let bytes = serialize(&unpinned);
    let (_, opened, prims) = reopen(bytes);
    assert_eq!(
        coordination::read(&opened.metadata, &prims)
            .unwrap_err()
            .code,
        ErrorCode::UnsupportedVersion
    );
    assert!(!coordination::pinned_v02(&opened.metadata).unwrap());
}

#[test]
fn unknown_record_is_refused() {
    let extra_field = format!(
        "#usda 1.0\n(metersPerUnit = 0.001\nupAxis = \"Z\"\ncustomLayerData = {{ string bimSchemaDigest = \"{}\" }})\ndef Scope \"World\" {{ def Scope \"Coordination\" {{ def Scope \"Sections\" {{ def Scope \"SEC_{}\" (kind = \"group\") {{ custom string bim:schemaVersion = \"0.2.0\"\n custom string bim:section:id = \"{SECTION}\"\n custom string bim:section:code = \"KM\"\n custom string bim:section:label = \"L\"\n custom string[] bim:section:modules = [\"steel\"]\n custom string future:field = \"kept elsewhere, refused here\"\n }} }} }} }}\n",
        pin(),
        hex(SECTION)
    );
    let (_, opened, prims) = reopen(extra_field.into_bytes());
    assert_eq!(
        coordination::read(&opened.metadata, &prims)
            .unwrap_err()
            .code,
        ErrorCode::InvalidArgument
    );

    // A prim that is no record at all inside a registry scope.
    let stranger = format!(
        "#usda 1.0\n(metersPerUnit = 0.001\nupAxis = \"Z\"\ncustomLayerData = {{ string bimSchemaDigest = \"{}\" }})\ndef Scope \"World\" {{ def Scope \"Coordination\" {{ def Scope \"Zones\" {{ def Xform \"ZN_{}\" (kind = \"group\") {{ }} }} }} }}\n",
        pin(),
        hex(ZONE)
    );
    let (_, opened, prims) = reopen(stranger.into_bytes());
    assert_eq!(
        coordination::read(&opened.metadata, &prims)
            .unwrap_err()
            .code,
        ErrorCode::InvalidArgument
    );

    // A nested prim below a record is outside the one-segment record shape.
    let nested = format!(
        "#usda 1.0\n(metersPerUnit = 0.001\nupAxis = \"Z\"\ncustomLayerData = {{ string bimSchemaDigest = \"{}\" }})\ndef Scope \"World\" {{ def Scope \"Coordination\" {{ def Scope \"Decisions\" {{ def Scope \"DEC_{}\" (kind = \"group\") {{ custom string bim:schemaVersion = \"0.2.0\"\n }} def Scope \"DEC_{}_child\" {{ }} }} }} }}\n",
        pin(),
        hex(DECISION_A),
        hex(DECISION_A)
    );
    let (_, opened, prims) = reopen(nested.into_bytes());
    assert_eq!(
        coordination::read(&opened.metadata, &prims)
            .unwrap_err()
            .code,
        ErrorCode::InvalidArgument
    );

    // An unknown coordination scope next to the three registries.
    let stranger_scope = format!(
        "#usda 1.0\n(metersPerUnit = 0.001\nupAxis = \"Z\"\ncustomLayerData = {{ string bimSchemaDigest = \"{}\" }})\ndef Scope \"World\" {{ def Scope \"Coordination\" {{ def Scope \"Other\" {{ }} }} }}\n",
        pin()
    );
    let (_, opened, prims) = reopen(stranger_scope.into_bytes());
    assert_eq!(
        coordination::read(&opened.metadata, &prims)
            .unwrap_err()
            .code,
        ErrorCode::InvalidArgument
    );
}

#[test]
fn oversized_record_is_refused() {
    // One record over the registry budget.
    let mut data = v02_layer();
    for ordinal in 0..=MAX_SECTIONS {
        let id = format!("0199e7a0-0000-7000-8000-{ordinal:012}");
        let code = format!("S{ordinal}");
        coordination::author_section(&mut data, &section(&id, &code)).unwrap();
    }
    let bytes = serialize(&data);
    let (_, opened, prims) = reopen(bytes);
    assert_eq!(
        coordination::read(&opened.metadata, &prims)
            .unwrap_err()
            .code,
        ErrorCode::LimitExceeded
    );

    // One record whose code exceeds the byte budget.
    let long_code = "A".repeat(33);
    let mut data = v02_layer();
    assert!(coordination::author_section(&mut data, &section(SECTION, &long_code)).is_err());
    let mut data = v02_layer();
    let long_label = "L".repeat(257);
    assert!(
        coordination::author_zone(
            &mut data,
            &ZoneRecord {
                id: ZONE.into(),
                code: "K1".into(),
                label: long_label
            }
        )
        .is_err()
    );
}

#[test]
fn malformed_record_is_refused() {
    let frame = |body: &str| {
        format!(
            "#usda 1.0\n(metersPerUnit = 0.001\nupAxis = \"Z\"\ncustomLayerData = {{ string bimSchemaDigest = \"{}\" }})\n{body}\n",
            pin()
        )
    };
    let open = |body: &str| {
        let (_, opened, prims) = reopen(frame(body).into_bytes());
        coordination::read(&opened.metadata, &prims)
    };
    let world = |scope: &str, body: &str| {
        format!(
            "def Scope \"World\" {{ def Scope \"Coordination\" {{ def Scope \"{scope}\" {{ {body} }} }} }}\n"
        )
    };

    // Identity fields: a wrong-type attribute, a non-UUIDv7 identity and a path disagreement.
    let wrong_type = world(
        "Sections",
        &format!(
            "def Scope \"SEC_{}\" (kind = \"group\") {{ custom string bim:schemaVersion = \"0.2.0\"\n custom token bim:section:id = \"{SECTION}\"\n custom string bim:section:code = \"KM\"\n custom string bim:section:label = \"L\"\n custom string[] bim:section:modules = []\n }}\n",
            hex(SECTION)
        ),
    );
    assert_eq!(
        open(&wrong_type).unwrap_err().code,
        ErrorCode::InvalidArgument
    );

    let bad_identity = world(
        "Sections",
        &format!(
            "def Scope \"SEC_{}\" (kind = \"group\") {{ custom string bim:schemaVersion = \"0.2.0\"\n custom string bim:section:id = \"not-a-uuid\"\n custom string bim:section:code = \"KM\"\n custom string bim:section:label = \"L\"\n custom string[] bim:section:modules = []\n }}\n",
            hex(SECTION)
        ),
    );
    assert_eq!(open(&bad_identity).unwrap_err().code, ErrorCode::InvalidId);

    let path_disagrees = world(
        "Sections",
        &format!(
            "def Scope \"SEC_{}\" (kind = \"group\") {{ custom string bim:schemaVersion = \"0.2.0\"\n custom string bim:section:id = \"{SECTION_B}\"\n custom string bim:section:code = \"KM\"\n custom string bim:section:label = \"L\"\n custom string[] bim:section:modules = []\n }}\n",
            hex(SECTION)
        ),
    );
    assert_eq!(
        open(&path_disagrees).unwrap_err().code,
        ErrorCode::InvalidId
    );

    // Verdicts: unknown token, a rejection without a closed reason, a reason on an acceptance.
    let decision = |verdict: &str, reason: Option<&str>, id: &str| {
        world(
            "Decisions",
            &format!(
                "def Scope \"DEC_{}\" (kind = \"group\") {{ custom string bim:schemaVersion = \"0.2.0\"\n custom string bim:decision:id = \"{id}\"\n custom string bim:decision:proposal = \"{SECTION}\"\n custom string bim:decision:proposalDigest = \"{}\"\n custom string bim:decision:hostRevision = \"0199e7a0-0000-7000-8000-000000000018\"\n custom token bim:decision:verdict = \"{verdict}\"\n{} }}\n",
                hex(id),
                digest(b"proposal bytes"),
                reason
                    .map(|r| format!(" custom token bim:decision:reason = \"{r}\"\n"))
                    .unwrap_or_default(),
            ),
        )
    };
    let unknown_verdict = decision("maybe", None, DECISION_A);
    assert_eq!(
        open(&unknown_verdict).unwrap_err().code,
        ErrorCode::InvalidArgument
    );
    let missing_reason = decision("rejected", None, DECISION_A);
    assert_eq!(
        open(&missing_reason).unwrap_err().code,
        ErrorCode::InvalidArgument
    );
    let stray_reason = decision("accepted", Some("size"), DECISION_A);
    assert_eq!(
        open(&stray_reason).unwrap_err().code,
        ErrorCode::InvalidArgument
    );
    let unknown_reason = decision("rejected", Some("mood"), DECISION_A);
    assert_eq!(
        open(&unknown_reason).unwrap_err().code,
        ErrorCode::InvalidArgument
    );

    // Registry uniqueness: two sections sharing one code.
    let duplicate = world(
        "Sections",
        &format!(
            "{}{}",
            format_args!(
                "def Scope \"SEC_{}\" (kind = \"group\") {{ custom string bim:schemaVersion = \"0.2.0\"\n custom string bim:section:id = \"{SECTION}\"\n custom string bim:section:code = \"KM\"\n custom string bim:section:label = \"L\"\n custom string[] bim:section:modules = []\n }}\n",
                hex(SECTION)
            ),
            format_args!(
                "def Scope \"SEC_{}\" (kind = \"group\") {{ custom string bim:schemaVersion = \"0.2.0\"\n custom string bim:section:id = \"{SECTION_B}\"\n custom string bim:section:code = \"KM\"\n custom string bim:section:label = \"L\"\n custom string[] bim:section:modules = []\n }}\n",
                hex(SECTION_B)
            ),
        ),
    );
    assert_eq!(open(&duplicate).unwrap_err().code, ErrorCode::DuplicateId);

    // Project metadata: an organization without the required code and name pair.
    let partial = format!(
        "#usda 1.0\n(metersPerUnit = 0.001\nupAxis = \"Z\"\ncustomLayerData = {{ string bimSchemaDigest = \"{}\" string bimProjectOrganization = \"Apibim\" }})\n",
        pin()
    );
    let (_, opened, prims) = reopen(partial.into_bytes());
    assert_eq!(
        coordination::read(&opened.metadata, &prims)
            .unwrap_err()
            .code,
        ErrorCode::InvalidArgument
    );

    // Authored provenance: a zone claim without a section is never admitted.
    let mut data = v02_layer();
    assert!(
        coordination::author_scope(
            &mut data,
            "/World/Steel/Parts/PRT_x",
            Some(&AuthoredScope {
                section: "".into(),
                zone: Some(ZONE.into()),
            })
        )
        .is_err()
    );
}

#[test]
fn line_0_1_keeps_its_contracts() {
    // An ordinary valid 0.1 plate, exactly as the 0.1 mapping authors it.
    let identity = schema_identity().unwrap().digest.to_string();
    let plate = |digest: &str| {
        format!(
            "#usda 1.0\n(metersPerUnit = 0.001\nupAxis = \"Z\"\ncustomLayerData = {{ string bimSchemaDigest = \"{digest}\" }})\ndef Xform \"World\" (kind = \"assembly\") {{ def Scope \"Steel\" {{ def Scope \"Parts\" {{ def Xform \"PRT_{}\" (kind = \"component\", displayName = \"Plate\")\n{{ custom string bim:id = \"{PLATE}\"\n custom string bim:name = \"Plate\"\n custom token bim:elementKind = \"plate\"\n custom token bim:geometryMode = \"parametric\"\n custom string bim:schemaVersion = \"0.1.0\"\n custom string bim:createdBy = \"{ACTOR}\"\n custom double3[] steel:plate:contourPointsMm = [(0, 0, 0), (1000, 0, 0), (1000, 500, 0), (0, 500, 0)]\n custom double steel:plate:thicknessMm = 10\n double3 xformOp:translate = (0, 0, 0)\n quatd xformOp:orient = (1, 0, 0, 0)\n uniform token[] xformOpOrder = [\"xformOp:translate\", \"xformOp:orient\"]\n}} }} }} }}\n",
            hex(PLATE)
        )
    };
    let base = request(vec![asset("authored.usda", plate(&identity).into_bytes())]);
    assert!(validate_layers(base, schema_identity().unwrap(), &control()).is_ok());

    // The same 0.1 layer carrying 0.2 record content: preserved inert, contracts unchanged.
    let mut data = openusd::usda::parse(&plate(&identity)).unwrap();
    populate(&mut data);
    let carried = serialize(&data);
    let validation = validate_layers(
        request(vec![asset("authored.usda", carried.clone())]),
        schema_identity().unwrap(),
        &control(),
    )
    .unwrap();
    assert_eq!(validation.profile, "u02-structural-v1");
    // The record content stays uninterpreted on the 0.1 line.
    let (_, opened, prims) = reopen(carried);
    assert_eq!(
        coordination::read(&opened.metadata, &prims)
            .unwrap_err()
            .code,
        ErrorCode::UnsupportedVersion
    );

    // Nothing advertises 0.2.0: pinning the 0.2 mapping on the 0.1 line is refused outright.
    let advertised = plate(&pin());
    let request = request(vec![asset("authored.usda", advertised.into_bytes())]);
    assert_eq!(
        validate_layers(request, schema_identity().unwrap(), &control())
            .unwrap_err()
            .code,
        ErrorCode::UnsupportedVersion
    );
}
