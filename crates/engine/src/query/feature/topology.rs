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

/// The six axis roles of a box body, without the face-table's `f.` prefix.
const AXES: [&str; 6] = ["x0", "x1", "y0", "y1", "z0", "z1"];

fn axis_role(token: &str) -> bool {
    AXES.contains(&token)
}

/// The `h<ordinal>` head of a cut wall token.
fn hole_ordinal(token: &str) -> Option<usize> {
    let n = token.strip_prefix('h')?;
    let value = n.parse::<usize>().ok()?;
    (value <= 999 && n == value.to_string()).then_some(value)
}

/// The `.w<crossing>` component of a member cutter's wall token.
fn hole_crossing(token: &str) -> Option<usize> {
    let k = token.strip_prefix('w')?;
    let n = k.parse::<usize>().ok()?;
    (n <= 999 && n.to_string() == k).then_some(n)
}

/// The `.s<side>` component of a band wall token.
fn hole_side(token: &str) -> Option<usize> {
    let j = token.strip_prefix('s')?;
    let n = j.parse::<usize>().ok()?;
    (n <= 1023 && n.to_string() == j).then_some(n)
}

/// The `.v<point>` cavity-point component of a corner edge token.
fn hole_point(token: &str) -> Option<usize> {
    let j = token.strip_prefix('v')?;
    let n = j.parse::<usize>().ok()?;
    (n <= 1023 && n.to_string() == j).then_some(n)
}

/// A canonical decimal followed by the rest of the token from the next dot on.
fn leading_decimal(text: &str, max: usize) -> Option<(usize, &str)> {
    let end = text.find('.').unwrap_or(text.len());
    Some((decimal(text.get(..end)?, max)?, text.get(end..)?))
}

/// The parsed wall-band role: ordinal, crossing, side, run.
pub(super) type CutBand = (usize, Option<usize>, Option<usize>, Option<usize>);

/// The wall-band role of a prism cut: `f.h<ordinal>[.w<crossing>].s<side>[.p<run>]`, the
/// grammar the certified cutters name their walls with. A component out of place, a run
/// suffix without a side, or a crossing without a side is not a band role.
pub(super) fn cut_band_role(role: &str) -> Option<CutBand> {
    let (ordinal, rest) = leading_decimal(role.strip_prefix("f.h")?, 999)?;
    let (crossing, rest) = match rest.strip_prefix(".w") {
        Some(rest) => {
            let (k, rest) = leading_decimal(rest, 999)?;
            (Some(k), rest)
        }
        None => (None, rest),
    };
    let (side, rest) = match rest.strip_prefix(".s") {
        Some(rest) => {
            let (j, rest) = leading_decimal(rest, 1023)?;
            (Some(j), rest)
        }
        None => (None, rest),
    };
    let (run, rest) = match rest.strip_prefix(".p") {
        Some(rest) => {
            let (r, rest) = leading_decimal(rest, 1023)?;
            (Some(r), rest)
        }
        None => (None, rest),
    };
    if !rest.is_empty() {
        return None;
    }
    (side.is_some() || (run.is_none() && crossing.is_none()))
        .then_some((ordinal, crossing, side, run))
}

/// The rim vertex of a prism cut opening: `h<ordinal>[.w<crossing>].<axis>.<point>` — the
/// body axis face the opening's rim lies on and the cavity point the vertex closes.
fn cut_rim_vertex(text: &str) -> bool {
    let parts: Vec<&str> = text.split('.').collect();
    let Some((&head, rest)) = parts.split_first() else {
        return false;
    };
    if hole_ordinal(head).is_none() {
        return false;
    }
    match rest {
        [axis, point] => axis_role(axis) && decimal(point, 1023).is_some(),
        [crossing, axis, point] => {
            hole_crossing(crossing).is_some() && axis_role(axis) && decimal(point, 1023).is_some()
        }
        _ => false,
    }
}

/// A named cut edge: the rim segment `h<ordinal>[.w<crossing>].s<side>.<axis>` of one band
/// wall on one body face, or the cut corner `h<ordinal>[.w<crossing>].v<point>` between
/// the two band walls of one crossing at one cavity point.
fn cut_edge(text: &str) -> bool {
    let parts: Vec<&str> = text.split('.').collect();
    let Some((&head, rest)) = parts.split_first() else {
        return false;
    };
    if hole_ordinal(head).is_none() {
        return false;
    }
    match rest {
        [point] => hole_point(point).is_some(),
        [first, second] if first.starts_with('w') => {
            hole_crossing(first).is_some() && hole_point(second).is_some()
        }
        [side, axis] => hole_side(side).is_some() && axis_role(axis),
        [crossing, side, axis] => {
            hole_crossing(crossing).is_some() && hole_side(side).is_some() && axis_role(axis)
        }
        _ => false,
    }
}

