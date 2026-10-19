// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! V05.1/V05.2 tools, as pure logic: the entered parameters become the argument list of the same
//! `beamcraft create-plate` / `create-member` / `update-element` / `delete-element` commands the
//! CLI user types, and the CLI's JSON reply becomes a result or an error to show. Nothing here
//! touches a window, a GPU or a process, so all of it is unit-testable; [`crate::command`] runs
//! the process.
//!
//! The UI never computes engineering geometry. It checks only that a number is a finite decimal
//! and that required choices are made, then forwards the text. Every rule about shapes, limits,
//! profiles and materials stays in the engine, and its refusal (a stable code and a message) is
//! shown as it is.

use beamcraft_protocol::json::{self, Json};
use beamcraft_protocol::message;
use std::fmt;
use std::path::Path;

/// The longest accepted text field, in characters.
pub const MAX_TEXT: usize = 256;

/// The tools of this slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    /// A rectangular plate (`create-plate`).
    Plate,
    /// A straight beam or column (`create-member`).
    Member,
    /// A selected element moved to a new placement translation
    /// (`update-element --translation`, absolute, rotation kept).
    Move,
    /// A selected element removed (`delete-element`, no cascade).
    Delete,
}

impl ToolKind {
    /// The CLI command this tool runs.
    pub fn command(self) -> &'static str {
        match self {
            Self::Plate => "create-plate",
            Self::Member => "create-member",
            Self::Move => "update-element",
            Self::Delete => "delete-element",
        }
    }

    fn noun(self) -> Option<&'static str> {
        match self {
            Self::Plate => Some("plate"),
            Self::Member => Some("member"),
            Self::Move | Self::Delete => None,
        }
    }
}

/// The member kinds the CLI accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberKind {
    /// `--kind beam`.
    Beam,
    /// `--kind column`.
    Column,
}

impl MemberKind {
    /// The value of `--kind`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Beam => "beam",
            Self::Column => "column",
        }
    }
}

/// A refused form field: shown next to the tool, nothing is run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormError {
    /// The field label.
    pub field: &'static str,
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for FormError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.field, self.message)
    }
}

/// The plate form, as typed (millimetres).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlateForm {
    /// `--width`.
    pub width: String,
    /// `--length`.
    pub length: String,
    /// `--thickness`.
    pub thickness: String,
    /// `--name`, optional.
    pub name: String,
    /// `--material-digest`, optional: a material definition of the store.
    pub material: Option<String>,
}

/// The straight-member form, as typed (millimetres).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberForm {
    /// `--kind`.
    pub kind: MemberKind,
    /// `--start X,Y,Z`.
    pub start: [String; 3],
    /// `--end X,Y,Z`.
    pub end: [String; 3],
    /// `--profile-digest`: a profile definition of the store (required).
    pub profile: Option<String>,
    /// `--material-digest`, optional.
    pub material: Option<String>,
    /// `--name`, optional.
    pub name: String,
}

impl Default for MemberForm {
    fn default() -> Self {
        Self {
            kind: MemberKind::Beam,
            start: ["0".into(), "0".into(), "0".into()],
            end: ["1000".into(), "0".into(), "0".into()],
            profile: None,
            material: None,
            name: String::new(),
        }
    }
}

/// One command, ready to run: the arguments after the binary name and a display summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    /// The tool.
    pub kind: ToolKind,
    /// The arguments exactly as passed to the CLI, starting with the command name.
    pub args: Vec<String>,
}

impl Prepared {
    /// The argument list, one JSON string per line. It is not a shell command line: no shell
    /// quoting rule is claimed, and every argument (spaces, quotes, backslashes, control
    /// characters) is shown unambiguously as the process receives it.
    pub fn argument_list(&self) -> String {
        let mut list = String::new();
        for arg in &self.args {
            if !list.is_empty() {
                list.push('\n');
            }
            list.push_str(&crate::json_string(arg));
        }
        list
    }
}

fn number(field: &'static str, text: &str) -> Result<String, FormError> {
    let text = text.trim();
    if text.is_empty() || text.len() > 32 {
        return Err(FormError {
            field,
            message: "enter a number".into(),
        });
    }
    match text.parse::<f64>() {
        Ok(value) if value.is_finite() => Ok(text.to_owned()),
        _ => Err(FormError {
            field,
            message: format!("\"{text}\" is not a finite number"),
        }),
    }
}

fn point(field: &'static str, parts: &[String; 3]) -> Result<String, FormError> {
    let [x, y, z] = parts;
    Ok(format!(
        "{},{},{}",
        number(field, x)?,
        number(field, y)?,
        number(field, z)?
    ))
}

fn push_name(args: &mut Vec<String>, name: &str) -> Result<(), FormError> {
    let name = name.trim();
    if name.chars().count() > MAX_TEXT {
        return Err(FormError {
            field: "Name",
            message: format!("at most {MAX_TEXT} characters"),
        });
    }
    if !name.is_empty() {
        args.extend(["--name".into(), name.to_owned()]);
    }
    Ok(())
}

fn push_material(args: &mut Vec<String>, material: Option<&String>) {
    if let Some(digest) = material {
        args.extend(["--material-digest".into(), digest.clone()]);
    }
}

impl PlateForm {
    /// The `create-plate` command for this form on `store`.
    pub fn prepare(&self, store: &Path) -> Result<Prepared, FormError> {
        let mut args = vec![
            "create-plate".to_owned(),
            "--store".into(),
            store.to_string_lossy().into_owned(),
            "--width".into(),
            number("Width", &self.width)?,
            "--length".into(),
            number("Length", &self.length)?,
            "--thickness".into(),
            number("Thickness", &self.thickness)?,
        ];
        push_name(&mut args, &self.name)?;
        push_material(&mut args, self.material.as_ref());
        Ok(Prepared {
            kind: ToolKind::Plate,
            args,
        })
    }
}

