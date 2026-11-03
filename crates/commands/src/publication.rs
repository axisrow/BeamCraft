// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Shared prepared-layer publication and verified revision commit boundary.

use session::Refusal;
use session::draft::{self, Admitted};
use session::pipeline::{DERIVED_LAYER, ROOT_LAYER, host_uuid, integrity};
use std::collections::BTreeMap;

use crate::plate::{PlateVerifier, check_footprint};
use beamcraft_engine::footprint::Footprint;
use beamcraft_host::revision::{
    CommitHook, CommitOutcome, Committed, ExpectedBase, LayerEntry, LayerRole, RevisionDraft,
    RevisionInfo, RevisionStore, Uuid7,
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
///
/// While a draft batch is open nothing is published: the prepared layer set is admitted into
/// the draft, and the returned outcome previews the revision this command would create —
/// the batch's single commit later publishes the accumulated state as one revision with the
/// batch operation's own identity.
pub(crate) fn publish_revision(
    store: &RevisionStore,
    request: &PublishRequest<'_>,
    hook: &mut dyn CommitHook,
) -> Result<Committed, Refusal> {
    if draft::is_open() {
        return draft_admit(request);
    }
    let generation = request.base_generation.checked_add(1).ok_or(Refusal::new(
        ErrorCode::LimitExceeded,
        "generation exhausted",
    ))?;
    let mut entries = Vec::new();
    for layer in &request.prepared.layers {
        store
            .blobs()
            .publish(&layer.bytes)
            .map_err(|e| Refusal::new(e.code(), "layer blob could not be stored"))?;
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
            .map_err(|e| Refusal::new(e.code(), "foreign layer blob could not be stored"))?;
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
    // U11: the persisted evidence carries the immutable geometry assets the batch's mesh
    // transfers pin, so a later rebase can admit the shared asset prims' deltas.
    let evidence = crate::plate::with_transfer_assets(request.footprint, request.prepared)?;
    let pin =
        crate::command_evidence::store(store, &request.op, request.fingerprint, &evidence, &[])?;
    Ok(store.commit_with_evidence(
        &draft,
        request.expected,
        Some(pin),
        Some(&PlateVerifier),
        hook,
    )?)
}

/// Admits one prepared candidate into the open draft batch and previews the revision it
/// would create: the manifest is assembled exactly as the batch's commit will assemble the
/// accumulated state, with this command's own operation, revision id and sequence. No blob,
/// no manifest and no ledger record is written.
fn draft_admit(request: &PublishRequest<'_>) -> Result<Committed, Refusal> {
    // U11: the drafted command's declared footprint is augmented with its own mesh-transfer
    // assets exactly as a committed publication would be, so the batch's union evidence
    // carries them and a later rebase can admit the shared asset prims' deltas.
    let footprint = crate::plate::with_transfer_assets(request.footprint, request.prepared)?;
    let admitted = draft::with(|state| {
        state.admit(
            &request.prepared.layers,
            &request.prepared.retained,
            request.op.as_str(),
            request.fingerprint,
            &footprint,
        )
    })
    .ok_or_else(|| integrity("a draft batch vanished while a command ran"))??;
    let info = preview_of(request, &admitted)?;
    Ok(Committed {
        outcome: CommitOutcome::Drafted,
        info,
    })
}

/// Assembles the preview manifest of one drafted command: the layer entries the batch commit
/// will carry, at the command's own generation and sequence, over the batch's pinned base.
fn preview_of(request: &PublishRequest<'_>, admitted: &Admitted) -> Result<RevisionInfo, Refusal> {
    let mut entries = Vec::new();
    for layer in &request.prepared.layers {
        entries.push(LayerEntry::from_bytes(
            &layer.name,
            role_of(&layer.name),
            &layer.bytes,
        )?);
    }
    for layer in &request.prepared.retained {
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
        workspace_generation: admitted.generation,
        schema: request.schema.digest,
        root_layer: ROOT_LAYER.into(),
        entries,
    };
    Ok(RevisionInfo::preview(
        &draft,
        admitted.sequence,
        admitted.base.clone(),
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
) -> Result<(Committed, Vec<String>), Refusal> {
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
) -> Result<Committed, Refusal> {
    commit_verified_with_evidence(store, draft, expected, None, hook)
}

pub(crate) fn commit_verified_with_evidence(
    store: &RevisionStore,
    draft: &RevisionDraft,
    expected: &ExpectedBase,
    evidence: Option<beamcraft_host::revision::EvidencePin>,
    hook: &mut dyn CommitHook,
) -> Result<Committed, Refusal> {
    // Belt for the draft batch's admission rules: the direct materializer funnel (forking,
    // publishing, undo/redo, compensation) must not write the store behind an open batch's
    // back, whatever word reached it. The batch's own commit ends the draft before it calls
    // here, so it is never refused by this.
    if session::draft::is_open() {
        return Err(Refusal::new(
            ErrorCode::InvalidArgument,
            "this command commits directly and may not run inside a draft batch",
        ));
    }
    Ok(store.commit_with_evidence(draft, expected, evidence, Some(&PlateVerifier), hook)?)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

    use super::*;
    use beamcraft_host::ids::IdSource;
    use beamcraft_host::revision::RevisionStore as Store;
    use beamcraft_host::revision::{ExpectedBase, NoFault};
    use beamcraft_host::storage::BlobStore;
    use std::collections::BTreeSet;
    use std::sync::atomic::{AtomicU32, Ordering};

    static NEXT: AtomicU32 = AtomicU32::new(0);

    /// The direct materializer funnel refuses while a draft batch is open, and the refusal
    /// is the guard's own: once the batch ends, the same call gets past it (to whatever the
    /// commit itself makes of the draft on an empty store).
    #[test]
    fn the_direct_funnel_refuses_under_an_open_draft() {
        let root = std::env::temp_dir().join(format!(
            "beamcraft-funnel-guard-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let store = Store::new(BlobStore::new(root).unwrap());
        let mut ids = IdSource::system();
        let mut fresh = || ids.next_id().unwrap().to_string();
        let base = session::pipeline::Base {
            project: fresh(),
            workspace: fresh(),
            digest: session::pipeline::digest_of(b"funnel-guard-base").unwrap(),
            generation: 1,
            assets: Vec::new(),
            external: BTreeSet::new(),
            expected: ExpectedBase::Initial,
        };
        let guard = session::draft::hold(base, 1).unwrap();
        let draft = RevisionDraft {
            project: session::pipeline::host_uuid(&fresh()).unwrap(),
            workspace: session::pipeline::host_uuid(&fresh()).unwrap(),
            revision: session::pipeline::host_uuid(&fresh()).unwrap(),
            op: session::pipeline::host_uuid(&fresh()).unwrap(),
            workspace_generation: 2,
            schema: session::pipeline::digest_of(b"schema").unwrap(),
            root_layer: session::pipeline::ROOT_LAYER.into(),
            entries: vec![
                LayerEntry::from_bytes(
                    "authored.usda",
                    LayerRole::Authored,
                    session::pipeline::EMPTY_LAYER.as_bytes(),
                )
                .unwrap(),
            ],
        };
        let expected = ExpectedBase::Initial;
        let refused = commit_verified(&store, &draft, &expected, &mut NoFault).unwrap_err();
        assert_eq!(refused.code, ErrorCode::InvalidArgument);
        assert_eq!(
            refused.message,
            "this command commits directly and may not run inside a draft batch"
        );
        // The batch's own commit ends the draft first, so the funnel opens for it.
        drop(session::draft::take());
        let error = commit_verified(&store, &draft, &expected, &mut NoFault).unwrap_err();
        assert_ne!(
            error.message,
            "this command commits directly and may not run inside a draft batch"
        );
        drop(guard);
    }
}
