// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A11 lofted plate: a plate between two contours that need not be coplanar.
//!
//! The body is the G13 ruled loft (`beamcraft_kernel::ruled::loft`) of the two authored face
//! contours: one straight ruling per vertex pair, the wall quads along the contour edges and
//! the two caps as the G05 triangulations. The sheet thickness is the distance between the
//! contours themselves, so the command carries no thickness, holes, cuts or corner treatments.
//! Every contour is admitted by the shared F04 contour admission (`plate_contour::admit`),
//! which enforces the winding, planarity band and exact simplicity whose failure is the
//! self-intersecting refusal of the task card; the twisted (renumbered) correspondence is
//! refused by the G09 seal inside the loft. A contour the admission accepts only as
//! near-planar is refused rather than repaired into an exact body, as for framed plates.

use crate::command::{MaterialInfo, MaterialLookup};
use crate::element::ElementKind;
use crate::footprint::Footprint;
use crate::scene::SceneElement;
use crate::solid::SolidMesh;
use beamcraft_geom::Translation3;
use beamcraft_kernel::plate_contour;
use beamcraft_kernel::profile::Point2;
use beamcraft_kernel::ruled::{Loft, LoftFailure, RuledContour};
use beamcraft_model::command::{CommandError, CreateLoftedPlate, Placement, label_is_valid};
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::ElementId;

/// Algorithm label of a lofted plate's volume record.
pub const LOFT_VOLUME_ALGORITHM: &str = "lofted-plate-ruled-v1";

/// The authored state of one lofted plate: exactly the inputs the A11 command stores in the
/// model layer. Nothing derived is in it.
#[derive(Debug, Clone, PartialEq)]
pub struct AuthoredLoftedPlate {
    /// Full element identity.
    pub id: ElementId,
    /// Label.
    pub name: String,
    /// Authored integer part class (P15); zero is the default.
    pub class: i32,
    /// Element-local start face contour as submitted, in millimetres.
    pub start_points_mm: Vec<[f64; 3]>,
    /// Element-local end face contour as submitted, the same vertex count.
    pub end_points_mm: Vec<[f64; 3]>,
    /// Rigid placement as submitted.
    pub placement: Placement,
    /// Pinned physical material definition; absence means the material and density are unknown.
    pub material: Option<ContentDigest>,
}

impl AuthoredLoftedPlate {
    /// The authored fields of an accepted command (the command's arguments, unchanged).
    pub fn from_command(command: &CreateLoftedPlate) -> Self {
        Self {
            id: command.new_id,
            name: command.name.clone(),
            class: command.class,
            start_points_mm: command.start_points_mm.clone(),
            end_points_mm: command.end_points_mm.clone(),
            placement: command.placement,
            material: command.material_digest,
        }
    }

    /// The `CreateLoftedPlate` that would author exactly this state; used to compare content.
    pub fn to_command(&self) -> CreateLoftedPlate {
        CreateLoftedPlate {
            new_id: self.id,
            name: self.name.clone(),
            class: self.class,
            start_points_mm: self.start_points_mm.clone(),
            end_points_mm: self.end_points_mm.clone(),
            placement: self.placement,
            material_digest: self.material,
        }
    }
}

/// A lofted plate evaluated by the kernel: the sealed certified body.
#[derive(Debug, Clone, PartialEq)]
pub struct LoftedPlateBody {
    /// The certified mesh of the loft as authored, in element-local millimetres.
    pub solid: SolidMesh,
    /// Volume of the body in cubic millimetres: the midpoint of the certified enclosure.
    /// With no operations gross and net are the same value.
    pub volume_mm3: f64,
    /// Surface area in square millimetres, when the G06 measurement was certified.
    pub area_mm2: Option<f64>,
}

impl LoftedPlateBody {
    /// The quantity records of this body with no material: volume only, mass `unknown`.
    pub fn quantities(&self) -> crate::command::Quantities {
        crate::command::quantities_without_material(
            crate::command::Volumes::unchanged(self.volume_mm3),
            LOFT_VOLUME_ALGORITHM,
        )
    }

