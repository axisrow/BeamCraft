// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Mesh measurements with explicit grade, and the F06 section 8 status rules.
//!
//! Volume, area and centroid of a certified shell are computed from exact
//! integer sums of the submitted binary64 vertices and returned as outward-rounded
//! enclosures of the polyhedron those vertices define. They are mesh-computed
//! values with an error enclosure, never an F06 engineering quantity: in 0.1.0
//! only the analytic builders produce `current` volume, area or mass, so
//! [`MeshMeasurement::quantity_status`] is always `unsupported`. An open,
//! non-manifold, inward, degenerate, overlapping or uncertain mesh yields no value at all.
//!
//! References: the divergence theorem (signed tetrahedron sum
//! `V = sum(v0 . (v1 x v2)) / 6`), the volume centroid
//! `C = sum(det_t * (v0 + v1 + v2)) / (4 * sum(det_t))`, and triangle area
//! `|(v1 - v0) x (v2 - v0)| / 2`, all derived from that theorem and vector algebra.

use crate::PlateResult;
use crate::bigint::Big;
use crate::diagnostic::{Code, Stage};
use crate::enclosure::Enclosure;
use crate::exact::{coordinate_sum, min_exponent, triangle_cross, triangle_det};
use crate::spatial::Aabb;
use crate::topology::Analysis;
use crate::trimesh::TriMesh;
use crate::verdict::{Fault, MeshCode, MeshError, ShellStatus};

/// F04/F06 section 8 quantity status values, exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuantityStatus {
    /// Computed analytically by a first-scope builder from a sealed build.
    Current,
    /// Inputs changed and the rebuild is not complete; host-owned, never produced here.
    Stale,
    /// A required input is absent or unavailable.
    Unknown,
    /// Geometry outside the scope, or an open, inward or uncertified mesh.
    Unsupported,
}

impl QuantityStatus {
    /// Stable lowercase wire token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Stale => "stale",
            Self::Unknown => "unknown",
            Self::Unsupported => "unsupported",
        }
    }
}

/// How a reported value was obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Basis {
    /// Closed-form value of a first-scope analytic primitive.
    Analytic,
    /// Exact sums over a certified triangle shell, with a rounding enclosure.
    MeshEnclosure,
}

/// Mass status by F06 section 8 precedence.
///
/// `unsupported` volume wins (even with absent density), then `unknown` when the
/// density is absent or the volume unknown, then `stale`, else `current`.
/// `density` is the density input's status, or `None` when absent.
pub fn mass_status(volume: QuantityStatus, density: Option<QuantityStatus>) -> QuantityStatus {
    match (volume, density) {
        (QuantityStatus::Unsupported, _) => QuantityStatus::Unsupported,
        (QuantityStatus::Unknown, _)
        | (_, None | Some(QuantityStatus::Unknown | QuantityStatus::Unsupported)) => {
            QuantityStatus::Unknown
        }
        (QuantityStatus::Stale, _) | (_, Some(QuantityStatus::Stale)) => QuantityStatus::Stale,
        (QuantityStatus::Current, Some(QuantityStatus::Current)) => QuantityStatus::Current,
    }
}

/// A mass record: status plus a value only when `current`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MassRecord {
    /// Quantity status.
    pub status: QuantityStatus,
    /// Mass in kilograms, present exactly when `status` is `current`.
    pub kg: Option<f64>,
}

/// Mass of an analytic primitive: `fl(fl(V_mm3 * density) / 1e9)` (F06 section 2 order).
///
/// Absent density keeps mass `unknown`, never zero or defaulted. A non-finite
/// density is `GEO_NON_FINITE`, a negative one `GEO_NOT_POSITIVE`. A density of
/// exactly zero is the massless auxiliary material and gives a current mass of zero. A
/// product that overflows, or underflows below the normal binary64 range (to a subnormal value or
/// to zero), is `GEO_NUMERIC_UNCERTAIN`: a subnormal has lost the precision of the comparison
/// rule, and zero is never the current mass of a positive-density solid.
pub fn analytic_mass_kg(volume_mm3: f64, density_kg_m3: Option<f64>) -> Result<MassRecord, Code> {
    if !volume_mm3.is_finite() || volume_mm3 <= 0.0 {
        return Err(Code::NumericUncertain);
    }
    let Some(density) = density_kg_m3 else {
        return Ok(MassRecord {
            status: QuantityStatus::Unknown,
            kg: None,
        });
    };
    if !density.is_finite() {
        return Err(Code::NonFinite);
    }
    if density < 0.0 {
        return Err(Code::NotPositive);
    }
    let mass = (volume_mm3 * density) / 1_000_000_000.0;
    // Only an explicit zero density (the massless auxiliary material) may give a zero mass, and a
    // positive density never gives a subnormal one (G11, `g11.subnormal.*`).
    if !mass.is_finite() || mass < 0.0 || (density > 0.0 && mass < f64::MIN_POSITIVE) {
        return Err(Code::NumericUncertain);
    }
    Ok(MassRecord {
        status: QuantityStatus::Current,
        kg: Some(mass),
    })
}

