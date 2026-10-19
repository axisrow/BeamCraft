// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A08 numbering preview: groups equivalent parts and assemblies and proposes position numbers.
//!
//! The preview is a pure function of the facts the adapter reads from a committed revision. It
//! assigns nothing and stores nothing: approving numbers is A09. Rules (own wording, from the
//! numbering requirements 24.1 to 24.4 and the observation that identical parts share one
//! number):
//!
//! * Parts are identical when kind, pinned profile, pinned material, dimensions (within the
//!   tolerance) and holes (compared in the part's own frame, within the tolerance) agree. The
//!   name is compared only when asked. Placement, level and assembly membership never matter.
//! * The oldest part of a group (the smallest, i.e. earliest, ElementId) is its reference; groups
//!   are numbered in the order of their references, so adding a copy of an existing part never
//!   renumbers anything.
//! * Each part kind has a series (prefix and start number); the position of a group is the
//!   prefix followed by `start + index`. Assemblies are grouped by the multiset of the groups of
//!   all parts they hold (nested assemblies included) and marked `prefix-number`.
//! * If two groups would get the same position, only the first (in series order plate, beam,
//!   column, assembly) is numbered; the other is `overlapping` and its parts stay unnumbered.
//! * The preview token covers every fact that influences a position, so it changes exactly when
//!   an edit would change the proposed numbering (a thickness, a material, a hole, a new or
//!   deleted part) and not for a move or, unless the name is compared, a rename.

use beamcraft_model::digest::{ContentDigest, Sha256};
use beamcraft_model::id::ElementId;
use std::collections::{BTreeMap, BTreeSet};

/// Parts and assemblies the preview handles at most.
pub const MAX_ITEMS: usize = 65_536;
/// Longest series prefix in characters.
pub const MAX_PREFIX: usize = 16;
/// Largest series start number.
pub const MAX_START: u64 = 1_000_000_000;
/// Largest tolerance in millimetres.
pub const MAX_TOLERANCE_MM: f64 = 1000.0;
/// Deepest assembly nesting the walk follows.
const MAX_DEPTH: usize = 64;
/// Angles (member roll) closer than this share a number, in radians.
pub const ANGLE_TOLERANCE_RAD: f64 = 1e-9;
/// The most part comparisons one preview makes. Parts are looked up by kind, profile, material
/// and the range of their first dimension, so ordinary models make a handful per part; a model
/// built to defeat that index is refused instead of taking unbounded time.
pub const MAX_COMPARISONS: u64 = 50_000_000;

/// The kind of a numbered part.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PartKind {
    /// A plate.
    Plate,
    /// A beam.
    Beam,
    /// A column.
    Column,
}

impl PartKind {
    /// The wire token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Plate => "plate",
            Self::Beam => "beam",
            Self::Column => "column",
        }
    }
}

/// What the numbering needs to know about one part.
#[derive(Debug, Clone, PartialEq)]
pub struct PartFact {
    /// Identity.
    pub id: ElementId,
    /// Kind.
    pub kind: PartKind,
    /// Label, compared only on request.
    pub name: String,
    /// Pinned profile (members).
    pub profile: Option<ContentDigest>,
    /// Pinned material.
    pub material: Option<ContentDigest>,
    /// Lengths in millimetres, in a fixed order per kind (plate: width, length, thickness;
    /// member: axis length), compared within the tolerance.
    pub dimensions: Vec<f64>,
    /// Angles in radians (member: roll), compared within [`ANGLE_TOLERANCE_RAD`], never within
    /// the length tolerance.
    pub angles_rad: Vec<f64>,
    /// Through-hole polygons in the part's own frame.
    pub holes: Vec<Vec<[f64; 2]>>,
    /// The assembly the part belongs to.
    pub assembly: Option<ElementId>,
}

/// What the numbering needs to know about one assembly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssemblyFact {
    /// Identity.
    pub id: ElementId,
    /// Label, compared only on request.
    pub name: String,
    /// The assembly it is nested in.
    pub parent: Option<ElementId>,
}

/// A numbering series: prefix and start number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Series {
    /// Position prefix. Part prefixes use letters, digits and `_`; an assembly prefix may also
    /// hold `-`.
    pub prefix: String,
    /// First number.
    pub start: u64,
}

/// The settings of one preview.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// Plate series.
    pub plate: Series,
    /// Beam series.
    pub beam: Series,
    /// Column series.
    pub column: Series,
    /// Assembly series (marks are `prefix-number`).
    pub assembly: Series,
    /// Dimensions and hole coordinates closer than this share a number.
    pub tolerance_mm: f64,
    /// Compare the names of parts and assemblies.
    pub compare_name: bool,
}