impl MemberForm {
    /// The `create-member` command for this form on `store`.
    pub fn prepare(&self, store: &Path) -> Result<Prepared, FormError> {
        let start = point("Start", &self.start)?;
        let end = point("End", &self.end)?;
        let profile = self.profile.as_ref().ok_or_else(|| FormError {
            field: "Profile",
            message: "choose a profile definition of the store".into(),
        })?;
        let mut args = vec![
            "create-member".to_owned(),
            "--store".into(),
            store.to_string_lossy().into_owned(),
            "--kind".into(),
            self.kind.as_str().into(),
            "--start".into(),
            start,
            "--end".into(),
            end,
            "--profile-digest".into(),
            profile.clone(),
        ];
        push_name(&mut args, &self.name)?;
        push_material(&mut args, self.material.as_ref());
        Ok(Prepared {
            kind: ToolKind::Member,
            args,
        })
    }
}

/// One element of the loaded snapshot, as the panel's Elements list shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct ElementRow {
    /// The element id (what `--id` takes).
    pub id: String,
    /// The kind token ("plate", "beam", "column", ...).
    pub kind: String,
    /// The placement translation (millimetres) the snapshot reports.
    pub placement_mm: [f64; 3],
}

/// A refusal or failure of a command, with the CLI's stable code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolError {
    /// The CLI's stable error code (for example `INVALID_ARGUMENT`), or a local one
    /// (`UNREADABLE_REPLY`, `CLI_FAILED`).
    pub code: String,
    /// The CLI's message.
    pub message: String,
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl ToolError {
    pub(crate) fn local(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
        }
    }
}

/// The accepted revision a committed command reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revision {
    /// The revision's id.
    pub revision_id: String,
    /// The revision's manifest digest, which is what the view snapshot pins.
    pub manifest_digest: String,
    /// The revision's sequence number.
    pub sequence: u64,
}

fn read_revision(reply: &Json) -> Result<Revision, ToolError> {
    let unreadable = || ToolError::local("UNREADABLE_REPLY", "the CLI reply lacks a field");
    let revision = reply.get("revision").map_err(|_| unreadable())?;
    Ok(Revision {
        revision_id: text(revision, "revision_id").ok_or_else(unreadable)?,
        manifest_digest: text(revision, "manifest_digest").ok_or_else(unreadable)?,
        sequence: text(revision, "sequence")
            .and_then(|s| s.parse().ok())
            .ok_or_else(unreadable)?,
    })
}

/// The element object of a create or update reply, as the panel reads it.
struct ElementReply {
    element_id: String,
    name: String,
    mesh_digest: String,
    volume_mm3: Option<f64>,
}

/// Whether the reply's outcome names the applied (`false`) or the replayed (`true`) token of
/// `accepted`; any other outcome is an unreadable reply.
fn applied_or_replayed(reply: &Json, accepted: [&str; 2]) -> Result<bool, ToolError> {
    let outcome = text(reply, "outcome").unwrap_or_default();
    let replayed = outcome == accepted[1];
    if replayed || outcome == accepted[0] {
        Ok(replayed)
    } else {
        Err(ToolError::local(
            "UNREADABLE_REPLY",
            format!(
                "unexpected outcome \"{}\"",
                outcome.chars().take(40).collect::<String>()
            ),
        ))
    }
}

/// Reads the element object under `key`; the outcome must be one of `accepted[0]` (applied) or
/// `accepted[1]` (an already applied operation answered again).
fn element_reply(
    reply: &Json,
    key: &str,
    accepted: [&str; 2],
) -> Result<(ElementReply, bool), ToolError> {
    let replayed = applied_or_replayed(reply, accepted)?;
    let unreadable = || ToolError::local("UNREADABLE_REPLY", "the CLI reply lacks a field");
    let element = reply.get(key).map_err(|_| unreadable())?;
    let volume = element
        .get("quantities")
        .and_then(|q| q.get("volume"))
        .ok()
        .filter(|v| text(v, "status").as_deref() == Some("current"))
        .and_then(|v| v.get("mm3").ok())
        .and_then(|v| v.float().ok());
    Ok((
        ElementReply {
            element_id: text(element, "element_id").ok_or_else(unreadable)?,
            name: text(element, "name").unwrap_or_default(),
            mesh_digest: text(element, "mesh_digest").ok_or_else(unreadable)?,
            volume_mm3: volume,
        },
        replayed,
    ))
}

/// What a committed create command made, as the CLI reported it.
#[derive(Debug, Clone, PartialEq)]
pub struct Created {
    /// The tool.
    pub kind: ToolKind,
    /// The new element's id.
    pub element_id: String,
    /// The element's name.
    pub name: String,
    /// The digest of its mesh (shape key).
    pub mesh_digest: String,
    /// Its volume in mm3, when the CLI reports a current one.
    pub volume_mm3: Option<f64>,
    /// The accepted revision that contains it.
    pub revision: Revision,
    /// True when the CLI answered an already applied operation again.
    pub replayed: bool,
}

fn decode(stdout: &[u8]) -> Result<Json, ToolError> {
    json::decode(stdout, &message::read_limits())
        .map_err(|_| ToolError::local("UNREADABLE_REPLY", "the CLI reply is not valid JSON"))
}

fn text(json: &Json, key: &str) -> Option<String> {
    json.get(key)
        .ok()
        .and_then(|v| v.string().ok())
        .map(str::to_owned)
}

