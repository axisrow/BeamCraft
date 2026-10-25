// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Display data of a view snapshot: one vertex buffer per distinct mesh and one rigid transform
//! per element that uses it (instancing by mesh digest). Nothing here is engineering geometry.

use crate::Error;
use beamcraft_engine::{Representation, ViewSnapshot};
use beamcraft_model::digest::ContentDigest;
use std::collections::BTreeSet;

/// Bytes of one expanded vertex: position and normal, 3 x f32 each.
pub const VERTEX_BYTES: usize = 24;
/// Bytes of one instance: a translation and quaternion, 7 x f32.
pub const INSTANCE_BYTES: usize = 28;
/// Largest total of expanded vertex bytes a scene may upload.
pub const MAX_VERTEX_BYTES: usize = 256 * 1024 * 1024;

/// One distinct mesh and the elements drawn with it.
#[derive(Debug, Clone, PartialEq)]
pub struct DrawMesh {
    /// The engine's mesh digest.
    pub digest: ContentDigest,
    /// Triangles expanded to position + flat normal, little-endian f32.
    pub vertices: Vec<u8>,
    /// Vertex count (three per triangle).
    pub vertex_count: u32,
    /// Element-local point (millimetres) the vertices are relative to: the centre of the mesh's
    /// own bounds, so large element-local coordinates keep their precision in f32.
    pub anchor_mm: [f64; 3],
    /// Element transform plus the rotated anchor, relative to [`Scene::origin_mm`], little-endian f32.
    pub instances: Vec<u8>,
    /// Instance count.
    pub instance_count: u32,
    /// The device already holds this mesh (the same digest), so `vertices` was not expanded and
    /// is empty; the upload reuses the device's buffer.
    pub resident: bool,
}

/// The display-only ghost preview of the member being placed (issue #134): the engine's own
/// body mesh for the two picked points, expanded for drawing and tinted by its own pipeline.
/// Nothing here decides whether the placement is acceptable; the engine says so at commit.
#[derive(Debug, Clone, PartialEq)]
pub struct GhostMesh {
    /// Expanded position/normal triangles, relative to [`GhostMesh::anchor_mm`].
    pub vertices: Vec<u8>,
    /// Corner count (three per triangle).
    pub vertex_count: u32,
    /// World point (millimetres) the vertices are relative to: the centre of the ghost bounds.
    pub anchor_mm: [f64; 3],
    /// The one instance: the anchor relative to the scene origin, identity orientation.
    pub instance: [u8; INSTANCE_BYTES],
}

impl GhostMesh {
    /// Expands the engine's member body (already in world coordinates: the placement rotation
    /// composes into the body) into a ghost drawn at the scene origin.
    pub fn of(vertices: &[[f64; 3]], triangles: &[[u32; 3]], origin_mm: [f64; 3]) -> Self {
        let mut low = [f64::INFINITY; 3];
        let mut high = [f64::NEG_INFINITY; 3];
        for p in vertices {
            for ((l, h), x) in low.iter_mut().zip(high.iter_mut()).zip(*p) {
                *l = l.min(x);
                *h = h.max(x);
            }
        }
        let mut anchor = [0.0; 3];
        for ((a, l), h) in anchor.iter_mut().zip(low).zip(high) {
            *a = (l + h) / 2.0;
        }
        let mut out = Vec::with_capacity(triangles.len() * 3 * VERTEX_BYTES);
        for triangle in triangles {
            let mut corners = [[0.0; 3]; 3];
            for (k, index) in triangle.iter().enumerate() {
                // Kernel-validated indices; a stray one degrades one triangle, never a crash.
                if let Some(slot) = corners.get_mut(k) {
                    *slot = vertices.get(*index as usize).copied().unwrap_or([0.0; 3]);
                }
            }
            let [a, b, c] = corners;
            let n = normal(a, b, c);
            for p in corners {
                let local = sub(p, anchor).map(|x| x as f32);
                for value in local.into_iter().chain(n) {
                    out.extend_from_slice(&value.to_le_bytes());
                }
            }
        }
        let mut instance = [0u8; INSTANCE_BYTES];
        for (k, (a, o)) in anchor.iter().zip(origin_mm).enumerate() {
            if let Some(slot) = instance.get_mut(k * 4..k * 4 + 4) {
                slot.copy_from_slice(&((*a - o) as f32).to_le_bytes());
            }
        }
        instance[24..28].copy_from_slice(&1.0f32.to_le_bytes());
        Self {
            vertex_count: (triangles.len() * 3) as u32,
            vertices: out,
            anchor_mm: anchor,
            instance,
        }
    }
}