/// Measurements of one analyzed mesh.
///
/// Values are present only for a certified shell. They are mesh-computed
/// enclosures, not engineering quantities (see the module notes).
#[derive(Debug, Clone, PartialEq)]
pub struct MeshMeasurement {
    status: ShellStatus,
    bounds: Aabb,
    volume_mm3: Option<Enclosure>,
    area_mm2: Option<Enclosure>,
    centroid_mm: Option<[Enclosure; 3]>,
}

impl MeshMeasurement {
    /// The shell verdict the measurement is gated by.
    pub const fn shell_status(&self) -> ShellStatus {
        self.status
    }
    /// Always [`Basis::MeshEnclosure`]: this crate's mesh values are never analytic.
    pub const fn basis(&self) -> Basis {
        Basis::MeshEnclosure
    }
    /// Engineering status of the mesh's volume, area and mass: always `unsupported`
    /// (F06 section 8: a mesh from any other source gets no engineering quantity).
    pub const fn quantity_status(&self) -> QuantityStatus {
        QuantityStatus::Unsupported
    }
    /// Exact min/max of the vertices that triangles use, always available.
    ///
    /// This is a bounding box of vertices, never a volume or any exact quantity.
    pub const fn bounds(&self) -> &Aabb {
        &self.bounds
    }
    /// Enclosure of the enclosed volume in mm3; `None` unless certified.
    pub const fn volume_mm3(&self) -> Option<Enclosure> {
        self.volume_mm3
    }
    /// Enclosure of the surface area in mm2; `None` unless certified.
    pub const fn area_mm2(&self) -> Option<Enclosure> {
        self.area_mm2
    }
    /// Enclosure of the volume centroid in mm; `None` unless certified.
    pub const fn centroid_mm(&self) -> Option<[Enclosure; 3]> {
        self.centroid_mm
    }
    /// Volume in m3, dividing by the exactly representable 1e9.
    pub fn volume_m3(&self) -> Option<Enclosure> {
        self.volume_mm3?.div(Enclosure::point(1_000_000_000.0))
    }
    /// Area in m2, dividing by the exactly representable 1e6.
    pub fn area_m2(&self) -> Option<Enclosure> {
        self.area_mm2?.div(Enclosure::point(1_000_000.0))
    }
    /// Mass record for a mesh: always `unsupported`, even without a density.
    ///
    /// Unsupported geometry outranks an absent density (F06 section 8, F06-NEG-18),
    /// and no value is ever returned.
    pub fn mass(&self, density: Option<QuantityStatus>) -> MassRecord {
        MassRecord {
            status: mass_status(self.quantity_status(), density),
            kg: None,
        }
    }
}

fn magnitude(e: Enclosure) -> Option<Enclosure> {
    if e.lo >= 0.0 {
        Some(e)
    } else if e.hi <= 0.0 {
        Enclosure::new(-e.hi, -e.lo)
    } else {
        None
    }
}

fn poll(cancelled: &mut impl FnMut(Stage) -> bool, stage: Stage) -> Result<(), MeshError> {
    if cancelled(stage) {
        Err(MeshError::cancelled(stage))
    } else {
        Ok(())
    }
}

/// Measures a mesh against its analysis with no cancellation.
pub fn measure(mesh: &TriMesh, analysis: &Analysis) -> Result<MeshMeasurement, MeshError> {
    measure_with_cancel(mesh, analysis, |_| false)
}

