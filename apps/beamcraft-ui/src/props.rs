// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! V06 properties and publication, as pure logic: the CLI's `plate-inspect` reply becomes the
//! full properties of the selected element (dimensions, profile, material, volume and mass with
//! their statuses), a property edit becomes the exact `update-element` argument list, and the
//! workspace replies become the publication panel's state. Picking itself lives in
//! [`crate::edit`] (V05.2); nothing here touches a window, a GPU or a process, so all of it is
//! unit-testable, and [`crate::command`] runs the processes.
//!
//! Like the create tools, the UI computes no engineering geometry: every property value is the
//! engine's report, and a property edit forwards the typed patch to `update-element`, whose
//! refusal is shown as it is.

use crate::tools::{FormError, MAX_TEXT, ToolError, number, reply_json, text};
use beamcraft_protocol::json::Json;
use std::path::Path;

/// A definition digest with the readable name the CLI reports for it (when it has one).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Named {
    /// The definition's content digest (what an update command takes).
    pub digest: String,
    /// The definition's display name, or `None` when it was published without one.
    pub name: Option<String>,
}

impl Named {
    /// A short panel label: the name when there is one, else the digest head.
    pub fn label(&self) -> &str {
        match &self.name {
            Some(name) if !name.is_empty() => name,
            _ => self.digest.get(..8).unwrap_or(&self.digest),
        }
    }
}

/// The properties of one element, exactly as the CLI's `plate-inspect` reports them.
#[derive(Debug, Clone, PartialEq)]
pub struct Properties {
    /// The element's id.
    pub element_id: String,
    /// The element's kind (`plate`, `beam`, `column`).
    pub kind: String,
    /// The element's name.
    pub name: String,
    /// The dimensions the engine reports (millimetres), in a fixed key order.
    pub dimensions: Vec<(&'static str, f64)>,
    /// The pinned profile definition, for a member.
    pub profile: Option<Named>,
    /// The pinned material definition, when the element has one.
    pub material: Option<Named>,
    /// The placement translation (millimetres).
    pub translation_mm: [f64; 3],
    /// The volume in mm3, when the engine reports a value.
    pub volume_mm3: Option<f64>,
    /// The volume's status (`current` or `unknown`).
    pub volume_status: String,
    /// The mass in kg, when the engine reports a value.
    pub mass_kg: Option<f64>,
    /// The mass's status (`current` or `unknown`).
    pub mass_status: String,
}

impl Properties {
    /// One reported dimension by its key.
    pub fn dimension(&self, key: &str) -> Option<f64> {
        self.dimensions
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| *v)
    }
}

fn field<'a>(json: &'a Json, key: &str) -> Option<&'a Json> {
    json.get(key).ok()
}

/// Reads one element record of `plate-inspect` into [`Properties`].
fn properties(element: &Json) -> Result<Properties, ToolError> {
    let unreadable = || ToolError::local("UNREADABLE_REPLY", "the CLI reply lacks a field");
    let point = |json: &Json| -> Result<[f64; 3], ToolError> {
        let values = json.array().map_err(|_| unreadable())?;
        if values.len() < 3 {
            return Err(unreadable());
        }
        let mut xyz = [0.0; 3];
        for (value, item) in std::iter::zip(&mut xyz, values) {
            *value = item.float().map_err(|_| unreadable())?;
        }
        Ok(xyz)
    };
    let named = |digest_key: &str, name_key: &str| -> Option<Named> {
        text(element, digest_key).map(|digest| Named {
            digest,
            name: text(element, name_key),
        })
    };
    let dimensions = field(element, "dimensions_mm")
        .or_else(|| field(element, "section_mm"))
        .map(|size| {
            ["width", "length", "thickness", "height"]
                .into_iter()
                .filter_map(|key| {
                    field(size, key)
                        .and_then(|v| v.float().ok())
                        .map(|v| (key, v))
                })
                .collect()
        })
        .unwrap_or_default();
    let translation = field(element, "placement")
        .and_then(|p| field(p, "translation_mm"))
        .map_or(Ok([0.0; 3]), point)?;
    let quantity = |key: &str, value: &str| -> (Option<f64>, String) {
        field(element, "quantities")
            .and_then(|q| field(q, key))
            .map_or((None, String::new()), |q| {
                (
                    field(q, value).and_then(|v| v.float().ok()),
                    text(q, "status").unwrap_or_default(),
                )
            })
    };
    let (volume_mm3, volume_status) = quantity("volume", "mm3");
    let (mass_kg, mass_status) = quantity("mass", "kg");
    Ok(Properties {
        element_id: text(element, "element_id").ok_or_else(unreadable)?,
        kind: text(element, "kind").unwrap_or_default(),
        name: text(element, "name").unwrap_or_default(),
        dimensions,
        profile: named("profile_digest", "profile_name"),
        material: named("material_digest", "material_name"),
        translation_mm: translation,
        volume_mm3,
        volume_status,
        mass_kg,
        mass_status,
    })
}

