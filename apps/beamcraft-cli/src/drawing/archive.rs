// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D03 immutable source resources. They are not composed into the drawing stage. Reopening
//! restores original virtual layer names and runs the ordinary strict model/kernel inspection.

use crate::plate::{Inspection, PlateError, inspect_manifest_assets, integrity};
use beamcraft_engine::drawing::SourceModelPin;
use beamcraft_host::lease::StoreLease;
use beamcraft_host::revision::{MAX_MANIFEST_BYTES, MAX_NAME_BYTES, Manifest, RevisionStore};
use beamcraft_host::storage::BlobStore;
use beamcraft_layer_engine::authored::schema_identity;
use beamcraft_layer_engine::{Asset, Budgets};
use beamcraft_model::digest::Sha256;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{ProjectId, RevisionId};
use std::path::Path;

/// Non-USD provenance metadata asset, excluded from all drawing/model composition requests.
pub const MANIFEST_RESOURCE: &str = "source-manifest";
/// Namespace for archived exact source USD; original virtual names are restored only for reading.
pub const SOURCE_PREFIX: &str = "model/";

/// Verified source pin and complete immutable resources owned by a retained document version.
/// Source bytes outlive the live model store; no live-history lookup is needed to inspect them.
#[derive(Debug)]
pub struct SourceArchive {
    pin: SourceModelPin,
    manifest: Manifest,
    resources: Vec<Asset>,
}

fn limit() -> PlateError {
    PlateError {
        rule_id: None,
        code: ErrorCode::LimitExceeded,
        message: "source archive exceeds existing model budgets",
    }
}

fn check_sizes(manifest: &Manifest) -> Result<(), PlateError> {
    let entries = manifest.content().entries();
    let budgets = Budgets::MODEL;
    if entries.len() > budgets.layers {
        return Err(limit());
    }
    let mut total = 0_u64;
    for entry in entries {
        if entry.size() > budgets.layer_bytes as u64 {
            return Err(limit());
        }
        total = total.checked_add(entry.size()).ok_or_else(limit)?;
        if total > budgets.total_bytes as u64 {
            return Err(limit());
        }
        // The retained resource must also fit the host's existing virtual-name cap.
        if SOURCE_PREFIX.len() + entry.name().len() > MAX_NAME_BYTES {
            return Err(limit());
        }
    }
    Ok(())
}

impl SourceArchive {
    /// Captures one exact head under a shared maintenance guard before compaction can remove it.
    /// Ordinary model commits may continue. This owns a separate store handle for the whole call;
    /// all capture handles/leases are dropped before returning independent immutable byte resources.
    pub fn capture(root: &Path) -> Result<Self, PlateError> {
        let blobs = BlobStore::new(root).map_err(|e| PlateError {
            rule_id: None,
            code: e.code(),
            message: "source store is not usable",
        })?;
        let _guard = StoreLease::shared(&blobs).map_err(|e| PlateError {
            rule_id: None,
            code: e.code(),
            message: "source maintenance guard is unavailable",
        })?;
        let store = RevisionStore::new(blobs);
        let head = store
            .head()?
            .ok_or_else(|| integrity("source model has no revision"))?;
        check_sizes(head.manifest())?;
        let revision = store.open_at(head.sequence())?;
        if revision.info() != &head {
            return Err(integrity("captured source identity changed"));
        }
        let content = revision.manifest().content();
        let bad_id = |_| integrity("source manifest has an invalid identity");
        let pin = SourceModelPin {
            project: ProjectId::parse(content.project().as_str()).map_err(bad_id)?,
            revision: RevisionId::parse(content.revision().as_str()).map_err(bad_id)?,
            manifest: head.manifest_digest(),
            schema: content.schema(),
            generation: content.workspace_generation(),
        };
        let mut resources = Vec::with_capacity(content.entries().len() + 1);
        resources.push(Asset {
            name: MANIFEST_RESOURCE.to_owned(),
            digest: pin.manifest,
            bytes: revision.manifest().encode(),
        });
        for entry in content.entries() {
            resources.push(Asset {
                name: format!("{SOURCE_PREFIX}{}", entry.name()),
                digest: entry.digest(),
                bytes: revision.layer_bytes(entry.name())?,
            });
        }
        Self::verify(pin, resources)
    }

