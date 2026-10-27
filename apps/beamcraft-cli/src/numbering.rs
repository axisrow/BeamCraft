// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A08 numbering preview over a committed revision.
//!
//! Reads plates, members and assemblies of the verified head, hands the facts to
//! `beamcraft_engine::numbering` and returns the proposal. It writes nothing: no revision, no
//! layer, no attribute. Approving the numbers is A09 ([`crate::numbering_approval`]); the preview
//! also reports the approval stored in the model.

use crate::assembly::assemblies;
use crate::numbering_approval::head_marks;
use crate::plate::{PlateError, inspect};
use beamcraft_engine::numbering::{AssemblyFact, PartFact, PartKind, Preview, Settings, preview};
use beamcraft_engine::numbering_approval::{Mark, Report, StoredMark, assess};
use beamcraft_engine::query::committed::PlateRecord;
use beamcraft_host::revision::RevisionStore;
use beamcraft_model::command::MemberKind;
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::ElementId;
use std::collections::BTreeMap;

/// A preview and what it was made from.
#[derive(Debug, Clone, PartialEq)]
pub struct NumberingView {
    /// The revision the facts were read from.
    pub revision_id: String,
    /// The settings used.
    pub settings: Settings,
    /// The proposal.
    pub preview: Preview,
    /// Parts considered.
    pub parts: usize,
    /// Assemblies considered.
    pub assemblies: usize,
    /// Whether a token given by the caller no longer matches (`None` when none was given).
    pub stale: Option<bool>,
    /// The approved numbering stored in the model, judged against the model (A09).
    pub approval: Report,
    /// The marks stored on parts and assemblies, by element.
    pub stored: BTreeMap<ElementId, Mark>,
}

fn error(code: ErrorCode, message: &'static str) -> PlateError {
    PlateError::new(code, message)
}

/// The minimum corner of the bounding box of a plate's contour. The outline and the holes of a
/// plate are compared relative to it, so two plates that differ only by where their contour sits
/// in the element frame are the same part, and two whose holes sit differently from their edges
/// are not. For the origin rectangle it is the origin, so nothing moves.
fn contour_origin(record: &PlateRecord) -> [f64; 2] {
    let points = &record.authored.contour_points_mm;
    [
        points.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min),
        points.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min),
    ]
}

/// The outline of a plate whose contour is not the origin rectangle, relative to
/// [`contour_origin`]; empty for the origin rectangle (its width and length already say it all).
fn outline_of(record: &PlateRecord) -> Vec<[f64; 2]> {
    if record.body.is_rectangular_box() {
        return Vec::new();
    }
    let [low_x, low_y] = contour_origin(record);
    record
        .authored
        .contour_points_mm
        .iter()
        .map(|p| [p[0] - low_x, p[1] - low_y])
        .collect()
}

/// The through holes of a plate relative to [`contour_origin`], in their authored order.
fn holes_of(record: &PlateRecord) -> Vec<Vec<[f64; 2]>> {
    let [low_x, low_y] = contour_origin(record);
    record
        .authored
        .holes_mm
        .iter()
        .map(|hole| hole.iter().map(|p| [p[0] - low_x, p[1] - low_y]).collect())
        .collect()
}

/// The part and assembly facts of one revision and the marks stored on it.
pub(crate) struct State {
    /// The revision everything below was read from.
    pub(crate) revision_id: String,
    /// Plates, beams and columns.
    pub(crate) parts: Vec<PartFact>,
    /// Assemblies.
    pub(crate) assemblies: Vec<AssemblyFact>,
    /// The approved marks stored on parts and assemblies (A09).
    pub(crate) stored: Vec<StoredMark>,
}

pub(crate) fn state(store: &RevisionStore) -> Result<State, PlateError> {
    // Plates, assemblies and marks are read by three readers of the head. If the head moves
    // between the reads they describe different revisions, so they are read again (a few times).
    for _ in 0..3 {
        let before = store.head()?.map(|h| h.revision().as_str().to_owned());
        let inspection = inspect(store)?;
        inspection.require_evaluated_geometry()?;
        let views = assemblies(store)?;
        let (marks_revision, stored) = head_marks(store)?;
        let after = store.head()?.map(|h| h.revision().as_str().to_owned());
        if before != after
            || before.as_deref() != Some(inspection.revision.revision_id.as_str())
            || marks_revision != inspection.revision.revision_id
        {
            continue;
        }
        let mut holder: BTreeMap<ElementId, ElementId> = BTreeMap::new();
        for view in &views {
            for part in &view.parts {
                holder.insert(*part, view.id);
            }
        }
        let mut parts = Vec::new();
        for record in &inspection.plates {
            let a = &record.authored;
            parts.push(PartFact {
                id: a.id,
                kind: PartKind::Plate,
                name: a.name.clone(),
                profile: None,
                material: a.material,
                dimensions: vec![record.body.width_mm, record.body.length_mm, a.thickness_mm],
                angles_rad: Vec::new(),
                holes: holes_of(record),
                outline: outline_of(record),
                assembly: holder.get(&a.id).copied(),
            });
        }
        for record in &inspection.members {
            let a = &record.authored;
            // The numbering length is the body's own: the axis chord for a straight member, the
            // polyline path length for a polybeam.
            let length = record.body.axis_length_mm;
            parts.push(PartFact {
                id: a.id,
                kind: match a.kind {
                    MemberKind::Beam => PartKind::Beam,
                    MemberKind::Column => PartKind::Column,
                },
                name: a.name.clone(),
                profile: Some(a.profile),
                material: a.material,
                dimensions: vec![length],
                angles_rad: vec![a.roll_rad.unwrap_or(0.0)],
                holes: Vec::new(),
                outline: Vec::new(),
                assembly: holder.get(&a.id).copied(),
            });
        }
        let assembly_facts = views
            .iter()
            .map(|v| AssemblyFact {
                id: v.id,
                name: v.name.clone(),
                parent: v.parent,
            })
            .collect();
        return Ok(State {
            revision_id: inspection.revision.revision_id,
            parts,
            assemblies: assembly_facts,
            stored,
        });
    }
    Err(error(
        ErrorCode::DependencyUnavailable,
        "the head moved while the numbering facts were read",
    ))
}

/// Previews the numbering of the verified head. `previous` is the token of an earlier preview;
/// the result says whether the numbering it describes is out of date. The view also reports the
/// approval (A09) stored in the model and how every group compares with the marks stored on its
/// members; none of it changes the proposal, which is a function of the revision's facts.
pub fn numbering_preview(
    store: &RevisionStore,
    settings: &Settings,
    previous: Option<ContentDigest>,
) -> Result<NumberingView, PlateError> {
    let State {
        revision_id,
        parts,
        assemblies,
        stored,
    } = state(store)?;
    let proposal = preview(&parts, &assemblies, settings)
        .map_err(|e| error(ErrorCode::InvalidArgument, e.0))?;
    let approval = assess(&parts, &assemblies, &stored).map_err(|e| error(e.code(), e.message))?;
    let stored: BTreeMap<ElementId, Mark> = stored.into_iter().map(|m| (m.id, m.mark)).collect();
    Ok(NumberingView {
        revision_id,
        settings: settings.clone(),
        stale: previous.map(|token| token != proposal.token),
        parts: parts.len(),
        assemblies: assemblies.len(),
        preview: proposal,
        approval,
        stored,
    })
}
