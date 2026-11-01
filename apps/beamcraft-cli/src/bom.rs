// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! E03 adapter: the bill-of-materials rows of one verified revision.
//!
//! Maps the engine's re-evaluated records of [`crate::plate::inspect`] to
//! [`beamcraft_engine::bom::BomRow`]s: the labels come from the pinned definitions, the derived
//! areas divide the current volume, and the mark is the element's approved numbering mark (A09)
//! read from the stored `steel:numbering:*` properties, falling back to the element id for an
//! unnumbered element. No geometry is computed here.

use crate::definition::DefinitionRecord;
use crate::plate::Inspection;
use beamcraft_engine::bom::BomRow;
use beamcraft_engine::element::ElementKind;
use beamcraft_engine::numbering::GroupKind;
use beamcraft_engine::numbering_approval::StoredMark;
use beamcraft_model::digest::ContentDigest;
use session::Refusal;
use steel::engine::committed::{MemberRecord, PlateRecord};
use steel::model::command::MemberKind;

type Run<T> = Result<T, Refusal>;

/// One report row per plate and member of the verified head.
pub fn rows(inspection: &Inspection, marks: &[StoredMark]) -> Run<Vec<BomRow>> {
    inspection.require_evaluated_geometry()?;
    let mut result = Vec::with_capacity(
        inspection
            .plates
            .len()
            .saturating_add(inspection.members.len()),
    );
    for record in &inspection.plates {
        result.push(plate_row(inspection, marks, record)?);
    }
    for record in &inspection.members {
        result.push(member_row(inspection, marks, record)?);
    }
    Ok(result)
}

/// The mark of an element: its approved numbering mark (A09) as the reader rule renders it,
/// else the element id — the fallback for an element that is not numbered.
fn mark_of(marks: &[StoredMark], id: &beamcraft_model::id::ElementId, kind: GroupKind) -> String {
    marks
        .iter()
        .find(|m| &m.id == id)
        .map(|m| m.mark.text(kind))
        .unwrap_or_else(|| id.to_string())
}

/// The readable label of a pinned profile definition: its designation, else its display name,
/// else the digest. A digest with no definition (damaged pin) reports the digest.
fn profile_label(inspection: &Inspection, digest: ContentDigest) -> String {
    inspection
        .definitions
        .get(&digest)
        .and_then(|record| match record {
            DefinitionRecord::RectangleProfile { section, .. }
            | DefinitionRecord::SectionProfile { section, .. } => section
                .designation
                .as_deref()
                .or(section.display_name.as_deref())
                .map(str::to_owned),
            _ => None,
        })
        .unwrap_or_else(|| digest.to_string())
}

/// The material label: the grade, else the display name, else the digest; empty when the
/// element pins no material.
fn material_label(inspection: &Inspection, digest: Option<ContentDigest>) -> String {
    let Some(digest) = digest else {
        return String::new();
    };
    inspection
        .definitions
        .get(&digest)
        .and_then(|record| match record {
            DefinitionRecord::Material {
                grade,
                display_name,
                ..
            } => grade
                .as_deref()
                .or(display_name.as_deref())
                .map(str::to_owned),
            _ => None,
        })
        .unwrap_or_else(|| digest.to_string())
}

fn plate_row(inspection: &Inspection, marks: &[StoredMark], record: &PlateRecord) -> Run<BomRow> {
    let thickness = record.body.thickness_mm;
    Ok(BomRow {
        id: record.authored.id,
        mark: mark_of(
            marks,
            &record.authored.id,
            GroupKind::Part(beamcraft_engine::numbering::PartKind::Plate),
        ),
        kind: ElementKind::Plate,
        name: record.authored.name.clone(),
        profile: format!("PL{thickness}"),
        material: material_label(inspection, record.authored.material),
        length_mm: None,
        // The net face area: the volume is the face area times the thickness (holes included).
        area_mm2: (thickness > 0.0).then(|| record.quantities.volume_mm3 / thickness),
        volume_mm3: record.quantities.volume_mm3,
        mass_kg: record.quantities.mass_kg,
    })
}

fn member_row(inspection: &Inspection, marks: &[StoredMark], record: &MemberRecord) -> Run<BomRow> {
    // The member length is the body's own: the axis chord for a straight member, the polyline
    // path length for a polybeam.
    let length = record.body.axis_length_mm;
    Ok(BomRow {
        id: record.authored.id,
        mark: mark_of(
            marks,
            &record.authored.id,
            GroupKind::Part(match record.authored.kind {
                MemberKind::Beam => beamcraft_engine::numbering::PartKind::Beam,
                MemberKind::Column => beamcraft_engine::numbering::PartKind::Column,
            }),
        ),
        kind: match record.authored.kind {
            MemberKind::Beam => ElementKind::Beam,
            MemberKind::Column => ElementKind::Column,
        },
        name: record.authored.name.clone(),
        profile: profile_label(inspection, record.authored.profile),
        material: material_label(inspection, record.authored.material),
        length_mm: (length > 0.0).then_some(length),
        // The net area of the detailed outline: the volume is that area times the axis length.
        area_mm2: (length > 0.0).then(|| record.quantities.volume_mm3 / length),
        volume_mm3: record.quantities.volume_mm3,
        mass_kg: record.quantities.mass_kg,
    })
}
