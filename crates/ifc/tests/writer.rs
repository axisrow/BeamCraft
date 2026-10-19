// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The writer against its own structural reader: determinism, structure, volume, bounds and the
//! optional-entity rules of E01 (`IFCX-EMIT-001`, `IFCX-DET-*`, `IFCX-GEO-008`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

use beamcraft_ifc::guid;
use beamcraft_ifc::verify::{self, Value};
use beamcraft_ifc::{
    Declared, ElementData, Entity, ExportModel, LevelData, Limits, MaterialData, Mesh, Pose,
    QuantityData, Subject, WriteError, write_ifc,
};
use beamcraft_model::digest::{ContentDigest, Sha256};
use beamcraft_model::id::Uuid7;

fn uuid(last: u16) -> Uuid7 {
    Uuid7::parse(&format!("019a7bf0-0000-7000-8000-{last:012x}")).unwrap()
}

fn digest(seed: &str) -> ContentDigest {
    Sha256::digest(seed.as_bytes()).unwrap()
}

/// An axis-aligned box with the vertex order `i = x + 2y + 4z` and outward counter-clockwise
/// triangles.
fn cube(a: f32, b: f32, c: f32) -> Mesh {
    let points = (0..8u32)
        .map(|i| {
            [
                if i & 1 == 0 { 0.0 } else { a },
                if i & 2 == 0 { 0.0 } else { b },
                if i & 4 == 0 { 0.0 } else { c },
            ]
        })
        .collect();
    let triangles = vec![
        [0, 2, 1],
        [1, 2, 3],
        [4, 5, 6],
        [5, 7, 6],
        [0, 1, 5],
        [0, 5, 4],
        [2, 6, 3],
        [3, 6, 7],
        [0, 4, 2],
        [2, 4, 6],
        [1, 3, 5],
        [3, 7, 5],
    ];
    Mesh {
        points,
        triangles,
        closed: true,
    }
}

fn quantities(volume_m3: f64, mass: Option<f64>, gross: bool) -> QuantityData {
    QuantityData {
        volume_status: "current".into(),
        mass_status: if mass.is_some() { "current" } else { "unknown" }.into(),
        shape_signature: Some(digest("shape")),
        mass_signature: Some(digest("mass")),
        algorithm: Some("kernel-test/1".into()),
        net_volume_m3: Some(volume_m3),
        gross_volume_m3: gross.then_some(volume_m3),
        net_weight_kg: mass,
        gross_weight_kg: if gross { mass } else { None },
    }
}

fn element(last: u16, entity: Entity, name: &str) -> ElementData {
    ElementData {
        id: uuid(last),
        entity,
        kind_token: match entity {
            Entity::Beam => "beam",
            Entity::Column => "column",
            Entity::Plate => "plate",
            Entity::Proxy => "assembly",
        }
        .into(),
        name: name.into(),
        schema_version: "1".into(),
        geometry_mode: "parametric".into(),
        pose: Pose::from_quaternion([10.0, 20.0, 30.0], [0.0, 0.0, 0.0, 1.0]),
        mesh: Some(cube(100.0, 200.0, 10.0)),
        level: None,
        material: None,
        profile: None,
        hole_count: None,
        quantities: None,
        declared: None,
        assembly: None,
    }
}

fn model() -> ExportModel {
    let mut plate = element(0x20, Entity::Plate, "Base plate");
    plate.hole_count = Some(0);
    plate.material = Some(digest("steel"));
    plate.level = Some(uuid(0x101));
    plate.quantities = Some(quantities(0.0002, Some(1.57), true));
    let mut beam = element(0x21, Entity::Beam, "Beam B1");
    beam.profile = Some(digest("profile"));
    beam.material = Some(digest("steel"));
    beam.quantities = Some(quantities(0.0002, Some(1.57), true));
    beam.declared = Some(Declared {
        designation: Some("IPE 200".into()),
        standard: None,
        area_mm2: Some(2848.0),
        mass_per_meter_kg: Some(22.4),
        surface_per_meter_m2: None,
    });
    let mut assembly = element(0x22, Entity::Proxy, "Assembly A");
    assembly.pose = None;
    assembly.mesh = None;
    let mut part = element(0x23, Entity::Column, "Column C1");
    part.assembly = Some(uuid(0x22));
    part.level = Some(uuid(0x101));
    ExportModel {
        project_id: uuid(1),
        project_name: Some("Frame".into()),
        revision_unix_ms: 1_000_000_000_000,
        levels: vec![
            LevelData {
                id: uuid(0x101),
                name: "Level 1".into(),
                elevation_mm: 3000.0,
            },
            LevelData {
                id: uuid(0x102),
                name: "Empty level".into(),
                elevation_mm: 6000.0,
            },
        ],
        materials: vec![MaterialData {
            digest: digest("steel"),
            grade: Some("S355".into()),
            standard: Some("EN 10025-2".into()),
            density_kg_m3: Some(7850.0),
        }],
        elements: vec![plate, beam, assembly, part],
    }
}

