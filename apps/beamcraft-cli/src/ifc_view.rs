// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The display bundle of a received IFC file: the reading half of `export-view --source`.
//! The shapes reader ([`beamcraft_ifc::shapes`]) reads the placed triangle bodies; the
//! engine's foreign display conversion ([`beamcraft_engine::scene::foreign_display_element`])
//! turns each into one bounded reference-kind element; the snapshot is built over a pin of
//! the file's own SHA-256, so the same bytes give the same bundle and `from_bundle` accepts
//! exactly what a store snapshot would. Nothing is written to any store, and no body
//! becomes a BeamCraft element: the display is read-only, like the reference view it rides.

use crate::plate_cmd::invalid;
use beamcraft_commands::source_file::read_file;
use beamcraft_engine::scene::{foreign_display_element, foreign_display_id};
use beamcraft_engine::{ElementId, Limits, ViewPin, ViewSnapshot};
use beamcraft_ifc::read::{self, MAX_INPUT_BYTES};
use beamcraft_ifc::shapes;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::reference::check_source_path;
use session::Refusal;

/// What one bundle was built from, for the command summary.
#[derive(Debug)]
pub struct Facts {
    /// SHA-256 of the source bytes.
    pub digest: String,
    /// Size of the source bytes.
    pub bytes: u64,
    /// The snapshot token.
    pub token: String,
    /// Elements in the snapshot (one per read body).
    pub elements: usize,
    /// Distinct meshes in the snapshot.
    pub meshes: usize,
    /// Bundle bytes.
    pub bundle_bytes: usize,
    /// Bodies the reader reported.
    pub shape_count: u64,
    /// Products the reader reported.
    pub product_count: u64,
}

/// Builds the view bundle of one received IFC file.
pub fn bundle(source: &str) -> Result<(Vec<u8>, Facts), Refusal> {
    // Before the file system is touched: a URL or a network path is refused on its text alone.
    check_source_path(source).map_err(Refusal::from)?;
    let cap = usize::try_from(MAX_INPUT_BYTES).map_err(|_| invalid("input cap"))?;
    let bytes = read_file(source, cap).map_err(|code| Refusal {
        rule_id: None,
        code,
        message: "the source file is not readable",
    })?;
    build(&bytes).map_err(|message| Refusal {
        rule_id: None,
        code: ErrorCode::InvalidArgument,
        message,
    })
}

/// The bundle of IFC bytes that are already read; separate so tests can pass synthetic
/// bytes. The string is a static reason the build was refused.
pub fn build(bytes: &[u8]) -> Result<(Vec<u8>, Facts), &'static str> {
    let shapes = match shapes::read(bytes) {
        Ok(shapes) => shapes,
        Err(read::ReadError::NotAStepFile(_)) => {
            return Err("the source file is not an IFC STEP file");
        }
        Err(read::ReadError::TooLarge) => {
            return Err("the source file is larger than the 32 MiB input cap");
        }
    };
    let limits = Limits::MAXIMA;
    if shapes.shape_count == 0 {
        return Err("the file holds no readable tessellated body; ifc-shapes lists why");
    }
    if shapes.shapes_truncated || shapes.shape_count as usize > limits.max_elements() {
        return Err(
            "the file holds more bodies than the display budget admits; the picture would \
             be incomplete, so nothing is bundled",
        );
    }
    let digest_bytes = shapes.digest.as_bytes();
    let mut elements = Vec::with_capacity(shapes.shapes.len());
    for (ordinal, shape) in shapes.shapes.iter().enumerate() {
        let id = element_id(digest_bytes, ordinal)?;
        let points: Vec<[f64; 3]> = shape
            .vertices_mm
            .iter()
            .map(|p| [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])])
            .collect();
        let element = foreign_display_element(id, [0.0; 3], &points, &shape.triangles)
            .map_err(|_| "a body is outside the display bounds")?;
        if let Some(element) = element {
            elements.push(element);
        }
    }
    let pin = ViewPin::new(shapes.digest, 1);
    let snapshot = ViewSnapshot::build(pin, elements, &limits).map_err(|e| match e.f04_code() {
        ErrorCode::LimitExceeded => "the bundle exceeds the display budgets",
        _ => "the display bundle was refused",
    })?;
    let bundle = snapshot
        .to_bundle()
        .map_err(|_| "the display bundle could not be encoded")?;
    let facts = Facts {
        digest: shapes.digest.to_string(),
        bytes: shapes.bytes,
        token: snapshot.token().as_str().to_owned(),
        elements: snapshot.len(),
        meshes: snapshot.meshes().len(),
        bundle_bytes: bundle.len(),
        shape_count: shapes.shape_count,
        product_count: shapes.product_count,
    };
    Ok((bundle, facts))
}