/// The most elements one element lookup walks (the reply itself is capped by the output limit).
pub const MAX_LOOKUP: usize = 10_000;

/// Finds one element of a `plate-inspect` reply by its id; `None` when it is not in the reply.
pub fn parse_element(
    exit_code: Option<i32>,
    stdout: &[u8],
    id: &str,
) -> Result<Option<Properties>, ToolError> {
    let reply = reply_json(exit_code, stdout)?;
    let unreadable = || ToolError::local("UNREADABLE_REPLY", "the CLI reply lacks a field");
    for list in ["plates", "members"] {
        let elements = reply
            .get(list)
            .and_then(Json::array)
            .map_err(|_| unreadable())?;
        for element in elements.iter().take(MAX_LOOKUP) {
            if text(element, "element_id").as_deref() == Some(id) {
                return Ok(Some(properties(element)?));
            }
        }
    }
    Ok(None)
}

/// What a successful change reply reports: the outcome name, the touched element (when the
/// command has one) and the accepted revision's sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// The CLI's outcome (`element_updated`, `workspace_published`, `workspace_undone`, ...).
    pub outcome: String,
    /// The element the command touched, when the reply names one.
    pub element_id: Option<String>,
    /// The accepted revision's sequence number, when the reply reports it.
    pub sequence: Option<u64>,
}

/// Reads a successful change reply (`update-element`, `workspace-publish`, `workspace-undo`,
/// `workspace-redo`).
pub fn parse_change(exit_code: Option<i32>, stdout: &[u8]) -> Result<Change, ToolError> {
    let reply = reply_json(exit_code, stdout)?;
    let unreadable = || ToolError::local("UNREADABLE_REPLY", "the CLI reply lacks a field");
    let sequence = reply
        .get("revision")
        .ok()
        .and_then(|revision| text(revision, "sequence"))
        .and_then(|s| s.parse().ok());
    Ok(Change {
        outcome: text(&reply, "outcome").ok_or_else(unreadable)?,
        element_id: reply
            .get("element")
            .ok()
            .and_then(|element| text(element, "element_id")),
        sequence,
    })
}

/// What to do with an element's material in an update.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum MaterialEdit {
    /// Leave the material as it is (no argument).
    #[default]
    Keep,
    /// `--material-digest none`: remove the material.
    Clear,
    /// Pin this definition digest.
    Set(String),
}

/// The property edits as typed. An untouched field changes nothing: `update-element` is a typed
/// patch and only receives the options the user actually edited.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpdateForm {
    /// `--name`; empty changes nothing.
    pub name: String,
    /// `--material-digest`.
    pub material: MaterialEdit,
    /// `--thickness` (a plate); empty changes nothing.
    pub thickness: String,
    /// `--translation X,Y,Z`, absolute; all three empty change nothing.
    pub translation: [String; 3],
}

impl UpdateForm {
    /// The `update-element` argument list for this form on `store`, selecting `id`.
    pub fn prepare(&self, store: &Path, id: &str) -> Result<Vec<String>, FormError> {
        let mut args = vec![
            "update-element".to_owned(),
            "--store".into(),
            store.to_string_lossy().into_owned(),
            "--id".into(),
            id.to_owned(),
        ];
        let mut changed = false;
        let name = self.name.trim();
        if name.chars().count() > MAX_TEXT {
            return Err(FormError {
                field: "Name",
                message: format!("at most {MAX_TEXT} characters"),
            });
        }
        if !name.is_empty() {
            args.extend(["--name".into(), name.to_owned()]);
            changed = true;
        }
        match &self.material {
            MaterialEdit::Keep => {}
            MaterialEdit::Clear => {
                args.extend(["--material-digest".into(), "none".into()]);
                changed = true;
            }
            MaterialEdit::Set(digest) => {
                args.extend(["--material-digest".into(), digest.clone()]);
                changed = true;
            }
        }
        let thickness = self.thickness.trim();
        if !thickness.is_empty() {
            args.extend(["--thickness".into(), number("Thickness", thickness)?]);
            changed = true;
        }
        let typed = self.translation.iter().any(|t| !t.trim().is_empty());
        if typed {
            let mut xyz = Vec::with_capacity(3);
            for t in &self.translation {
                xyz.push(number("Translation", t)?);
            }
            args.extend(["--translation".into(), xyz.join(",")]);
            changed = true;
        }
        if !changed {
            return Err(FormError {
                field: "Properties",
                message: "change at least one property, or cancel".into(),
            });
        }
        Ok(args)
    }
}

