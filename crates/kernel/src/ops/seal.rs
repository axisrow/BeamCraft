// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Canonicalisation, certification and sealing of an operation's candidate mesh.

use super::body::Body;
use super::diagnostic::{OpCode, OpDefect, OpFailure, OpField};
use super::{OpLimits, OpStats, poll};
use crate::diagnostic::Stage;
use crate::enclosure::Enclosure;
use crate::measure::measure_with_cancel;
use crate::spatial::Aabb;
use crate::topology::analyze_with_tolerance;
use crate::trimesh::TriMesh;
use crate::verdict::ShellStatus;
use beamcraft_geom::tolerance::Tolerance;
use beamcraft_model::digest::{ContentDigest, Sha256};

/// Which operation produced a result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpKind {
    /// A plane cut keeping one half-space.
    PlaneCut,
    /// A straight through hole.
    ThroughHole,
    /// A transverse hole through the walls of a member section (G14.1).
    MemberThroughHole,
    /// A section swept along a polyline or arc path (G12).
    Sweep,
    /// A ruled surface between two contours (G13).
    Ruled,
}

/// An uncertified candidate in operation-local numbering.
pub struct Candidate {
    /// The vertices.
    pub vertices: Vec<[f64; 3]>,
    /// The triangles.
    pub triangles: Vec<[u32; 3]>,
    /// The faces.
    pub faces: Vec<u16>,
    /// The face ids.
    pub face_ids: Vec<String>,
    /// The normals.
    pub normals: Vec<Option<[f64; 3]>>,
}

/// What the sealed candidate must satisfy beyond the generic G06 certificate.
pub struct Expect {
    /// The kind.
    pub kind: OpKind,
    /// The euler.
    pub euler: Option<i64>,
    /// The genus.
    pub genus: Option<u32>,
    /// The analytic volume.
    pub analytic_volume: Option<Enclosure>,
    /// The plane deviation mm.
    pub plane_deviation_mm: Option<f64>,
    /// The stats.
    pub stats: OpStats,
}

/// A sealed operation result: a new certified [`Body`] with its blob, digest and quantities.
#[derive(Debug, Clone)]
pub struct OpResult {
    kind: OpKind,
    body: Body,
    blob: Vec<u8>,
    digest: ContentDigest,
    bounds: Aabb,
    analytic_volume_mm3: Option<Enclosure>,
    plane_deviation_mm: Option<f64>,
    removed_faces: Vec<String>,
    stats: OpStats,
}

impl OpResult {
    /// The operation that produced the result.
    pub const fn kind(&self) -> OpKind {
        self.kind
    }
    /// The certified result body; feed it to the next operation.
    pub const fn body(&self) -> &Body {
        &self.body
    }
    /// Consumes the result and returns the certified body.
    pub fn into_body(self) -> Body {
        self.body
    }
    /// The sealed BCMESH01 blob (layout 1, element-local, canonical vertex and triangle order).
    pub fn blob(&self) -> &[u8] {
        &self.blob
    }
    /// SHA-256 of the exact blob.
    pub const fn mesh_digest(&self) -> ContentDigest {
        self.digest
    }
    /// Local face ids in face-table order (surviving input faces, then the new faces).
    pub fn face_table(&self) -> &[String] {
        self.body.face_table()
    }
    /// Outward unit normal per face; `None` for a curved face.
    pub fn face_normals(&self) -> &[Option<[f64; 3]>] {
        self.body.face_normals()
    }
    /// Input faces that disappeared because the operation removed all their triangles.
    pub fn removed_faces(&self) -> &[String] {
        &self.removed_faces
    }
    /// Explicit USD-compatible subdivision scheme; never rely on a default.
    pub const fn subdivision_scheme(&self) -> &'static str {
        "none"
    }
    /// Front faces have counter-clockwise winding seen from outside.
    pub const fn orientation(&self) -> &'static str {
        "rightHanded"
    }
    /// Enclosure of the mesh volume in mm3 (exact signed-volume sum, enclosed once).
    pub const fn volume_mm3(&self) -> Enclosure {
        self.body.volume_mm3()
    }
    /// Enclosure of the mesh volume in m3 (division by exactly 1e9).
    pub fn volume_m3(&self) -> Option<Enclosure> {
        self.body
            .volume_mm3()
            .div(Enclosure::point(1_000_000_000.0))
    }
    /// Analytic volume from the inputs, when one exists: for a hole it is the exact body volume
    /// minus the exact prism volume `area * thickness`, enclosed outward. A plane cut has no
    /// closed form independent of its own section, so it is `None`; the tests use closed forms.
    pub const fn analytic_volume_mm3(&self) -> Option<Enclosure> {
        self.analytic_volume_mm3
    }
    /// Exact vertex bounds of the result.
    pub const fn bounds(&self) -> &Aabb {
        &self.bounds
    }
    /// For a plane cut, a certified upper bound of the distance of every cap vertex from the
    /// plane (the cap is planar only up to the rounding of the new vertices); `None` for a hole.
    pub const fn plane_deviation_mm(&self) -> Option<f64> {
        self.plane_deviation_mm
    }
    /// Euler characteristic `V - E + F` of the single result shell.
    pub fn euler_characteristic(&self) -> i64 {
        self.body.analysis().euler_characteristic()
    }
    /// Genus of the single result shell.
    pub fn genus(&self) -> Option<u32> {
        self.body.analysis().shells().first().and_then(|s| s.genus)
    }
    /// Work counters.
    pub const fn stats(&self) -> &OpStats {
        &self.stats
    }
}

