// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! U19: how one store's edit commands route, decided once per command from the store's
//! layout and the workspace's authorized scope (F10 0.2 `DV-SCOPE-001..004`).
//!
//! A store whose manifest still carries the classic `authored.usda` keeps the unscoped 0.1
//! behaviour (`EditRouting::Classic`). A store organized into authored section/zone scopes
//! (`<code>[_<zone>]_<section>.usda` plus the partition's grids/levels layer
//! `<code>[_<zone>]_Grids.usda`, T01 §5.2/§6) answers only to a
//! workspace whose metadata authorizes one active section and zone partition; anything else
//! is refused with `PERMISSION_DENIED` — a file name, an installed module or a store path
//! grants no authority. The routing is derived again for every command from trusted inputs
//! (the verified manifest and the workspace metadata), never taken from the request.

use crate::plate::{AUTHORED_LAYER, DERIVED_LAYER, PlateError};
use crate::workspace::peek_meta;
use beamcraft_engine::scope::{self, WorkspaceScope};
use beamcraft_host::revision::{Revision, RevisionInfo, RevisionStore};
use beamcraft_layer_engine::authored::{Author, EditScope, EditTargets, SchemaIdentity};
use beamcraft_layer_engine::{Control, OpenRequest, Opened};
use std::collections::BTreeSet;

/// How the edit commands of one command run route their authoring.
#[derive(Debug, Clone)]
pub(crate) enum EditRouting {
    /// F10 0.1 store: the classic authored/derived pair, no scope.
    Classic,
    /// Authorized scope: the element routing plus the optional shared grids/levels routing.
    Scoped(Box<ScopedRouting>),
    /// A sectioned store without an authorized workspace scope; the error says why.
    Unauthorized(PlateError),
}

/// The routed targets and scopes of one authorized workspace.
#[derive(Debug, Clone)]
pub(crate) struct ScopedRouting {
    /// Where element commands author, and the scope they are admitted under.
    element: (EditTargets, EditScope),
    /// Where grid/level commands author, when the workspace is authorized for the shared
    /// scope.
    shared: Option<(EditTargets, EditScope)>,
}

impl EditRouting {
    fn classic() -> EditTargets {
        EditTargets {
            authored: AUTHORED_LAYER.into(),
            derived: DERIVED_LAYER.into(),
        }
    }

    /// The targets and scope of an ordinary element command.
    pub(crate) fn element(&self) -> Result<(EditTargets, Option<EditScope>), PlateError> {
        match self {
            EditRouting::Classic => Ok((Self::classic(), None)),
            EditRouting::Scoped(routing) => {
                Ok((routing.element.0.clone(), Some(routing.element.1.clone())))
            }
            EditRouting::Unauthorized(error) => Err(*error),
        }
    }

    /// The targets and scope of an explicitly shared grid/level command.
    pub(crate) fn shared(&self) -> Result<(EditTargets, Option<EditScope>), PlateError> {
        match self {
            EditRouting::Classic => Ok((Self::classic(), None)),
            EditRouting::Scoped(routing) => match &routing.shared {
                Some((targets, scope)) => Ok((targets.clone(), Some(scope.clone()))),
                None => Err(PlateError {
                    rule_id: Some(scope::RULE_SHARED_AUTHORIZED),
                    code: beamcraft_model::error::ErrorCode::PermissionDenied,
                    message: "the workspace is not authorized for the shared grids and levels",
                }),
            },
            EditRouting::Unauthorized(error) => Err(*error),
        }
    }
}

const fn unauthorized(message: &'static str) -> PlateError {
    PlateError {
        rule_id: Some(scope::RULE_SCOPE_AUTHORIZED),
        code: beamcraft_model::error::ErrorCode::PermissionDenied,
        message,
    }
}

const fn integrity(message: &'static str) -> PlateError {
    PlateError {
        rule_id: None,
        code: beamcraft_model::error::ErrorCode::IntegrityError,
        message,
    }
}