/// The properties panel's state machine: edit, preview (stage one update), commit or cancel.
/// Commit runs only what was previewed and only while the form still produces that command.
/// Selecting another element starts from an empty form.
#[derive(Debug, Clone, Default)]
pub struct Props {
    /// The typed edits.
    pub form: UpdateForm,
    staged: Option<Vec<String>>,
    busy: bool,
}

impl Props {
    /// Stages the form's update for review; nothing is run or written.
    pub fn preview(&mut self, store: &Path, id: &str) -> Result<(), FormError> {
        self.staged = Some(self.form.prepare(store, id)?);
        Ok(())
    }

    /// The staged argument list, if any.
    pub fn staged(&self) -> Option<&[String]> {
        self.staged.as_deref()
    }

    /// True while a committed update runs.
    pub fn busy(&self) -> bool {
        self.busy
    }

    /// True when an update is staged and the form still produces it.
    pub fn can_commit(&self, store: &Path, id: &str) -> bool {
        !self.busy
            && self
                .staged
                .as_ref()
                .is_some_and(|s| self.form.prepare(store, id).is_ok_and(|now| now == *s))
    }

    /// Drops the staged update. Nothing was run, so nothing is published.
    pub fn cancel(&mut self) {
        self.staged = None;
    }

    /// Takes the staged update to run and marks the panel busy; `None` when nothing is staged,
    /// the form changed since the preview, or an update is already running.
    pub fn begin_commit(&mut self, store: &Path, id: &str) -> Option<Vec<String>> {
        if !self.can_commit(store, id) {
            return None;
        }
        self.busy = true;
        self.staged.take()
    }

    /// Ends the running update; the next edit starts from the same form values.
    pub fn finish(&mut self) {
        self.busy = false;
    }
}

/// Where a draft stands against the published head, as `workspace-status` reports it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspaceState {
    /// True while the published head is still the revision the draft forked from; a moved head
    /// makes the next publish fail (`BASE_REVISION_MISMATCH`, fast-forward only).
    pub base_is_current: bool,
    /// Draft revisions not published yet.
    pub unsent_revisions: u64,
    /// The draft head: revision id and sequence.
    pub draft_head: (String, u64),
    /// The published head: revision id and sequence.
    pub published_head: (String, u64),
}

/// The publication panel's state: the workspace status plus the undo and redo units the draft
/// can restore (from `workspace-history`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspacePanel {
    /// The workspace status.
    pub state: WorkspaceState,
    /// Undo units the draft can restore.
    pub undo_units: u64,
    /// Redo units the draft can restore.
    pub redo_units: u64,
}

fn revision(json: &Json) -> Result<(String, u64), ToolError> {
    let unreadable = || ToolError::local("UNREADABLE_REPLY", "the CLI reply lacks a field");
    let id = text(json, "revision_id").ok_or_else(unreadable)?;
    let sequence = text(json, "sequence")
        .and_then(|s| s.parse().ok())
        .ok_or_else(unreadable)?;
    Ok((id, sequence))
}

/// Reads the reply of `workspace-status`.
pub fn parse_workspace_state(
    exit_code: Option<i32>,
    stdout: &[u8],
) -> Result<WorkspaceState, ToolError> {
    let reply = reply_json(exit_code, stdout)?;
    let unreadable = || ToolError::local("UNREADABLE_REPLY", "the CLI reply lacks a field");
    let workspace = reply.get("workspace").map_err(|_| unreadable())?;
    let heads = |key: &str| -> Result<(String, u64), ToolError> {
        workspace
            .get(key)
            .map_err(|_| unreadable())
            .and_then(revision)
    };
    Ok(WorkspaceState {
        base_is_current: workspace
            .get("base_is_current")
            .ok()
            .and_then(|v| v.boolean().ok())
            .unwrap_or(false),
        unsent_revisions: text(workspace, "unsent_revisions")
            .and_then(|s| s.parse().ok())
            .ok_or_else(unreadable)?,
        draft_head: heads("draft_head")?,
        published_head: heads("published_head")?,
    })
}

