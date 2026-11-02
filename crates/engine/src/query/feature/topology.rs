// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Semantic topology reconstructed from kernel-authored face roles, never buffer addresses.

use super::*;
use std::collections::{BTreeMap, BTreeSet};

fn decimal(text: &str, max: usize) -> Option<usize> {
    let n = text.parse::<usize>().ok()?;
    (n <= max && n.to_string() == text).then_some(n)
}

fn point_token(token: &str) -> bool {
    if let Some(n) = token.strip_prefix('o') {
        return decimal(n, 1023).is_some();
    }
    token
        .strip_prefix('c')
        .and_then(|s| s.split_once('.'))
        .is_some_and(|(c, k)| decimal(c, 255).is_some() && decimal(k, 1023).is_some())
}

pub(super) fn parse_role(body: BodyKind, token: &str) -> Option<FeatureKind> {
    if body == BodyKind::Cut {
        let face = token.strip_prefix("f.")?;
        if let Some(p) = face.strip_prefix('p') {
            return decimal(p, 999).map(|_| FeatureKind::Face);
        }
        let hole = face.strip_prefix('h')?;
        let valid = if let Some((h, side)) = hole.split_once(".s") {
            decimal(h, 999).is_some() && decimal(side, 1023).is_some()
        } else {
            decimal(hole, 999).is_some()
        };
        return valid.then_some(FeatureKind::Face);
    }
    if let Some(face) = token.strip_prefix("f.") {
        let valid = matches!(face, "z0" | "z1")
            || face
                .strip_prefix('o')
                .is_some_and(|n| decimal(n, 1023).is_some())
            || face.strip_prefix('c').is_some_and(|s| {
                if let Some((c, k)) = s.split_once(".s") {
                    decimal(c, 255).is_some() && decimal(k, 1023).is_some()
                } else {
                    decimal(s, 255).is_some()
                }
            });
        return valid.then_some(FeatureKind::Face);
    }
    let (kind, rest) = if let Some(s) = token.strip_prefix("v.") {
        (FeatureKind::Vertex, s)
    } else {
        (FeatureKind::Edge, token.strip_prefix("e.")?)
    };
    let (level, point) = rest.split_once('.')?;
    (point_token(point)
        && (matches!(level, "z0" | "z1") || (kind == FeatureKind::Edge && level == "v")))
        .then_some(kind)
}

/// A contour wall's authored contour and starting point.
fn wall(role: &str) -> Option<(String, usize)> {
    if let Some(k) = role.strip_prefix("f.o") {
        return Some(("o".into(), decimal(k, 1023)?));
    }
    let (c, k) = role.strip_prefix("f.c")?.split_once(".s")?;
    Some((format!("c{}", decimal(c, 255)?), decimal(k, 1023)?))
}

fn contour_point(contour: &str, k: usize) -> String {
    if contour == "o" {
        format!("o{k}")
    } else {
        format!("{contour}.{k}")
    }
}

#[derive(Clone)]
pub(crate) struct NamedFace {
    pub(crate) id: FeatureId,
    pub(crate) triangles: Vec<[V3; 3]>,
    pub(crate) boundary: Vec<[V3; 2]>,
    normal: Option<V3>,
    vertex_ids: Vec<FeatureId>,
}

pub(crate) struct NamedEdge {
    pub(crate) id: FeatureId,
    pub(crate) ends: [V3; 2],
    vertex_ids: [FeatureId; 2],
}

pub(crate) struct NamedFeatures {
    pub(crate) element: ElementId,
    pub(crate) faces: Vec<NamedFace>,
    pub(crate) edges: Vec<NamedEdge>,
    pub(crate) vertices: Vec<(FeatureId, V3)>,
}