impl Default for Settings {
    fn default() -> Self {
        let series = |prefix: &str| Series {
            prefix: prefix.into(),
            start: 1,
        };
        Self {
            plate: series("PL"),
            beam: series("B"),
            column: series("C"),
            assembly: series("A"),
            // Sub-micron differences between lengths computed from different coordinates are
            // floating-point noise, not different parts (the F04 length floor is 0.001 mm).
            tolerance_mm: 0.001,
            compare_name: false,
        }
    }
}

/// Why a preview could not be made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NumberingError(pub &'static str);

impl core::fmt::Display for NumberingError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.0)
    }
}

impl core::error::Error for NumberingError {}

/// What a group is a group of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GroupKind {
    /// Parts of one kind.
    Part(PartKind),
    /// Assemblies.
    Assembly,
}

/// Status of a group (the stored-number statuses belong to A09).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Would receive its position.
    New,
    /// Its position is taken by an earlier group; stays unnumbered.
    Overlapping,
}

impl Status {
    /// The wire token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Overlapping => "overlapping",
        }
    }
}

/// One group of equivalent parts or assemblies.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    /// What it groups.
    pub kind: GroupKind,
    /// The number within its series.
    pub number: u64,
    /// The proposed position or mark.
    pub position: String,
    /// New or overlapping.
    pub status: Status,
    /// The position that took this one, for an overlapping group.
    pub overlaps_with: Option<String>,
    /// The oldest member.
    pub reference: ElementId,
    /// All members in ascending id order.
    pub members: Vec<ElementId>,
    /// The facts that make the members equivalent, as text.
    pub description: String,
}

/// The preview.
#[derive(Debug, Clone, PartialEq)]
pub struct Preview {
    /// Part groups then assembly groups, each in the order of their references.
    pub groups: Vec<Group>,
    /// Digest of everything that influences the proposed numbering.
    pub token: ContentDigest,
}

fn float_key(v: f64) -> String {
    format!("{:016x}", (v + 0.0).to_bits())
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    if tol == 0.0 {
        a == b
    } else {
        (a - b).abs() <= tol
    }
}

/// A hole polygon in a canonical form that keeps its edges: counter-clockwise, starting at its
/// smallest vertex. Two rings with the same vertices in a different order are different holes.
fn canonical_polygon(points: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let mut ring = points.to_vec();
    let twice_area: f64 = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .map(|(p, q)| p[0] * q[1] - q[0] * p[1])
        .sum();
    if twice_area < 0.0 {
        ring.reverse();
    }
    let start = ring
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal))
        .map_or(0, |(i, _)| i);
    ring.rotate_left(start);
    ring
}

fn canonical_holes(holes: &[Vec<[f64; 2]>]) -> Vec<Vec<[f64; 2]>> {
    let mut out: Vec<Vec<[f64; 2]>> = holes.iter().map(|h| canonical_polygon(h)).collect();
    out.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    out
}

/// An order-preserving key of a finite float, for range lookups.
fn sortable(v: f64) -> u64 {
    let bits = (v + 0.0).to_bits();
    if bits >> 63 == 1 {
        !bits
    } else {
        bits | (1 << 63)
    }
}

fn same_part(
    a: &PartFact,
    b: &PartFact,
    holes_a: &[Vec<[f64; 2]>],
    holes_b: &[Vec<[f64; 2]>],
    s: &Settings,
) -> bool {
    a.kind == b.kind
        && a.profile == b.profile
        && a.material == b.material
        && (!s.compare_name || a.name == b.name)
        && a.dimensions.len() == b.dimensions.len()
        && a.dimensions
            .iter()
            .zip(&b.dimensions)
            .all(|(x, y)| close(*x, *y, s.tolerance_mm))
        && a.angles_rad.len() == b.angles_rad.len()
        && a.angles_rad
            .iter()
            .zip(&b.angles_rad)
            .all(|(x, y)| close(*x, *y, ANGLE_TOLERANCE_RAD))
        && holes_a.len() == holes_b.len()
        && holes_a.iter().zip(holes_b).all(|(p, q)| {
            p.len() == q.len()
                && p.iter().zip(q).all(|(u, v)| {
                    close(u[0], v[0], s.tolerance_mm) && close(u[1], v[1], s.tolerance_mm)
                })
        })
}