/// The refusal in a reply, if the reply is one.
fn rejection(reply: &Json, exit_code: Option<i32>) -> Option<ToolError> {
    let outcome = text(reply, "outcome");
    let rejected = matches!(outcome.as_deref(), Some("rejected" | "outcome_unknown"));
    if !rejected && exit_code == Some(0) {
        return None;
    }
    let code = text(reply, "code")
        .filter(|c| c.len() <= 48 && c.bytes().all(|b| b.is_ascii_uppercase() || b == b'_'))
        .unwrap_or_else(|| "CLI_FAILED".to_owned());
    let mut message = text(reply, "message").unwrap_or_else(|| match exit_code {
        Some(code) => format!("the CLI exited with status {code}"),
        None => "the CLI ended without a status".to_owned(),
    });
    if outcome.as_deref() == Some("outcome_unknown") {
        message.push_str(" (the result is unknown: reload to see whether it was accepted)");
    }
    message.truncate(message.floor_char_boundary(512));
    Some(ToolError { code, message })
}

/// Reads the reply of a create command: the created element or the CLI's refusal.
pub fn parse_created(
    kind: ToolKind,
    exit_code: Option<i32>,
    stdout: &[u8],
) -> Result<Created, ToolError> {
    let reply = match decode(stdout) {
        Ok(reply) => reply,
        Err(unreadable) => {
            return Err(match exit_code {
                Some(0) => unreadable,
                _ => ToolError::local("CLI_FAILED", "the CLI failed without a JSON reply"),
            });
        }
    };
    if let Some(error) = rejection(&reply, exit_code) {
        return Err(error);
    }
    let noun = kind
        .noun()
        .ok_or_else(|| ToolError::local("UNREADABLE_REPLY", "not a create command"))?;
    let (element, replayed) = element_reply(
        &reply,
        noun,
        [&format!("{noun}_created"), &format!("{noun}_replayed")],
    )?;
    Ok(Created {
        kind,
        element_id: element.element_id,
        name: element.name,
        mesh_digest: element.mesh_digest,
        volume_mm3: element.volume_mm3,
        revision: read_revision(&reply)?,
        replayed,
    })
}

/// What a committed move (`update-element`) changed, as the CLI reported it.
#[derive(Debug, Clone, PartialEq)]
pub struct Updated {
    /// The element's id.
    pub element_id: String,
    /// The element's name.
    pub name: String,
    /// Its volume in mm3, when the CLI reports a current one.
    pub volume_mm3: Option<f64>,
    /// The accepted revision that holds the change.
    pub revision: Revision,
    /// True when the CLI answered an already applied operation again.
    pub replayed: bool,
}

/// Reads the reply of an update command: the element after the change or the CLI's refusal.
pub fn parse_updated(exit_code: Option<i32>, stdout: &[u8]) -> Result<Updated, ToolError> {
    let reply = decode(stdout)?;
    if let Some(error) = rejection(&reply, exit_code) {
        return Err(error);
    }
    let (element, replayed) = element_reply(
        &reply,
        "element",
        ["element_updated", "element_update_replayed"],
    )?;
    Ok(Updated {
        element_id: element.element_id,
        name: element.name,
        volume_mm3: element.volume_mm3,
        revision: read_revision(&reply)?,
        replayed,
    })
}

/// What a committed delete command removed, as the CLI reported it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deleted {
    /// The deleted element's id.
    pub element_id: String,
    /// The kind the CLI removed ("plate", "member", ...); `None` on a replay, where the element
    /// is already absent.
    pub kind: Option<String>,
    /// The accepted revision that no longer holds it.
    pub revision: Revision,
    /// True when the CLI answered an already applied operation again.
    pub replayed: bool,
}

/// Reads the reply of a delete command: what was removed or the CLI's refusal.
pub fn parse_deleted(exit_code: Option<i32>, stdout: &[u8]) -> Result<Deleted, ToolError> {
    let reply = decode(stdout)?;
    if let Some(error) = rejection(&reply, exit_code) {
        return Err(error);
    }
    let replayed = applied_or_replayed(&reply, ["element_deleted", "element_delete_replayed"])?;
    let unreadable = || ToolError::local("UNREADABLE_REPLY", "the CLI reply lacks a field");
    Ok(Deleted {
        element_id: text(&reply, "element_id").ok_or_else(unreadable)?,
        kind: text(&reply, "kind"),
        revision: read_revision(&reply)?,
        replayed,
    })
}

/// What one committed tool command answered, by its kind.
#[derive(Debug, Clone, PartialEq)]
pub enum EditResult {
    /// A create command: the element it made.
    Created(Created),
    /// A move command: the element after the move.
    Updated(Updated),
    /// A delete command: what it removed.
    Deleted(Deleted),
}

impl EditResult {
    /// The created element, when this is a create reply.
    pub fn created(self) -> Option<Created> {
        match self {
            Self::Created(created) => Some(created),
            _ => None,
        }
    }

    /// The panel's success note: what changed and the revision that holds it.
    pub fn summary(&self) -> String {
        let pinned = |r: &Revision| {
            format!(
                "Accepted revision {} (sequence {})",
                r.manifest_digest.get(..12).unwrap_or(&r.manifest_digest),
                r.sequence
            )
        };
        let named = |name: &str| {
            if name.is_empty() {
                "unnamed".to_owned()
            } else {
                name.to_owned()
            }
        };
        let volume =
            |v: &Option<f64>| v.map_or(String::new(), |v| format!(", volume {:.3} m3", v / 1e9));
        match self {
            Self::Created(c) => format!(
                "Created {} {} ({}{}). {}.",
                c.kind.noun().unwrap_or("element"),
                c.element_id,
                named(&c.name),
                volume(&c.volume_mm3),
                pinned(&c.revision)
            ),
            Self::Updated(u) => format!(
                "Moved element {} ({}{}). {}{}.",
                u.element_id,
                named(&u.name),
                volume(&u.volume_mm3),
                pinned(&u.revision),
                if u.replayed { " (replayed)" } else { "" }
            ),
            Self::Deleted(d) => format!(
                "Deleted {} {}{}. {}.",
                d.kind.as_deref().unwrap_or("element"),
                d.element_id,
                if d.replayed { " (already absent)" } else { "" },
                pinned(&d.revision),
            ),
        }
    }
}