impl NamedFeatures {
    pub(crate) fn local(snapshot: &ViewSnapshot, entry: &ElementEntry) -> Option<Self> {
        Self::placed(snapshot, entry, false)
    }
    pub(crate) fn world(snapshot: &ViewSnapshot, entry: &ElementEntry) -> Option<Self> {
        Self::placed(snapshot, entry, true)
    }
    fn placed(snapshot: &ViewSnapshot, entry: &ElementEntry, world: bool) -> Option<Self> {
        BodyKind::of(entry.kind)?;
        if entry.full_face_table.is_empty() || BoxFeatures::local(snapshot, entry).is_some() {
            return None;
        }
        let member = entry.kind.is_linear_member();
        let names: Vec<String> = entry
            .full_face_table
            .iter()
            .map(|s| {
                if member {
                    if let Some(role) = s.strip_prefix("extrusion:") {
                        return format!("profile:{role}");
                    }
                    if let Some(role) = s.strip_prefix("plate:") {
                        return format!("member:{role}");
                    }
                }
                s.clone()
            })
            .collect();
        let mesh = snapshot.mesh_at(entry.full.mesh_slot)?;
        let bytes = mesh.bytes();
        let v = usize::try_from(mesh.vertex_count()).ok()?;
        let t = usize::try_from(mesh.triangle_count()).ok()?;
        let indices = 32usize.checked_add(v.checked_mul(24)?)?;
        let faces_at = indices.checked_add(t.checked_mul(12)?.checked_add(7)? & !7)?;
        let mut positions = Vec::with_capacity(v);
        for i in 0..v {
            let mut p = [0.0; 3];
            for (axis, coordinate) in p.iter_mut().enumerate() {
                let at = 32 + i * 24 + axis * 8;
                *coordinate = f64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?);
            }
            if world {
                p = add(rotate_vector(p, entry.orientation_xyzw), entry.placement_mm);
            }
            positions.push(p);
        }
        let mut incident = vec![BTreeSet::new(); v];
        let mut edge_faces: BTreeMap<(u32, u32), BTreeSet<usize>> = BTreeMap::new();
        let mut triangles = vec![Vec::new(); names.len()];
        let mut face_vertices = vec![BTreeSet::new(); names.len()];
        for i in 0..t {
            let at = indices + i * 12;
            let a = read_u32(bytes, at)?;
            let b = read_u32(bytes, at + 4)?;
            let c = read_u32(bytes, at + 8)?;
            let face_at = faces_at + i * 2;
            let f = usize::from(u16::from_le_bytes(
                bytes.get(face_at..face_at + 2)?.try_into().ok()?,
            ));
            for vertex in [a, b, c] {
                incident.get_mut(usize::try_from(vertex).ok()?)?.insert(f);
                face_vertices.get_mut(f)?.insert(vertex);
            }
            triangles.get_mut(f)?.push([
                *positions.get(usize::try_from(a).ok()?)?,
                *positions.get(usize::try_from(b).ok()?)?,
                *positions.get(usize::try_from(c).ok()?)?,
            ]);
            for (x, y) in [(a, b), (b, c), (c, a)] {
                edge_faces
                    .entry((x.min(y), x.max(y)))
                    .or_default()
                    .insert(f);
            }
        }
        let mut by_vertex = BTreeMap::new();
        let mut named_positions = BTreeMap::new();
        for (i, adjacent) in incident.iter().enumerate() {
            let local = vertex_name(adjacent, &names)?;
            if let Some(local) = local {
                let id = FeatureId::from_local(entry.id, &local).ok()?;
                // Ambiguous names are never repaired by selecting a mesh index.
                if named_positions
                    .insert(id.clone(), *positions.get(i)?)
                    .is_some()
                {
                    return None;
                }
                by_vertex.insert(u32::try_from(i).ok()?, id);
            }
        }
        let vertices = named_positions.into_iter().collect();
        let mut boundary = vec![Vec::new(); names.len()];
        for ((a, b), adjacent) in &edge_faces {
            if adjacent.len() > 1 {
                let ends = [
                    *positions.get(usize::try_from(*a).ok()?)?,
                    *positions.get(usize::try_from(*b).ok()?)?,
                ];
                for f in adjacent {
                    boundary.get_mut(*f)?.push(ends);
                }
            }
        }
        let mut edges = Vec::new();
        let mut seen = BTreeSet::new();
        for ((a, b), adjacent) in edge_faces {
            if adjacent.len() < 2 {
                continue;
            }
            let (Some(a_id), Some(b_id)) = (by_vertex.get(&a), by_vertex.get(&b)) else {
                continue;
            };
            let Some(local) = edge_name(a_id, b_id, &adjacent, &names) else {
                continue;
            };
            let id = FeatureId::from_local(entry.id, &local).ok()?;
            if !seen.insert(id.clone()) {
                continue;
            }
            edges.push(NamedEdge {
                id,
                ends: [
                    *positions.get(usize::try_from(a).ok()?)?,
                    *positions.get(usize::try_from(b).ok()?)?,
                ],
                vertex_ids: [a_id.clone(), b_id.clone()],
            });
        }
        let mut faces = Vec::new();
        for (i, triangles) in triangles.into_iter().enumerate() {
            if triangles.is_empty() {
                continue;
            }
            let id = FeatureId::from_local(entry.id, names.get(i)?).ok()?;
            let normals: Vec<V3> = triangles
                .iter()
                .map(|[a, b, c]| {
                    let n = cross(sub(*b, *a), sub(*c, *a));
                    scale(n, 1.0 / norm(n))
                })
                .collect();
            let normal = normals.first().copied().filter(|n| {
                finite(*n)
                    && normals
                        .iter()
                        .all(|other| finite(*other) && norm(sub(*n, *other)) < 1e-9)
            });
            let vertex_ids = face_vertices
                .get(i)?
                .iter()
                .filter_map(|v| by_vertex.get(v).cloned())
                .collect();
            faces.push(NamedFace {
                id,
                triangles,
                boundary: boundary.get(i)?.clone(),
                normal,
                vertex_ids,
            });
        }
        (!faces.is_empty()).then_some(Self {
            element: entry.id,
            faces,
            edges,
            vertices,
        })
    }

    pub(crate) fn ids(&self) -> Vec<FeatureId> {
        self.faces
            .iter()
            .map(|f| f.id.clone())
            .chain(self.edges.iter().map(|e| e.id.clone()))
            .chain(self.vertices.iter().map(|(id, _)| id.clone()))
            .collect()
    }
    pub(crate) fn geometry(&self, id: &FeatureId) -> Option<FeatureGeometry> {
        if let Some(face) = self.faces.iter().find(|f| &f.id == id) {
            return Some(FeatureGeometry::Surface {
                normal: face.normal,
                triangles: face.triangles.clone(),
                vertex_ids: face.vertex_ids.clone(),
            });
        }
        if let Some(edge) = self.edges.iter().find(|e| &e.id == id) {
            return Some(FeatureGeometry::Edge {
                ends: edge.ends,
                vertex_ids: edge.vertex_ids.clone(),
            });
        }
        self.vertices
            .iter()
            .find(|(v, _)| v == id)
            .map(|(_, position)| FeatureGeometry::Vertex {
                position: *position,
            })
    }
}