/// The local id of a named cut feature when it parses as `kind` and fits the 64-byte
/// qualified bound (`fid1:` + a canonical 36-character ElementId + `:` = 42 bytes before
/// the local id). Any other spelling stays unnamed; a name is never repaired and never
/// poisons the whole body.
fn named_cut_local(token: &str, kind: FeatureKind) -> Option<String> {
    let local = format!("cut:{token}");
    (parse_role(BodyKind::Cut, token) == Some(kind)
        && MAX_FEATURE_ID_BYTES > local.len() + FEATURE_RULE.len() + 1 + 36)
        .then_some(local)
}

/// The parsed rim-vertex role of a validated cut vertex id: ordinal, crossing, the body
/// axis the rim lies on, and the cavity point. The role keeps its `v.` head.
fn rim_parts(role: &str) -> Option<(usize, Option<usize>, String, usize)> {
    let parts: Vec<&str> = role.strip_prefix("v.")?.split('.').collect();
    let (&head, rest) = parts.split_first()?;
    let ordinal = hole_ordinal(head)?;
    match rest {
        [axis, point] => Some((ordinal, None, (*axis).to_owned(), point.parse().ok()?)),
        [crossing, axis, point] => Some((
            ordinal,
            hole_crossing(crossing),
            (*axis).to_owned(),
            point.parse().ok()?,
        )),
        _ => None,
    }
}

/// How many side walls one opening (ordinal, crossing) has in the face table: the cavity
/// point count the cyclic side naming needs.
fn band_side_count(names: &[String], ordinal: usize, crossing: Option<usize>) -> Option<usize> {
    let mut count = 0usize;
    for name in names {
        let (family, role) = name.split_once(':')?;
        if family == "cut"
            && let Some((n, w, Some(_), _)) = cut_band_role(role)
            && n == ordinal
            && w == crossing
        {
            count += 1;
        }
    }
    (count > 0).then_some(count)
}