    /// The quantity records of this body with the physical material of the element, in the
    /// manner of a plate: mass is `current` only with a density the definition authors.
    pub fn quantities_for(
        &self,
        material: Option<MaterialInfo>,
    ) -> Result<crate::command::Quantities, CommandError> {
        crate::command::quantities_with_material(
            crate::command::Volumes::unchanged(self.volume_mm3),
            LOFT_VOLUME_ALGORITHM,
            material,
        )
    }
}

/// Everything one accepted `CreateLoftedPlate` produces for the layers and the host.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateLoftedPlateOutcome {
    /// The authored state for the model layer.
    pub authored: AuthoredLoftedPlate,
    /// The evaluated body for the derived layer.
    pub body: LoftedPlateBody,
    /// Quantity records.
    pub quantities: crate::command::Quantities,
    /// Digest of the unmodified typed command arguments.
    pub fingerprint: ContentDigest,
    /// Declared write set of the creation.
    pub footprint: Footprint,
}

/// Applies `CreateLoftedPlate` to an isolated draft in memory, in the order of
/// [`crate::command::create_plate`]: argument admission, an unused id, the material, then the
/// kernel evaluation. Nothing is published.
pub fn create_lofted_plate(
    command: &CreateLoftedPlate,
    existing: &dyn Fn(&ElementId) -> bool,
    materials: MaterialLookup<'_>,
) -> Result<CreateLoftedPlateOutcome, CommandError> {
    command.validate_arguments()?;
    if existing(&command.new_id) {
        return Err(CommandError::new(
            ErrorCode::DuplicateId,
            "new_id",
            "element id is already owned",
        ));
    }
    let material = command
        .material_digest
        .as_ref()
        .map(materials)
        .transpose()?;
    let footprint = Footprint::for_create(command.new_id, command.material_digest)?;
    let authored = AuthoredLoftedPlate::from_command(command);
    let body = evaluate_lofted_plate(&authored)?;
    let quantities = body.quantities_for(material)?;
    Ok(CreateLoftedPlateOutcome {
        fingerprint: command.content_fingerprint()?,
        authored,
        body,
        quantities,
        footprint,
    })
}

/// Evaluates a committed lofted plate with the kernel. The caller is expected to have run
/// [`CreateLoftedPlate::validate_arguments`] for the label, counts, caps and placement; this
/// function repeats the geometry admission and never panics on any input.
pub fn evaluate_lofted_plate(
    authored: &AuthoredLoftedPlate,
) -> Result<LoftedPlateBody, CommandError> {
    if authored.start_points_mm.len() != authored.end_points_mm.len() {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "end_points_mm",
            "the two contours must have the same vertex count",
        ));
    }
    if !label_is_valid(&authored.name) {
        return Err(CommandError::new(
            ErrorCode::InvalidArgument,
            "name",
            "committed label is invalid",
        ));
    }
    let start = ruled_contour(&authored.start_points_mm, "start_points_mm")?;
    let end = ruled_contour(&authored.end_points_mm, "end_points_mm")?;
    let result = Loft { start, end };
    let lofted = beamcraft_kernel::ruled::loft(&result).map_err(loft_error)?;
    let solid = SolidMesh::from_loft(&lofted);
    Ok(LoftedPlateBody {
        volume_mm3: solid.volume_mm3(),
        area_mm2: lofted.area_mm2().map(|area| area.midpoint()),
        solid,
    })
}