fn pad8(n: usize) -> usize {
    n.div_ceil(8) * 8
}

fn encode(
    vertices: &[[f64; 3]],
    triangles: &[[u32; 3]],
    faces: &[u16],
    face_count: usize,
) -> Option<Vec<u8>> {
    let (v, t) = (vertices.len(), triangles.len());
    let total = 32usize
        .checked_add(v.checked_mul(24)?)?
        .checked_add(pad8(t.checked_mul(12)?))?
        .checked_add(pad8(t.checked_mul(2)?))?;
    let mut blob: Vec<u8> = Vec::new();
    blob.try_reserve_exact(total).ok()?;
    blob.extend_from_slice(b"BCMESH01");
    for word in [
        1u32,
        u32::try_from(v).ok()?,
        u32::try_from(t).ok()?,
        u32::try_from(face_count).ok()?,
        0,
        0,
    ] {
        blob.extend_from_slice(&word.to_le_bytes());
    }
    for value in vertices.iter().flatten() {
        let value = if *value == 0.0 { 0.0 } else { *value };
        blob.extend_from_slice(&value.to_le_bytes());
    }
    for index in triangles.iter().flatten() {
        blob.extend_from_slice(&index.to_le_bytes());
    }
    blob.resize(32 + 24 * v + pad8(12 * t), 0);
    for face in faces {
        blob.extend_from_slice(&face.to_le_bytes());
    }
    blob.resize(total, 0);
    (blob.len() == total).then_some(blob)
}

struct Canonical {
    blob: Vec<u8>,
    face_ids: Vec<String>,
    normals: Vec<Option<[f64; 3]>>,
    removed: Vec<String>,
}