/// One definition of the store that a form can choose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    /// The definition's content digest (what the CLI takes).
    pub digest: String,
    /// A short label for the list.
    pub label: String,
}

/// The profile and material definitions of the accepted revision.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Definitions {
    /// Profiles, for `--profile-digest`.
    pub profiles: Vec<Choice>,
    /// Materials, for `--material-digest`.
    pub materials: Vec<Choice>,
}

/// The arguments of the command that lists the definitions of `store`.
pub fn definitions_args(store: &Path) -> Vec<String> {
    vec![
        "definitions".into(),
        "--store".into(),
        store.to_string_lossy().into_owned(),
    ]
}

/// The most definitions listed in the panel.
pub const MAX_CHOICES: usize = 1000;

fn short(digest: &str) -> &str {
    digest.get(..8).unwrap_or(digest)
}

fn dimension(definition: &Json, key: &str) -> Option<String> {
    definition
        .get(key)
        .ok()
        .and_then(|v| v.float().ok())
        .map(|v| format!("{v}"))
}

/// Reads the reply of `definitions`. Every definition that is not a material is offered as a
/// profile; whether the engine accepts a kind for a member is the engine's decision.
pub fn parse_definitions(exit_code: Option<i32>, stdout: &[u8]) -> Result<Definitions, ToolError> {
    let reply = decode(stdout)?;
    if let Some(error) = rejection(&reply, exit_code) {
        return Err(error);
    }
    let unreadable = || ToolError::local("UNREADABLE_REPLY", "the CLI reply lacks a field");
    let mut out = Definitions::default();
    for definition in reply
        .get("definitions")
        .and_then(Json::array)
        .map_err(|_| unreadable())?
        .iter()
        .take(MAX_CHOICES)
    {
        let digest = text(definition, "digest").ok_or_else(unreadable)?;
        let kind = text(definition, "kind").ok_or_else(unreadable)?;
        if kind == "material" {
            let grade = text(definition, "grade").unwrap_or_else(|| "material".into());
            let density = dimension(definition, "density_kg_m3")
                .map(|d| format!(" {d} kg/m3"))
                .unwrap_or_default();
            out.materials.push(Choice {
                label: format!("{grade}{density} ({})", short(&digest)),
                digest,
            });
        } else {
            let name = text(definition, "designation").unwrap_or_else(|| kind.clone());
            let size = match (
                dimension(definition, "width_mm"),
                dimension(definition, "height_mm"),
            ) {
                (Some(w), Some(h)) => format!(" {w} x {h} mm"),
                _ => String::new(),
            };
            out.profiles.push(Choice {
                label: format!("{name}{size} ({})", short(&digest)),
                digest,
            });
        }
    }
    Ok(out)
}

/// The tools panel's state machine: edit, preview (stage one command), commit or cancel.
/// Commit runs only what was previewed and only while the form still produces that command.
#[derive(Debug, Clone)]
pub struct Tools {
    /// The selected tool.
    pub active: ToolKind,
    /// The plate form.
    pub plate: PlateForm,
    /// The member form.
    pub member: MemberForm,
    /// The store's definitions, as of the last refresh.
    pub definitions: Definitions,
    /// The id of the element selected for the move and delete tools; its row is looked up in
    /// the panel's list, so kind and placement always come from the current revision.
    pub selection: Option<String>,
    /// The move form: the new absolute placement translation (millimetres), pre-filled from
    /// the snapshot's placement of the selection.
    pub move_to: [String; 3],
    staged: Option<Prepared>,
    busy: bool,
}

impl Default for Tools {
    fn default() -> Self {
        Self {
            active: ToolKind::Plate,
            plate: PlateForm::default(),
            member: MemberForm::default(),
            definitions: Definitions::default(),
            selection: None,
            move_to: ["0".into(), "0".into(), "0".into()],
            staged: None,
            busy: false,
        }
    }
}

impl Tools {
    /// Applies a definitions refresh. A failed refresh clears the choices (they would be stale)
    /// and returns the reason to show; a store without a revision has no definitions yet and
    /// is not an error. A selected choice that no longer exists is deselected.
    pub fn apply_definitions(&mut self, result: Result<Definitions, ToolError>) -> Option<String> {
        let (definitions, problem) = match result {
            Ok(definitions) => (definitions, None),
            Err(e) if e.code == "NOT_FOUND" => (Definitions::default(), None),
            Err(e) => (Definitions::default(), Some(format!("definitions: {e}"))),
        };
        let known = |list: &[Choice], digest: &Option<String>| {
            digest
                .as_ref()
                .is_some_and(|d| list.iter().any(|c| &c.digest == d))
        };
        if !known(&definitions.profiles, &self.member.profile) {
            self.member.profile = None;
        }
        if !known(&definitions.materials, &self.member.material) {
            self.member.material = None;
        }
        if !known(&definitions.materials, &self.plate.material) {
            self.plate.material = None;
        }
        self.definitions = definitions;
        problem
    }
    /// Selects an element for the move and delete tools. A new selection (a different id)
    /// pre-fills the move form with its placement as the snapshot reports it; selecting the
    /// already selected element again keeps the typed coordinates. A changed selection (or
    /// move text) invalidates a staged command, because `prepare` no longer produces it.
    pub fn select(&mut self, row: &ElementRow) {
        if self.selection.as_deref() != Some(row.id.as_str()) {
            // Debug formatting: the shortest round-trip form, which stays within the
            // `number` length cap for every finite value (exponents included).
            self.move_to = row.placement_mm.map(|v| format!("{v:?}"));
            self.selection = Some(row.id.clone());
        }
    }

    /// Drops the selection when it is not among `present` (the accepted revision moved on and
    /// no longer lists it).
    pub fn retain_selection(&mut self, present: impl Fn(&str) -> bool) {
        if self.selection.as_deref().is_some_and(|id| !present(id)) {
            self.selection = None;
        }
    }