fn vertex_name(adjacent: &BTreeSet<usize>, names: &[String]) -> Option<Option<String>> {
    if adjacent.len() != 3 {
        return Some(None);
    }
    let local: Vec<(&str, &str)> = adjacent
        .iter()
        .map(|f| names.get(*f)?.split_once(':'))
        .collect::<Option<_>>()?;
    let Some((family, _)) = local.first().copied() else {
        return Some(None);
    };
    if local.iter().any(|(kind, _)| *kind != family) {
        return Some(None);
    }
    if matches!(family, "plate" | "member") {
        let mut axes: Vec<&str> = local
            .iter()
            .filter_map(|(_, r)| r.strip_prefix("f."))
            .collect();
        axes.sort_unstable();
        let token = format!("{family}:v.{}", axes.join("."));
        let role = format!("v.{}", axes.join("."));
        return Some(VERTEX_ROLES.contains(&role.as_str()).then_some(token));
    }
    if !matches!(family, "extrusion" | "profile") {
        return Some(None);
    }
    let Some(level) = local
        .iter()
        .find_map(|(_, r)| r.strip_prefix("f.z").filter(|s| matches!(*s, "0" | "1")))
    else {
        return Some(None);
    };
    let walls: Vec<_> = local.iter().filter_map(|(_, r)| wall(r)).collect();
    let [a, b] = walls.as_slice() else {
        return Some(None);
    };
    if a.0 != b.0 {
        return Some(None);
    }
    let count = names
        .iter()
        .filter_map(|s| s.split_once(':'))
        .filter(|(f, _)| *f == family)
        .filter_map(|(_, r)| wall(r))
        .filter(|(c, _)| c == &a.0)
        .map(|(_, k)| k + 1)
        .max()?;
    let point = if (a.1 + 1) % count == b.1 {
        b.1
    } else if (b.1 + 1) % count == a.1 {
        a.1
    } else {
        return Some(None);
    };
    Some(Some(format!(
        "{family}:v.z{level}.{}",
        contour_point(&a.0, point)
    )))
}

fn edge_name(
    a: &FeatureId,
    b: &FeatureId,
    adjacent: &BTreeSet<usize>,
    names: &[String],
) -> Option<String> {
    if a.body() != b.body() {
        return None;
    }
    let family = a.body().as_str();
    if matches!(a.body(), BodyKind::Plate | BodyKind::Member) {
        let left: BTreeSet<_> = a.role().strip_prefix("v.")?.split('.').collect();
        let right: BTreeSet<_> = b.role().strip_prefix("v.")?.split('.').collect();
        let common: Vec<_> = left.intersection(&right).copied().collect();
        return (common.len() == 2).then(|| format!("{family}:e.{}", common.join(".")));
    }
    let (al, ap) = a.role().strip_prefix("v.")?.split_once('.')?;
    let (bl, bp) = b.role().strip_prefix("v.")?.split_once('.')?;
    if ap == bp && al != bl {
        return Some(format!("{family}:e.v.{ap}"));
    }
    if al != bl {
        return None;
    }
    for face in adjacent {
        let (f, r) = names.get(*face)?.split_once(':')?;
        if f == family
            && let Some((contour, k)) = wall(r)
        {
            return Some(format!("{family}:e.{al}.{}", contour_point(&contour, k)));
        }
    }
    None
}