// Vertices that no triangle uses disappear, faces without triangles are dropped (their ids are
// reported), vertices are sorted by coordinates and every triangle is rotated to start at its
// smallest index and sorted by (face, indices): the bytes depend on the geometry and the face
// order only, never on how the input was numbered.
fn canonicalise(c: Candidate, limits: &OpLimits) -> Result<Canonical, OpFailure> {
    let internal = || OpFailure::self_check(OpDefect::ResultNotCertified, Stage::Build);
    if c.triangles.len() > limits.max_triangles {
        return Err(OpFailure::new(
            OpCode::AboveLimit,
            OpField::Limits,
            OpDefect::BudgetTriangles,
            Stage::Build,
        ));
    }
    if c.triangles.len() != c.faces.len() || c.face_ids.len() != c.normals.len() {
        return Err(internal());
    }
    let mut vertex_used = vec![false; c.vertices.len()];
    let mut face_used = vec![false; c.face_ids.len()];
    for (t, f) in c.triangles.iter().zip(&c.faces) {
        for i in t {
            *vertex_used
                .get_mut(usize::try_from(*i).map_err(|_| internal())?)
                .ok_or_else(internal)? = true;
        }
        *face_used.get_mut(usize::from(*f)).ok_or_else(internal)? = true;
    }
    let mut order: Vec<usize> = (0..c.vertices.len())
        .filter(|i| vertex_used.get(*i).copied().unwrap_or(false))
        .collect();
    // Negative zero is the same coordinate as positive zero but sorts before it.
    let key = |i: usize| {
        c.vertices
            .get(i)
            .copied()
            .unwrap_or([0.0; 3])
            .map(|v| if v == 0.0 { 0.0 } else { v })
    };
    order.sort_by(|&a, &b| {
        let (p, q) = (key(a), key(b));
        p[0].total_cmp(&q[0])
            .then(p[1].total_cmp(&q[1]))
            .then(p[2].total_cmp(&q[2]))
            .then(a.cmp(&b))
    });
    let mut new_index = vec![u32::MAX; c.vertices.len()];
    for (new, old) in order.iter().enumerate() {
        *new_index.get_mut(*old).ok_or_else(internal)? =
            u32::try_from(new).map_err(|_| internal())?;
    }
    let vertices: Vec<[f64; 3]> = order.iter().map(|&i| key(i)).collect();

    let mut face_map = vec![u16::MAX; c.face_ids.len()];
    let (mut face_ids, mut normals, mut removed) = (Vec::new(), Vec::new(), Vec::new());
    for (i, id) in c.face_ids.iter().enumerate() {
        if face_used.get(i).copied().unwrap_or(false) {
            *face_map.get_mut(i).ok_or_else(internal)? =
                u16::try_from(face_ids.len()).map_err(|_| internal())?;
            face_ids.push(id.clone());
            normals.push(c.normals.get(i).copied().ok_or_else(internal)?);
        } else {
            removed.push(id.clone());
        }
    }
    let mut items: Vec<(u16, [u32; 3])> = Vec::with_capacity(c.triangles.len());
    for (t, f) in c.triangles.iter().zip(&c.faces) {
        let mut tri = [0u32; 3];
        for (slot, i) in tri.iter_mut().zip(t) {
            *slot = *new_index
                .get(usize::try_from(*i).map_err(|_| internal())?)
                .ok_or_else(internal)?;
        }
        let [a, b, d] = tri;
        let rotated = if a <= b && a <= d {
            [a, b, d]
        } else if b <= d {
            [b, d, a]
        } else {
            [d, a, b]
        };
        items.push((
            *face_map.get(usize::from(*f)).ok_or_else(internal)?,
            rotated,
        ));
    }
    items.sort_unstable();
    let triangles: Vec<[u32; 3]> = items.iter().map(|(_, t)| *t).collect();
    let faces: Vec<u16> = items.iter().map(|(f, _)| *f).collect();
    let blob = encode(&vertices, &triangles, &faces, face_ids.len()).ok_or_else(internal)?;
    Ok(Canonical {
        blob,
        face_ids,
        normals,
        removed,
    })
}