pub(super) fn parse_role(body: BodyKind, token: &str) -> Option<FeatureKind> {
    if body == BodyKind::Cut {
        if let Some(corner) = token.strip_prefix("v.") {
            if cut_corner(corner) {
                return Some(FeatureKind::Vertex);
            }
            return cut_rim_vertex(corner).then_some(FeatureKind::Vertex);
        }
        if let Some(edge) = token.strip_prefix("e.") {
            return cut_edge(edge).then_some(FeatureKind::Edge);
        }
        let face = token.strip_prefix("f.")?;
        if let Some(p) = face.strip_prefix('p') {
            return decimal(p, 999).map(|_| FeatureKind::Face);
        }
        // The hole walls: `cut:f.h<ordinal>` (one curved wall), the F06 amendment's
        // `cut:f.h<ordinal>.s<side>` (the plate cutter) and the G14.2 member cutter's
        // `cut:f.h<ordinal>.w<crossing>.s<side>` with a `.p<run>` suffix past the first
        // planar run of a side.
        let hole = face.strip_prefix('h')?;
        let (hole, run) = match hole.split_once(".p") {
            Some((h, r)) => (h, Some(r)),
            None => (hole, None),
        };
        let valid = if let Some((h, side)) = hole.split_once(".s") {
            let (h, crossing) = match h.split_once(".w") {
                Some((h, w)) => (h, Some(w)),
                None => (h, None),
            };
            decimal(h, 999).is_some()
                && crossing.is_none_or(|w| decimal(w, 999).is_some())
                && decimal(side, 1023).is_some()
                && run.is_none_or(|r| decimal(r, 1023).is_some())
        } else {
            run.is_none() && decimal(hole, 999).is_some()
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

/// The F06 profile-features amendment's cap corner `p<ordinal>.<corner>`: the corner where the
/// cap of cut record `<ordinal>` crosses one longitudinal edge of the member, written as the two
/// member faces of that edge in axis order (`x0.y1`) or as the profile contour point of that
/// edge (`o3`, `c0.2`).
fn cut_corner(corner: &str) -> bool {
    let Some((ordinal, edge)) = corner.strip_prefix('p').and_then(|s| s.split_once('.')) else {
        return false;
    };
    decimal(ordinal, 999).is_some() && (point_token(edge) || member_edge(edge))
}

/// A longitudinal edge of a member box: one `x` side face and one `y` side face, in that order
/// (`x0.y1`). The member axis is local `z`, so a pair with `z0`/`z1` is a transverse edge of an
/// end face, where a cap corner stays unnamed.
fn member_edge(edge: &str) -> bool {
    edge.split_once('.')
        .is_some_and(|(a, b)| matches!(a, "x0" | "x1") && matches!(b, "y0" | "y1"))
}

/// The name of a cut-created vertex shared by faces of more than one family. A cap corner
/// (the cap-corner amendment) is a vertex of one cap and two faces of the body itself; a
/// rim vertex (this amendment) is a vertex of two band walls of one prism-cut opening and
/// one axis-aligned face of the body itself. Every other mixed vertex stays unnamed.
fn cut_vertex_name(local: &[(&str, &str)], names: &[String]) -> Option<String> {
    if let Some(caps) = local
        .iter()
        .filter(|(family, _)| *family == "cut")
        .map(|(_, role)| role.strip_prefix("f.p"))
        .collect::<Option<Vec<_>>>()
    {
        return cut_cap_corner_name(&caps, local, names);
    }
    // The rim: two band walls of one opening and one axis-aligned face of the body itself.
    // The opening must sit strictly inside that face, so the vertex sees no other face.
    let mut bands = Vec::new();
    let mut axis = None;
    for (family, role) in local {
        if *family == "cut" {
            bands.push(cut_band_role(role)?);
        } else if matches!(*family, "plate" | "member") {
            if axis.replace(role.strip_prefix("f.")?).is_some() {
                return None;
            }
        } else {
            return None;
        }
    }
    let [(n, crossing, sa, run_a), (m_ord, crossing_b, sb, run_b)] = bands.as_slice() else {
        return None;
    };
    let (Some(sa), Some(sb)) = (*sa, *sb) else {
        return None;
    };
    if *n != *m_ord || crossing != crossing_b || run_a.is_some() || run_b.is_some() || sa == sb {
        return None;
    }
    let axis = axis?;
    if !AXES.contains(&axis) {
        return None;
    }
    // The vertex closes exactly one side's start: the side after it in the cavity's
    // clockwise order. The other side runs from this vertex to the next corner.
    let sides = band_side_count(names, *n, *crossing)?;
    let point = if (sa + 1) % sides == sb {
        sb
    } else if (sb + 1) % sides == sa {
        sa
    } else {
        return None;
    };
    let token = match crossing {
        Some(k) => format!("v.h{n}.w{k}.{axis}.{point}"),
        None => format!("v.h{n}.{axis}.{point}"),
    };
    named_cut_local(&token, FeatureKind::Vertex)
}

/// The cap corner name over the already collected cap roles (`cut:v.p<cap>.<edge>`).
fn cut_cap_corner_name(caps: &[&str], local: &[(&str, &str)], names: &[String]) -> Option<String> {
    let [cap] = caps else {
        return None;
    };
    let sides: Vec<(&str, &str)> = local
        .iter()
        .copied()
        .filter(|(family, _)| *family != "cut")
        .collect();
    let [(family, a), (other, b)] = sides.as_slice() else {
        return None;
    };
    if family != other {
        return None;
    }
    let edge = if *family == "member" {
        let mut axes = [a.strip_prefix("f.")?, b.strip_prefix("f.")?];
        axes.sort_unstable();
        axes.join(".")
    } else if *family == "profile" {
        shared_point(family, &wall(a)?, &wall(b)?, names).flatten()?
    } else {
        return None;
    };
    let token = format!("v.p{cap}.{edge}");
    cut_corner(token.strip_prefix("v.")?).then(|| format!("cut:{token}"))
}

/// The contour point token shared by two walls `a` and `b` of one contour of `family`: the
/// start of whichever wall follows the other. `Some(None)` when the walls are not adjacent on
/// one contour; `None` when the face table holds no wall of that contour at all.
fn shared_point(
    family: &str,
    a: &(String, usize),
    b: &(String, usize),
    names: &[String],
) -> Option<Option<String>> {
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
    Some(Some(contour_point(&a.0, point)))
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
        let mut ambiguous = BTreeSet::new();
        for (i, adjacent) in incident.iter().enumerate() {
            let local = vertex_name(adjacent, &names)?;
            if let Some(local) = local {
                let id = FeatureId::from_local(entry.id, &local).ok()?;
                // Ambiguous names are never repaired by selecting a mesh index. A cap corner that
                // occurs twice (a cap crossing one member edge more than once) is left out on its
                // own; any other repeated name refuses the whole body as before.
                if ambiguous.contains(&id) {
                    continue;
                }
                if named_positions
                    .insert(id.clone(), *positions.get(i)?)
                    .is_some()
                {
                    if id.body() != BodyKind::Cut {
                        return None;
                    }
                    named_positions.remove(&id);
                    by_vertex.retain(|_, named| named != &id);
                    ambiguous.insert(id);
                    continue;
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
        return Some(cut_vertex_name(&local, names));
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
    let Some(point) = shared_point(family, a, b, names)? else {
        return Some(None);
    };
    Some(Some(format!("{family}:v.z{level}.{point}")))
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
    // Cut ids name the edges of a prism cut's walls between named rim vertices; the cap's
    // own edges and every other cut pair stay unnamed (the cap-corner amendment).
    if a.body() == BodyKind::Cut {
        return cut_edge_name(a, b, adjacent, names);
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

/// The name of the edge between two named rim vertices of one prism cut opening. The rim
/// edge of side `j` runs on the body face the opening sits on, between that face and the
/// band wall `s<j>`; the corner edge at cavity point `j` runs along the cut between the
/// two band walls of one crossing. Rim and corner ids follow the walls' own `cut:f.h`
/// vocabulary (`cut:e.h<n>[.w<k>].s<j>.<axis>`, `cut:e.h<n>[.w<k>].v<j>`); a cap edge, a
/// mixed pair or anything the walls do not certify stays unnamed.
fn cut_edge_name(
    a: &FeatureId,
    b: &FeatureId,
    adjacent: &BTreeSet<usize>,
    names: &[String],
) -> Option<String> {
    let (va, vb) = (rim_parts(a.role())?, rim_parts(b.role())?);
    if va.0 != vb.0 || va.1 != vb.1 {
        return None;
    }
    let mut bands = Vec::new();
    let mut axis = None;
    for face in adjacent {
        let (family, role) = names.get(*face)?.split_once(':')?;
        if family == "cut" {
            bands.push(cut_band_role(role)?);
        } else if matches!(family, "plate" | "member") && axis.is_none() {
            axis = Some(role.strip_prefix("f.")?);
        } else {
            return None;
        }
    }
    if !matches!(adjacent.len(), 2) {
        return None;
    }
    match bands.as_slice() {
        // The rim edge of side `j`: the band wall and the axis face its opening sits on.
        [(n, crossing, Some(side), None)] => {
            let axis = axis?;
            if !AXES.contains(&axis)
                || *n != va.0
                || crossing != &va.1
                || va.2 != axis
                || vb.2 != axis
            {
                return None;
            }
            let token = match crossing {
                Some(k) => format!("e.h{n}.w{k}.s{side}.{axis}"),
                None => format!("e.h{n}.s{side}.{axis}"),
            };
            named_cut_local(&token, FeatureKind::Edge)
        }
        // The corner edge at cavity point `j`: the two band walls of one crossing.
        [
            (n, crossing, Some(sx), None),
            (m, crossing_y, Some(sy), None),
        ] => {
            let (sx, sy) = (*sx, *sy);
            if n != m
                || crossing != crossing_y
                || *n != va.0
                || crossing != &va.1
                || va.2 == vb.2
                || va.3 != vb.3
            {
                return None;
            }
            let sides = band_side_count(names, *n, *crossing)?;
            let point = if (sx + 1) % sides == sy {
                sy
            } else if (sy + 1) % sides == sx {
                sx
            } else {
                return None;
            };
            if va.3 != point {
                return None;
            }
            let token = match crossing {
                Some(k) => format!("e.h{n}.w{k}.v{point}"),
                None => format!("e.h{n}.v{point}"),
            };
            named_cut_local(&token, FeatureKind::Edge)
        }
        _ => None,
    }
}
