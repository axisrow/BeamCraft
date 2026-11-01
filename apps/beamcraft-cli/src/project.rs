// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Creating projects, and putting an environment into one.
//!
//! [`create_project`] makes the folder inside an existing parent and commits its first revision:
//! an empty model whose root layer records the ProjectId (D1). Reading project folders, suggesting
//! names and listing environments is platform state and lives in the `session` crate, which the
//! window uses as well.

use crate::library::load_environment;
use crate::publication::commit_verified;
use beamcraft_host::revision::{
    CommitHook, CommitOutcome, LayerEntry, LayerRole, RevisionDraft, RevisionStore,
};
use beamcraft_host::storage::BlobStore;
use beamcraft_layer_engine::authored::{PROJECT_RECORD_KEY, schema_identity};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{OpId, ProjectId, RevisionId, Uuid7, WorkspaceId};
use session::Refusal;
use session::environment::ENVIRONMENT_FILE;
pub use session::environment::{
    ENVIRONMENTS_DIR, EnvironmentEntry, EnvironmentState, builtin_environment_dirs,
    environment_folders, list_environments,
};
use session::pipeline::{DERIVED_LAYER, ROOT_LAYER, claim_project, host_uuid, load_base};
pub use session::project::{
    MAX_LISTED, MAX_SCANNED, ProjectInfo, ProjectState, is_store, list_projects, project_info,
    project_is_missing,
};
use session::project::{check_project_name, numbered_name};
use std::fs;
use std::path::{Path, PathBuf};

/// Folder of a project that holds the copy of its environment, made by `create-project`
/// `--environment`: the project carries the libraries it was created with, so it opens the same
/// on another machine and the next session needs no setting.
pub const PROJECT_ENVIRONMENT_DIR: &str = "environment";

/// The section/zone scopes of a new sectioned project (U19): one authored layer per scope
/// named `<code>[_<zone>]_<section>.usda`, plus the grids/levels layer
/// `<code>[_<zone>]_Grids.usda` of every zone partition the layout names (T01 §5.2/§6): the
/// unzoned scopes keep the common grids in `<code>_Grids.usda`, each zone keeps its own in
/// `<code>_<zone>_Grids.usda`. The codes are labels; the section and zone registries of F12
/// are a later package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectLayout {
    /// The project code the layer names are built from.
    pub code: String,
    /// One `(zone, section)` pair per authored section/zone scope.
    pub scopes: Vec<(Option<String>, String)>,
}

impl ProjectLayout {
    /// Parses `--code` and the `--sections` list ("KM", "K1:KMD"), validating every code.
    pub fn parse(code: &str, specs: &[&str]) -> Result<Self, Refusal> {
        beamcraft_engine::scope::check_code(code).map_err(Refusal::from)?;
        let mut scopes = Vec::new();
        for spec in specs {
            let (zone, section) = match spec.split_once(':') {
                Some((zone, section)) => (Some(zone.to_owned()), section),
                None => (None, *spec),
            };
            beamcraft_engine::scope::section_layer(code, zone.as_deref(), section)
                .map_err(Refusal::from)?;
            scopes.push((zone, section.to_owned()));
        }
        if scopes.is_empty() {
            return Err(Refusal::new(
                ErrorCode::InvalidArgument,
                "a sectioned project names at least one section",
            ));
        }
        Ok(Self {
            code: code.to_owned(),
            scopes,
        })
    }

    /// The authored layer names of the layout: one grids/levels layer per zone partition the
    /// scopes name and one layer per section/zone scope, deduplicated and sorted; the derived
    /// layer is not included. A partition whose grids layer is absent could never hold a grid,
    /// because a workspace is authorized only for a partition that has both of its layers.
    pub fn layer_names(&self) -> Result<Vec<String>, Refusal> {
        let mut names = std::collections::BTreeSet::new();
        let mut grids = |zone: Option<&str>| -> Result<(), Refusal> {
            names.insert(
                beamcraft_engine::scope::grids_layer(&self.code, zone).map_err(Refusal::from)?,
            );
            Ok(())
        };
        if self.scopes.iter().any(|(zone, _)| zone.is_none()) {
            grids(None)?;
        }
        for zone in self.scopes.iter().filter_map(|(zone, _)| zone.as_deref()) {
            grids(Some(zone))?;
        }
        for (zone, section) in &self.scopes {
            names.insert(
                beamcraft_engine::scope::section_layer(&self.code, zone.as_deref(), section)
                    .map_err(Refusal::from)?,
            );
        }
        Ok(names.into_iter().collect())
    }
}

