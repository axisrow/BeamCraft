// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! E03 bill of materials: the report tables and their CSV rendering, as pure computation.
//!
//! One [`BomRow`] per element goes in; the sorted per-element table and the grouped summary go
//! out, together with their CSV text. The module knows nothing about USD, stores or definitions:
//! the caller resolves labels and fills the quantities, and this module owns the report rules —
//! sort order, grouping, unknown-mass handling, derived columns and the CSV schema of
//! [the design note](../../../docs/design/e03-bom-reports.md).
//!
//! Only `current` quantities enter a report: the caller re-evaluates every element (as
//! `inspect` does), so a volume is always current and a mass is present exactly when it is
//! current. An unknown mass is never zero: the cell is empty, the element is excluded from the
//! mass totals and counted in the summary's `mass_unknown` column.

use crate::element::ElementKind;
use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;
use std::collections::BTreeMap;

/// One element of the report as the caller fills it.
#[derive(Debug, Clone, PartialEq)]
pub struct BomRow {
    /// Full element identity.
    pub id: crate::ElementId,
    /// The mark of the element. Until A08/A09 numbering exists this is the element id, as the
    /// issue's fallback requires.
    pub mark: String,
    /// Plate, beam or column.
    pub kind: ElementKind,
    /// The user label as authored.
    pub name: String,
    /// Profile label: the designation, else a caller-chosen readable label. Plates use their
    /// `PL<thickness>` label.
    pub profile: String,
    /// Material label, or the empty string when the element pins no material.
    pub material: String,
    /// Axis length of a member; `None` for a plate.
    pub length_mm: Option<f64>,
    /// Area in mm2 the caller derives from the current volume (`volume/length` for a member,
    /// `volume/thickness` for a plate); `None` when the denominator is not usable.
    pub area_mm2: Option<f64>,
    /// Current volume in mm3.
    pub volume_mm3: f64,
    /// Current mass in kg; `None` when it is unknown (no density).
    pub mass_kg: Option<f64>,
}

/// One row of the per-element table.
#[derive(Debug, Clone, PartialEq)]
pub struct BomElement {
    /// The mark (the element id until numbering exists).
    pub mark: String,
    /// Full element id.
    pub id: String,
    /// Kind token.
    pub kind: &'static str,
    /// User label.
    pub name: String,
    /// Profile label.
    pub profile: String,
    /// Material label, empty when none.
    pub material: String,
    /// Axis length of a member; `None` for a plate.
    pub length_mm: Option<f64>,
    /// Derived area in mm2.
    pub area_mm2: Option<f64>,
    /// Volume in mm3 and m3.
    pub volume_mm3: f64,
    /// Volume in m3 (mm3 divided by 1e9).
    pub volume_m3: f64,
    /// Current mass in kg; `None` when unknown.
    pub mass_kg: Option<f64>,
}

/// One group of the summary table.
#[derive(Debug, Clone, PartialEq)]
pub struct BomGroup {
    /// Kind token of the group.
    pub kind: &'static str,
    /// Profile label of the group key.
    pub profile: String,
    /// Material label of the group key, empty when none.
    pub material: String,
    /// Elements in the group.
    pub count: u64,
    /// Total axis length; `None` when the group has no member lengths (plates).
    pub total_length_mm: Option<f64>,
    /// Total area in m2 (known areas only).
    pub total_area_m2: f64,
    /// Total volume in m3.
    pub total_volume_m3: f64,
    /// Total mass in kg of the elements whose mass is current; `None` when none is.
    pub total_mass_kg: Option<f64>,
    /// Elements of the group whose mass is unknown.
    pub mass_unknown: u64,
}

/// The two report tables of one revision.
#[derive(Debug, Clone, PartialEq)]
pub struct Bom {
    /// Elements in report order: kind in registry order, then profile, then mark.
    pub elements: Vec<BomElement>,
    /// Groups in the same order.
    pub groups: Vec<BomGroup>,
}