/// Measures a mesh; the analysis must have been computed from this exact mesh.
///
/// Polls, in order: `Validation` at entry, then (certified meshes only)
/// `Quantities` before the pass and about every 512 triangles. A cancelled call
/// returns no measurement. A non-certified analysis yields a measurement with
/// no volume, area or centroid; it is not an error.
pub fn measure_with_cancel(
    mesh: &TriMesh,
    analysis: &Analysis,
    mut cancelled: impl FnMut(Stage) -> bool,
) -> Result<MeshMeasurement, MeshError> {
    poll(&mut cancelled, Stage::Validation)?;
    if analysis.fingerprint() != mesh.fingerprint() {
        return Err(MeshError::new(
            MeshCode::NotCertified,
            Fault::Analysis,
            Stage::Validation,
        ));
    }
    let refused = MeshMeasurement {
        status: analysis.status(),
        bounds: *analysis.bounds(),
        volume_mm3: None,
        area_mm2: None,
        centroid_mm: None,
    };
    if !analysis.is_certified() {
        return Ok(refused);
    }
    poll(&mut cancelled, Stage::Quantities)?;
    let numeric = || MeshError::numeric(Stage::Quantities);
    let exponent = min_exponent(mesh.vertices().iter().flatten().copied());
    let mut volume6 = Big::ZERO;
    let mut moment = [Big::ZERO, Big::ZERO, Big::ZERO];
    let mut area2 = Enclosure::point(0.0);
    for t in 0..mesh.triangles().len() {
        if t > 0 && t % 512 == 0 {
            poll(&mut cancelled, Stage::Quantities)?;
        }
        let [a, b, c] = mesh.triangle_points(t).ok_or_else(numeric)?;
        let det = triangle_det(exponent, a, b, c).map_err(|_| numeric())?;
        volume6 = volume6.add(&det).map_err(|_| numeric())?;
        let sums = coordinate_sum(exponent, a, b, c).map_err(|_| numeric())?;
        for (slot, s) in moment.iter_mut().zip(&sums) {
            *slot = slot
                .add(&det.mul(s).map_err(|_| numeric())?)
                .map_err(|_| numeric())?;
        }
        let cross = triangle_cross(exponent, a, b, c).map_err(|_| numeric())?;
        let mut squares = Enclosure::point(0.0);
        for component in &cross {
            let e = component
                .enclose_clamped(2 * exponent)
                .and_then(magnitude)
                .ok_or_else(numeric)?;
            squares = squares
                .add(e.mul(e).ok_or_else(numeric)?)
                .ok_or_else(numeric)?;
        }
        area2 = area2
            .add(squares.sqrt().ok_or_else(numeric)?)
            .ok_or_else(numeric)?;
    }
    let s_enc = volume6.enclose(3 * exponent).ok_or_else(numeric)?;
    if s_enc.lo <= 0.0 {
        return Err(numeric());
    }
    let volume = s_enc.div(Enclosure::point(6.0)).ok_or_else(numeric)?;
    let area = area2.scaled_pow2(-1).ok_or_else(numeric)?;
    let denominator = s_enc.mul(Enclosure::point(4.0)).ok_or_else(numeric)?;
    let mut centroid = [Enclosure::point(0.0); 3];
    for (out, m) in centroid.iter_mut().zip(&moment) {
        let numerator = m.enclose_clamped(4 * exponent).ok_or_else(numeric)?;
        *out = numerator.div(denominator).ok_or_else(numeric)?;
    }
    Ok(MeshMeasurement {
        volume_mm3: Some(volume),
        area_mm2: Some(area),
        centroid_mm: Some(centroid),
        ..refused
    })
}

/// Independent mesh cross-check of a sealed G02 plate (G04 section 2 and 3.3).
#[derive(Debug, Clone, PartialEq)]
pub struct PlateCrossCheck {
    /// Shell verdict of the plate's own BCMESH01 blob.
    pub shell: ShellStatus,
    /// Analytic volume `(width*length)*thickness` in mm3.
    pub analytic_volume_mm3: f64,
    /// Status of the analytic volume: `current`, as the builder sealed it.
    pub analytic_status: QuantityStatus,
    /// Analytic enclosure `[V - r, V + r]` with `r = g(3)/(1-g(3)) * V`, rounded outward.
    pub analytic_enclosure_mm3: Enclosure,
    /// Exact-sum mesh enclosure of the blob's volume, when the shell certified.
    pub mesh_volume_mm3: Option<Enclosure>,
    /// True when the shell certified and the two enclosures overlap.
    pub agrees: bool,
}

/// Re-reads a plate's blob through the generic validator and compares volumes.
///
/// The analytic bound is G04 section 3.2's `g(3)` for `((a*b)*c)`, relative to the
/// ideal value; the mesh enclosure comes from exact integer sums. The two must
/// overlap. This is a consistency gate, not a replacement for the builder.
pub fn cross_check_plate(plate: &PlateResult) -> Result<PlateCrossCheck, MeshError> {
    let mesh = TriMesh::from_blob(plate.mesh().blob())?;
    let analysis = crate::topology::analyze_with_tolerance(
        &mesh,
        plate.tolerance(),
        crate::topology::Limits::DEFAULT,
        |_| false,
    )?;
    let measurement = measure(&mesh, &analysis)?;
    let v = plate.volume_mm3();
    let u = 1.110_223_024_625_156_5e-16_f64;
    let g3 = (3.0 * u / (1.0 - 3.0 * u)).next_up();
    let radius = ((g3 / (1.0 - g3)) * v.abs()).next_up();
    let analytic = Enclosure {
        lo: (v - radius).next_down(),
        hi: (v + radius).next_up(),
    };
    let mesh_volume = measurement.volume_mm3();
    Ok(PlateCrossCheck {
        shell: analysis.status(),
        analytic_volume_mm3: v,
        analytic_status: QuantityStatus::Current,
        analytic_enclosure_mm3: analytic,
        mesh_volume_mm3: mesh_volume,
        agrees: mesh_volume.is_some_and(|m| m.overlaps(analytic)),
    })
}