const fn refuse(code: ErrorCode, message: &'static str) -> Refusal {
    Refusal::new(code, message)
}

const fn invalid(message: &'static str) -> Refusal {
    refuse(ErrorCode::InvalidArgument, message)
}

/// Identities of one project creation.
#[derive(Debug, Clone)]
pub struct ProjectContext {
    /// The logical operation of the first revision.
    pub op_id: OpId,
    /// Identity of the first revision.
    pub revision_id: RevisionId,
    /// The author (trusted context).
    pub actor: Uuid7,
    /// The new project's identity.
    pub project: ProjectId,
    /// The workspace that authors the first revision.
    pub workspace: WorkspaceId,
}

/// Creates the folder `parent/name` and commits the first revision of a new, empty project.
///
/// The parent must exist; a folder of that name must not. When the commit fails, the folder this
/// call created is removed again, so a failed creation leaves nothing behind.
pub fn create_project(
    parent: &Path,
    name: &str,
    context: &ProjectContext,
    hook: &mut dyn CommitHook,
) -> Result<ProjectInfo, Refusal> {
    create_named_project(parent, name, false, context, hook, None)
}

/// Creates a project organized into authored section/zone scopes (U19): the root composes one
/// authored layer per scope plus the shared grids/levels layer and the derived layer. Editing
/// it requires a workspace authorized for one scope (`create-workspace --scope-*`).
pub fn create_project_scoped(
    parent: &Path,
    name: &str,
    unique: bool,
    layout: &ProjectLayout,
    context: &ProjectContext,
    hook: &mut dyn CommitHook,
) -> Result<ProjectInfo, Refusal> {
    create_named_project(parent, name, unique, context, hook, Some(layout))
}

/// Create with the first free name: `name`, `name 1`, `name 2`, up to 10,000 candidates.
/// Directory reservation is atomic; an existing file, directory or link is never overwritten.
pub fn create_project_unique(
    parent: &Path,
    name: &str,
    context: &ProjectContext,
    hook: &mut dyn CommitHook,
) -> Result<ProjectInfo, Refusal> {
    create_named_project(parent, name, true, context, hook, None)
}

fn create_named_project(
    parent: &Path,
    name: &str,
    unique: bool,
    context: &ProjectContext,
    hook: &mut dyn CommitHook,
    layout: Option<&ProjectLayout>,
) -> Result<ProjectInfo, Refusal> {
    check_project_name(name)?;
    let parent =
        std::path::absolute(parent).map_err(|_| invalid("the path cannot be made absolute"))?;
    if !parent.is_dir() {
        return Err(refuse(
            ErrorCode::NotFound,
            "the parent folder does not exist",
        ));
    }
    let mut reserved = None;
    for index in 0..if unique { 10_000 } else { 1 } {
        let candidate = numbered_name(name, index);
        check_project_name(&candidate)?;
        let dir = parent.join(&candidate);
        match fs::create_dir(&dir) {
            Ok(()) => {
                reserved = Some(dir);
                break;
            }
            Err(error) if unique && error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(match e.kind() {
                    std::io::ErrorKind::AlreadyExists => refuse(
                        ErrorCode::DuplicateId,
                        "a file or folder of that name already exists",
                    ),
                    std::io::ErrorKind::PermissionDenied => {
                        refuse(ErrorCode::PermissionDenied, "the folder cannot be created")
                    }
                    _ => invalid("the folder cannot be created"),
                });
            }
        }
    }
    let dir = reserved.ok_or_else(|| {
        refuse(
            ErrorCode::LimitExceeded,
            "no free project name within the candidate limit",
        )
    })?;
    match first_revision(&dir, context, hook, layout) {
        Ok(()) => Ok(project_info(&dir)?),
        Err(error) => {
            // Only what this call made is removed: the folder did not exist before it.
            let _ = fs::remove_dir_all(&dir);
            Err(error)
        }
    }
}