fn check_series(series: &Series, assembly: bool) -> Result<(), NumberingError> {
    let allowed = |c: char| c.is_ascii_alphanumeric() || c == '_' || (assembly && c == '-');
    if series.prefix.chars().count() > MAX_PREFIX || !series.prefix.chars().all(allowed) {
        return Err(NumberingError(
            "a series prefix is too long or has a character outside letters, digits and underscore",
        ));
    }
    if series.start > MAX_START {
        return Err(NumberingError("a series start number is above the bound"));
    }
    Ok(())
}

/// Computes the preview. Nothing is read from or written to a store.
pub fn preview(
    parts: &[PartFact],
    assemblies: &[AssemblyFact],
    settings: &Settings,
) -> Result<Preview, NumberingError> {
    if parts.len() > MAX_ITEMS || assemblies.len() > MAX_ITEMS {
        return Err(NumberingError("too many parts or assemblies"));
    }
    for (series, assembly) in [
        (&settings.plate, false),
        (&settings.beam, false),
        (&settings.column, false),
        (&settings.assembly, true),
    ] {
        check_series(series, assembly)?;
    }
    if !settings.tolerance_mm.is_finite()
        || settings.tolerance_mm < 0.0
        || settings.tolerance_mm > MAX_TOLERANCE_MM
    {
        return Err(NumberingError("the tolerance is outside 0 to 1000 mm"));
    }
    let mut ordered: Vec<&PartFact> = parts.iter().collect();
    ordered.sort_by_key(|p| p.id);
    if ordered
        .windows(2)
        .any(|w| matches!(w, [a, b] if a.id == b.id))
    {
        return Err(NumberingError("an element id appears twice"));
    }
    if parts.iter().any(|p| {
        p.dimensions
            .iter()
            .chain(&p.angles_rad)
            .any(|v| !v.is_finite())
            || p.holes
                .iter()
                .flatten()
                .any(|pt| !pt[0].is_finite() || !pt[1].is_finite())
    }) {
        return Err(NumberingError("a dimension is not finite"));
    }
    // Part groups: in id order, a part joins the earliest group whose reference it matches.
    // Candidates come from an index on the exact identity fields and the range of the first
    // dimension, so the work does not grow with the number of unrelated groups.
    struct Building<'a> {
        reference: &'a PartFact,
        holes: Vec<Vec<[f64; 2]>>,
        members: Vec<ElementId>,
    }
    type Coarse = (
        PartKind,
        Option<ContentDigest>,
        Option<ContentDigest>,
        Option<String>,
        usize,
        usize,
        usize,
    );
    let mut buildings: Vec<Building<'_>> = Vec::new();
    let mut index: BTreeMap<Coarse, BTreeMap<u64, Vec<usize>>> = BTreeMap::new();
    let mut comparisons: u64 = 0;
    let mut group_of: BTreeMap<ElementId, usize> = BTreeMap::new();
    for part in &ordered {
        let holes = canonical_holes(&part.holes);
        let coarse: Coarse = (
            part.kind,
            part.profile,
            part.material,
            settings.compare_name.then(|| part.name.clone()),
            part.dimensions.len(),
            part.angles_rad.len(),
            holes.len(),
        );
        let first = part.dimensions.first().copied().unwrap_or(0.0);
        let range =
            sortable(first - settings.tolerance_mm)..=sortable(first + settings.tolerance_mm);
        let mut candidates: Vec<usize> = index
            .get(&coarse)
            .map(|by_first| {
                by_first
                    .range(range)
                    .flat_map(|(_, g)| g.iter().copied())
                    .collect()
            })
            .unwrap_or_default();
        candidates.sort_unstable();
        let mut found = None;
        for candidate in candidates {
            comparisons += 1;
            if comparisons > MAX_COMPARISONS {
                return Err(NumberingError(
                    "the comparison work exceeds the bound; use a smaller tolerance",
                ));
            }
            if buildings
                .get(candidate)
                .is_some_and(|g| same_part(g.reference, part, &g.holes, &holes, settings))
            {
                found = Some(candidate);
                break;
            }
        }
        let at = match found {
            Some(i) => i,
            None => {
                buildings.push(Building {
                    reference: part,
                    holes,
                    members: Vec::new(),
                });
                let i = buildings.len() - 1;
                index
                    .entry(coarse)
                    .or_default()
                    .entry(sortable(first))
                    .or_default()
                    .push(i);
                i
            }
        };
        if let Some(group) = buildings.get_mut(at) {
            group.members.push(part.id);
        }
        group_of.insert(part.id, at);
    }
    let mut groups: Vec<Group> = Vec::new();
    let mut counters: BTreeMap<PartKind, u64> = BTreeMap::new();
    for building in &buildings {
        let kind = building.reference.kind;
        let series = match kind {
            PartKind::Plate => &settings.plate,
            PartKind::Beam => &settings.beam,
            PartKind::Column => &settings.column,
        };
        let counter = counters.entry(kind).or_insert(0);
        let number = series
            .start
            .checked_add(*counter)
            .ok_or(NumberingError("a position number overflows"))?;
        *counter += 1;
        groups.push(Group {
            kind: GroupKind::Part(kind),
            number,
            position: format!("{}{}", series.prefix, number),
            status: Status::New,
            overlaps_with: None,
            reference: building.reference.id,
            members: building.members.clone(),
            description: describe(building.reference, settings),
        });
    }
    // Series order: plates, beams, columns (each already in reference order), then assemblies.
    groups.sort_by_key(|g| (g.kind, g.reference));
    // The group number of each part, for the assembly signatures.
    let part_group: BTreeMap<ElementId, ElementId> = group_of
        .iter()
        .filter_map(|(id, idx)| Some((*id, buildings.get(*idx)?.reference.id)))
        .collect();

    // Assemblies: signature = sorted references of the groups of every descendant part, plus the
    // name when compared.
    let parent: BTreeMap<ElementId, Option<ElementId>> =
        assemblies.iter().map(|a| (a.id, a.parent)).collect();
    if parent.len() != assemblies.len() {
        return Err(NumberingError("an assembly id appears twice"));
    }
    let mut signatures: BTreeMap<ElementId, Vec<ElementId>> =
        assemblies.iter().map(|a| (a.id, Vec::new())).collect();
    let mut add_to_chain =
        |start: Option<ElementId>, reference: ElementId| -> Result<(), NumberingError> {
            let mut at = start;
            let mut depth = 0;
            while let Some(id) = at {
                depth += 1;
                if depth > MAX_DEPTH {
                    return Err(NumberingError(
                        "assemblies are nested deeper than the bound",
                    ));
                }
                match signatures.get_mut(&id) {
                    Some(list) => list.push(reference),
                    None => return Ok(()),
                }
                at = parent.get(&id).copied().flatten();
            }
            Ok(())
        };
    for part in &ordered {
        if let (Some(reference), Some(_)) = (part_group.get(&part.id), part.assembly) {
            add_to_chain(part.assembly, *reference)?;
        }
    }
    let mut sorted_assemblies: Vec<&AssemblyFact> = assemblies.iter().collect();
    sorted_assemblies.sort_by_key(|a| a.id);
    let mut assembly_groups: Vec<(ElementId, Vec<ElementId>, String)> = Vec::new();
    let mut by_signature: BTreeMap<ContentDigest, usize> = BTreeMap::new();
    for assembly in sorted_assemblies {
        let mut references = signatures.remove(&assembly.id).unwrap_or_default();
        references.sort();
        let mut key = references
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");
        if settings.compare_name {
            key = format!("{}|{key}", assembly.name);
        }
        // The signature of a large assembly is long; its digest is the key.
        let signature = Sha256::digest(key.as_bytes())
            .map_err(|_| NumberingError("an assembly signature is too long"))?;
        match by_signature.get(&signature) {
            Some(&at) => {
                if let Some(group) = assembly_groups.get_mut(at) {
                    group.1.push(assembly.id);
                }
            }
            None => {
                by_signature.insert(signature, assembly_groups.len());
                assembly_groups.push((
                    assembly.id,
                    vec![assembly.id],
                    format!("{} parts", references.len()),
                ));
            }
        }
    }
    for (index, (reference, members, description)) in assembly_groups.into_iter().enumerate() {
        let number = settings
            .assembly
            .start
            .checked_add(index as u64)
            .ok_or(NumberingError("a position number overflows"))?;
        groups.push(Group {
            kind: GroupKind::Assembly,
            number,
            position: format!("{}-{}", settings.assembly.prefix, number),
            status: Status::New,
            overlaps_with: None,
            reference,
            members,
            description,
        });
    }
    // Overlaps: the first group to claim a position keeps it.
    let mut taken: BTreeMap<String, ()> = BTreeMap::new();
    for group in &mut groups {
        if taken.insert(group.position.clone(), ()).is_some() {
            group.status = Status::Overlapping;
            group.overlaps_with = Some(group.position.clone());
        }
    }
    let token = token_of(&groups, settings);
    Ok(Preview { groups, token })
}

