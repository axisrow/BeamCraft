// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Display data of a view snapshot: one vertex buffer per distinct mesh and one translation per
//! element that uses it (instancing by mesh digest). Nothing here is engineering geometry.

use crate::Error;
use beamcraft_engine::{Representation, ViewSnapshot};
use beamcraft_model::digest::ContentDigest;

/// Bytes of one expanded vertex: position and normal, 3 x f32 each.
pub const VERTEX_BYTES: usize = 24;
/// Bytes of one instance: a translation, 3 x f32.
pub const INSTANCE_BYTES: usize = 12;
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
    /// Element translations plus the anchor, relative to [`Scene::origin_mm`], little-endian f32.
    pub instances: Vec<u8>,
    /// Instance count.
    pub instance_count: u32,
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

/// Small, buffer-free description retained by a window after uploading a [`Scene`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneInfo {
    /// Engineering bounds relative to the display origin, millimetres.
    pub min: [f32; 3],
    /// Engineering bounds relative to the display origin, millimetres.
    pub max: [f32; 3],
    /// Number of elements drawn.
    pub elements: usize,
    /// Number of triangles drawn over all instances.
    pub triangles: u64,
    counts: (usize, u64),
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
    let point = |i: usize| -> Result<[f64; 3], Error> {
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
    };
    let mut low = [f64::INFINITY; 3];
    let mut high = [f64::NEG_INFINITY; 3];
    for i in 0..v {
        let p = point(i)?;
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
    let index_start = v
        .checked_mul(24)
        .and_then(|n| n.checked_add(32))
        .ok_or(Error::Input("mesh offset"))?;
    let size = t
        .checked_mul(3 * VERTEX_BYTES)
        .ok_or(Error::Input("mesh size"))?;
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
            *corner = point(usize::try_from(index).map_err(|_| Error::Input("mesh index"))?)?;
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
                    let (vertices, anchor_mm) =
                        expand(entry.bytes(), entry.vertex_count(), entry.triangle_count())?;
                    total = total
                        .checked_add(vertices.len())
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
            for value in sub([px + ax, py + ay, pz + az], origin_mm) {
                let value = value as f32;
                if !value.is_finite() {
                    return Err(Error::Input("placement"));
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
        self.info().framing()
    }

    /// Metadata needed after upload, without retaining expanded vertices or instances.
    pub fn info(&self) -> SceneInfo {
        SceneInfo {
            min: self.min,
            max: self.max,
            elements: self.elements,
            triangles: self.triangles,
            counts: self.counts(),
        }
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

impl SceneInfo {
    /// Centre and size of the bounds, for camera framing.
    pub fn framing(&self) -> ([f32; 3], f32) {
        let centre = [
            (self.min[0] + self.max[0]) / 2.0,
            (self.min[1] + self.max[1]) / 2.0,
            (self.min[2] + self.max[2]) / 2.0,
        ];
        let d = [
            self.max[0] - self.min[0],
            self.max[1] - self.min[1],
            self.max[2] - self.min[2],
        ];
        let extent = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        (centre, extent.max(1.0))
    }

    /// Distinct meshes and instances in total.
    pub fn counts(&self) -> (usize, u64) {
        self.counts
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
    fn upload_metadata_survives_dropping_all_display_buffers() {
        let scene = Scene::from_snapshot(&snapshot(30), Representation::Full).unwrap();
        let framing = scene.framing();
        let counts = scene.counts();
        let info = scene.info();
        assert!(scene.meshes.iter().any(|m| !m.vertices.is_empty()));
        drop(scene);
        assert_eq!(info.framing(), framing);
        assert_eq!(info.counts(), counts);
        assert_eq!(info.elements, 30);
        assert_eq!(info.triangles, 360);
        assert!(std::mem::size_of_val(&info) <= 80);
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