/// Reads the reply of `workspace-history`: the undo and redo units of the draft.
pub fn parse_history(exit_code: Option<i32>, stdout: &[u8]) -> Result<(u64, u64), ToolError> {
    let reply = reply_json(exit_code, stdout)?;
    let unreadable = || ToolError::local("UNREADABLE_REPLY", "the CLI reply lacks a field");
    let units = |key: &str| -> Result<u64, ToolError> {
        text(&reply, key)
            .and_then(|s| s.parse().ok())
            .ok_or_else(unreadable)
    };
    Ok((units("undo_units")?, units("redo_units")?))
}

/// The arguments of one workspace command that names both stores, exactly as a user types them
/// (`workspace-status`, `workspace-publish`).
pub fn workspace_args(command: &str, published: &Path, dir: &Path) -> Vec<String> {
    vec![
        command.to_owned(),
        "--store".into(),
        published.to_string_lossy().into_owned(),
        "--workspace-dir".into(),
        dir.to_string_lossy().into_owned(),
    ]
}

/// The arguments of one workspace command that reads the draft only (`workspace-history`,
/// `workspace-undo`, `workspace-redo`).
pub fn draft_args(command: &str, dir: &Path) -> Vec<String> {
    vec![
        command.to_owned(),
        "--workspace-dir".into(),
        dir.to_string_lossy().into_owned(),
    ]
}

