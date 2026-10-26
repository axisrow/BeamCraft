// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! F06-ordered validation and construction of the G02 rectangular primitive.

use crate::diagnostic::{Code, Diagnostic, Failure, Field, Stage};
use crate::mesh::{PLATE_TRIANGLES, PlateMesh, encode, self_check};
use crate::{PlateResult, RectangularPlate};
use beamcraft_geom::tolerance::{DIMENSION_CAP_MM, Tolerance};
use beamcraft_geom::{Point3, Vec3};

/// Builds a closed origin rectangular plate, validating every submitted dimension.
///
/// Width/length/thickness are checked in that order at each stage: finiteness,
/// upper caps and local scale, then positivity and scaled inclusive minima.
/// No allocation, rotation, subdivision, snapping or input-derived indexing occurs.
pub fn rectangular_plate(input: RectangularPlate) -> Result<PlateResult, Failure> {
    rectangular_plate_with_cancel(input, |_| false)
}

/// Builds a plate with F06 section 10 cancellation polls P0 through P3.
///
/// The callback receives Validation, Build, Checks, Quantities, in that order.
/// It returns true to discard the candidate. There is no poll after sealing;
/// the caller owns its callback and must not panic in it.
pub fn rectangular_plate_with_cancel(
    input: RectangularPlate,
    mut cancelled: impl FnMut(Stage) -> bool,
) -> Result<PlateResult, Failure> {
    poll(&mut cancelled, Stage::Validation)?;
    let fields = [
        (Field::Width, input.width_mm),
        (Field::Length, input.length_mm),
        (Field::Thickness, input.thickness_mm),
    ];
    validate(fields, Stage::Finiteness, |v| {
        if v.is_finite() {
            None
        } else {
            Some(Code::NonFinite)
        }
    })?;
    // Upper dimension caps are one-sided: even huge finite negatives reach ranges.
    validate(fields, Stage::Caps, |v| {
        if v > DIMENSION_CAP_MM {
            Some(Code::AboveLimit)
        } else {
            None
        }
    })?;
    // Nonpositive dimensions contribute zero to the prospective rectangle scale;
    // ranges reports them after scale, without squaring enormous negative input.
    let a = if input.width_mm > 0.0 {
        input.width_mm
    } else {
        0.0
    };
    let b = if input.length_mm > 0.0 {
        input.length_mm
    } else {
        0.0
    };
    let contour = [
        Point3::ORIGIN,
        Point3::new(a, 0.0, 0.0),
        Point3::new(a, b, 0.0),
        Point3::new(0.0, b, 0.0),
    ];
    let tolerance = Tolerance::for_plate(&contour)
        .map_err(|_| Failure::one(Code::NumericUncertain, Field::LocalSize, Stage::Caps))?;
    validate(fields, Stage::Ranges, |v| {
        if v <= 0.0 {
            Some(Code::NotPositive)
        } else if v < tolerance.eps_geom_mm() {
            Some(Code::BelowMinimum)
        } else {
            None
        }
    })?;
    poll(&mut cancelled, Stage::Build)?;
    let numeric = || Failure::one(Code::NumericUncertain, Field::Mesh, Stage::Build);
    let e1 = Vec3::new(a, 0.0, 0.0).map_err(|_| numeric())?;
    let e2 = Vec3::new(0.0, b, 0.0).map_err(|_| numeric())?;
    let e3 = Vec3::Z.scaled(input.thickness_mm).map_err(|_| numeric())?;
    let mut vertices = [Point3::ORIGIN; 8];
    for (i, slot) in vertices.iter_mut().enumerate() {
        let xb = f64::from(u8::from(i & 1 != 0));
        let yb = f64::from(u8::from(i & 2 != 0));
        let zb = f64::from(u8::from(i & 4 != 0));
        // F06 section 6: ((o + xb*e1) + yb*e2) + zb*e3.
        *slot = Point3::ORIGIN
            .translated(e1.scaled(xb).map_err(|_| numeric())?)
            .and_then(|p| p.translated(e2.scaled(yb)?))
            .and_then(|p| p.translated(e3.scaled(zb)?))
            .map_err(|_| numeric())?;
    }
    let mut normals = [Vec3::ZERO; 6];
    for (out, [x, y, z]) in normals.iter_mut().zip([
        [-1.0, 0.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, -1.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, -1.0],
        [0.0, 0.0, 1.0],
    ]) {
        *out = Vec3::new(x, y, z).map_err(|_| numeric())?;
    }
    let (blob, digest) = encode(&vertices)?;
    poll(&mut cancelled, Stage::Checks)?;
    // Positive orthogonal axis extents certify a simple convex box by construction.
    // Incidence/orientation/Euler checks independently catch table/build corruption.
    self_check(&vertices, &PLATE_TRIANGLES, &normals)?;
    poll(&mut cancelled, Stage::Quantities)?;
    let volume_mm3 = (input.width_mm * input.length_mm) * input.thickness_mm;
    if !volume_mm3.is_finite() || volume_mm3 <= 0.0 {
        return Err(Failure::one(
            Code::NumericUncertain,
            Field::Volume,
            Stage::Quantities,
        ));
    }
    Ok(PlateResult {
        mesh: PlateMesh {
            face_ids: &crate::feature_id::PLATE_FACES,
            vertices,
            normals,
            blob,
            digest,
        },
        volume_mm3,
        tolerance,
    })
}

fn poll(cancelled: &mut impl FnMut(Stage) -> bool, stage: Stage) -> Result<(), Failure> {
    if cancelled(stage) {
        Err(Failure::one(Code::Cancelled, Field::Cancellation, stage))
    } else {
        Ok(())
    }
}

fn validate(
    fields: [(Field, f64); 3],
    stage: Stage,
    check: impl Fn(f64) -> Option<Code>,
) -> Result<(), Failure> {
    let diagnostics =
        fields.map(|(field, v)| check(v).map(|code| Diagnostic { code, field, stage }));
    if diagnostics.iter().any(Option::is_some) {
        Err(Failure { diagnostics })
    } else {
        Ok(())
    }
}