    fn selected(&self) -> Result<&str, FormError> {
        self.selection.as_deref().ok_or(FormError {
            field: "Element",
            message: "select an element in the Elements list first".into(),
        })
    }

    /// The `update-element` move of the selection to the entered translation (absolute; the
    /// engine keeps the rotation and rebuilds nothing).
    fn prepare_move(&self, store: &Path) -> Result<Prepared, FormError> {
        let id = self.selected()?;
        Ok(Prepared {
            kind: ToolKind::Move,
            args: vec![
                "update-element".into(),
                "--store".into(),
                store.to_string_lossy().into_owned(),
                "--id".into(),
                id.to_owned(),
                "--translation".into(),
                point("Translation", &self.move_to)?,
            ],
        })
    }

    /// The `delete-element` of the selection (the whole element; the engine refuses one that
    /// another element still links to).
    fn prepare_delete(&self, store: &Path) -> Result<Prepared, FormError> {
        let id = self.selected()?;
        Ok(Prepared {
            kind: ToolKind::Delete,
            args: vec![
                "delete-element".into(),
                "--store".into(),
                store.to_string_lossy().into_owned(),
                "--id".into(),
                id.to_owned(),
            ],
        })
    }

    /// The command of the selected form.
    pub fn prepare(&self, store: &Path) -> Result<Prepared, FormError> {
        match self.active {
            ToolKind::Plate => self.plate.prepare(store),
            ToolKind::Member => self.member.prepare(store),
            ToolKind::Move => self.prepare_move(store),
            ToolKind::Delete => self.prepare_delete(store),
        }
    }

    /// Stages the selected form's command for review; nothing is run or written.
    pub fn preview(&mut self, store: &Path) -> Result<&Prepared, FormError> {
        let prepared = self.prepare(store)?;
        Ok(self.staged.insert(prepared))
    }

    /// The staged command, if any.
    pub fn staged(&self) -> Option<&Prepared> {
        self.staged.as_ref()
    }

    /// True while a committed command runs.
    pub fn busy(&self) -> bool {
        self.busy
    }

    /// True when a command is staged and the form still produces it.
    pub fn can_commit(&self, store: &Path) -> bool {
        !self.busy
            && self
                .staged
                .as_ref()
                .is_some_and(|s| self.prepare(store).is_ok_and(|now| now == *s))
    }

    /// Drops the staged command. Nothing was run, so nothing is published.
    pub fn cancel(&mut self) {
        self.staged = None;
    }

    /// Takes the staged command to run and marks the panel busy; `None` when nothing is staged,
    /// the form changed since the preview, or a command is already running.
    pub fn begin_commit(&mut self, store: &Path) -> Option<Prepared> {
        if !self.can_commit(store) {
            return None;
        }
        self.busy = true;
        self.staged.take()
    }

    /// Ends the running command; the next edit starts from the same form values.
    pub fn finish(&mut self) {
        self.busy = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> &'static Path {
        Path::new("S")
    }

    fn plate() -> PlateForm {
        PlateForm {
            width: " 1000 ".into(),
            length: "500".into(),
            thickness: "10".into(),
            ..PlateForm::default()
        }
    }

    const DIGEST: &str = "5d660dddce23e62c2544f796a80fd32de3725defa13dee514572833f0d886cb1";

    #[test]
    fn a_plate_maps_to_the_cli_arguments() {
        let mut form = plate();
        let args = form.prepare(store()).unwrap().args;
        assert_eq!(
            args,
            [
                "create-plate",
                "--store",
                "S",
                "--width",
                "1000",
                "--length",
                "500",
                "--thickness",
                "10"
            ]
        );
        form.name = "  P1 ".into();
        form.material = Some(DIGEST.into());
        let args = form.prepare(store()).unwrap().args;
        assert_eq!(
            args.get(9..),
            Some(
                &[
                    "--name".to_owned(),
                    "P1".into(),
                    "--material-digest".into(),
                    DIGEST.into()
                ][..]
            )
        );
    }

    #[test]
    fn a_member_maps_to_the_cli_arguments() {
        let mut form = MemberForm {
            kind: MemberKind::Column,
            end: ["0".into(), "0".into(), "3000.5".into()],
            profile: Some(DIGEST.into()),
            ..MemberForm::default()
        };
        let prepared = form.prepare(store()).unwrap();
        assert_eq!(prepared.kind, ToolKind::Member);
        assert_eq!(
            prepared.args,
            [
                "create-member",
                "--store",
                "S",
                "--kind",
                "column",
                "--start",
                "0,0,0",
                "--end",
                "0,0,3000.5",
                "--profile-digest",
                DIGEST
            ]
        );
        form.profile = None;
        let error = form.prepare(store()).unwrap_err();
        assert_eq!(error.field, "Profile");
    }

    #[test]
    fn bad_numbers_are_refused_before_anything_runs() {
        for bad in ["", "  ", "abc", "1,5", "NaN", "inf", "-inf", "1e999"] {
            let mut form = plate();
            form.width = bad.into();
            let error = form.prepare(store()).unwrap_err();
            assert_eq!(error.field, "Width", "{bad:?}");
        }
        let mut member = MemberForm {
            profile: Some(DIGEST.into()),
            ..MemberForm::default()
        };
        member.end[2] = "x".into();
        assert_eq!(member.prepare(store()).unwrap_err().field, "End");
        // Engineering rules (positive size, caps) are the engine's: the UI forwards them.
        let mut form = plate();
        form.thickness = "-4".into();
        assert!(form.prepare(store()).is_ok());
        form.name = "n".repeat(MAX_TEXT + 1);
        assert_eq!(form.prepare(store()).unwrap_err().field, "Name");
    }

