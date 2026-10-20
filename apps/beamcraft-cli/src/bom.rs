// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! E03 adapter: the bill-of-materials rows of one verified revision.
//!
//! Maps the engine's re-evaluated records of [`crate::plate::inspect`] to
//! [`beamcraft_engine::bom::BomRow`]s: the labels come from the pinned definitions, the derived
//! areas divide the current volume, and the mark is the element's assigned or approved mark
//! (A09), falling back to the element id for an unnumbered element. No geometry is computed
//! here.

use crate::definition::DefinitionRecord;
use crate::plate::{Inspection, PlateError};
use beamcraft_engine::bom::BomRow;
use beamcraft_engine::element::ElementKind;
use beamcraft_engine::query::committed::{MemberRecord, PlateRecord};
use beamcraft_model::command::MemberKind;
use beamcraft_model::digest::ContentDigest;

type Run<T> = Result<T, PlateError>;

/// One report row per plate and member of the verified head.
pub fn rows(inspection: &Inspection) -> Run<Vec<BomRow>> {
    let mut result = Vec::with_capacity(
        inspection
            .plates
            .len()
            .saturating_add(inspection.members.len()),
    );
    for record in &inspection.plates {
        result.push(plate_row(inspection, record)?);
    }
    for record in &inspection.members {
        result.push(member_row(inspection, record)?);
    }
    Ok(result)
}

/// The mark of an element: its assigned or approved mark (A09), else the element id — the
/// issue's fallback for an element that is not numbered.
fn mark_of(inspection: &Inspection, id: &beamcraft_model::id::ElementId) -> String {
    inspection
        .marks
        .mark(id)
        .map(|mark| mark.value.clone())
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

fn plate_row(inspection: &Inspection, record: &PlateRecord) -> Run<BomRow> {
    let thickness = record.body.thickness_mm;
    Ok(BomRow {
        id: record.authored.id,
        mark: mark_of(inspection, &record.authored.id),
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

fn member_row(inspection: &Inspection, record: &MemberRecord) -> Run<BomRow> {
    let [p1, p2] = record.authored.axis_points_mm;
    let d = [p2[0] - p1[0], p2[1] - p1[1], p2[2] - p1[2]];
    let length = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    Ok(BomRow {
        id: record.authored.id,
        mark: mark_of(inspection, &record.authored.id),
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