/// Builds the tables from one row per element. More than `max_rows` rows is
/// `LIMIT_EXCEEDED` before anything is rendered.
pub fn tables(rows: &[BomRow], max_rows: usize) -> Result<Bom, CommandError> {
    if rows.len() > max_rows {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "element_count",
            "the model has more elements than a report may list",
        ));
    }
    let mut ordered: Vec<&BomRow> = rows.iter().collect();
    ordered.sort_by(|a, b| {
        (a.kind, a.profile.as_str(), a.mark.as_str()).cmp(&(
            b.kind,
            b.profile.as_str(),
            b.mark.as_str(),
        ))
    });
    let elements = ordered
        .iter()
        .map(|r| BomElement {
            mark: r.mark.clone(),
            id: r.id.to_string(),
            kind: r.kind.as_token(),
            name: r.name.clone(),
            profile: r.profile.clone(),
            material: r.material.clone(),
            length_mm: r.length_mm,
            area_mm2: r.area_mm2,
            volume_mm3: r.volume_mm3,
            volume_m3: r.volume_mm3 / 1e9,
            mass_kg: r.mass_kg,
        })
        .collect();
    // The group key is the label tuple, so equal labels group together; the BTreeMap keeps the
    // first-seen order of the sorted rows.
    let mut keys = BTreeMap::new();
    for r in ordered {
        let key = (r.kind, r.profile.clone(), r.material.clone());
        let group = keys.entry(key).or_insert_with(|| BomGroup {
            kind: r.kind.as_token(),
            profile: r.profile.clone(),
            material: r.material.clone(),
            count: 0,
            total_length_mm: None,
            total_area_m2: 0.0,
            total_volume_m3: 0.0,
            total_mass_kg: None,
            mass_unknown: 0,
        });
        group.count += 1;
        if let Some(length) = r.length_mm {
            group.total_length_mm = Some(group.total_length_mm.unwrap_or(0.0) + length);
        }
        if let Some(area) = r.area_mm2 {
            group.total_area_m2 += area / 1e6;
        }
        group.total_volume_m3 += r.volume_mm3 / 1e9;
        match r.mass_kg {
            Some(mass) => group.total_mass_kg = Some(group.total_mass_kg.unwrap_or(0.0) + mass),
            None => group.mass_unknown += 1,
        }
    }
    Ok(Bom {
        elements,
        groups: keys.into_values().collect(),
    })
}

/// Appends `text` as one CSV field, quoted per RFC 4180 when it contains a separator, a quote
/// or a line break.
fn field(text: &str, out: &mut String) {
    if text.contains([',', '"', '\n', '\r']) {
        out.push('"');
        for c in text.chars() {
            if c == '"' {
                out.push('"');
            }
            out.push(c);
        }
        out.push('"');
    } else {
        out.push_str(text);
    }
}

fn csv(header: &str, rows: impl IntoIterator<Item = Vec<String>>) -> String {
    let mut out = String::from(header);
    out.push('\n');
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            field(cell, &mut out);
        }
        out.push('\n');
    }
    out
}

/// The per-element table as CSV.
pub fn elements_csv(bom: &Bom) -> String {
    let rows = bom.elements.iter().map(|e| {
        vec![
            e.mark.clone(),
            e.id.clone(),
            e.kind.to_owned(),
            e.name.clone(),
            e.profile.clone(),
            e.material.clone(),
            e.length_mm.map_or(String::new(), num),
            e.area_mm2.map_or(String::new(), num),
            num(e.volume_mm3),
            num(e.volume_m3),
            e.mass_kg.map_or(String::new(), num),
        ]
    });
    csv(
        "mark,element_id,kind,name,profile,material,length_mm,area_mm2,volume_mm3,volume_m3,mass_kg",
        rows,
    )
}

/// The summary table as CSV.
pub fn summary_csv(bom: &Bom) -> String {
    let rows = bom.groups.iter().map(|g| {
        vec![
            g.kind.to_owned(),
            g.profile.clone(),
            g.material.clone(),
            g.count.to_string(),
            g.total_length_mm.map_or(String::new(), num),
            num(g.total_area_m2),
            num(g.total_volume_m3),
            g.total_mass_kg.map_or(String::new(), num),
            g.mass_unknown.to_string(),
        ]
    });
    csv(
        "kind,profile,material,count,total_length_mm,total_area_m2,total_volume_m3,total_mass_kg,mass_unknown",
        rows,
    )
}