/// The arguments of `workspace-history`, which reads the draft only.
pub fn history_args(dir: &Path) -> Vec<String> {
    draft_args("workspace-history", dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> &'static Path {
        Path::new("S")
    }

    const DIGEST: &str = "5d660dddce23e62c2544f796a80fd32de3725defa13dee514572833f0d886cb1";
    const MATERIAL: &str = "36281e26dd4affda6b39d884fe8a8bb04deb71684215741c188dac0806710ba9";

    const PLATE: &str = r#"{"element_id":"e1","kind":"plate","name":"P1","material_digest":"36281e26dd4affda6b39d884fe8a8bb04deb71684215741c188dac0806710ba9","dimensions_mm":{"width":1000.0,"length":500.0,"thickness":10.0},"placement":{"translation_mm":[5.0,0.0,-2.0],"orientation_xyzw":[0.0,0.0,0.0,1.0]},"quantities":{"volume":{"mm3":5000000.0,"m3":0.005,"status":"current","algorithm":"w"}},"mesh_digest":"m1"}"#;

    const MEMBER: &str = r#"{"element_id":"e2","kind":"column","name":"C1","profile_digest":"5d660dddce23e62c2544f796a80fd32de3725defa13dee514572833f0d886cb1","material_digest":null,"profile_name":"RECT 200x300","material_name":null,"section_mm":{"width":200.0,"height":300.0},"placement":{"translation_mm":[0.0,0.0,0.0],"orientation_xyzw":[0.0,0.0,0.0,1.0]},"quantities":{"volume":{"mm3":180000000.0,"m3":0.18,"status":"current","algorithm":"w"},"mass":{"kg":1413.0,"density_kg_m3":7850.0,"status":"current"}},"mesh_digest":"m2"}"#;

    fn inspection(plates: &str, members: &str) -> String {
        format!(
            r#"{{"outcome":"plate_inspection","revision":{{"revision_id":"r1","sequence":"7","manifest_digest":"d1"}},"plate_count":1,"member_count":1,"other_element_count":0,"plates":[{plates}],"members":[{members}],"project_id_record":null,"levels":[],"snapshot":{{"synthetic":false,"token":"t","pin_revision":"r1","pin_generation":"1","elements":[],"meshes":[]}}}}"#
        )
    }

    #[test]
    fn a_plate_reply_is_read_with_its_statuses() {
        let reply = inspection(PLATE, "");
        let found = parse_element(Some(0), reply.as_bytes(), "e1")
            .unwrap()
            .unwrap();
        assert_eq!(found.element_id, "e1");
        assert_eq!(found.kind, "plate");
        assert_eq!(found.name, "P1");
        assert_eq!(found.dimension("width"), Some(1000.0));
        assert_eq!(found.dimension("length"), Some(500.0));
        assert_eq!(found.dimension("thickness"), Some(10.0));
        assert_eq!(found.profile, None);
        assert_eq!(
            found.material,
            Some(Named {
                digest: MATERIAL.to_owned(),
                name: None,
            })
        );
        assert_eq!(found.translation_mm, [5.0, 0.0, -2.0]);
        assert_eq!(found.volume_mm3, Some(5_000_000.0));
        assert_eq!(found.volume_status, "current");
        assert_eq!(found.mass_kg, None);
        assert_eq!(found.mass_status, "");
    }

    #[test]
    fn a_member_reply_is_read_with_its_profile() {
        let reply = inspection("", MEMBER);
        let found = parse_element(Some(0), reply.as_bytes(), "e2")
            .unwrap()
            .unwrap();
        assert_eq!(found.kind, "column");
        assert_eq!(found.dimension("width"), Some(200.0));
        assert_eq!(found.dimension("height"), Some(300.0));
        assert_eq!(found.dimension("thickness"), None);
        assert_eq!(
            found.profile,
            Some(Named {
                digest: DIGEST.to_owned(),
                name: Some("RECT 200x300".to_owned()),
            })
        );
        assert_eq!(found.material, None);
        assert_eq!(found.mass_kg, Some(1413.0));
        assert_eq!(found.mass_status, "current");
        assert_eq!(
            found.profile.as_ref().map(Named::label).unwrap(),
            "RECT 200x300"
        );
    }

    #[test]
    fn a_missing_element_is_none_and_a_broken_reply_is_an_error() {
        let reply = inspection(PLATE, MEMBER);
        assert_eq!(
            parse_element(Some(0), reply.as_bytes(), "gone").unwrap(),
            None
        );
        assert!(parse_element(Some(0), b"{}", "e1").is_err());
        let refused = br#"{"code":"NOT_FOUND","message":"no revision","outcome":"rejected"}"#;
        assert_eq!(
            parse_element(Some(2), refused, "e1").unwrap_err().code,
            "NOT_FOUND"
        );
    }

    #[test]
    fn a_change_reply_is_read() {
        let reply = br#"{"outcome":"element_updated","op_id":"o","effect":{"name":true,"material":false,"shape":false,"placement":false,"level":false},"changed_layers":["authored.usda"],"element":{"element_id":"e1","kind":"plate","name":"P2"},"revision":{"revision_id":"r2","sequence":"8","manifest_digest":"d2"}}"#;
        let change = parse_change(Some(0), reply).unwrap();
        assert_eq!(change.outcome, "element_updated");
        assert_eq!(change.element_id.as_deref(), Some("e1"));
        assert_eq!(change.sequence, Some(8));
        let refused =
            br#"{"code":"BASE_REVISION_MISMATCH","message":"moved","outcome":"rejected"}"#;
        assert_eq!(
            parse_change(Some(2), refused).unwrap_err().code,
            "BASE_REVISION_MISMATCH"
        );
    }

    #[test]
    fn an_update_form_maps_exactly_the_changed_properties() {
        let mut form = UpdateForm::default();
        // Nothing typed: nothing is staged.
        assert_eq!(form.prepare(store(), "e1").unwrap_err().field, "Properties");
        form.name = "  P2 ".into();
        form.material = MaterialEdit::Clear;
        form.thickness = "12.5".into();
        form.translation = ["10".into(), " 0 ".into(), "-3".into()];
        assert_eq!(
            form.prepare(store(), "e1").unwrap(),
            [
                "update-element",
                "--store",
                "S",
                "--id",
                "e1",
                "--name",
                "P2",
                "--material-digest",
                "none",
                "--thickness",
                "12.5",
                "--translation",
                "10,0,-3"
            ]
        );
        // A pinned material passes the digest; the untouched fields stay absent.
        let one = UpdateForm {
            material: MaterialEdit::Set(MATERIAL.to_owned()),
            ..UpdateForm::default()
        };
        assert_eq!(
            one.prepare(store(), "e1").unwrap(),
            [
                "update-element",
                "--store",
                "S",
                "--id",
                "e1",
                "--material-digest",
                MATERIAL
            ]
        );
        // Engineering rules stay the engine's; a bad number is refused before anything runs.
        let negative = UpdateForm {
            thickness: "-4".into(),
            ..UpdateForm::default()
        };
        assert!(negative.prepare(store(), "e1").is_ok());
        let bad = UpdateForm {
            translation: ["1".into(), "x".into(), String::new()],
            ..UpdateForm::default()
        };
        assert_eq!(bad.prepare(store(), "e1").unwrap_err().field, "Translation");
        let empty_in_middle = UpdateForm {
            translation: ["1".into(), String::new(), "3".into()],
            ..UpdateForm::default()
        };
        assert_eq!(
            empty_in_middle.prepare(store(), "e1").unwrap_err().field,
            "Translation"
        );
        let long = UpdateForm {
            name: "n".repeat(MAX_TEXT + 1),
            ..UpdateForm::default()
        };
        assert_eq!(long.prepare(store(), "e1").unwrap_err().field, "Name");
    }

    #[test]
    fn the_update_state_machine_commits_only_what_was_previewed() {
        let mut props = Props::default();
        // Nothing staged: nothing to commit.
        assert!(!props.can_commit(store(), "e1"));
        assert!(props.begin_commit(store(), "e1").is_none());
        // A form error stages nothing.
        assert!(props.preview(store(), "e1").is_err());
        assert!(props.staged().is_none());
        props.form.name = "P2".into();
        props.preview(store(), "e1").unwrap();
        let staged = props.staged().unwrap().to_owned();
        assert!(props.can_commit(store(), "e1"));
        // Editing after the preview invalidates it; another element does not accept it either.
        props.form.name = "P3".into();
        assert!(!props.can_commit(store(), "e1"));
        assert!(props.begin_commit(store(), "e2").is_none());
        props.form.name = "P2".into();
        assert_eq!(props.begin_commit(store(), "e2"), None);
        assert_eq!(props.begin_commit(store(), "e1"), Some(staged));
        assert!(props.busy());
        assert!(props.staged().is_none());
        assert!(props.begin_commit(store(), "e1").is_none());
        props.finish();
        assert!(!props.busy());
        // Cancelling drops the staged command.
        props.preview(store(), "e1").unwrap();
        props.cancel();
        assert!(props.staged().is_none());
    }

    const STATUS: &str = r#"{"outcome":"workspace_status","workspace":{"workspace_id":"w","user_id":"u","base_revision":"rb","base_is_current":true,"unsent_revisions":"2","draft_head":{"revision_id":"rd","sequence":"3","manifest_digest":"d"},"published_head":{"revision_id":"rb","sequence":"1","manifest_digest":"d"}}}"#;

    #[test]
    fn a_workspace_status_reply_is_read() {
        let state = parse_workspace_state(Some(0), STATUS.as_bytes()).unwrap();
        assert!(state.base_is_current);
        assert_eq!(state.unsent_revisions, 2);
        assert_eq!(state.draft_head, ("rd".to_owned(), 3));
        assert_eq!(state.published_head, ("rb".to_owned(), 1));
        let refused = br#"{"code":"BACKEND_UNAVAILABLE","message":"no meta","outcome":"rejected"}"#;
        assert_eq!(
            parse_workspace_state(Some(3), refused).unwrap_err().code,
            "BACKEND_UNAVAILABLE"
        );
    }

    #[test]
    fn a_history_reply_is_read() {
        let reply = br#"{"outcome":"workspace_history","draft_head":{"revision_id":"rd","sequence":"3","manifest_digest":"d"},"undo_units":"1","redo_units":"2","undo_restores_sequences":["2"],"redo_restores_sequences":["1","0"]}"#;
        assert_eq!(parse_history(Some(0), reply).unwrap(), (1, 2));
        assert!(parse_history(Some(0), b"{}").is_err());
    }

    #[test]
    fn workspace_arguments_are_exactly_what_a_user_types() {
        assert_eq!(
            workspace_args("workspace-publish", Path::new("PUB"), Path::new("WS")),
            [
                "workspace-publish",
                "--store",
                "PUB",
                "--workspace-dir",
                "WS"
            ]
        );
        assert_eq!(
            history_args(Path::new("WS")),
            ["workspace-history", "--workspace-dir", "WS"]
        );
        assert_eq!(
            draft_args("workspace-undo", Path::new("WS")),
            ["workspace-undo", "--workspace-dir", "WS"]
        );
    }
}