/// The snapshot-builder input for one evaluated lofted plate, in the manner of a plate scene
/// element: the mesh stays element-local and the element carries its authored rigid pose.
pub fn scene_element(
    authored: &AuthoredLoftedPlate,
    body: &LoftedPlateBody,
) -> Result<SceneElement, CommandError> {
    authored.placement.validate()?;
    let [x, y, z] = authored.placement.translation_mm;
    let translation = Translation3::new(x, y, z).map_err(|_| {
        CommandError::new(
            ErrorCode::NonFiniteNumber,
            "placement.translation_mm",
            "value is not finite",
        )
    })?;
    Ok(SceneElement::new(
        authored.id,
        ElementKind::Plate,
        translation,
        body.solid.scene_mesh(),
    )
    .with_orientation(authored.placement.orientation_xyzw))
}

/// Maps a kernel loft refusal to its typed command error: the contour stage names the field
/// the admission refused, the profile and seal stages name the contours together.
fn loft_error(failure: LoftFailure) -> CommandError {
    let (field, message) = match &failure {
        LoftFailure::Contour(_) => ("start_points_mm", "the loft contours are refused"),
        LoftFailure::Profile(_) => ("contour_points_mm", "a loft contour is refused"),
        LoftFailure::Body(_) => (
            "contour_points_mm",
            "the loft of the two contours is refused by the certification",
        ),
    };
    CommandError::new(failure.f04_code(), field, message)
}

/// Admits one authored contour and encodes it for the ruled primitive: the shared F04
/// admission (count, floor, winding, planarity band, exact simplicity) with a nominal
/// thickness of one millimetre, which exercises the same dimension-cap path while no loft
/// input carries a thickness. Exactly-planar contours only: a near-planar admitted polygon
/// is not repaired into an exact frame, as for framed plates. The 2-D coordinates are the
/// authored points re-encoded in the certified frame; the G09 seal certifies the placed
/// result of that encoding, so an encoding the certification cannot accept refuses here.
fn ruled_contour(points: &[[f64; 3]], field: &'static str) -> Result<RuledContour, CommandError> {
    let contour = plate_contour::admit(points, 1.0).map_err(|error| CommandError {
        rule_id: None,
        code: error.code,
        field,
        message: error.message,
    })?;
    let frame = contour.frame.ok_or(CommandError::new(
        ErrorCode::UnsupportedGeometry,
        field,
        "contour is not exactly planar",
    ))?;
    let origin = contour.origin;
    let mut projected = Vec::with_capacity(points.len());
    for point in points {
        let delta = origin
            .vector_to(beamcraft_geom::Point3::new(point[0], point[1], point[2]))
            .map_err(|_| uncertain(field))?;
        let x = delta.dot(frame.x()).map_err(|_| uncertain(field))?;
        let y = delta.dot(frame.y()).map_err(|_| uncertain(field))?;
        projected.push(Point2::new(x, y));
    }
    RuledContour::polygon(origin, frame.x(), frame.y(), projected).map_err(|_| {
        CommandError::new(
            ErrorCode::UnsupportedGeometry,
            field,
            "the contour frame could not be certified",
        )
    })
}