fn write(model: &ExportModel) -> Vec<u8> {
    write_ifc(model, &Limits::DEFAULT, Vec::new()).unwrap().0
}

#[test]
fn a_storey_global_id_is_the_one_in_the_file() {
    let m = model();
    let file = verify::parse(&write(&m)).unwrap();
    let storeys = file.all("IFCBUILDINGSTOREY");
    for (storey, level) in storeys.iter().zip(&m.levels) {
        assert_eq!(
            file.text(storey.args.first()).unwrap(),
            beamcraft_ifc::storey_global_id(&m.project_id, &level.id).as_str()
        );
        assert_ne!(
            file.text(storey.args.first()).unwrap(),
            guid::encode(&level.id).as_str(),
            "the storey id is a synthetic id, not the encoding of the LevelId"
        );
    }
    assert_eq!(
        beamcraft_ifc::storey_global_id(&uuid(1), &uuid(0x101)).as_str(),
        "2886jimw3h7h75lUh8nRj7"
    );
}

#[test]
fn the_same_model_gives_the_same_bytes() {
    let m = model();
    let (a, b) = (write(&m), write(&m));
    assert_eq!(a, b);
    let (_, outcome) = write_ifc(&m, &Limits::DEFAULT, Vec::new()).unwrap();
    assert_eq!(outcome.sha256, Sha256::digest(&a).unwrap());
    assert_eq!(outcome.bytes, a.len() as u64);
}