/// Canonicalises, parses back, certifies and measures a candidate; seals it only if every
/// proof holds.
pub fn seal(
    candidate: Candidate,
    tolerance: Tolerance,
    limits: &OpLimits,
    expect: Expect,
    cancelled: &mut impl FnMut(Stage) -> bool,
) -> Result<OpResult, OpFailure> {
    let canonical = canonicalise(candidate, limits)?;
    let mesh =
        TriMesh::from_blob(&canonical.blob).map_err(|e| OpFailure::from_mesh(e, OpField::Mesh))?;
    poll(cancelled, Stage::Checks)?;
    let analysis = analyze_with_tolerance(&mesh, tolerance, limits.analysis, &mut *cancelled)
        .map_err(|e| OpFailure::from_mesh(e, OpField::Mesh))?;
    match analysis.status() {
        ShellStatus::Certified => {}
        ShellStatus::Uncertain(_) => {
            return Err(OpFailure::numeric(
                OpField::Mesh,
                OpDefect::ResultUncertain,
                Stage::Checks,
            ));
        }
        ShellStatus::Degenerate => {
            return Err(OpFailure::unsupported(
                OpField::Mesh,
                OpDefect::ResultDegenerate,
                Stage::Checks,
            ));
        }
        ShellStatus::Overlapping => {
            return Err(OpFailure::unsupported(
                OpField::Mesh,
                OpDefect::ResultOverlapping,
                Stage::Checks,
            ));
        }
        _ => {
            return Err(OpFailure::self_check(
                OpDefect::ResultNotCertified,
                Stage::Checks,
            ));
        }
    }
    let [shell] = analysis.shells() else {
        return Err(OpFailure::unsupported(
            OpField::Mesh,
            OpDefect::ResultShells,
            Stage::Checks,
        ));
    };
    if expect
        .euler
        .is_some_and(|e| e != shell.euler_characteristic)
        || expect.genus.is_some_and(|g| shell.genus != Some(g))
    {
        return Err(OpFailure::self_check(
            OpDefect::ResultTopology,
            Stage::Checks,
        ));
    }
    let measurement = measure_with_cancel(&mesh, &analysis, &mut *cancelled)
        .map_err(|e| OpFailure::from_mesh(e, OpField::Volume))?;
    let volume = measurement.volume_mm3().ok_or_else(|| {
        OpFailure::numeric(OpField::Volume, OpDefect::NumericRange, Stage::Quantities)
    })?;
    if expect.analytic_volume.is_some_and(|a| !a.overlaps(volume)) {
        return Err(OpFailure::self_check(
            OpDefect::ResultVolume,
            Stage::Quantities,
        ));
    }
    poll(cancelled, Stage::Quantities)?;
    let digest = Sha256::digest(&canonical.blob)
        .map_err(|_| OpFailure::self_check(OpDefect::ResultNotCertified, Stage::Quantities))?;
    let mut stats = expect.stats;
    stats.triangles_out = mesh.triangles().len();
    let bounds = *measurement.bounds();
    let body = Body::from_sealed(
        mesh,
        canonical.face_ids,
        canonical.normals,
        tolerance,
        analysis,
        volume,
    );
    Ok(OpResult {
        kind: expect.kind,
        body,
        blob: canonical.blob,
        digest,
        bounds,
        analytic_volume_mm3: expect.analytic_volume,
        plane_deviation_mm: expect.plane_deviation_mm,
        removed_faces: canonical.removed,
        stats,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube(lo: f64, hi: f64) -> Candidate {
        let vertices: Vec<[f64; 3]> = (0..8)
            .map(|i| {
                let pick = |bit: u32| if i & bit == 0 { lo } else { hi };
                [pick(1), pick(2), pick(4)]
            })
            .collect();
        let triangles = vec![
            [0, 4, 6],
            [0, 6, 2],
            [1, 3, 7],
            [1, 7, 5],
            [0, 1, 5],
            [0, 5, 4],
            [2, 6, 7],
            [2, 7, 3],
            [0, 2, 3],
            [0, 3, 1],
            [4, 5, 7],
            [4, 7, 6],
        ];
        Candidate {
            vertices,
            triangles,
            faces: vec![0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5],
            face_ids: ["t:f.a", "t:f.b", "t:f.c", "t:f.d", "t:f.e", "t:f.f"]
                .map(str::to_owned)
                .to_vec(),
            normals: vec![None; 6],
        }
    }

    fn expect() -> Expect {
        Expect {
            kind: OpKind::PlaneCut,
            euler: None,
            genus: None,
            analytic_volume: None,
            plane_deviation_mm: None,
            stats: OpStats::default(),
        }
    }

    fn tolerance() -> Tolerance {
        Tolerance::for_plate(&[
            beamcraft_geom::Point3::new(0.0, 0.0, 0.0),
            beamcraft_geom::Point3::new(4.0, 0.0, 0.0),
            beamcraft_geom::Point3::new(4.0, 4.0, 0.0),
        ])
        .unwrap()
    }

    fn run(c: Candidate, e: Expect) -> Result<OpResult, OpFailure> {
        seal(c, tolerance(), &OpLimits::DEFAULT, e, &mut |_| false)
    }

    fn refusal(c: Candidate) -> (OpCode, OpDefect) {
        let e = run(c, expect()).unwrap_err();
        (e.code, e.defect)
    }

    #[test]
    fn a_valid_candidate_seals_and_corrupt_candidates_never_do() {
        let ok = run(cube(0.0, 4.0), expect()).unwrap();
        assert_eq!(ok.volume_mm3().lo, 64.0);
        assert_eq!(ok.stats().triangles_out, 12);
        // One flipped triangle, one missing triangle and a fully inverted cube.
        let mut flipped = cube(0.0, 4.0);
        flipped.triangles[3].swap(1, 2);
        assert_eq!(
            refusal(flipped),
            (OpCode::SelfCheckFailed, OpDefect::ResultNotCertified)
        );
        let mut open = cube(0.0, 4.0);
        open.triangles.pop();
        open.faces.pop();
        assert_eq!(
            refusal(open),
            (OpCode::SelfCheckFailed, OpDefect::ResultNotCertified)
        );
        let mut inward = cube(0.0, 4.0);
        inward.triangles.iter_mut().for_each(|t| t.swap(1, 2));
        assert_eq!(
            refusal(inward),
            (OpCode::SelfCheckFailed, OpDefect::ResultNotCertified)
        );
        // A collapsed vertex makes zero-area triangles; a doubled cube is two shells.
        let mut flat = cube(0.0, 4.0);
        flat.vertices[7] = flat.vertices[6];
        assert_eq!(
            refusal(flat),
            (OpCode::OpUnsupported, OpDefect::ResultDegenerate)
        );
        let mut a = cube(0.0, 4.0);
        let b = cube(10.0, 14.0);
        let shift = u32::try_from(a.vertices.len()).unwrap();
        a.vertices.extend(b.vertices);
        a.triangles
            .extend(b.triangles.iter().map(|t| t.map(|i| i + shift)));
        a.faces.extend(b.faces.iter().map(|f| f + 6));
        a.face_ids
            .extend(["u:f.a", "u:f.b", "u:f.c", "u:f.d", "u:f.e", "u:f.f"].map(str::to_owned));
        a.normals.extend(vec![None; 6]);
        assert_eq!(refusal(a), (OpCode::OpUnsupported, OpDefect::ResultShells));
        // Cubes that share a face region overlap.
        let mut touching = cube(0.0, 4.0);
        let c = cube(2.0, 6.0);
        touching.vertices.extend(c.vertices);
        touching
            .triangles
            .extend(c.triangles.iter().map(|t| t.map(|i| i + shift)));
        touching.faces.extend(c.faces.iter().map(|f| f + 6));
        touching
            .face_ids
            .extend(["u:f.a", "u:f.b", "u:f.c", "u:f.d", "u:f.e", "u:f.f"].map(str::to_owned));
        touching.normals.extend(vec![None; 6]);
        assert_eq!(
            refusal(touching),
            (OpCode::OpUnsupported, OpDefect::ResultOverlapping)
        );
    }

    #[test]
    fn inconsistent_tables_and_expectations_are_refused() {
        let mut bad_index = cube(0.0, 4.0);
        bad_index.triangles[0][0] = 99;
        assert_eq!(
            refusal(bad_index),
            (OpCode::SelfCheckFailed, OpDefect::ResultNotCertified)
        );
        let mut bad_face = cube(0.0, 4.0);
        bad_face.faces[0] = 40;
        assert_eq!(
            refusal(bad_face),
            (OpCode::SelfCheckFailed, OpDefect::ResultNotCertified)
        );
        let mut short = cube(0.0, 4.0);
        short.faces.pop();
        assert_eq!(
            refusal(short),
            (OpCode::SelfCheckFailed, OpDefect::ResultNotCertified)
        );
        // A wrong analytic volume, Euler characteristic or genus is a failed self-check.
        let wrong_volume = Expect {
            analytic_volume: Enclosure::new(60.0, 61.0),
            ..expect()
        };
        let e = run(cube(0.0, 4.0), wrong_volume).unwrap_err();
        assert_eq!(
            (e.code, e.defect),
            (OpCode::SelfCheckFailed, OpDefect::ResultVolume)
        );
        let right_volume = Expect {
            analytic_volume: Enclosure::new(63.9, 64.1),
            ..expect()
        };
        assert!(run(cube(0.0, 4.0), right_volume).is_ok());
        let e = run(
            cube(0.0, 4.0),
            Expect {
                euler: Some(0),
                ..expect()
            },
        )
        .unwrap_err();
        assert_eq!(e.defect, OpDefect::ResultTopology);
        let e = run(
            cube(0.0, 4.0),
            Expect {
                genus: Some(1),
                ..expect()
            },
        )
        .unwrap_err();
        assert_eq!(e.defect, OpDefect::ResultTopology);
        assert!(
            run(
                cube(0.0, 4.0),
                Expect {
                    euler: Some(2),
                    genus: Some(0),
                    ..expect()
                }
            )
            .is_ok()
        );
        // The triangle budget applies before anything else.
        let limits = OpLimits {
            max_triangles: 11,
            ..OpLimits::DEFAULT
        };
        let e = seal(cube(0.0, 4.0), tolerance(), &limits, expect(), &mut |_| {
            false
        })
        .unwrap_err();
        assert_eq!(e.defect, OpDefect::BudgetTriangles);
        let limits = OpLimits {
            max_triangles: 12,
            ..OpLimits::DEFAULT
        };
        assert!(
            seal(cube(0.0, 4.0), tolerance(), &limits, expect(), &mut |_| {
                false
            })
            .is_ok()
        );
    }

    #[test]
    fn unused_vertices_and_faces_disappear_and_the_bytes_ignore_numbering() {
        let base = run(cube(0.0, 4.0), expect()).unwrap();
        // Extra unused vertices, an extra unused face, reversed vertex order and rotated corners.
        let mut c = cube(0.0, 4.0);
        c.vertices.push([9.0, 9.0, 9.0]);
        c.face_ids.push("t:f.unused".to_owned());
        c.normals.push(None);
        let n = u32::try_from(c.vertices.len()).unwrap();
        c.vertices.reverse();
        c.triangles.iter_mut().for_each(|t| {
            *t = t.map(|i| n - 1 - i);
            t.rotate_left(1);
        });
        c.triangles.reverse();
        c.faces.reverse();
        let r = run(c, expect()).unwrap();
        assert_eq!(r.blob(), base.blob());
        assert_eq!(r.removed_faces(), ["t:f.unused".to_owned()]);
        assert_eq!(r.face_table().len(), 6);
    }
}