/// The display scene of one snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    /// World point (millimetres) that display coordinates are relative to: the centre of the
    /// engineering bounds.
    pub origin_mm: [f64; 3],
    /// Engineering bounds relative to the origin, millimetres.
    pub min: [f32; 3],
    /// Engineering bounds relative to the origin, millimetres.
    pub max: [f32; 3],
    /// Distinct meshes in digest order.
    pub meshes: Vec<DrawMesh>,
    /// Elements drawn.
    pub elements: usize,
    /// Triangles drawn per frame over all instances.
    pub triangles: u64,
}

/// What the window keeps of a scene once it is on the device: framing and counts, no buffers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneSummary {
    /// See [`Scene::origin_mm`].
    pub origin_mm: [f64; 3],
    /// See [`Scene::min`].
    pub min: [f32; 3],
    /// See [`Scene::max`].
    pub max: [f32; 3],
    /// Distinct meshes.
    pub meshes: usize,
    /// Instances over all meshes.
    pub instances: u64,
    /// Elements drawn.
    pub elements: usize,
    /// Triangles drawn per frame over all instances.
    pub triangles: u64,
}

/// Centre and size of bounds, for camera framing.
fn framing(min: [f32; 3], max: [f32; 3]) -> ([f32; 3], f32) {
    let centre = [
        (min[0] + max[0]) / 2.0,
        (min[1] + max[1]) / 2.0,
        (min[2] + max[2]) / 2.0,
    ];
    let d = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];
    let extent = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    (centre, extent.max(1.0))
}

impl SceneSummary {
    /// Centre and size of the bounds, for camera framing.
    pub fn framing(&self) -> ([f32; 3], f32) {
        framing(self.min, self.max)
    }

    /// Distinct meshes and instances in total.
    pub const fn counts(&self) -> (usize, u64) {
        (self.meshes, self.instances)
    }
}

fn read<const N: usize>(bytes: &[u8], offset: usize) -> Result<[u8; N], Error> {
    let end = offset.checked_add(N).ok_or(Error::Input("mesh offset"))?;
    bytes
        .get(offset..end)
        .and_then(|b| <[u8; N]>::try_from(b).ok())
        .ok_or(Error::Input("mesh offset"))
}

fn sub([x, y, z]: [f64; 3], [a, b, c]: [f64; 3]) -> [f64; 3] {
    [x - a, y - b, z - c]
}

