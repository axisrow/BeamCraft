// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Shared prepared-layer publication and verified revision commit boundary.

use std::collections::BTreeMap;

use crate::plate::{
    DERIVED_LAYER, PlateError, PlateVerifier, ROOT_LAYER, check_footprint, host_uuid, integrity,
};
use beamcraft_engine::footprint::Footprint;
use beamcraft_host::revision::{
    CommitHook, CommitOutcome, Committed, ExpectedBase, LayerEntry, LayerRole, RevisionDraft,
    RevisionStore, Uuid7,
};
use beamcraft_layer_engine::authored::{Author, Change, Prepare, Prepared, SchemaIdentity};
use beamcraft_layer_engine::{Control, Opened};
use beamcraft_model::{digest::ContentDigest, error::ErrorCode, id::RevisionId};

fn role_of(name: &str) -> LayerRole {
    if name == DERIVED_LAYER {
        LayerRole::Derived
    } else {
        LayerRole::Authored
    }
}

/// The names of prepared layers whose digest differs from the base revision, in prepared order:
/// the `changed_layers` report shared by every copy/edit command.
pub(crate) fn changed_layers(
    prepared: &Prepared,
    before: &BTreeMap<String, ContentDigest>,
) -> Vec<String> {
    prepared
        .layers
        .iter()
        .filter(|l| before.get(&l.name) != Some(&l.digest))
        .map(|l| l.name.clone())
        .collect()
}

/// What `publish_revision` needs to turn a prepared candidate into a committed revision.
pub(crate) struct PublishRequest<'a> {
    pub(crate) footprint: &'a Footprint,
    pub(crate) fingerprint: ContentDigest,
    pub(crate) prepared: &'a Prepared,
    pub(crate) schema: SchemaIdentity,
    pub(crate) project: &'a str,
    pub(crate) workspace: &'a str,
    pub(crate) revision_id: &'a RevisionId,
    pub(crate) op: Uuid7,
    pub(crate) base_generation: u64,
    pub(crate) expected: &'a ExpectedBase,
}

/// Stores the layer blobs of a prepared candidate and commits them as the next revision through
/// U06/U07, with the model content verifier.
pub(crate) fn publish_revision(
    store: &RevisionStore,
    request: &PublishRequest<'_>,
    hook: &mut dyn CommitHook,
) -> Result<Committed, PlateError> {
    let generation = request
        .base_generation
        .checked_add(1)
        .ok_or(PlateError::new(
            ErrorCode::LimitExceeded,
            "generation exhausted",
        ))?;
    let mut entries = Vec::new();
    for layer in &request.prepared.layers {
        store
            .blobs()
            .publish(&layer.bytes)
            .map_err(|e| PlateError::new(e.code(), "layer blob could not be stored"))?;
        entries.push(LayerEntry::from_bytes(
            &layer.name,
            role_of(&layer.name),
            &layer.bytes,
        )?);
    }
    // Foreign sources are carried into every revision byte for byte as external entries. Their
    // blob is published if absent (publication compares existing bytes in full) and is never
    // rewritten.
    for layer in &request.prepared.retained {
        store
            .blobs()
            .publish(&layer.bytes)
            .map_err(|e| PlateError::new(e.code(), "foreign layer blob could not be stored"))?;
        entries.push(LayerEntry::from_bytes(
            &layer.name,
            LayerRole::External,
            &layer.bytes,
        )?);
    }
    let draft = RevisionDraft {
        project: host_uuid(request.project)?,
        workspace: host_uuid(request.workspace)?,
        revision: host_uuid(&request.revision_id.to_string())?,
        op: request.op.clone(),
        workspace_generation: generation,
        schema: request.schema.digest,
        root_layer: ROOT_LAYER.into(),
        entries,
    };
    let pin =
        crate::command_evidence::store(store, &request.op, request.fingerprint, request.footprint)?;
    Ok(store.commit_with_evidence(
        &draft,
        request.expected,
        Some(pin),
        Some(&PlateVerifier),
        hook,
    )?)
}

/// Identity and ledger context of one prepare→publish tail, shared by the copy, edit and
/// delete adapters. The command-specific changes, footprint and replay decision stay with
/// the caller: that is where the adapters genuinely differ.
pub(crate) struct PublicationTail<'a> {
    pub(crate) store: &'a RevisionStore,
    pub(crate) control: &'a Control,
    pub(crate) schema: SchemaIdentity,
    pub(crate) fingerprint: ContentDigest,
    pub(crate) project: &'a str,
    pub(crate) workspace: &'a str,
    pub(crate) revision_id: &'a RevisionId,
    pub(crate) op: Uuid7,
    pub(crate) op_id: String,
    pub(crate) actor: String,
    pub(crate) base_generation: u64,
    pub(crate) expected: &'a ExpectedBase,
}

/// The genuinely common tail of the copy/edit/delete adapters: prepare the authored candidate
/// with U02, check the layer engine's write set against the declared footprint, report the
/// changed layers, and commit through U06/U07 with the strict verifier. Fails closed when the
/// host reports an unseen operation as replayed.
pub(crate) fn prepare_and_publish(
    author: &mut Author,
    opened: &Opened,
    changes: Vec<Change>,
    footprint: &Footprint,
    before: &BTreeMap<String, ContentDigest>,
    tail: PublicationTail<'_>,
    hook: &mut dyn CommitHook,
) -> Result<(Committed, Vec<String>), PlateError> {
    let prepared = author.prepare(
        Prepare {
            context: opened.context.clone(),
            op_id: tail.op_id,
            actor: tail.actor,
            schema: tail.schema,
            resources: Vec::new(),
            changes,
        },
        tail.control,
    )?;
    check_footprint(&prepared, footprint)?;
    let changed_layers = changed_layers(&prepared, before);
    let committed = publish_revision(
        tail.store,
        &PublishRequest {
            footprint,
            fingerprint: tail.fingerprint,
            prepared: &prepared,
            schema: tail.schema,
            project: tail.project,
            workspace: tail.workspace,
            revision_id: tail.revision_id,
            op: tail.op,
            base_generation: tail.base_generation,
            expected: tail.expected,
        },
        hook,
    )?;
    if committed.outcome == CommitOutcome::Replayed {
        return Err(integrity("an unseen operation was reported as replayed"));
    }
    Ok((committed, changed_layers))
}

/// Commits a complete layer set through U06/U07 with the model content verifier (U03
/// index, structural validation, kernel re-evaluation of every element). Used where the layers
/// already exist as bytes, such as forking and publishing a workspace.
pub(crate) fn commit_verified(
    store: &RevisionStore,
    draft: &RevisionDraft,
    expected: &ExpectedBase,
    hook: &mut dyn CommitHook,
) -> Result<Committed, PlateError> {
    commit_verified_with_evidence(store, draft, expected, None, hook)
}

pub(crate) fn commit_verified_with_evidence(
    store: &RevisionStore,
    draft: &RevisionDraft,
    expected: &ExpectedBase,
    evidence: Option<beamcraft_host::revision::EvidencePin>,
    hook: &mut dyn CommitHook,
) -> Result<Committed, PlateError> {
    Ok(store.commit_with_evidence(draft, expected, evidence, Some(&PlateVerifier), hook)?)
}