/// A deterministic display identity of one body: the file digest mixed with the ordinal, in
/// the engine's synthetic id shape, so the same body of the same bytes always carries the
/// same id. The derivation lives in the engine ([`foreign_display_id`]), shared with every
/// consumer that maps a picked id back to its body.
fn element_id(digest_bytes: &[u8], ordinal: usize) -> Result<ElementId, &'static str> {
    foreign_display_id(digest_bytes, ordinal).ok_or("a display identity could not be derived")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shapes_file() -> Vec<u8> {
        [
            "ISO-10303-21;",
            "HEADER;",
            "FILE_SCHEMA(('IFC4'));",
            "ENDSEC;",
            "DATA;",
            "#1=IFCPROJECT('0YvucVMOX9kw8jhtQ9abcd',*,$,$,$,$,$,$,#2);",
            "#2=IFCUNITASSIGNMENT((#3));",
            "#3=IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.);",
            "#4=IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,1.E-05,#5,$);",
            "#5=IFCAXIS2PLACEMENT3D(#6,$,$);",
            "#6=IFCCARTESIANPOINT((0.,0.,0.));",
            "#10=IFCPLATE('1Anr5VMOX0PO8jvHqZabcd',*,'PL-A',$,$,#11,#12,$,.NOTDEFINED.);",
            "#11=IFCLOCALPLACEMENT($,#5);",
            "#12=IFCPRODUCTDEFINITIONSHAPE($,$,(#13));",
            "#13=IFCSHAPEREPRESENTATION(#4,'Body','Tessellation',(#14));",
            "#14=IFCTRIANGULATEDFACESET(#15,$,.T.,((1,2,3),(1,3,4)),$);",
            "#15=IFCCARTESIANPOINTLIST3D(((0.,0.,0.),(100.,0.,0.),(100.,80.,0.),(0.,80.,0.)));",
            "ENDSEC;",
            "END-ISO-10303-21;",
            "",
        ]
        .join("\n")
        .into_bytes()
    }

    #[test]
    fn a_file_builds_a_deterministic_reopenable_bundle() {
        let bytes = shapes_file();
        let (bundle, facts) = build(&bytes).unwrap();
        assert_eq!(facts.elements, 1);
        assert_eq!(facts.product_count, 1);
        assert!(bundle.starts_with(b"BCVIEW03"));
        // The same bytes give the same bundle, and the engine reopens it as a snapshot.
        let (again, same_facts) = build(&bytes).unwrap();
        assert_eq!(bundle, again);
        assert_eq!(facts.token, same_facts.token);
        let reopened = ViewSnapshot::from_bundle(&bundle, &Limits::MAXIMA).unwrap();
        assert_eq!(reopened.len(), 1);
        assert_eq!(
            reopened.elements()[0].kind,
            beamcraft_engine::ElementKind::Reference
        );
    }

    #[test]
    fn a_file_without_readable_bodies_is_refused_with_the_reason() {
        let file = shapes_file();
        // Replace the body line with a product without a shape.
        let text = String::from_utf8(file).unwrap();
        let text = text.replace(
            "#10=IFCPLATE('1Anr5VMOX0PO8jvHqZabcd',*,'PL-A',$,$,#11,#12,$,.NOTDEFINED.);",
            "#10=IFCWALL('1Anr5VMOX0PO8jvHqZabcd',*,'W',$,$,$,$,$,$);",
        );
        let file = text.into_bytes();
        let error = build(&file).unwrap_err();
        assert!(error.contains("no readable tessellated body"), "{error}");
    }

    #[test]
    fn a_display_identity_is_deterministic_and_distinct_per_ordinal() {
        let bytes = [0u8; 16];
        let first = element_id(&bytes, 0).unwrap();
        let second = element_id(&bytes, 1).unwrap();
        assert_ne!(first, second);
        assert_eq!(first, element_id(&bytes, 0).unwrap());
    }
}