/// Display-only flat normal of a triangle; a degenerate one gets +Z.
fn normal(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> [f32; 3] {
    let [ux, uy, uz] = sub(b, a);
    let [vx, vy, vz] = sub(c, a);
    let n = [uy * vz - uz * vy, uz * vx - ux * vz, ux * vy - uy * vx];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if !len.is_finite() || len <= f64::MIN_POSITIVE {
        return [0.0, 0.0, 1.0];
    }
    [
        (n[0] / len) as f32,
        (n[1] / len) as f32,
        (n[2] / len) as f32,
    ]
}

/// Vertex `i` of a BCMESH01 blob with `v` vertices.
fn point(blob: &[u8], v: usize, i: usize) -> Result<[f64; 3], Error> {
    if i >= v {
        return Err(Error::Input("mesh index"));
    }
    let mut p = [0.0; 3];
    for (axis, value) in p.iter_mut().enumerate() {
        let offset = i
            .checked_mul(24)
            .and_then(|o| o.checked_add(32 + axis * 8))
            .ok_or(Error::Input("mesh offset"))?;
        *value = f64::from_le_bytes(read::<8>(blob, offset)?);
    }
    Ok(p)
}

/// The centre of a blob's vertex bounds, computed in f64: the point the display vertices are
/// relative to. It depends on the blob alone, so equal digests have equal anchors.
fn anchor(blob: &[u8], vertex_count: u32) -> Result<[f64; 3], Error> {
    let v = usize::try_from(vertex_count).map_err(|_| Error::Input("mesh size"))?;
    let mut low = [f64::INFINITY; 3];
    let mut high = [f64::NEG_INFINITY; 3];
    for i in 0..v {
        let p = point(blob, v, i)?;
        for ((l, h), x) in low.iter_mut().zip(high.iter_mut()).zip(p) {
            *l = l.min(x);
            *h = h.max(x);
        }
    }
    let mut anchor = [0.0; 3];
    for ((a, l), h) in anchor.iter_mut().zip(low).zip(high) {
        let centre = (l + h) / 2.0;
        *a = if centre.is_finite() { centre } else { 0.0 };
    }
    Ok(anchor)
}

/// Bytes of the expanded display vertices of `triangle_count` triangles.
fn expanded_size(triangle_count: u32) -> Result<usize, Error> {
    usize::try_from(triangle_count)
        .ok()
        .and_then(|t| t.checked_mul(3 * VERTEX_BYTES))
        .ok_or(Error::Input("mesh size"))
}

/// Expands an engine-validated BCMESH01 blob (F06 section 6) into position/normal triangles,
/// positions relative to the centre of the mesh's bounds (computed in f64 before the f32 cast).
/// Returns the vertex bytes and that centre.
fn expand(
    blob: &[u8],
    vertex_count: u32,
    triangle_count: u32,
) -> Result<(Vec<u8>, [f64; 3]), Error> {
    let v = usize::try_from(vertex_count).map_err(|_| Error::Input("mesh size"))?;
    let t = usize::try_from(triangle_count).map_err(|_| Error::Input("mesh size"))?;
    let anchor = anchor(blob, vertex_count)?;
    let vertex = |i: usize| point(blob, v, i);
    let index_start = v
        .checked_mul(24)
        .and_then(|n| n.checked_add(32))
        .ok_or(Error::Input("mesh offset"))?;
    let size = expanded_size(triangle_count)?;
    let mut out = Vec::new();
    out.try_reserve_exact(size).map_err(|_| Error::Allocation)?;
    for triangle in 0..t {
        let mut corners = [[0.0; 3]; 3];
        for (k, corner) in corners.iter_mut().enumerate() {
            let offset = triangle
                .checked_mul(12)
                .and_then(|o| o.checked_add(index_start + k * 4))
                .ok_or(Error::Input("mesh offset"))?;
            let index = u32::from_le_bytes(read::<4>(blob, offset)?);
            *corner = vertex(usize::try_from(index).map_err(|_| Error::Input("mesh index"))?)?;
        }
        let [a, b, c] = corners;
        let n = normal(a, b, c);
        for p in corners {
            let local = sub(p, anchor).map(|x| x as f32);
            for value in local.into_iter().chain(n) {
                out.extend_from_slice(&value.to_le_bytes());
            }
        }
    }
    Ok((out, anchor))
}

impl Scene {
    /// Builds the display scene of `snapshot` with the chosen representation.
    pub fn from_snapshot(
        snapshot: &ViewSnapshot,
        representation: Representation,
    ) -> Result<Self, Error> {
        Self::from_snapshot_reusing(snapshot, representation, &BTreeSet::new())
    }

    /// [`Scene::from_snapshot`] for a device that already holds the meshes in `resident`: those
    /// are not expanded again (their [`DrawMesh::resident`] is set and `vertices` is empty), but
    /// they count against the vertex budget as if they were, so the same snapshot is accepted or
    /// refused either way.
    pub fn from_snapshot_reusing(
        snapshot: &ViewSnapshot,
        representation: Representation,
        resident: &BTreeSet<ContentDigest>,
    ) -> Result<Self, Error> {
        let bounds = snapshot.engineering_bounds();
        let origin_mm = bounds.map_or([0.0; 3], |b| {
            [
                (b.min[0] + b.max[0]) / 2.0,
                (b.min[1] + b.max[1]) / 2.0,
                (b.min[2] + b.max[2]) / 2.0,
            ]
        });
        let (min, max) = bounds.map_or(([0.0; 3], [0.0; 3]), |b| {
            (
                sub(b.min, origin_mm).map(|x| x as f32),
                sub(b.max, origin_mm).map(|x| x as f32),
            )
        });
        let mut meshes: Vec<DrawMesh> = Vec::new();
        let mut slots: Vec<Option<usize>> = vec![None; snapshot.meshes().len()];
        let mut total = 0usize;
        let mut triangles = 0u64;
        for element in snapshot.elements() {
            let reference = element.representation(representation);
            let slot = usize::try_from(reference.mesh_slot).map_err(|_| Error::Input("slot"))?;
            let at = match slots.get(slot).copied().flatten() {
                Some(at) => at,
                None => {
                    let entry = snapshot
                        .mesh_at(reference.mesh_slot)
                        .ok_or(Error::Input("slot"))?;
                    let reuse = resident.contains(&entry.digest());
                    let (vertices, anchor_mm) = if reuse {
                        (Vec::new(), anchor(entry.bytes(), entry.vertex_count())?)
                    } else {
                        expand(entry.bytes(), entry.vertex_count(), entry.triangle_count())?
                    };
                    total = total
                        .checked_add(expanded_size(entry.triangle_count())?)
                        .filter(|t| *t <= MAX_VERTEX_BYTES)
                        .ok_or(Error::Input("scene vertex budget"))?;
                    meshes.push(DrawMesh {
                        digest: entry.digest(),
                        vertex_count: entry
                            .triangle_count()
                            .checked_mul(3)
                            .ok_or(Error::Input("mesh size"))?,
                        vertices,
                        anchor_mm,
                        instances: Vec::new(),
                        instance_count: 0,
                        resident: reuse,
                    });
                    let at = meshes.len() - 1;
                    if let Some(s) = slots.get_mut(slot) {
                        *s = Some(at);
                    }
                    at
                }
            };
            let mesh = meshes.get_mut(at).ok_or(Error::Input("slot"))?;
            let [px, py, pz] = element.placement_mm;
            let [ax, ay, az] = mesh.anchor_mm;
            let [qx, qy, qz, qw] = element.orientation_xyzw;
            let norm = (((qx * qx + qy * qy) + qz * qz) + qw * qw).sqrt();
            if !(norm.is_finite() && norm > 0.0) {
                return Err(Error::Input("orientation"));
            }
            let [x, y, z, w] = [qx / norm, qy / norm, qz / norm, qw / norm];
            let anchor = [
                (1.0 - 2.0 * (y * y + z * z)) * ax
                    + 2.0 * (x * y - z * w) * ay
                    + 2.0 * (x * z + y * w) * az,
                2.0 * (x * y + z * w) * ax
                    + (1.0 - 2.0 * (x * x + z * z)) * ay
                    + 2.0 * (y * z - x * w) * az,
                2.0 * (x * z - y * w) * ax
                    + 2.0 * (y * z + x * w) * ay
                    + (1.0 - 2.0 * (x * x + y * y)) * az,
            ];
            for value in sub([px + anchor[0], py + anchor[1], pz + anchor[2]], origin_mm) {
                let value = value as f32;
                if !value.is_finite() {
                    return Err(Error::Input("placement"));
                }
                mesh.instances.extend_from_slice(&value.to_le_bytes());
            }
            for value in [x, y, z, w] {
                let value = value as f32;
                if !value.is_finite() {
                    return Err(Error::Input("orientation"));
                }
                mesh.instances.extend_from_slice(&value.to_le_bytes());
            }
            mesh.instance_count = mesh
                .instance_count
                .checked_add(1)
                .ok_or(Error::Input("instances"))?;
            triangles = triangles.saturating_add(u64::from(mesh.vertex_count / 3));
            // Instancing makes drawn triangles independent of uploaded bytes: the snapshot's
            // projection limit bounds what one frame may submit.
            if triangles > u64::from(snapshot.limits().max_projected_triangles()) {
                return Err(Error::View(beamcraft_engine::ViewError::new(
                    beamcraft_engine::ViewCode::BufferLimit,
                )));
            }
        }
        meshes.sort_by_key(|m| m.digest);
        Ok(Self {
            origin_mm,
            min,
            max,
            meshes,
            elements: snapshot.len(),
            triangles,
        })
    }

    /// Centre and size of the bounds, for camera framing.
    pub fn framing(&self) -> ([f32; 3], f32) {
        framing(self.min, self.max)
    }

    /// The framing and counts the window keeps after the upload, without any buffer.
    pub fn summary(&self) -> SceneSummary {
        let (meshes, instances) = self.counts();
        SceneSummary {
            origin_mm: self.origin_mm,
            min: self.min,
            max: self.max,
            meshes,
            instances,
            elements: self.elements,
            triangles: self.triangles,
        }
    }

    /// CPU bytes of the display buffers (expanded vertices and instances) this scene holds.
    pub fn buffer_bytes(&self) -> u64 {
        self.meshes
            .iter()
            .map(|m| (m.vertices.capacity() as u64).saturating_add(m.instances.capacity() as u64))
            .fold(0, u64::saturating_add)
    }

    /// Distinct meshes and instances in total.
    pub fn counts(&self) -> (usize, u64) {
        (
            self.meshes.len(),
            self.meshes
                .iter()
                .map(|m| u64::from(m.instance_count))
                .sum(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use beamcraft_engine::synthetic::plate_scene;
    use beamcraft_engine::{Limits, ViewPin};
    use beamcraft_model::digest::Sha256;

    fn snapshot(count: u32) -> ViewSnapshot {
        let pin = ViewPin::new(Sha256::digest(b"revision").unwrap(), 1);
        ViewSnapshot::build(pin, plate_scene(count, false).unwrap(), &Limits::default()).unwrap()
    }

    #[test]
    fn every_element_is_one_instance_of_its_mesh_and_coordinates_are_recentred() {
        let s = snapshot(30);
        let scene = Scene::from_snapshot(&s, Representation::Full).unwrap();
        let (meshes, instances) = scene.counts();
        assert_eq!(instances, 30);
        assert_eq!(meshes, s.meshes().len());
        assert!(meshes <= 30);
        for mesh in &scene.meshes {
            assert_eq!(
                mesh.vertices.len(),
                mesh.vertex_count as usize * VERTEX_BYTES
            );
            assert_eq!(
                mesh.instances.len(),
                mesh.instance_count as usize * INSTANCE_BYTES
            );
        }
        // Bounds are symmetric about the origin after re-centring.
        for axis in 0..3 {
            assert!((scene.min[axis] + scene.max[axis]).abs() < 1e-3);
        }
        let (_, extent) = scene.framing();
        assert!(extent > 1.0);
    }

    #[test]
    fn a_rotated_element_uploads_the_full_instance_quaternion() {
        let half = core::f64::consts::FRAC_1_SQRT_2;
        let elements = plate_scene(1, false)
            .unwrap()
            .into_iter()
            .map(|element| element.with_orientation([0.0, 0.0, half, half]))
            .collect();
        let pin = ViewPin::new(Sha256::digest(b"rotated revision").unwrap(), 1);
        let snapshot = ViewSnapshot::build(pin, elements, &Limits::default()).unwrap();
        let scene = Scene::from_snapshot(&snapshot, Representation::Full).unwrap();
        let instance = scene.meshes[0].instances.as_slice();
        assert_eq!(instance.len(), INSTANCE_BYTES);
        let orientation: Vec<f32> = instance[12..]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|part| f32::from_le_bytes(*part))
            .collect();
        assert_eq!(orientation, [0.0, 0.0, half as f32, half as f32]);
    }

    #[test]
    fn drawn_triangles_respect_the_snapshot_projection_limit() {
        let pin = ViewPin::new(Sha256::digest(b"revision").unwrap(), 1);
        // Ten 12-triangle plates are 120 triangles; a limit of 100 refuses the scene.
        let limits = Limits::new(100, usize::MAX, 100, 10);
        let s = ViewSnapshot::build(pin, plate_scene(10, false).unwrap(), &limits).unwrap();
        assert!(matches!(
            Scene::from_snapshot(&s, Representation::Full),
            Err(Error::View(e)) if e.code() == beamcraft_engine::ViewCode::BufferLimit
        ));
    }

    #[test]
    fn flat_normals_are_unit_and_degenerate_triangles_get_up() {
        let n = normal([0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        assert_eq!(n, [0.0, 0.0, 1.0]);
        let d = normal([0.0; 3], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]);
        assert_eq!(d, [0.0, 0.0, 1.0]);
        let n = normal([0.0; 3], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]);
        assert_eq!(n, [0.0, 1.0, 0.0]);
    }

    #[test]
    fn a_ghost_mesh_is_relative_to_its_own_anchor_and_places_at_the_scene_origin() {
        // A unit quad at z = 7, two triangles: the engine's member body stands in for it.
        let vertices = vec![
            [0.0, 0.0, 7.0],
            [10.0, 0.0, 7.0],
            [10.0, 10.0, 7.0],
            [0.0, 10.0, 7.0],
        ];
        let triangles = vec![[0u32, 1, 2], [0, 2, 3]];
        let ghost = GhostMesh::of(&vertices, &triangles, [2.0, 1.0, 7.0]);
        assert_eq!(ghost.anchor_mm, [5.0, 5.0, 7.0]);
        assert_eq!(ghost.vertex_count, 6);
        assert_eq!(ghost.vertices.len(), 6 * VERTEX_BYTES);
        // The first corner is (0, 0, 7) minus the anchor.
        let part = |at: usize| f32::from_le_bytes(ghost.vertices[at..at + 4].try_into().unwrap());
        assert_eq!((part(0), part(4), part(8)), (-5.0, -5.0, 0.0));
        // The instance shifts the anchor relative to the scene origin, orientation identity.
        let words: Vec<f32> = ghost.instance[12..]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect();
        assert_eq!(words, [0.0, 0.0, 0.0, 1.0]);
        let shift: Vec<f32> = ghost.instance[..12]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect();
        assert_eq!(shift, [3.0, 4.0, 0.0]);
    }

    #[test]
    fn resident_meshes_are_not_expanded_but_keep_anchor_instances_and_budget() {
        let s = snapshot(30);
        let full = Scene::from_snapshot(&s, Representation::Full).unwrap();
        let resident: BTreeSet<ContentDigest> =
            full.meshes.iter().step_by(2).map(|m| m.digest).collect();
        let reused = Scene::from_snapshot_reusing(&s, Representation::Full, &resident).unwrap();
        assert_eq!(reused.meshes.len(), full.meshes.len());
        for (a, b) in full.meshes.iter().zip(&reused.meshes) {
            assert_eq!(a.digest, b.digest);
            assert_eq!(b.resident, resident.contains(&a.digest));
            assert_eq!(b.vertices.is_empty(), b.resident);
            if !b.resident {
                assert_eq!(a.vertices, b.vertices);
            }
            // Same anchor and the same instance translations: the reused buffer fits them.
            assert_eq!(a.anchor_mm, b.anchor_mm);
            assert_eq!(a.instances, b.instances);
            assert_eq!(a.vertex_count, b.vertex_count);
        }
        assert_eq!(full.summary(), reused.summary());
        assert!(reused.buffer_bytes() < full.buffer_bytes());
    }

    #[test]
    fn the_summary_keeps_framing_and_counts_without_buffers() {
        let s = snapshot(12);
        let scene = Scene::from_snapshot(&s, Representation::Full).unwrap();
        let summary = scene.summary();
        assert_eq!(summary.framing(), scene.framing());
        assert_eq!(summary.counts(), scene.counts());
        assert_eq!(summary.elements, scene.elements);
        assert_eq!(summary.triangles, scene.triangles);
        assert!(scene.buffer_bytes() > 0);
    }

    #[test]
    fn an_empty_snapshot_is_an_empty_scene() {
        let pin = ViewPin::new(Sha256::digest(b"revision").unwrap(), 1);
        let empty = ViewSnapshot::build(pin, Vec::new(), &Limits::default()).unwrap();
        let scene = Scene::from_snapshot(&empty, Representation::Full).unwrap();
        assert_eq!(scene.counts(), (0, 0));
        assert_eq!(scene.origin_mm, [0.0; 3]);
    }
}

#[cfg(test)]
mod precision_tests {
    use super::*;

    /// A minimal valid BCMESH01 blob: one triangle with the given vertices.
    fn blob(points: [[f64; 3]; 3]) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(b"BCMESH01");
        for n in [1u32, 3, 1, 1, 0, 0] {
            b.extend_from_slice(&n.to_le_bytes());
        }
        for p in points {
            for x in p {
                b.extend_from_slice(&x.to_le_bytes());
            }
        }
        for i in [0u32, 1, 2] {
            b.extend_from_slice(&i.to_le_bytes());
        }
        b.extend_from_slice(&[0u8; 4]);
        b.extend_from_slice(&[0u8; 8]);
        b
    }

    #[test]
    fn far_element_local_coordinates_keep_their_size_in_f32() {
        // A 20 mm feature near X = 999 999 936 mm: both X values round to one f32 without the anchor.
        let x = 999_999_936.0;
        let (bytes, anchor) = expand(
            &blob([[x - 10.0, 0.0, 0.0], [x + 10.0, 0.0, 0.0], [x, 5.0, 0.0]]),
            3,
            1,
        )
        .unwrap();
        assert_eq!(anchor[0], x);
        let first = f32::from_le_bytes(bytes[0..4].try_into().unwrap());
        let second = f32::from_le_bytes(bytes[24..28].try_into().unwrap());
        assert_eq!(second - first, 20.0);
    }
}