/// Commits the initial layer set of a store without a revision: the root that records the
/// ProjectId, and empty authored and derived layers. An empty model is not an edit, so it is not
/// authored through the layer engine; it is the same base every first command starts from, and
/// it is verified at commit like every revision (U03 index, structural validation). A sectioned
/// project (U19) instead starts with one authored layer per section/zone scope plus the shared
/// grids/levels layer, composed by the root in that order.
fn first_revision(
    dir: &Path,
    context: &ProjectContext,
    hook: &mut dyn CommitHook,
    layout: Option<&ProjectLayout>,
) -> Result<(), Refusal> {
    let blobs = BlobStore::new(dir).map_err(|e| refuse(e.code(), "store root is not usable"))?;
    let store = RevisionStore::new(blobs);
    let _claim = claim_project(context.project);
    let (head, base) = load_base(&store, &context.project, &context.workspace)?;
    if head.is_some() {
        return Err(refuse(
            ErrorCode::DuplicateId,
            "the folder already holds a project",
        ));
    }
    let mut entries = Vec::with_capacity(base.assets.len());
    if let Some(layout) = layout {
        let mut names = layout.layer_names()?;
        names.push(session::pipeline::DERIVED_LAYER.to_owned());
        let root = scoped_root(&names, &context.project);
        for name in &names {
            let role = if name == session::pipeline::DERIVED_LAYER {
                LayerRole::Derived
            } else {
                LayerRole::Authored
            };
            store
                .blobs()
                .publish(session::pipeline::EMPTY_LAYER.as_bytes())
                .map_err(|e| refuse(e.code(), "layer blob could not be stored"))?;
            entries.push(LayerEntry::from_bytes(
                name,
                role,
                session::pipeline::EMPTY_LAYER.as_bytes(),
            )?);
        }
        store
            .blobs()
            .publish(root.as_bytes())
            .map_err(|e| refuse(e.code(), "layer blob could not be stored"))?;
        entries.push(LayerEntry::from_bytes(
            session::pipeline::ROOT_LAYER,
            LayerRole::Authored,
            root.as_bytes(),
        )?);
        // The manifest is strictly ascending by layer name.
        entries.sort_by(|a, b| a.name().cmp(b.name()));
    } else {
        for asset in &base.assets {
            store
                .blobs()
                .publish(&asset.bytes)
                .map_err(|e| refuse(e.code(), "layer blob could not be stored"))?;
            let role = if asset.name == DERIVED_LAYER {
                LayerRole::Derived
            } else {
                LayerRole::Authored
            };
            entries.push(LayerEntry::from_bytes(&asset.name, role, &asset.bytes)?);
        }
    }
    let draft = RevisionDraft {
        project: host_uuid(&base.project)?,
        workspace: host_uuid(&base.workspace)?,
        revision: host_uuid(&context.revision_id.to_string())?,
        op: host_uuid(&context.op_id.to_string())?,
        workspace_generation: base
            .generation
            .checked_add(1)
            .ok_or(refuse(ErrorCode::LimitExceeded, "generation exhausted"))?,
        schema: schema_identity()?.digest,
        root_layer: ROOT_LAYER.into(),
        entries,
    };
    let committed = commit_verified(&store, &draft, &base.expected, hook)?;
    if committed.outcome == CommitOutcome::Replayed {
        return Err(refuse(
            ErrorCode::IntegrityError,
            "a new project was reported as replayed",
        ));
    }
    Ok(())
}