fn describe(part: &PartFact, s: &Settings) -> String {
    let dims: Vec<String> = part.dimensions.iter().map(|d| format!("{d}")).collect();
    let mut text = format!("dimensions {}", dims.join(" x "));
    if part.angles_rad.iter().any(|a| *a != 0.0) {
        let angles: Vec<String> = part.angles_rad.iter().map(|a| format!("{a}")).collect();
        text.push_str(&format!(", roll {} rad", angles.join(" ")));
    }
    if !part.holes.is_empty() {
        text.push_str(&format!(", {} holes", part.holes.len()));
    }
    if s.compare_name {
        text.push_str(&format!(", name {}", part.name));
    }
    text
}

fn token_of(groups: &[Group], s: &Settings) -> ContentDigest {
    let mut text = String::from("BCNUM1\n");
    for (name, series) in [
        ("plate", &s.plate),
        ("beam", &s.beam),
        ("column", &s.column),
        ("assembly", &s.assembly),
    ] {
        text.push_str(&format!(
            "series {name} {} {}\n",
            series.prefix, series.start
        ));
    }
    text.push_str(&format!(
        "tolerance {} name {}\n",
        float_key(s.tolerance_mm),
        s.compare_name
    ));
    for group in groups {
        text.push_str(&format!(
            "group {} {} {}",
            group.position,
            group.status.as_str(),
            group.reference
        ));
        for member in &group.members {
            text.push_str(&format!(" {member}"));
        }
        text.push('\n');
    }
    Sha256::digest(text.as_bytes()).unwrap_or_else(|_| ContentDigest::from_bytes([0; 32]))
}