    /// Checks untrusted retained resources against the exact externally selected source pin,
    /// including manifest/project/revision/schema/generation, every layer name/size/digest and
    /// the normal USD/schema/index/kernel checks. Missing/extra/duplicate resources refuse.
    pub fn verify(pin: SourceModelPin, resources: Vec<Asset>) -> Result<Self, PlateError> {
        if resources.is_empty() || resources.len() > Budgets::MODEL.layers + 1 {
            return Err(limit());
        }
        let mut total = 0usize;
        for resource in &resources {
            if resource.name.len() > MAX_NAME_BYTES {
                return Err(limit());
            }
            let cap = if resource.name == MANIFEST_RESOURCE {
                MAX_MANIFEST_BYTES
            } else {
                Budgets::MODEL.layer_bytes
            };
            if resource.bytes.len() > cap {
                return Err(limit());
            }
            total = total.checked_add(resource.bytes.len()).ok_or_else(limit)?;
            if total > Budgets::MODEL.total_bytes + MAX_MANIFEST_BYTES {
                return Err(limit());
            }
            if Sha256::digest(&resource.bytes).map_err(|_| limit())? != resource.digest {
                return Err(integrity("source archive resource digest disagrees"));
            }
        }
        let provenance = resources
            .iter()
            .find(|r| r.name == MANIFEST_RESOURCE)
            .ok_or_else(|| integrity("source manifest resource is missing"))?;
        if provenance.digest != pin.manifest {
            return Err(integrity(
                "archived source manifest disagrees with drawing pin",
            ));
        }
        let manifest = Manifest::parse(&provenance.bytes)?;
        check_sizes(&manifest)?;
        let content = manifest.content();
        if content.project().as_str() != pin.project.to_string()
            || content.revision().as_str() != pin.revision.to_string()
            || content.schema() != pin.schema
            || content.workspace_generation() != pin.generation
            || content.schema() != schema_identity()?.digest
        {
            return Err(integrity(
                "archived source identity or model schema disagrees",
            ));
        }
        if resources.len() != content.entries().len() + 1 {
            return Err(integrity("source archive resource set is incomplete"));
        }
        let archive = Self {
            pin,
            manifest,
            resources,
        };
        archive.inspect()?;
        Ok(archive)
    }

    /// Exact captured source identity, independent of any current live head.
    pub const fn pin(&self) -> SourceModelPin {
        self.pin
    }

    /// Exact resources to retain as content-addressed document resources, outside its USD stage.
    pub fn resources(&self) -> &[Asset] {
        &self.resources
    }

    /// Strictly reopens the archived model under its original names, without any source-store I/O.
    pub fn inspect(&self) -> Result<Inspection, PlateError> {
        let mut assets = Vec::with_capacity(self.manifest.content().entries().len());
        for entry in self.manifest.content().entries() {
            let name = format!("{SOURCE_PREFIX}{}", entry.name());
            let mut matching = self.resources.iter().filter(|r| r.name == name);
            let asset = matching
                .next()
                .ok_or_else(|| integrity("archived source layer is missing"))?;
            if matching.next().is_some()
                || asset.digest != entry.digest()
                || asset.bytes.len() as u64 != entry.size()
            {
                return Err(integrity("archived source layer identity disagrees"));
            }
            assets.push(Asset {
                name: entry.name().to_owned(),
                digest: asset.digest,
                bytes: asset.bytes.clone(),
            });
        }
        inspect_manifest_assets(&self.manifest, self.pin.manifest, assets)
    }
}
