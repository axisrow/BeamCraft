// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A08 numbering preview over a committed revision.
//!
//! Reads plates, members and assemblies of the verified head, hands the facts to
//! `beamcraft_engine::numbering` and returns the proposal. It writes nothing: no revision, no
//! layer, no attribute. Approving the numbers is A09.

use crate::assembly::assemblies;
use crate::plate::{PlateError, inspect};
use beamcraft_engine::numbering::{AssemblyFact, PartFact, PartKind, Preview, Settings, preview};
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
}

fn error(code: ErrorCode, message: &'static str) -> PlateError {
    PlateError { code, message }
}

fn facts(store: &RevisionStore) -> Result<(String, Vec<PartFact>, Vec<AssemblyFact>), PlateError> {
    // Plates and assemblies are read by two readers of the head. If the head moves between the
    // two reads they describe different revisions, so the pair is read again (a few times).
    for _ in 0..3 {
        let before = store.head()?.map(|h| h.revision().as_str().to_owned());
        let inspection = inspect(store)?;
        let views = assemblies(store)?;
        let after = store.head()?.map(|h| h.revision().as_str().to_owned());
        if before != after || before.as_deref() != Some(inspection.revision.revision_id.as_str()) {
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
                holes: a.holes_mm.clone(),
                assembly: holder.get(&a.id).copied(),
            });
        }
        for record in &inspection.members {
            let a = &record.authored;
            let [p, q] = a.axis_points_mm;
            let length =
                ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2) + (q[2] - p[2]).powi(2)).sqrt();
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
        return Ok((inspection.revision.revision_id, parts, assembly_facts));
    }
    Err(error(
        ErrorCode::DependencyUnavailable,
        "the head moved while the numbering facts were read",
    ))
}

/// Previews the numbering of the verified head. `previous` is the token of an earlier preview;
/// the result says whether the numbering it describes is out of date.
pub fn numbering_preview(
    store: &RevisionStore,
    settings: &Settings,
    previous: Option<ContentDigest>,
) -> Result<NumberingView, PlateError> {
    let (revision_id, parts, assemblies) = facts(store)?;
    let proposal = preview(&parts, &assemblies, settings)
        .map_err(|e| error(ErrorCode::InvalidArgument, e.0))?;
    Ok(NumberingView {
        revision_id,
        settings: settings.clone(),
        stale: previous.map(|token| token != proposal.token),
        parts: parts.len(),
        assemblies: assemblies.len(),
        preview: proposal,
    })
}
