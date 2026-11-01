// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D02 derived drawing USD: ordinary linear BasisCurves and exact double dimension metadata.
//! Values are always recomputed through the engine before writing or verifying saved bytes.

use beamcraft_engine::ViewSnapshot;
use beamcraft_engine::drawing::Drawing;
use beamcraft_engine::drawing::projection::{self, BodyProjection, DimensionProjection};
use beamcraft_layer_engine::Asset;
use beamcraft_model::digest::{ContentDigest, Sha256};
use beamcraft_model::error::ErrorCode;
use session::Refusal;
use session::pipeline::integrity;

/// Derived asset name, composed separately from authored intent.
pub const DERIVED_LAYER: &str = "drawing-derived.usda";
/// Root composes only the document's two USD layers, never the source archive/provenance.
pub const ROOT_LAYER: &str = "root.usda";
/// Tight derived-layer cap, including exact double coordinates and bounded labels.
pub const MAX_DERIVED_BYTES: usize = 16_384;
/// Closed encoding contract, independent of the source engineering schema.
pub const SCHEMA_DESCRIPTOR: &str = "beamcraft-drawing-derived/1;linear-BasisCurves;paper-mm-XY;display-points=point3f;exact-points=double3[];dimension=projected-model-mm-double;resolved-or-unresolved;recompute-before-verify";

/// Derived drawing schema digest.
pub fn schema_identity() -> Result<ContentDigest, Refusal> {
    Sha256::digest(SCHEMA_DESCRIPTOR.as_bytes())
        .map_err(|_| integrity("derived drawing schema digest failed"))
}

fn asset(name: &str, text: String) -> Result<Asset, Refusal> {
    if text.len() > MAX_DERIVED_BYTES {
        return Err(Refusal {
            rule_id: None,
            code: ErrorCode::LimitExceeded,
            message: "derived drawing byte cap exceeded",
        });
    }
    let bytes = text.into_bytes();
    Ok(Asset {
        name: name.to_owned(),
        digest: Sha256::digest(&bytes).map_err(|_| integrity("derived drawing digest failed"))?,
        bytes,
    })
}

/// Static ordinary USD root; model archives are deliberately absent from its sublayers.
pub fn root() -> Result<Asset, Refusal> {
    asset(ROOT_LAYER, "#usda 1.0\n(\n    defaultPrim = \"Drawing\"\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n    subLayers = [@drawing.usda@, @drawing-derived.usda@]\n)\n".to_owned())
}

fn curves(text: &mut String, segments: &[[[f64; 2]; 2]]) {
    // All callers supply fixed engine output: at most twelve edges or three dimension strokes.
    let counts = segments.iter().map(|_| "2").collect::<Vec<_>>().join(", ");
    let points = segments
        .iter()
        .flat_map(|ends| ends.iter())
        .map(|[x, y]| format!("({x}, {y}, 0)"))
        .collect::<Vec<_>>()
        .join(", ");
    text.push_str(&format!("        def BasisCurves \"Geometry\"\n        {{\n            uniform token type = \"linear\"\n            uniform token wrap = \"nonperiodic\"\n            int[] curveVertexCounts = [{counts}]\n            point3f[] points = [{points}]\n            custom double3[] drawing:exactPointsMm = [{points}]\n        }}\n"));
}

/// Derives geometry from the admitted intent and its exact source snapshot, not caller-supplied
/// coordinates. Standard point3f display geometry accompanies exact double3 points and double
/// measurement metadata. Missing geometry has a typed reason and no successful numeric value.
pub fn encode(drawing: &Drawing, snapshot: &ViewSnapshot) -> Result<Asset, Refusal> {
    let projection = projection::resolve(drawing, snapshot)?;
    let mut text = format!(
        "#usda 1.0\n(\n    metersPerUnit = 0.001\n    upAxis = \"Z\"\n)\nover \"Drawing\"\n{{\n    custom string drawing:derivedSchema = \"{}\"\n    custom string drawing:algorithm = \"{}\"\n    custom string drawing:derivedSourceManifest = \"{}\"\n    over \"View\"\n    {{\n",
        schema_identity()?,
        projection::ALGORITHM,
        drawing.spec().source.manifest
    );
    match projection.body {
        BodyProjection::Resolved(edges) => {
            text.push_str("        custom token drawing:state = \"resolved\"\n");
            let refs = edges.iter().map(|e| format!("\"{}\"", e.feature)).collect::<Vec<_>>().join(", ");
            text.push_str(&format!("        custom string[] drawing:edgeIds = [{refs}]\n"));
            curves(&mut text, &edges.into_iter().map(|e| e.paper_mm).collect::<Vec<_>>());
        }
        BodyProjection::Unresolved(reason) => text.push_str(&format!("        custom token drawing:state = \"unresolved\"\n        custom token drawing:reason = \"{}\"\n", reason.as_str())),
    }
    text.push_str("    }\n    over \"Dimension\"\n    {\n");
    match projection.dimension {
        DimensionProjection::Resolved(d) => {
            text.push_str(&format!("        custom token drawing:state = \"resolved\"\n        custom double drawing:projectedLengthMm = {}\n        custom string drawing:labelMm = \"{}\"\n", d.projected_length_mm, d.label_mm));
            let [a,b] = d.anchors_paper_mm;
            let [la,lb] = d.line_paper_mm;
            curves(&mut text, &[[a,la], [la,lb], [b,lb]]);
        }
        DimensionProjection::Unresolved(reason) => text.push_str(&format!("        custom token drawing:state = \"unresolved\"\n        custom token drawing:reason = \"{}\"\n", reason.as_str())),
    }
    text.push_str("    }\n}\n");
    asset(DERIVED_LAYER, text)
}

/// Checks saved geometry against a fresh engine resolution of the exact archived source. A
/// self-consistent rehash of fabricated coordinates or an old dimension value is insufficient.
pub fn verify(saved: &Asset, drawing: &Drawing, snapshot: &ViewSnapshot) -> Result<(), Refusal> {
    if saved.bytes.len() > MAX_DERIVED_BYTES {
        return Err(Refusal {
            rule_id: None,
            code: ErrorCode::LimitExceeded,
            message: "derived drawing byte cap exceeded",
        });
    }
    let expected = encode(drawing, snapshot)?;
    if saved.name != expected.name
        || saved.digest != expected.digest
        || saved.bytes != expected.bytes
    {
        return Err(integrity(
            "saved drawing geometry disagrees with its exact source and intent",
        ));
    }
    Ok(())
}