/// Derives the routing of one store from its verified head: the classic layout keeps the 0.1
/// behaviour, a sectioned one requires the workspace's authorized scope beside it.
pub(crate) fn routing_of(store: &RevisionStore, head: &RevisionInfo) -> EditRouting {
    let names: BTreeSet<&str> = head
        .manifest()
        .content()
        .entries()
        .iter()
        .map(|e| e.name())
        .collect();
    if names.contains(AUTHORED_LAYER) {
        return EditRouting::Classic;
    }
    let meta = match peek_meta(store.root()) {
        Ok(Some(meta)) => meta,
        Ok(None) => {
            return EditRouting::Unauthorized(unauthorized(
                "the store is organized into section scopes; edit it through a workspace \
                 authorized for one section and zone partition",
            ));
        }
        Err(error) => return EditRouting::Unauthorized(error),
    };
    let Some(layout) = meta.scope.as_ref() else {
        return EditRouting::Unauthorized(unauthorized(
            "the workspace holds no authorized section scope",
        ));
    };
    if layout.validate().is_err() {
        return EditRouting::Unauthorized(unauthorized(
            "the workspace's authorized scope is not usable",
        ));
    }
    let Ok(section_layer) = layout.section_layer() else {
        return EditRouting::Unauthorized(unauthorized(
            "the workspace's authorized scope is not usable",
        ));
    };
    let Ok(grids_layer) = layout.grids_layer() else {
        return EditRouting::Unauthorized(unauthorized(
            "the workspace's authorized scope is not usable",
        ));
    };
    if !names.contains(section_layer.as_str()) || !names.contains(grids_layer.as_str()) {
        return EditRouting::Unauthorized(integrity(
            "the authorized scope does not match the store's layer layout",
        ));
    }
    EditRouting::Scoped(Box::new(ScopedRouting {
        element: (
            EditTargets {
                authored: section_layer.clone(),
                derived: DERIVED_LAYER.into(),
            },
            EditScope {
                write_layer: section_layer.clone(),
                allowed: BTreeSet::from([section_layer]),
                shared: BTreeSet::from([grids_layer.clone()]),
                shared_authorized: false,
            },
        ),
        shared: layout.shared.then(|| {
            (
                EditTargets {
                    authored: grids_layer.clone(),
                    derived: DERIVED_LAYER.into(),
                },
                EditScope {
                    write_layer: grids_layer.clone(),
                    allowed: BTreeSet::from([grids_layer.clone()]),
                    shared: BTreeSet::from([grids_layer]),
                    shared_authorized: true,
                },
            )
        }),
    }))
}

/// Opens the authoring stage of one command with the routing derived from the store: element
/// commands (`shared = false`) author inside the active section/zone scope, grid/level
/// commands (`shared = true`) inside the authorized shared grids/levels scope. Classic 0.1
/// stores keep the unscoped behaviour.
pub(crate) fn open_author(
    store: &RevisionStore,
    request: OpenRequest,
    shared: bool,
    schema: SchemaIdentity,
    control: &Control,
) -> Result<(Author, Opened), PlateError> {
    // A store without a revision is still classic: its first command builds the initial
    // base (P01), which no section layout can exist for yet.
    let routing = match store.head().map_err(PlateError::from)? {
        Some(head) => routing_of(store, &head),
        None => EditRouting::Classic,
    };
    let (targets, scope) = if shared {
        routing.shared()?
    } else {
        routing.element()?
    };
    Author::open(request, targets, scope, schema, control).map_err(PlateError::from)
}

/// The edit targets of the rebase and compensation materializers: the workspace's own
/// recorded delta is replayed into its authorized section layer (U19). Without a scope this
/// is the classic 0.1 pair.
pub(crate) fn materializer_targets(
    scope: &Option<WorkspaceScope>,
) -> Result<EditTargets, PlateError> {
    match scope {
        None => Ok(EditRouting::classic()),
        Some(layout) => {
            let section = layout.section_layer().map_err(PlateError::from)?;
            Ok(EditTargets {
                authored: section,
                derived: DERIVED_LAYER.into(),
            })
        }
    }
}

/// Refuses immutable definitions (materials, profiles, fasteners) on a sectioned store
/// (`DV-SCOPE-004`): a definition is store-wide shared data, its resource batch carries no
/// element change for the scope admission to see, and the separately authorized shared
/// definition scope arrives with the F12 registry work (U20). Classic stores are unchanged.
pub(crate) fn refuse_definitions(store: &RevisionStore) -> Result<(), PlateError> {
    let Some(head) = store.head().map_err(PlateError::from)? else {
        return Ok(());
    };
    match routing_of(store, &head) {
        EditRouting::Classic => Ok(()),
        EditRouting::Scoped(_) | EditRouting::Unauthorized(_) => Err(PlateError {
            rule_id: Some(scope::RULE_SHARED_AUTHORIZED),
            code: beamcraft_model::error::ErrorCode::PermissionDenied,
            message: "definitions are shared data and are not writable from a section scope",
        }),
    }
}

/// Validates a scope a workspace asks to be authorized for against the published head's
/// layout: the codes must be valid and both the section and the shared layer must already
/// participate in the store (`DV-SCOPE-001`: explicit authorization against the store).
pub(crate) fn authorize(layout: &WorkspaceScope, head: &Revision) -> Result<(), PlateError> {
    layout.validate().map_err(PlateError::from)?;
    let names: BTreeSet<&str> = head
        .manifest()
        .content()
        .entries()
        .iter()
        .map(|e| e.name())
        .collect();
    let section = layout.section_layer().map_err(PlateError::from)?;
    let grids = layout.grids_layer().map_err(PlateError::from)?;
    if !names.contains(section.as_str()) || !names.contains(grids.as_str()) {
        return Err(PlateError::new(
            beamcraft_model::error::ErrorCode::NotFound,
            "the store has no layer for the requested section or zone scope",
        ));
    }
    Ok(())
}