    #[test]
    fn a_failed_definitions_refresh_is_reported_and_clears_stale_choices() {
        let choice = |digest: &str| Choice {
            digest: digest.into(),
            label: digest.into(),
        };
        let listed = Definitions {
            profiles: vec![choice("p")],
            materials: vec![choice("m")],
        };
        let mut tools = Tools::default();
        tools.member.profile = Some("p".into());
        tools.member.material = Some("m".into());
        tools.plate.material = Some("gone".into());
        assert_eq!(tools.apply_definitions(Ok(listed)), None);
        assert_eq!(tools.member.profile.as_deref(), Some("p"));
        assert_eq!(tools.member.material.as_deref(), Some("m"));
        assert_eq!(tools.plate.material, None);
        let failure = ToolError {
            code: "CLI_FAILED".into(),
            message: "boom".into(),
        };
        let problem = tools.apply_definitions(Err(failure)).unwrap();
        assert_eq!(problem, "definitions: CLI_FAILED: boom");
        assert!(tools.definitions.profiles.is_empty());
        assert_eq!(tools.member.profile, None);
        // A store with no revision yet simply has no definitions.
        let empty = ToolError {
            code: "NOT_FOUND".into(),
            message: "no revision".into(),
        };
        assert_eq!(tools.apply_definitions(Err(empty)), None);
    }

    #[test]
    fn the_argument_list_round_trips_every_argument() {
        let mut form = plate();
        form.name = "Plate \"A\" \\ $x `y` 'z' é\nend".into();
        let prepared = form.prepare(Path::new("my store\\a b")).unwrap();
        let shown = prepared.argument_list();
        assert_eq!(shown.lines().count(), prepared.args.len());
        let array = format!("[{}]", shown.replace('\n', ","));
        let decoded = json::decode(array.as_bytes(), &message::read_limits()).unwrap();
        let back: Vec<&str> = decoded
            .array()
            .unwrap()
            .iter()
            .map(|a| a.string().unwrap())
            .collect();
        assert_eq!(back, prepared.args);
        assert!(shown.contains("\"Plate \\\"A\\\" \\\\ $x"));
    }

    const CREATED: &str = r#"{"footprint":{},"op_id":"o","outcome":"plate_created","plate":{"element_id":"e1","mesh_digest":"m1","name":"P1","quantities":{"mass":{"status":"unknown"},"volume":{"mm3":200000,"status":"current"}}},"revision":{"manifest_digest":"d1","revision_id":"r1","sequence":"3"}}"#;

    #[test]
    fn a_created_reply_is_read() {
        let created = parse_created(ToolKind::Plate, Some(0), CREATED.as_bytes()).unwrap();
        assert_eq!(created.element_id, "e1");
        assert_eq!(created.name, "P1");
        assert_eq!(created.mesh_digest, "m1");
        assert_eq!(created.volume_mm3, Some(200_000.0));
        assert_eq!(created.revision.revision_id, "r1");
        assert_eq!(created.revision.manifest_digest, "d1");
        assert_eq!(created.revision.sequence, 3);
        assert!(!created.replayed);
        // A member reply is not a plate reply.
        let error = parse_created(ToolKind::Member, Some(0), CREATED.as_bytes()).unwrap_err();
        assert_eq!(error.code, "UNREADABLE_REPLY");
    }

    #[test]
    fn a_stale_volume_is_not_shown_as_current() {
        let reply = CREATED.replace("\"current\"", "\"stale\"");
        let created = parse_created(ToolKind::Plate, Some(0), reply.as_bytes()).unwrap();
        assert_eq!(created.volume_mm3, None);
    }

    const UPDATED: &str = r#"{"changed_layers":["authored.usda"],"effect":{"level":false,"material":false,"name":false,"placement":true,"shape":false},"element":{"element_id":"e1","kind":"plate","mesh_digest":"m1","name":"P1","quantities":{"mass":{"status":"unknown"},"volume":{"mm3":5000000,"status":"current"}}},"footprint":null,"op_id":"o","outcome":"element_updated","revision":{"manifest_digest":"d2","revision_id":"r2","sequence":"2"}}"#;