/// The shortest round-trip text of a finite number: no exponent padding, no separators.
fn num(value: f64) -> String {
    format!("{value}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u8) -> crate::ElementId {
        crate::ElementId::parse(&format!("019a7bf0-0000-7000-8000-0000000000{n:02x}")).unwrap()
    }

    fn row(n: u8, kind: ElementKind, profile: &str, mass: Option<f64>) -> BomRow {
        BomRow {
            id: id(n),
            mark: id(n).to_string(),
            kind,
            name: format!("Part {n}"),
            profile: profile.to_owned(),
            material: "S355".into(),
            length_mm: (kind != ElementKind::Plate).then_some(3000.0),
            area_mm2: Some(2000.0),
            volume_mm3: 6e6,
            mass_kg: mass,
        }
    }

    #[test]
    fn tables_are_sorted_by_kind_then_profile_then_mark() {
        let rows = [
            row(3, ElementKind::Beam, "I 300", Some(47.1)),
            row(1, ElementKind::Plate, "PL10", None),
            row(2, ElementKind::Beam, "I 200", Some(30.0)),
            row(4, ElementKind::Column, "I 300", Some(47.1)),
        ];
        let bom = tables(&rows, 100).unwrap();
        let kinds: Vec<&str> = bom.elements.iter().map(|e| e.kind).collect();
        assert_eq!(kinds, ["plate", "beam", "beam", "column"]);
        assert_eq!(bom.elements[1].profile, "I 200");
        assert_eq!(bom.elements[2].mark, id(3).to_string());
        // The element row carries both the mark and the id, equal while unnumbered.
        assert_eq!(bom.elements[0].mark, bom.elements[0].id);
        assert_eq!(bom.elements[0].volume_m3, 0.006);
    }

    #[test]
    fn unknown_mass_is_excluded_from_totals_and_counted() {
        let rows = [
            row(1, ElementKind::Plate, "PL10", Some(50.0)),
            row(2, ElementKind::Plate, "PL10", None),
            row(3, ElementKind::Plate, "PL12", None),
        ];
        let bom = tables(&rows, 100).unwrap();
        assert_eq!(bom.groups.len(), 2);
        let pl10 = &bom.groups[0];
        assert_eq!(pl10.count, 2);
        assert_eq!(pl10.total_mass_kg, Some(50.0));
        assert_eq!(pl10.mass_unknown, 1);
        assert_eq!(pl10.total_length_mm, None);
        assert_eq!(pl10.total_volume_m3, 0.012);
        assert_eq!(pl10.total_area_m2, 0.004);
        let pl12 = &bom.groups[1];
        assert_eq!(pl12.total_mass_kg, None);
        assert_eq!(pl12.mass_unknown, 1);
    }

    #[test]
    fn rows_beyond_the_cap_are_refused_before_rendering() {
        let rows = [row(1, ElementKind::Plate, "PL10", None)];
        assert_eq!(tables(&rows, 0).unwrap_err().code, ErrorCode::LimitExceeded);
    }

    #[test]
    fn csv_quotes_only_fields_that_need_it() {
        let mut evil = BomRow {
            id: id(1),
            mark: id(1).to_string(),
            kind: ElementKind::Plate,
            name: "He said \"stop\", then".into(),
            profile: "PL10".into(),
            material: String::new(),
            length_mm: None,
            area_mm2: Some(1.5),
            volume_mm3: 10.0,
            mass_kg: None,
        };
        let bom = tables(&[evil.clone()], 10).unwrap();
        let text = elements_csv(&bom);
        assert!(text.contains("\"He said \"\"stop\"\", then\""), "{text}");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines[0],
            "mark,element_id,kind,name,profile,material,length_mm,area_mm2,volume_mm3,volume_m3,mass_kg"
        );
        // Empty cells stay empty, never zero: unknown mass and no material.
        assert!(lines[1].ends_with(",1.5,10,0.00000001,"), "{}", lines[1]);
        evil.mass_kg = Some(0.0785);
        let bom = tables(&[evil], 10).unwrap();
        assert!(
            elements_csv(&bom)
                .lines()
                .nth(1)
                .unwrap()
                .ends_with(",0.0785")
        );
        assert_eq!(
            summary_csv(&bom).lines().next().unwrap(),
            "kind,profile,material,count,total_length_mm,total_area_m2,total_volume_m3,total_mass_kg,mass_unknown"
        );
    }
}