/// The root layer of a sectioned project: it records the ProjectId (D1) and composes the
/// authored scope layers first and the derived layer last.
fn scoped_root(names: &[String], project: &ProjectId) -> String {
    let subs = names
        .iter()
        .map(|n| format!("@{n}@"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "#usda 1.0\n(\n    customLayerData = {{\n        string {PROJECT_RECORD_KEY} = \"{project}\"\n    }}\n    defaultPrim = \"World\"\n    metersPerUnit = 0.001\n    subLayers = [{subs}]\n    upAxis = \"Z\"\n)\n"
    )
}

/// The environment file of the project at `dir`, when the project was created with one.
pub fn project_environment_file(dir: &Path) -> Option<PathBuf> {
    let file = dir.join(PROJECT_ENVIRONMENT_DIR).join(ENVIRONMENT_FILE);
    file.is_file().then_some(file)
}

/// Copies the environment `file` and the libraries it lists into the project at `dir`
/// (`environment/`). The environment is read and admitted first, so a refused one writes nothing.
/// Commands of the project then resolve profile names against this copy.
pub fn install_environment(dir: &Path, file: &Path) -> Result<(), Refusal> {
    let path = file
        .to_str()
        .ok_or_else(|| invalid("the environment path is not UTF-8"))?;
    let environment = load_environment(path)?;
    let source = file.parent().unwrap_or_else(|| Path::new("."));
    let target = dir.join(PROJECT_ENVIRONMENT_DIR);
    let unusable = || {
        refuse(
            ErrorCode::BackendUnavailable,
            "the project environment could not be copied",
        )
    };
    fs::create_dir(&target).map_err(|_| unusable())?;
    fs::copy(file, target.join(ENVIRONMENT_FILE)).map_err(|_| unusable())?;
    for reference in &environment.info.libraries {
        let to = target.join(reference);
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent).map_err(|_| unusable())?;
        }
        fs::copy(source.join(reference), &to).map_err(|_| unusable())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

    use super::*;

    #[test]
    fn project_names_are_portable_folder_names() {
        for good in [
            "Каркас A-F 1-14",
            "307-KM",
            "a",
            "Nave.v2",
            "COM0",
            "Console",
        ] {
            assert!(check_project_name(good).is_ok(), "{good}");
        }
        let long = "x".repeat(101);
        for bad in [
            "", " a", "a ", "a.", ".", "..", "a/b", "a\\b", "a:b", "a*", "a?", "a\"", "a<", "a|",
            "a\tb", "CON", "con.txt", "COM1", "lpt9.x", "NUL", &long,
        ] {
            assert!(check_project_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_layout_names_one_grids_layer_per_zone_partition() {
        // Every partition the scopes name gets its grids layer: the common layer for the
        // unzoned scopes, the zone's own layer for each zone (T01 §5.2/§6).
        let mixed = ProjectLayout::parse("2401-DEMO", &["KM", "K1:KMD", "K2:KMD"]).unwrap();
        assert_eq!(
            mixed.layer_names().unwrap(),
            [
                "2401-DEMO_Grids.usda",
                "2401-DEMO_K1_Grids.usda",
                "2401-DEMO_K1_KMD.usda",
                "2401-DEMO_K2_Grids.usda",
                "2401-DEMO_K2_KMD.usda",
                "2401-DEMO_KM.usda",
            ]
        );
        // A layout without an unzoned scope has no common layer: nothing is authorized for a
        // partition whose section layer the store does not contain.
        let zoned = ProjectLayout::parse("2401-DEMO", &["K1:KM", "K2:KM"]).unwrap();
        assert_eq!(
            zoned.layer_names().unwrap(),
            [
                "2401-DEMO_K1_Grids.usda",
                "2401-DEMO_K1_KM.usda",
                "2401-DEMO_K2_Grids.usda",
                "2401-DEMO_K2_KM.usda",
            ]
        );
        // An unzoned project keeps the U19 shape: one common grids layer.
        let plain = ProjectLayout::parse("2402-DEMO", &["KM", "KMD"]).unwrap();
        assert_eq!(
            plain.layer_names().unwrap(),
            [
                "2402-DEMO_Grids.usda",
                "2402-DEMO_KM.usda",
                "2402-DEMO_KMD.usda"
            ]
        );
    }
}