    #[test]
    fn an_updated_reply_is_read() {
        let updated = parse_updated(Some(0), UPDATED.as_bytes()).unwrap();
        assert_eq!(updated.element_id, "e1");
        assert_eq!(updated.name, "P1");
        assert_eq!(updated.volume_mm3, Some(5_000_000.0));
        assert_eq!(updated.revision.manifest_digest, "d2");
        assert_eq!(updated.revision.sequence, 2);
        assert!(!updated.replayed);
        // A replay of the same move is answered from the head.
        let replay = UPDATED.replace("element_updated", "element_update_replayed");
        let updated = parse_updated(Some(0), replay.as_bytes()).unwrap();
        assert!(updated.replayed);
        // Another command's reply is not an update reply.
        let error = parse_updated(Some(0), CREATED.as_bytes()).unwrap_err();
        assert_eq!(error.code, "UNREADABLE_REPLY");
        let error = parse_updated(Some(0), br#"{"outcome":"element_updated"}"#).unwrap_err();
        assert_eq!(error.code, "UNREADABLE_REPLY");
        // A refusal keeps the CLI's code, a zero exit cannot hide it.
        let reply = r#"{"code":"NOT_FOUND","message":"the element is not in the head revision","outcome":"rejected"}"#;
        let error = parse_updated(Some(2), reply.as_bytes()).unwrap_err();
        assert_eq!(error.code, "NOT_FOUND");
        assert!(parse_updated(Some(0), reply.as_bytes()).is_err());
    }

    const DELETED: &str = r#"{"changed_layers":["authored.usda"],"element_id":"e1","footprint":null,"kind":"plate","op_id":"o","outcome":"element_deleted","revision":{"manifest_digest":"d3","revision_id":"r3","sequence":"3"}}"#;

    #[test]
    fn a_deleted_reply_is_read() {
        let deleted = parse_deleted(Some(0), DELETED.as_bytes()).unwrap();
        assert_eq!(deleted.element_id, "e1");
        assert_eq!(deleted.kind.as_deref(), Some("plate"));
        assert_eq!(deleted.revision.sequence, 3);
        assert!(!deleted.replayed);
        // On a replay the element is already absent, so the CLI answers no kind.
        let replay = DELETED
            .replace("element_deleted", "element_delete_replayed")
            .replace("\"kind\":\"plate\"", "\"kind\":null");
        let deleted = parse_deleted(Some(0), replay.as_bytes()).unwrap();
        assert!(deleted.replayed);
        assert_eq!(deleted.kind, None);
        assert_eq!(deleted.element_id, "e1");
        // A missing field is an error, never a success.
        let error = parse_deleted(Some(0), br#"{"outcome":"element_deleted"}"#).unwrap_err();
        assert_eq!(error.code, "UNREADABLE_REPLY");
        let error = parse_deleted(Some(0), UPDATED.as_bytes()).unwrap_err();
        assert_eq!(error.code, "UNREADABLE_REPLY");
    }

    #[test]
    fn a_move_and_a_delete_map_to_the_cli_arguments() {
        let row = ElementRow {
            id: "01990000-0000-7000-8000-00000000000a".into(),
            kind: "plate".into(),
            placement_mm: [0.0, 0.0, 0.0],
        };
        let mut tools = Tools {
            active: ToolKind::Move,
            ..Tools::default()
        };
        // Nothing selected: nothing is prepared, for either tool.
        assert_eq!(tools.prepare(store()).unwrap_err().field, "Element");
        tools.active = ToolKind::Delete;
        assert_eq!(tools.prepare(store()).unwrap_err().field, "Element");
        // Selecting pre-fills the move form with the snapshot's placement.
        tools.active = ToolKind::Move;
        tools.select(&row);
        assert_eq!(tools.move_to, ["0.0", "0.0", "0.0"]);
        // A placement with float noise still fits the number length cap and parses back,
        // because it is written in the shortest round-trip (Debug) form.
        let noisy = ElementRow {
            id: "01990000-0000-7000-8000-00000000000b".into(),
            kind: "plate".into(),
            placement_mm: [5.551115123125783e-17, 1.5e20, 0.0],
        };
        tools.select(&noisy);
        assert_eq!(tools.move_to, ["5.551115123125783e-17", "1.5e20", "0.0"]);
        assert!(tools.move_to.iter().all(|v| v.len() <= 32));
        assert!(tools.prepare(store()).is_ok());
        // Selecting the same element again keeps the typed coordinates.
        tools.move_to = ["500".into(), "0".into(), "0".into()];
        tools.select(&noisy);
        assert_eq!(tools.move_to, ["500", "0", "0"]);
        tools.select(&row);
        assert_eq!(tools.move_to, ["0.0", "0.0", "0.0"]);
        tools.move_to = [" 500 ".into(), "-250.5".into(), "0".into()];
        let prepared = tools.prepare(store()).unwrap();
        assert_eq!(prepared.kind, ToolKind::Move);
        assert_eq!(
            prepared.args,
            [
                "update-element",
                "--store",
                "S",
                "--id",
                "01990000-0000-7000-8000-00000000000a",
                "--translation",
                "500,-250.5,0"
            ]
        );
        // A bad number is refused before anything runs; a negative translation is the
        // engine's to judge (absolute placements may be negative).
        tools.move_to[1] = "x".into();
        assert_eq!(tools.prepare(store()).unwrap_err().field, "Translation");
        tools.move_to[1] = "-9999".into();
        assert!(tools.prepare(store()).is_ok());
        // The delete needs only the selection.
        tools.active = ToolKind::Delete;
        let prepared = tools.prepare(store()).unwrap();
        assert_eq!(prepared.kind, ToolKind::Delete);
        assert_eq!(
            prepared.args,
            [
                "delete-element",
                "--store",
                "S",
                "--id",
                "01990000-0000-7000-8000-00000000000a"
            ]
        );
        assert_eq!(ToolKind::Move.command(), "update-element");
        assert_eq!(ToolKind::Delete.command(), "delete-element");
    }

    #[test]
    fn a_staged_move_survives_only_its_own_selection_and_text() {
        let row = |id: &str, x: f64| ElementRow {
            id: id.into(),
            kind: "plate".into(),
            placement_mm: [x, 0.0, 0.0],
        };
        let mut tools = Tools {
            active: ToolKind::Move,
            ..Tools::default()
        };
        tools.select(&row("a", 0.0));
        tools.move_to = ["100".into(), "0".into(), "0".into()];
        tools.preview(store()).unwrap();
        assert!(tools.can_commit(store()));
        // Selecting the already selected element again keeps the typed translation and the
        // staged command, even though the snapshot's placement moved on.
        tools.select(&row("a", 50.0));
        assert_eq!(tools.move_to, ["100", "0", "0"]);
        assert!(tools.can_commit(store()));
        // Another element pre-fills its own placement and invalidates the staged command.
        tools.select(&row("b", 20.0));
        assert_eq!(tools.move_to, ["20.0", "0.0", "0.0"]);
        assert!(!tools.can_commit(store()));
        assert!(tools.begin_commit(store()).is_none());
        // Editing the text never revives another selection's staged command.
        tools.move_to = ["100".into(), "0".into(), "0".into()];
        assert!(!tools.can_commit(store()));
        // A selection that the accepted revision no longer lists is dropped; then nothing
        // can be prepared or committed.
        tools.retain_selection(|_| true);
        assert_eq!(tools.selection.as_deref(), Some("b"));
        tools.retain_selection(|id| id != "b");
        assert_eq!(tools.selection, None);
        assert!(tools.prepare(store()).is_err());
        assert!(!tools.can_commit(store()));
        // The success notes name what changed and the revision that holds it.
        let updated = parse_updated(Some(0), UPDATED.as_bytes()).unwrap();
        assert_eq!(
            EditResult::Updated(updated).summary(),
            "Moved element e1 (P1, volume 0.005 m3). Accepted revision d2 (sequence 2)."
        );
        let deleted = parse_deleted(Some(0), DELETED.as_bytes()).unwrap();
        assert_eq!(
            EditResult::Deleted(deleted).summary(),
            "Deleted plate e1. Accepted revision d3 (sequence 3)."
        );
        // A replayed delete carries one marker, not two.
        let replay = DELETED
            .replace("element_deleted", "element_delete_replayed")
            .replace("\"kind\":\"plate\"", "\"kind\":null");
        let deleted = parse_deleted(Some(0), replay.as_bytes()).unwrap();
        assert_eq!(
            EditResult::Deleted(deleted).summary(),
            "Deleted element e1 (already absent). Accepted revision d3 (sequence 3)."
        );
    }

    #[test]
    fn a_refusal_keeps_the_code_and_message() {
        let reply = r#"{"code":"INVALID_ARGUMENT","diagnostics":[],"message":"dimension is not positive","observed_context":null,"outcome":"rejected","transport_code":null}"#;
        let error = parse_created(ToolKind::Plate, Some(2), reply.as_bytes()).unwrap_err();
        assert_eq!(error.code, "INVALID_ARGUMENT");
        assert_eq!(error.message, "dimension is not positive");
        assert_eq!(
            error.to_string(),
            "INVALID_ARGUMENT: dimension is not positive"
        );
        // Even a zero exit status cannot turn a rejection into a success.
        assert!(parse_created(ToolKind::Plate, Some(0), reply.as_bytes()).is_err());
    }

    #[test]
    fn an_unknown_outcome_says_so() {
        let reply = r#"{"code":"OUTCOME_UNKNOWN","message":"commit not confirmed","outcome":"outcome_unknown"}"#;
        let error = parse_created(ToolKind::Plate, Some(4), reply.as_bytes()).unwrap_err();
        assert_eq!(error.code, "OUTCOME_UNKNOWN");
        assert!(error.message.contains("reload"));
    }

    #[test]
    fn garbage_and_failures_are_errors_never_successes() {
        for (code, bytes) in [
            (Some(0), &b""[..]),
            (Some(0), b"not json"),
            (Some(2), b"not json"),
            (None, b"{}"),
            (Some(1), CREATED.as_bytes()),
            (Some(0), b"{\"outcome\":\"plate_created\"}"),
        ] {
            assert!(
                parse_created(ToolKind::Plate, code, bytes).is_err(),
                "{code:?} {bytes:?}"
            );
        }
        // A forged code is not echoed.
        let reply = r#"{"code":"drop <b>","message":"m","outcome":"rejected"}"#;
        let error = parse_created(ToolKind::Plate, Some(2), reply.as_bytes()).unwrap_err();
        assert_eq!(error.code, "CLI_FAILED");
    }

    #[test]
    fn definitions_split_into_profiles_and_materials() {
        let reply = format!(
            r#"{{"definition_count":2,"definitions":[{{"density_kg_m3":7850,"digest":"{m}","grade":"S355","kind":"material","used_by":[]}},{{"digest":"{DIGEST}","height_mm":300,"kind":"rectangle","width_mm":200,"used_by":[]}}],"outcome":"definitions"}}"#,
            m = "36281e26dd4affda6b39d884fe8a8bb04deb71684215741c188dac0806710ba9"
        );
        let definitions = parse_definitions(Some(0), reply.as_bytes()).unwrap();
        assert_eq!(definitions.materials.len(), 1);
        assert_eq!(definitions.profiles.len(), 1);
        assert_eq!(definitions.profiles[0].digest, DIGEST);
        assert_eq!(
            definitions.profiles[0].label,
            "rectangle 200 x 300 mm (5d660ddd)"
        );
        assert_eq!(definitions.materials[0].label, "S355 7850 kg/m3 (36281e26)");
        let error = parse_definitions(
            Some(2),
            br#"{"code":"NOT_FOUND","message":"no revision","outcome":"rejected"}"#,
        )
        .unwrap_err();
        assert_eq!(error.code, "NOT_FOUND");
        assert_eq!(definitions_args(store()), ["definitions", "--store", "S"]);
    }

    #[test]
    fn commit_runs_only_what_was_previewed() {
        let mut tools = Tools {
            plate: plate(),
            ..Tools::default()
        };
        // Nothing staged: nothing to commit.
        assert!(!tools.can_commit(store()));
        assert!(tools.begin_commit(store()).is_none());
        // A form error stages nothing.
        tools.plate.width.clear();
        assert!(tools.preview(store()).is_err());
        assert!(tools.staged().is_none());
        tools.plate = plate();
        let staged = tools.preview(store()).unwrap().clone();
        assert!(tools.can_commit(store()));
        // Editing after the preview invalidates it.
        tools.plate.length = "600".into();
        assert!(!tools.can_commit(store()));
        assert!(tools.begin_commit(store()).is_none());
        tools.plate.length = "500".into();
        assert!(tools.can_commit(store()));
        // Commit takes the staged command and blocks a second commit until finished.
        assert_eq!(tools.begin_commit(store()), Some(staged));
        assert!(tools.busy());
        assert!(tools.staged().is_none());
        assert!(tools.begin_commit(store()).is_none());
        tools.finish();
        assert!(!tools.busy());
    }

    #[test]
    fn cancel_drops_the_staged_command() {
        let mut tools = Tools {
            plate: plate(),
            ..Tools::default()
        };
        tools.preview(store()).unwrap();
        tools.cancel();
        assert!(tools.staged().is_none());
        assert!(tools.begin_commit(store()).is_none());
    }
}