#[test]
fn the_file_has_the_fixed_header_and_a_valid_structure() {
    let bytes = write(&model());
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert!(text.starts_with(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION(('BeamCraft IFC export profile 0.1.0'),'2;1');\n\
         FILE_NAME('beamcraft-export.ifc','2001-09-09T01:46:40',(''),(''),"
    ));
    assert!(text.contains("FILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n"));
    assert!(text.ends_with("ENDSEC;\nEND-ISO-10303-21;\n"));
    let file = verify::parse(&bytes).unwrap();
    let summary = file.check().unwrap();
    for (name, count) in [
        ("IFCPROJECT", 1),
        ("IFCSITE", 1),
        ("IFCBUILDING", 1),
        ("IFCBUILDINGSTOREY", 2),
        ("IFCPLATE", 1),
        ("IFCBEAM", 1),
        ("IFCCOLUMN", 1),
        ("IFCBUILDINGELEMENTPROXY", 1),
        ("IFCMATERIAL", 1),
        ("IFCRELASSOCIATESMATERIAL", 1),
        ("IFCTRIANGULATEDFACESET", 3),
        ("IFCSHAPEREPRESENTATION", 3),
    ] {
        assert_eq!(summary.counts.get(name).copied(), Some(count), "{name}");
    }
}

#[test]
fn the_volume_recomputed_from_the_file_is_the_box_volume() {
    let file = verify::parse(&write(&model())).unwrap();
    let summary = file.check().unwrap();
    assert_eq!(summary.volumes.len(), 3);
    for volume in summary.volumes.values() {
        assert!((volume - 100.0 * 200.0 * 10.0).abs() < 1e-6, "{volume}");
    }
}

#[test]
fn global_ids_are_the_encoding_of_the_element_ids() {
    let file = verify::parse(&write(&model())).unwrap();
    let beam = file.all("IFCBEAM")[0];
    let id = file.text(beam.args.first()).unwrap();
    assert_eq!(id, guid::encode(&uuid(0x21)).as_str());
    let summary = file.check().unwrap();
    assert!(summary.global_ids.contains(&id.to_owned()));
    let project = file.all("IFCPROJECT")[0];
    assert_eq!(
        file.text(project.args.first()).unwrap(),
        guid::synthetic("project", &[uuid(1).as_bytes()]).as_str()
    );
}

#[test]
fn placements_are_relative_to_the_building_and_written_as_axes() {
    let file = verify::parse(&write(&model())).unwrap();
    let plate = file.all("IFCPLATE")[0];
    let placement = file.target(&plate.args[5]).unwrap();
    assert_eq!(placement.name, "IFCLOCALPLACEMENT");
    let building = file.all("IFCBUILDING")[0];
    assert_eq!(placement.args[0], building.args[5]);
    let axes = file.target(&placement.args[1]).unwrap();
    let location = file.target(&axes.args[0]).unwrap();
    assert_eq!(
        location.args[0],
        Value::List(vec![
            Value::Real(10.0),
            Value::Real(20.0),
            Value::Real(30.0)
        ])
    );
    let z = file.target(&axes.args[1]).unwrap();
    assert_eq!(
        z.args[0],
        Value::List(vec![Value::Real(0.0), Value::Real(0.0), Value::Real(1.0)])
    );
}

#[test]
fn storeys_contain_their_elements_and_the_rest_is_in_the_building() {
    let file = verify::parse(&write(&model())).unwrap();
    let contained = file.all("IFCRELCONTAINEDINSPATIALSTRUCTURE");
    // Level 1 holds the plate and the column; the beam and the assembly have no level; the
    // second level holds nothing and gets no relationship (IFCX-EMIT-001).
    assert_eq!(contained.len(), 2);
    let storeys = file.all("IFCBUILDINGSTOREY");
    assert_eq!(file.text(storeys[0].args.get(2)), Some("Level 1"));
    assert_eq!(storeys[0].args[9], Value::Real(3000.0));
    let first = contained[0];
    assert_eq!(first.args[5], Value::Ref(storeys[0].id));
    let Value::List(members) = &first.args[4] else {
        panic!("members");
    };
    assert_eq!(members.len(), 2);
    let building = contained[1];
    assert_eq!(building.args[5], Value::Ref(file.all("IFCBUILDING")[0].id));
}

#[test]
fn a_model_without_levels_has_no_storey_and_no_building_storey_aggregate() {
    let mut m = model();
    m.levels.clear();
    for e in &mut m.elements {
        e.level = None;
    }
    let file = verify::parse(&write(&m)).unwrap();
    file.check().unwrap();
    assert!(file.all("IFCBUILDINGSTOREY").is_empty());
    assert_eq!(file.all("IFCRELAGGREGATES").len(), 2);
}

#[test]
fn optional_entities_exist_only_with_content() {
    let mut m = model();
    m.materials.clear();
    m.elements.truncate(3);
    for e in &mut m.elements {
        e.material = None;
        e.declared = None;
        e.quantities = None;
    }
    let file = verify::parse(&write(&m)).unwrap();
    file.check().unwrap();
    assert!(file.all("IFCMATERIAL").is_empty());
    assert!(file.all("IFCRELASSOCIATESMATERIAL").is_empty());
    assert!(file.all("IFCELEMENTQUANTITY").is_empty());
    // One BeamCraft_Element set per element and no BeamCraft_Declared set.
    let sets = file.all("IFCPROPERTYSET");
    assert_eq!(sets.len(), 3);
    assert!(
        sets.iter()
            .all(|s| file.text(s.args.get(2)) == Some("BeamCraft_Element"))
    );
    // The assembly has no placement and no representation.
    let proxy = file.all("IFCBUILDINGELEMENTPROXY")[0];
    assert_eq!(proxy.args[5], Value::Null);
    assert_eq!(proxy.args[6], Value::Null);
    assert_eq!(file.text(proxy.args.get(4)), Some("assembly"));
}

#[test]
fn quantities_follow_the_base_quantity_sets() {
    let file = verify::parse(&write(&model())).unwrap();
    let sets = file.all("IFCELEMENTQUANTITY");
    let names: Vec<_> = sets
        .iter()
        .map(|s| file.text(s.args.get(2)).unwrap().to_owned())
        .collect();
    assert_eq!(names, ["Qto_PlateBaseQuantities", "Qto_BeamBaseQuantities"]);
    // A plate has volumes only; its mass is a property.
    assert_eq!(file.all("IFCQUANTITYVOLUME").len(), 4);
    assert_eq!(file.all("IFCQUANTITYWEIGHT").len(), 2);
    let plate_props: Vec<_> = file
        .all("IFCPROPERTYSINGLEVALUE")
        .iter()
        .filter_map(|p| file.text(p.args.first()).map(str::to_owned))
        .collect();
    assert!(plate_props.contains(&"NetWeightKg".to_owned()));
    assert!(plate_props.contains(&"GrossWeightKg".to_owned()));
    assert!(plate_props.contains(&"AssemblyId".to_owned()));
}

#[test]
fn the_material_carries_grade_digest_and_density() {
    let file = verify::parse(&write(&model())).unwrap();
    let material = file.all("IFCMATERIAL")[0];
    assert_eq!(file.text(material.args.first()), Some("S355"));
    assert_eq!(file.text(material.args.get(1)), Some("EN 10025-2"));
    let props = file.all("IFCMATERIALPROPERTIES");
    let names: Vec<_> = props
        .iter()
        .map(|p| file.text(p.args.first()).unwrap())
        .collect();
    assert_eq!(names, ["BeamCraft_Material", "Pset_MaterialCommon"]);
}

#[test]
fn a_material_without_a_grade_is_unspecified() {
    let mut m = model();
    m.materials[0].grade = None;
    m.materials[0].standard = None;
    m.materials[0].density_kg_m3 = None;
    let file = verify::parse(&write(&m)).unwrap();
    let material = file.all("IFCMATERIAL")[0];
    assert_eq!(file.text(material.args.first()), Some("Unspecified"));
    assert_eq!(material.args[1], Value::Null);
    assert_eq!(file.all("IFCMATERIALPROPERTIES").len(), 1);
}

#[test]
fn long_and_non_ascii_names_are_cut_escaped_and_reported() {
    let mut m = model();
    m.elements[0].name = "Балка ".repeat(60);
    m.elements[1].name = "it's \\ ok".into();
    let (bytes, outcome) = write_ifc(&m, &Limits::DEFAULT, Vec::new()).unwrap();
    assert!(bytes.is_ascii());
    assert_eq!(
        outcome.truncations,
        vec![beamcraft_ifc::Truncation {
            subject: Subject::Element(uuid(0x20)),
            field: "element.name"
        }]
    );
    let file = verify::parse(&bytes).unwrap();
    file.check().unwrap();
    let plate = file.all("IFCPLATE")[0];
    assert_eq!(file.text(plate.args.get(2)).unwrap().chars().count(), 255);
    let beam = file.all("IFCBEAM")[0];
    assert_eq!(file.text(beam.args.get(2)), Some("it's \\ ok"));
    // The full label survives in the BeamCraft_Element set.
    let label = file
        .all("IFCPROPERTYSINGLEVALUE")
        .into_iter()
        .find(|p| file.text(p.args.first()) == Some("Label"))
        .unwrap();
    let Value::Typed(_, inner) = &label.args[2] else {
        panic!("typed");
    };
    assert_eq!(file.text(inner.first()).unwrap(), "Балка ".repeat(60));
}

#[test]
fn bounds_refuse_before_any_byte() {
    let m = model();
    let few = Limits {
        max_elements: 3,
        ..Limits::DEFAULT
    };
    assert!(matches!(
        write_ifc(&m, &few, Vec::new()),
        Err(WriteError::Limit(_))
    ));
    let small_mesh = Limits {
        max_vertices: 7,
        ..Limits::DEFAULT
    };
    assert!(matches!(
        write_ifc(&m, &small_mesh, Vec::new()),
        Err(WriteError::Limit(_))
    ));
    let few_triangles = Limits {
        max_triangles: 11,
        ..Limits::DEFAULT
    };
    assert!(matches!(
        write_ifc(&m, &few_triangles, Vec::new()),
        Err(WriteError::Limit(_))
    ));
    let no_levels = Limits {
        max_levels: 1,
        ..Limits::DEFAULT
    };
    assert!(matches!(
        write_ifc(&m, &no_levels, Vec::new()),
        Err(WriteError::Limit(_))
    ));
    let tiny = Limits {
        max_bytes: 1000,
        ..Limits::DEFAULT
    };
    assert!(matches!(
        write_ifc(&m, &tiny, Vec::new()),
        Err(WriteError::Sink(_))
    ));
}

#[test]
fn inconsistent_input_is_refused() {
    let mut unordered = model();
    unordered.elements.swap(0, 1);
    assert!(matches!(
        write_ifc(&unordered, &Limits::DEFAULT, Vec::new()),
        Err(WriteError::Integrity(_))
    ));
    let mut dangling = model();
    dangling.elements[0].level = Some(uuid(0x999));
    assert!(matches!(
        write_ifc(&dangling, &Limits::DEFAULT, Vec::new()),
        Err(WriteError::Integrity(_))
    ));
    let mut no_material = model();
    no_material.materials.clear();
    assert!(matches!(
        write_ifc(&no_material, &Limits::DEFAULT, Vec::new()),
        Err(WriteError::Integrity(_))
    ));
    let mut bad_index = model();
    bad_index.elements[0].mesh.as_mut().unwrap().triangles[0] = [0, 1, 8];
    assert!(matches!(
        write_ifc(&bad_index, &Limits::DEFAULT, Vec::new()),
        Err(WriteError::Integrity(_))
    ));
    let mut nan = model();
    nan.elements[0].mesh.as_mut().unwrap().points[0][0] = f32::NAN;
    assert!(matches!(
        write_ifc(&nan, &Limits::DEFAULT, Vec::new()),
        Err(WriteError::Integrity(_))
    ));
    let mut infinite = model();
    infinite.levels[0].elevation_mm = f64::INFINITY;
    assert!(matches!(
        write_ifc(&infinite, &Limits::DEFAULT, Vec::new()),
        Err(WriteError::Invalid(_))
    ));
    let mut no_pose = model();
    no_pose.elements[0].pose = None;
    assert!(matches!(
        write_ifc(&no_pose, &Limits::DEFAULT, Vec::new()),
        Err(WriteError::Integrity(_))
    ));
}

#[test]
fn a_repeated_global_id_cannot_be_written() {
    // Two elements with one identity are an ordering violation first; the GlobalId check is the
    // second line of defence.
    let mut m = model();
    let twin = m.elements[0].clone();
    m.elements.insert(1, twin);
    assert!(write_ifc(&m, &Limits::DEFAULT, Vec::new()).is_err());
}

#[test]
fn a_mesh_that_is_not_closed_leaves_the_flag_unset() {
    let mut m = model();
    m.elements[2].entity = Entity::Proxy;
    m.elements[2].pose = Pose::from_quaternion([0.0; 3], [0.0, 0.0, 0.0, 1.0]);
    m.elements[2].mesh = Some(Mesh {
        closed: false,
        ..cube(1.0, 1.0, 1.0)
    });
    let file = verify::parse(&write(&m)).unwrap();
    file.check().unwrap();
    let sets = file.all("IFCTRIANGULATEDFACESET");
    assert_eq!(sets[2].args[2], Value::Null);
    assert_eq!(sets[0].args[2], Value::Enum("T".into()));
}

#[test]
fn the_pose_of_a_quarter_turn_and_of_negated_quaternions() {
    let s = std::f64::consts::FRAC_1_SQRT_2;
    let a = Pose::from_quaternion([0.0; 3], [0.0, 0.0, s, s]).unwrap();
    let b = Pose::from_quaternion([0.0; 3], [0.0, 0.0, -s, -s]).unwrap();
    assert_eq!(a, b);
    assert!((a.ref_direction[1] - 1.0).abs() < 1e-12);
    assert!((a.axis[2] - 1.0).abs() < 1e-12);
    let identity = Pose::from_quaternion([-0.0, 1.0, 2.0], [-0.0, 0.0, 0.0, 1.0]).unwrap();
    assert!(identity.translation_mm[0].is_sign_positive());
    assert_eq!(identity.ref_direction, [1.0, 0.0, 0.0]);
    assert_eq!(Pose::from_quaternion([0.0; 3], [0.0; 4]), None);
    assert_eq!(
        Pose::from_quaternion([f64::NAN, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0]),
        None
    );
}

#[test]
fn the_reader_rejects_damaged_files() {
    let good = String::from_utf8(write(&model())).unwrap();
    assert!(verify::parse(good.as_bytes()).unwrap().check().is_ok());
    let missing_target = good.replace("#1=", "#99999=");
    assert!(
        verify::parse(missing_target.as_bytes())
            .unwrap()
            .check()
            .is_err()
    );
    let crlf = good.replace('\n', "\r\n");
    assert!(verify::parse(crlf.as_bytes()).is_err());
    let open_shell = good.replacen("(1,3,2)", "(1,3,4)", 1);
    assert_ne!(open_shell, good);
    assert!(
        verify::parse(open_shell.as_bytes())
            .unwrap()
            .check()
            .is_err()
    );
}