fn uncertain(field: &'static str) -> CommandError {
    CommandError::new(
        ErrorCode::UnsupportedGeometry,
        field,
        "the contour admission was numerically uncertain",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::no_materials;
    use beamcraft_model::command::Placement;

    fn authored(start: Vec<[f64; 3]>, end: Vec<[f64; 3]>) -> AuthoredLoftedPlate {
        AuthoredLoftedPlate {
            id: ElementId::parse("019a7bf0-0000-7000-8000-000000110001").unwrap(),
            name: "Loft".into(),
            class: 0,
            start_points_mm: start,
            end_points_mm: end,
            placement: Placement::IDENTITY,
            material: None,
        }
    }

    fn rectangle(origin_z: f64, width: f64, height: f64) -> Vec<[f64; 3]> {
        vec![
            [0.0, 0.0, origin_z],
            [width, 0.0, origin_z],
            [width, height, origin_z],
            [0.0, height, origin_z],
        ]
    }

    fn evaluate(start: Vec<[f64; 3]>, end: Vec<[f64; 3]>) -> Result<LoftedPlateBody, CommandError> {
        evaluate_lofted_plate(&authored(start, end))
    }

    fn close(actual: f64, expected: f64) {
        let slack = expected.abs() * 1e-9 + 1e-6;
        assert!(
            (actual - expected).abs() <= slack,
            "{actual} is not within {slack} of {expected}"
        );
    }

    #[test]
    fn an_oblique_prism_matches_the_analytic_volume_and_area() {
        let body = evaluate(rectangle(0.0, 200.0, 100.0), rectangle(50.0, 200.0, 100.0)).unwrap();
        close(body.volume_mm3, 200.0 * 100.0 * 50.0);
        let area = body.area_mm2.unwrap();
        close(area, 2.0 * 200.0 * 100.0 + 2.0 * (200.0 + 100.0) * 50.0);
        // The translation self-check of the seal ran: the quantities are current.
        assert_eq!(body.quantities().volume_status.as_str(), "current");
    }

    #[test]
    fn a_tapered_loft_matches_the_prismatoid_closed_form() {
        // Ruled between two similar rectangles over one height, the body is a prismatoid:
        // V = h/6 * (A1 + 4*Am + A2) with the midsection area of the linear interpolation.
        let body = evaluate(rectangle(0.0, 200.0, 100.0), rectangle(60.0, 100.0, 50.0)).unwrap();
        let a1 = 200.0 * 100.0;
        let a2 = 100.0 * 50.0;
        let am = 150.0 * 75.0;
        close(body.volume_mm3, 60.0 / 6.0 * (a1 + 4.0 * am + a2));
    }

    #[test]
    fn mass_follows_the_density_and_is_unknown_without_one() {
        let body = evaluate(rectangle(0.0, 200.0, 100.0), rectangle(50.0, 200.0, 100.0)).unwrap();
        let without = body.quantities();
        assert_eq!(without.mass_status.as_str(), "unknown");
        assert!(without.mass_kg.is_none());
        let with = body
            .quantities_for(Some(MaterialInfo {
                density_kg_m3: Some(7850.0),
            }))
            .unwrap();
        close(with.mass_kg.unwrap(), 7850.0 * 1_000_000.0 / 1e9);
        assert_eq!(with.mass_status.as_str(), "current");
    }

    #[test]
    fn a_renumbered_end_contour_is_refused_by_the_certification() {
        // The 180-degree renumbering of the end contour twists every wall through the body;
        // the seal refuses the self-intersecting candidate instead of repairing it.
        let end = vec![
            [200.0, 100.0, 50.0],
            [0.0, 100.0, 50.0],
            [0.0, 0.0, 50.0],
            [200.0, 0.0, 50.0],
        ];
        let error = evaluate(rectangle(0.0, 200.0, 100.0), end).unwrap_err();
        assert_eq!(error.code, ErrorCode::UnsupportedGeometry);
        assert_eq!(error.field, "contour_points_mm");
    }

    #[test]
    fn a_self_intersecting_start_contour_is_refused() {
        let start = vec![
            [0.0, 0.0, 0.0],
            [200.0, 0.0, 0.0],
            [50.0, 100.0, 0.0],
            [150.0, 100.0, 0.0],
        ];
        let error = evaluate(start, rectangle(50.0, 200.0, 100.0)).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidArgument);
        assert_eq!(error.field, "start_points_mm");
    }

    #[test]
    fn a_clockwise_contour_is_refused() {
        let start = vec![
            [0.0, 0.0, 0.0],
            [0.0, 100.0, 0.0],
            [200.0, 100.0, 0.0],
            [200.0, 0.0, 0.0],
        ];
        let error = evaluate(start, rectangle(50.0, 200.0, 100.0)).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidArgument);
        assert_eq!(error.field, "start_points_mm");
    }

    #[test]
    fn a_near_planar_contour_is_refused_instead_of_repaired() {
        let mut start = rectangle(0.0, 200.0, 100.0);
        start[2][2] = 1.0e-3;
        let error = evaluate(start, rectangle(50.0, 200.0, 100.0)).unwrap_err();
        assert_eq!(error.code, ErrorCode::UnsupportedGeometry);
        assert_eq!(error.field, "start_points_mm");
    }

    #[test]
    fn a_vertex_count_mismatch_is_refused() {
        let end = vec![
            [0.0, 0.0, 50.0],
            [200.0, 0.0, 50.0],
            [200.0, 100.0, 50.0],
            [100.0, 150.0, 50.0],
            [0.0, 100.0, 50.0],
        ];
        let error = evaluate(rectangle(0.0, 200.0, 100.0), end).unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidArgument);
        assert_eq!(error.field, "end_points_mm");
    }

    #[test]
    fn create_admits_an_unused_id_and_refuses_a_used_one() {
        let command = CreateLoftedPlate {
            new_id: ElementId::parse("019a7bf0-0000-7000-8000-000000110002").unwrap(),
            name: "Loft".into(),
            class: 0,
            start_points_mm: rectangle(0.0, 200.0, 100.0),
            end_points_mm: rectangle(50.0, 200.0, 100.0),
            placement: Placement::IDENTITY,
            material_digest: None,
        };
        command.validate_arguments().unwrap();
        let outcome = create_lofted_plate(&command, &|_| false, &no_materials).unwrap();
        assert_eq!(outcome.authored.id, command.new_id);
        close(outcome.body.volume_mm3, 1_000_000.0);
        // The command fingerprint identifies the same content.
        assert_eq!(outcome.fingerprint, command.content_fingerprint().unwrap());
        let error = create_lofted_plate(&command, &|_| true, &no_materials).unwrap_err();
        assert_eq!(error.code, ErrorCode::DuplicateId);
    }

    #[test]
    fn the_scene_element_builds_into_the_view_snapshot() {
        let body = evaluate(rectangle(0.0, 200.0, 100.0), rectangle(50.0, 200.0, 100.0)).unwrap();
        let element = scene_element(
            &authored(rectangle(0.0, 200.0, 100.0), rectangle(50.0, 200.0, 100.0)),
            &body,
        )
        .unwrap();
        let pin = crate::ViewPin::new(
            beamcraft_model::digest::Sha256::digest(b"revision").unwrap(),
            1,
        );
        let snapshot = crate::ViewSnapshot::build(pin, vec![element], &crate::Limits::default())
            .unwrap_or_else(|error| panic!("snapshot build failed: {error:?}"));
        assert_eq!(snapshot.meshes().len(), 1);
    }

    #[test]
    fn the_shape_signature_follows_the_contours_only() {
        let command = CreateLoftedPlate {
            new_id: ElementId::parse("019a7bf0-0000-7000-8000-000000110003").unwrap(),
            name: "Loft".into(),
            class: 0,
            start_points_mm: rectangle(0.0, 200.0, 100.0),
            end_points_mm: rectangle(50.0, 200.0, 100.0),
            placement: Placement::IDENTITY,
            material_digest: None,
        };
        let outcome = create_lofted_plate(&command, &|_| false, &no_materials).unwrap();
        let algorithm = LOFT_VOLUME_ALGORITHM;
        let same = crate::quantity::lofted_plate_shape_signature(&outcome.authored, algorithm);
        let moved = crate::quantity::lofted_plate_shape_signature(
            &AuthoredLoftedPlate {
                placement: Placement {
                    translation_mm: [10.0, 0.0, 0.0],
                    ..outcome.authored.placement
                },
                ..outcome.authored.clone()
            },
            algorithm,
        );
        assert_eq!(same, moved);
        let reshaped = crate::quantity::lofted_plate_shape_signature(
            &AuthoredLoftedPlate {
                start_points_mm: rectangle(0.0, 210.0, 100.0),
                ..outcome.authored.clone()
            },
            algorithm,
        );
        assert_ne!(same, reshaped);
    }
}