/// The ids that do not receive a position: members of overlapping groups.
pub fn unnumbered(preview: &Preview) -> BTreeSet<ElementId> {
    preview
        .groups
        .iter()
        .filter(|g| g.status == Status::Overlapping)
        .flat_map(|g| g.members.iter().copied())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u16) -> ElementId {
        ElementId::parse(&format!("019a7bf0-0000-7000-8000-{n:012x}")).unwrap()
    }

    fn digest(seed: &str) -> ContentDigest {
        Sha256::digest(seed.as_bytes()).unwrap()
    }

    fn plate(n: u16, w: f64, l: f64, t: f64) -> PartFact {
        PartFact {
            id: id(n),
            kind: PartKind::Plate,
            name: format!("P{n}"),
            profile: None,
            material: Some(digest("s355")),
            dimensions: vec![w, l, t],
            angles_rad: Vec::new(),
            holes: Vec::new(),
            assembly: None,
        }
    }

    fn beam(n: u16, profile: &str, length: f64) -> PartFact {
        PartFact {
            id: id(n),
            kind: PartKind::Beam,
            name: format!("B{n}"),
            profile: Some(digest(profile)),
            material: Some(digest("s355")),
            dimensions: vec![length],
            angles_rad: vec![0.0],
            holes: Vec::new(),
            assembly: None,
        }
    }

    fn positions(p: &Preview) -> Vec<(String, usize)> {
        p.groups
            .iter()
            .map(|g| (g.position.clone(), g.members.len()))
            .collect()
    }

    #[test]
    fn identical_parts_share_one_number_and_the_oldest_is_the_reference() {
        let parts = [
            plate(3, 400.0, 300.0, 20.0),
            plate(1, 400.0, 300.0, 20.0),
            plate(2, 500.0, 300.0, 20.0),
        ];
        let p = preview(&parts, &[], &Settings::default()).unwrap();
        assert_eq!(positions(&p), [("PL1".into(), 2), ("PL2".into(), 1)]);
        assert_eq!(p.groups[0].reference, id(1));
        assert_eq!(p.groups[0].members, [id(1), id(3)]);
    }

    #[test]
    fn a_different_material_profile_or_length_splits_the_group() {
        let mut other_material = beam(2, "ipe200", 3000.0);
        other_material.material = Some(digest("s275"));
        let parts = [
            beam(1, "ipe200", 3000.0),
            other_material,
            beam(3, "ipe240", 3000.0),
            beam(4, "ipe200", 3500.0),
            beam(5, "ipe200", 3000.0),
        ];
        let p = preview(&parts, &[], &Settings::default()).unwrap();
        assert_eq!(
            positions(&p),
            [
                ("B1".into(), 2),
                ("B2".into(), 1),
                ("B3".into(), 1),
                ("B4".into(), 1)
            ]
        );
    }

    #[test]
    fn holes_split_groups_whatever_the_polygon_order() {
        let square = |x: f64| vec![[x, 0.0], [x + 10.0, 0.0], [x + 10.0, 10.0], [x, 10.0]];
        let mut a = plate(1, 400.0, 300.0, 20.0);
        let mut b = plate(2, 400.0, 300.0, 20.0);
        let mut c = plate(3, 400.0, 300.0, 20.0);
        let d = plate(4, 400.0, 300.0, 20.0);
        a.holes = vec![square(100.0), square(200.0)];
        b.holes = vec![square(200.0), square(100.0).into_iter().rev().collect()];
        c.holes = vec![square(100.0)];
        let p = preview(&[a, b, c, d], &[], &Settings::default()).unwrap();
        assert_eq!(
            positions(&p),
            [("PL1".into(), 2), ("PL2".into(), 1), ("PL3".into(), 1)]
        );
    }

    #[test]
    fn the_tolerance_merges_close_dimensions_only() {
        let parts = [
            plate(1, 400.0, 300.0, 20.0),
            plate(2, 400.04, 300.0, 20.0),
            plate(3, 400.2, 300.0, 20.0),
        ];
        let exact = preview(&parts, &[], &Settings::default()).unwrap();
        assert_eq!(exact.groups.len(), 3);
        let loose = Settings {
            tolerance_mm: 0.05,
            ..Settings::default()
        };
        let p = preview(&parts, &[], &loose).unwrap();
        assert_eq!(positions(&p), [("PL1".into(), 2), ("PL2".into(), 1)]);
    }

    #[test]
    fn the_name_matters_only_when_compared() {
        let parts = [plate(1, 400.0, 300.0, 20.0), plate(2, 400.0, 300.0, 20.0)];
        assert_eq!(
            preview(&parts, &[], &Settings::default())
                .unwrap()
                .groups
                .len(),
            1
        );
        let named = Settings {
            compare_name: true,
            ..Settings::default()
        };
        assert_eq!(preview(&parts, &[], &named).unwrap().groups.len(), 2);
    }

    #[test]
    fn series_overlap_leaves_the_later_group_unnumbered() {
        let parts = [
            plate(1, 1.0, 1.0, 1.0),
            plate(2, 2.0, 1.0, 1.0),
            beam(3, "a", 1.0),
            beam(4, "b", 1.0),
        ];
        let settings = Settings {
            plate: Series {
                prefix: "P".into(),
                start: 1,
            },
            beam: Series {
                prefix: "P".into(),
                start: 2,
            },
            ..Settings::default()
        };
        let p = preview(&parts, &[], &settings).unwrap();
        let status: Vec<_> = p
            .groups
            .iter()
            .map(|g| (g.position.as_str(), g.status))
            .collect();
        assert_eq!(
            status,
            [
                ("P1", Status::New),
                ("P2", Status::New),
                ("P2", Status::Overlapping),
                ("P3", Status::New)
            ]
        );
        assert_eq!(p.groups[2].overlaps_with.as_deref(), Some("P2"));
        assert_eq!(unnumbered(&p), BTreeSet::from([id(3)]));
    }

    #[test]
    fn assemblies_with_the_same_content_share_a_mark() {
        let mut a1 = beam(1, "ipe200", 3000.0);
        let mut a2 = plate(2, 400.0, 300.0, 20.0);
        let mut b1 = beam(3, "ipe200", 3000.0);
        let mut b2 = plate(4, 400.0, 300.0, 20.0);
        let mut c1 = beam(5, "ipe200", 3000.0);
        a1.assembly = Some(id(101));
        a2.assembly = Some(id(101));
        b1.assembly = Some(id(102));
        b2.assembly = Some(id(102));
        c1.assembly = Some(id(103));
        let assemblies = [
            AssemblyFact {
                id: id(101),
                name: "A".into(),
                parent: None,
            },
            AssemblyFact {
                id: id(102),
                name: "B".into(),
                parent: None,
            },
            AssemblyFact {
                id: id(103),
                name: "C".into(),
                parent: None,
            },
        ];
        let p = preview(&[a1, a2, b1, b2, c1], &assemblies, &Settings::default()).unwrap();
        let marks: Vec<_> = p
            .groups
            .iter()
            .filter(|g| g.kind == GroupKind::Assembly)
            .map(|g| (g.position.clone(), g.members.len()))
            .collect();
        assert_eq!(marks, [("A-1".into(), 2), ("A-2".into(), 1)]);
    }

    #[test]
    fn nested_assemblies_count_their_descendants() {
        let mut part = beam(1, "ipe200", 3000.0);
        part.assembly = Some(id(102));
        let assemblies = [
            AssemblyFact {
                id: id(101),
                name: "outer".into(),
                parent: None,
            },
            AssemblyFact {
                id: id(102),
                name: "inner".into(),
                parent: Some(id(101)),
            },
        ];
        let p = preview(&[part], &assemblies, &Settings::default()).unwrap();
        // Both hold the same single part group, so they would share a mark: the outer one
        // is the older reference.
        let marks: Vec<_> = p
            .groups
            .iter()
            .filter(|g| g.kind == GroupKind::Assembly)
            .collect();
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].members, [id(101), id(102)]);
    }

    #[test]
    fn the_token_follows_the_numbering_and_nothing_else() {
        let base = vec![plate(1, 400.0, 300.0, 20.0), plate(2, 400.0, 300.0, 20.0)];
        let token = |parts: &[PartFact], s: &Settings| preview(parts, &[], s).unwrap().token;
        let s = Settings::default();
        let original = token(&base, &s);
        assert_eq!(original, token(&base, &s));
        // A rename is not an edit of the numbering.
        let mut renamed = base.clone();
        renamed[0].name = "other".into();
        assert_eq!(original, token(&renamed, &s));
        // A move is not either (placement is not a fact).
        // A thickness edit, a material edit, a hole, a new part and a deleted part are.
        let mut thicker = base.clone();
        thicker[1].dimensions[2] = 25.0;
        let mut other_material = base.clone();
        other_material[1].material = Some(digest("s275"));
        let mut holed = base.clone();
        holed[1].holes = vec![vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]]];
        let mut more = base.clone();
        more.push(plate(3, 400.0, 300.0, 20.0));
        let fewer = base[..1].to_vec();
        for changed in [thicker, other_material, holed, more, fewer] {
            assert_ne!(original, token(&changed, &s));
        }
        // Settings are part of the token; the name only when compared.
        let named = Settings {
            compare_name: true,
            ..Settings::default()
        };
        assert_ne!(original, token(&base, &named));
        // With the name compared, a rename that splits a group changes the numbering.
        let mut same = base.clone();
        same[0].name = "x".into();
        same[1].name = "x".into();
        let mut split = same.clone();
        split[0].name = "y".into();
        assert_ne!(token(&same, &named), token(&split, &named));
        assert_eq!(token(&same, &s), token(&split, &s));
    }

    #[test]
    fn invalid_settings_and_facts_are_refused() {
        let parts = [plate(1, 1.0, 1.0, 1.0)];
        let refused = |s: Settings| preview(&parts, &[], &s).is_err();
        assert!(refused(Settings {
            tolerance_mm: -1.0,
            ..Settings::default()
        }));
        assert!(refused(Settings {
            tolerance_mm: f64::NAN,
            ..Settings::default()
        }));
        assert!(refused(Settings {
            tolerance_mm: 1001.0,
            ..Settings::default()
        }));
        assert!(refused(Settings {
            plate: Series {
                prefix: "A-".into(),
                start: 1
            },
            ..Settings::default()
        }));
        assert!(refused(Settings {
            plate: Series {
                prefix: "x".repeat(17),
                start: 1
            },
            ..Settings::default()
        }));
        assert!(refused(Settings {
            plate: Series {
                prefix: "P".into(),
                start: MAX_START + 1
            },
            ..Settings::default()
        }));
        let mut nan = plate(1, f64::NAN, 1.0, 1.0);
        assert!(preview(&[nan.clone()], &[], &Settings::default()).is_err());
        nan.dimensions[0] = 1.0;
        assert!(preview(&[nan.clone(), nan], &[], &Settings::default()).is_err());
        // An assembly prefix may hold a hyphen.
        let ok = Settings {
            assembly: Series {
                prefix: "AS-".into(),
                start: 5,
            },
            ..Settings::default()
        };
        assert!(preview(&parts, &[], &ok).is_ok());
    }

    #[test]
    fn start_numbers_offset_every_series() {
        let parts = [plate(1, 1.0, 1.0, 1.0), plate(2, 2.0, 1.0, 1.0)];
        let s = Settings {
            plate: Series {
                prefix: String::new(),
                start: 1000,
            },
            ..Settings::default()
        };
        let p = preview(&parts, &[], &s).unwrap();
        assert_eq!(positions(&p), [("1000".into(), 1), ("1001".into(), 1)]);
    }

    #[test]
    fn a_roll_is_never_merged_by_the_length_tolerance() {
        let mut a = beam(1, "ipe200", 3000.0);
        let mut b = beam(2, "ipe200", 3000.0);
        a.angles_rad = vec![0.0];
        b.angles_rad = vec![std::f64::consts::FRAC_PI_2];
        let loose = Settings {
            tolerance_mm: 2.0,
            ..Settings::default()
        };
        assert_eq!(
            preview(&[a.clone(), b.clone()], &[], &loose)
                .unwrap()
                .groups
                .len(),
            2
        );
        b.angles_rad = vec![5e-10];
        assert_eq!(preview(&[a, b], &[], &loose).unwrap().groups.len(), 1);
    }

    #[test]
    fn the_default_tolerance_treats_float_noise_as_the_same_length() {
        let parts = [beam(1, "ipe200", 3000.0), beam(2, "ipe200", 3000.0005)];
        let p = preview(&parts, &[], &Settings::default()).unwrap();
        assert_eq!(p.groups.len(), 1);
        let exact = Settings {
            tolerance_mm: 0.0,
            ..Settings::default()
        };
        assert_eq!(preview(&parts, &[], &exact).unwrap().groups.len(), 2);
    }

    #[test]
    fn holes_with_the_same_vertices_but_different_edges_are_different() {
        // Two concave rings through the same five points: areas 1200 and 1100 mm2.
        let first = vec![
            [10.0, 10.0],
            [10.0, 50.0],
            [25.0, 30.0],
            [50.0, 50.0],
            [50.0, 10.0],
        ];
        let second = vec![
            [10.0, 10.0],
            [10.0, 50.0],
            [50.0, 50.0],
            [25.0, 30.0],
            [50.0, 10.0],
        ];
        let mut a = plate(1, 400.0, 300.0, 20.0);
        let mut b = plate(2, 400.0, 300.0, 20.0);
        a.holes = vec![first.clone()];
        b.holes = vec![second];
        assert_eq!(
            preview(&[a.clone(), b], &[], &Settings::default())
                .unwrap()
                .groups
                .len(),
            2
        );
        // The same ring started elsewhere, or wound the other way, is the same hole.
        let mut rotated = first.clone();
        rotated.rotate_left(2);
        let mut reversed = first;
        reversed.reverse();
        let mut c = plate(3, 400.0, 300.0, 20.0);
        let mut d = plate(4, 400.0, 300.0, 20.0);
        c.holes = vec![rotated];
        d.holes = vec![reversed];
        assert_eq!(
            preview(&[a, c, d], &[], &Settings::default())
                .unwrap()
                .groups
                .len(),
            1
        );
    }

    #[test]
    fn many_distinct_parts_are_grouped_without_comparing_them_all() {
        let parts: Vec<PartFact> = (0..30_000u16)
            .map(|n| beam(n + 1, "ipe200", 1000.0 + f64::from(n)))
            .collect();
        let started = std::time::Instant::now();
        let p = preview(&parts, &[], &Settings::default()).unwrap();
        assert_eq!(p.groups.len(), 30_000);
        assert!(
            started.elapsed().as_secs() < 20,
            "took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn a_model_built_to_defeat_the_index_hits_the_work_budget() {
        // Same first dimension, every other dimension different: all in one bucket.
        let parts: Vec<PartFact> = (0..11_000u16)
            .map(|n| plate(n + 1, 400.0, 300.0, 1.0 + f64::from(n)))
            .collect();
        let error = preview(&parts, &[], &Settings::default()).unwrap_err();
        assert!(error.0.contains("comparison work"));
        // With the tolerance covering nothing extra the same model in smaller size is fine.
        assert!(preview(&parts[..2000], &[], &Settings::default()).is_ok());
    }

    #[test]
    fn many_assemblies_with_different_names_are_grouped_by_signature() {
        let assemblies: Vec<AssemblyFact> = (0..20_000u16)
            .map(|n| AssemblyFact {
                id: id(n + 1),
                name: format!("A{n}"),
                parent: None,
            })
            .collect();
        let named = Settings {
            compare_name: true,
            ..Settings::default()
        };
        let p = preview(&[], &assemblies, &named).unwrap();
        assert_eq!(p.groups.len(), 20_000);
        assert_eq!(
            preview(&[], &assemblies, &Settings::default())
                .unwrap()
                .groups
                .len(),
            1
        );
    }
}
