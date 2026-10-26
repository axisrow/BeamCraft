// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! V05.1 create tools, as pure logic: the entered parameters become the argument list of the same
//! `beamcraft create-plate` / `create-member` commands the CLI user types, and the CLI's JSON
//! reply becomes a result or an error to show. Nothing here touches a window, a GPU or a process,
//! so all of it is unit-testable; [`crate::command`] runs the process.
//!
//! The UI never computes engineering geometry. It checks only that a number is a finite decimal
//! and that required choices are made, then forwards the text. Every rule about shapes, limits,
//! profiles and materials stays in the engine, and its refusal (a stable code and a message) is
//! shown as it is.

use beamcraft_json::{Json, Limits};
use std::fmt;
use std::path::Path;

use crate::props::RevisionStamp;

/// The longest accepted text field: the engine counts label bytes, so the same limit is
/// checked in bytes before anything runs (the fields' character limit stays a soft affordance).
pub const MAX_TEXT: usize = 256;

/// The tools of this slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    /// A rectangular plate (`create-plate`).
    Plate,
    /// A straight beam or column (`create-member`).
    Member,
}

impl ToolKind {
    /// The CLI command this tool runs.
    pub fn command(self) -> &'static str {
        match self {
            Self::Plate => "create-plate",
            Self::Member => "create-member",
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Self::Plate => "plate",
            Self::Member => "member",
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
    /// Authored part class, empty means the default zero.
    pub class: String,
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
    /// Optional numeric expression overriding the Cartesian start fields.
    pub start_input: String,
    /// Optional numeric expression overriding the Cartesian end fields.
    pub end_input: String,
    /// Direction constraint through the start, shared with the engine and CLI.
    pub axis_lock: beamcraft_engine::coordinate::AxisLock,
    /// `--profile-digest`: a profile definition of the store (required).
    pub profile: Option<String>,
    /// `--material-digest`, optional.
    pub material: Option<String>,
    /// `--name`, optional.
    pub name: String,
    /// Authored part class, empty means the default zero.
    pub class: String,
    /// `--roll RAD`, optional (radians; empty means the flag is absent).
    pub roll: String,
    /// `--orientation-xyzw X,Y,Z,W`, optional (empty means the flag is absent).
    pub orientation: String,
}

impl Default for MemberForm {
    fn default() -> Self {
        Self {
            kind: MemberKind::Beam,
            start: ["0".into(), "0".into(), "0".into()],
            end: ["1000".into(), "0".into(), "0".into()],
            start_input: String::new(),
            end_input: String::new(),
            axis_lock: beamcraft_engine::coordinate::AxisLock::None,
            profile: None,
            material: None,
            name: String::new(),
            class: String::new(),
            roll: String::new(),
            orientation: String::new(),
        }
    }
}

/// The placement quaternion a member form's orientation text stands for: an empty field is the
/// identity the commit runs (prepare omits the flag), and `None` is text that would not even
/// reach the engine's validation — the ghost preview draws nothing then.
pub fn orientation_or_identity(text: &str) -> Option<[f64; 4]> {
    let text = text.trim();
    if text.is_empty() {
        return Some([0.0, 0.0, 0.0, 1.0]);
    }
    text.split(',')
        .map(|part| part.trim().parse::<f64>().ok())
        .collect::<Option<Vec<f64>>>()?
        .try_into()
        .ok()
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
        // Say why the text does not parse: a decimal comma is the usual reason, and the value
        // itself may well be finite.
        Ok(_) => Err(FormError {
            field,
            message: format!("\"{text}\" is not a finite number"),
        }),
        Err(_) => Err(FormError {
            field,
            message: format!("\"{text}\" is not a number; use a period as the decimal separator"),
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

/// `X,Y,Z,W`, each a finite decimal: the quaternion form of `--orientation-xyzw`.
fn quaternion(field: &'static str, text: &str) -> Result<String, FormError> {
    let text = text.trim();
    let parts: Vec<&str> = text.split(',').map(str::trim).collect();
    if parts.len() != 4 {
        return Err(FormError {
            field,
            message: "enter four numbers X,Y,Z,W".into(),
        });
    }
    for part in &parts {
        number(field, part)?;
    }
    Ok(text.to_owned())
}

fn push_name(args: &mut Vec<String>, name: &str) -> Result<(), FormError> {
    let name = name.trim();
    // The engine admits labels by byte length; counting characters let a Cyrillic name pass
    // the field and Preview and fail at Commit.
    if name.len() > MAX_TEXT {
        return Err(FormError {
            field: "Name",
            message: format!("at most {MAX_TEXT} bytes"),
        });
    }
    if !name.is_empty() {
        args.extend(["--name".into(), name.to_owned()]);
    }
    Ok(())
}

/// Appends `--actor-id` when the window records an author.
pub(crate) fn push_actor(args: &mut Vec<String>, actor: Option<&str>) {
    if let Some(actor) = actor {
        args.extend(["--actor-id".into(), actor.to_owned()]);
    }
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
        if !self.class.trim().is_empty() {
            let class = self.class.trim().parse::<i32>().map_err(|_| FormError {
                field: "Class",
                message: "enter an integer in the int32 range".into(),
            })?;
            args.extend(["--class".into(), class.to_string()]);
        }
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
        let start = if self.start_input.trim().is_empty() {
            point("Start", &self.start)?
        } else {
            self.start_input.trim().to_owned()
        };
        let end = if self.end_input.trim().is_empty() {
            point("End", &self.end)?
        } else {
            self.end_input.trim().to_owned()
        };
        let reject = |e: beamcraft_model::command::CommandError| FormError {
            field: "Coordinates",
            message: e.to_string(),
        };
        let origin = beamcraft_engine::coordinate::resolve(
            &start,
            [0.0; 3],
            beamcraft_engine::coordinate::AxisLock::None,
        )
        .map_err(reject)?;
        beamcraft_engine::coordinate::resolve(&end, origin, self.axis_lock).map_err(reject)?;
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
        if !self.class.trim().is_empty() {
            let class = self.class.trim().parse::<i32>().map_err(|_| FormError {
                field: "Class",
                message: "enter an integer in the int32 range".into(),
            })?;
            args.extend(["--class".into(), class.to_string()]);
        }
        push_material(&mut args, self.material.as_ref());
        // An empty field keeps the flag absent, so the planar form builds exactly the command it
        // always did; the engine owns every rule about rolls and quaternions.
        if self.axis_lock != beamcraft_engine::coordinate::AxisLock::None {
            args.extend(["--axis-lock".into(), self.axis_lock.token().into()]);
        }
        let roll = self.roll.trim();
        if !roll.is_empty() {
            args.extend(["--roll".into(), number("Roll", roll)?]);
        }
        let orientation = self.orientation.trim();
        if !orientation.is_empty() {
            args.extend([
                "--orientation-xyzw".into(),
                quaternion("Orientation", orientation)?,
            ]);
        }
        Ok(Prepared {
            kind: ToolKind::Member,
            args,
        })
    }

    /// Fills both endpoints from resolved world points (the two clicks of the viewport
    /// placement): the shortest decimal text that parses back exactly.
    pub fn set_endpoints(&mut self, start: [f64; 3], end: [f64; 3]) {
        let text = |p: [f64; 3]| {
            let [x, y, z] = p.map(|v| format!("{v}"));
            [x, y, z]
        };
        self.start_input.clear();
        self.end_input.clear();
        self.start = text(start);
        self.end = text(end);
    }
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

/// What a committed command created, as the CLI reported it.
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
    pub revision_id: String,
    /// The revision's manifest digest, which is what the view snapshot pins.
    pub manifest_digest: String,
    /// The revision's sequence number.
    pub sequence: u64,
    /// True when the CLI answered an already applied operation again.
    pub replayed: bool,
}

pub(crate) fn decode(stdout: &[u8]) -> Result<Json, ToolError> {
    beamcraft_json::decode(stdout, &Limits::DEFAULT)
        .map_err(|_| ToolError::local("UNREADABLE_REPLY", "the CLI reply is not valid JSON"))
}

pub(crate) fn text(json: &Json, key: &str) -> Option<String> {
    json.get(key)
        .ok()
        .and_then(|v| v.string().ok())
        .map(str::to_owned)
}

/// The refusal in a reply, if the reply is one.
pub(crate) fn rejection(reply: &Json, exit_code: Option<i32>) -> Option<ToolError> {
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
    let noun = kind.noun();
    let outcome = text(&reply, "outcome").unwrap_or_default();
    let replayed = outcome == format!("{noun}_replayed");
    if !replayed && outcome != format!("{noun}_created") {
        return Err(ToolError::local(
            "UNREADABLE_REPLY",
            format!(
                "unexpected outcome \"{}\"",
                outcome.chars().take(40).collect::<String>()
            ),
        ));
    }
    let unreadable = || ToolError::local("UNREADABLE_REPLY", "the CLI reply lacks a field");
    let element = reply.get(noun).map_err(|_| unreadable())?;
    let revision = reply.get("revision").map_err(|_| unreadable())?;
    let volume = element
        .get("quantities")
        .and_then(|q| q.get("volume"))
        .ok()
        .filter(|v| text(v, "status").as_deref() == Some("current"))
        .and_then(|v| v.get("mm3").ok())
        .and_then(|v| v.float().ok());
    Ok(Created {
        kind,
        element_id: text(element, "element_id").ok_or_else(unreadable)?,
        name: text(element, "name").unwrap_or_default(),
        mesh_digest: text(element, "mesh_digest").ok_or_else(unreadable)?,
        volume_mm3: volume,
        revision_id: text(revision, "revision_id").ok_or_else(unreadable)?,
        manifest_digest: text(revision, "manifest_digest").ok_or_else(unreadable)?,
        sequence: text(revision, "sequence")
            .and_then(|s| s.parse().ok())
            .ok_or_else(unreadable)?,
        replayed,
    })
}

/// One definition of the store that a form can choose.
#[derive(Debug, Clone, PartialEq)]
pub struct Choice {
    /// The definition's content digest (what the CLI takes).
    pub digest: String,
    /// A short label for the list.
    pub label: String,
    /// Width and height of a rectangle section, when the definition is one: the display-only
    /// ghost preview is drawn for these and only these (the engine builds the real shape).
    pub rect_mm: Option<[f64; 2]>,
}

/// The two-click 3D placement of the member tool (issue #134): the first resolved point arms
/// the start, the second fills the form's endpoints. The points are resolved outside (the
/// engine's pick on the viewed snapshot, else the ground plane); this only remembers them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BeamPlace {
    start: Option<[f64; 3]>,
}

impl BeamPlace {
    /// The armed start point, if any.
    pub fn start(&self) -> Option<[f64; 3]> {
        self.start
    }

    /// Records a click. The first arms the start; the second returns the `(start, end)` pair.
    pub fn click(&mut self, point: [f64; 3]) -> Option<([f64; 3], [f64; 3])> {
        match self.start {
            None => {
                self.start = Some(point);
                None
            }
            Some(start) => {
                self.start = None;
                Some((start, point))
            }
        }
    }

    /// Drops the armed start (Esc, a mode change, a commit).
    pub fn reset(&mut self) {
        self.start = None;
    }
}

/// The profile and material definitions of the accepted revision.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Definitions {
    /// Profiles, for `--profile-digest`.
    pub profiles: Vec<Choice>,
    /// Materials, for `--material-digest`.
    pub materials: Vec<Choice>,
    /// The revision the listing was read from: the store's head at that moment.
    pub head: Option<RevisionStamp>,
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
    let mut out = Definitions {
        head: reply
            .get("revision")
            .ok()
            .and_then(RevisionStamp::from_json),
        ..Definitions::default()
    };
    for definition in reply
        .get("definitions")
        .and_then(Json::array)
        .map_err(|_| unreadable())?
        .iter()
        .take(MAX_CHOICES)
    {
        let digest = text(definition, "digest").ok_or_else(unreadable)?;
        let kind = text(definition, "kind").ok_or_else(unreadable)?;
        // The readable name the engine stores with the definition (A07.2) leads the label;
        // definitions published before it fall back to the grade or designation they have.
        let display = text(definition, "display_name").filter(|n| !n.is_empty());
        if kind == "material" {
            let grade = display
                .or_else(|| text(definition, "grade"))
                .unwrap_or_else(|| "material".into());
            let density = dimension(definition, "density_kg_m3")
                .map(|d| format!(" {d} kg/m3"))
                .unwrap_or_default();
            out.materials.push(Choice {
                label: format!("{grade}{density} ({})", short(&digest)),
                digest,
                rect_mm: None,
            });
        } else {
            let name = display
                .or_else(|| text(definition, "designation"))
                .unwrap_or_else(|| kind.clone());
            let (width, height) = (
                dimension(definition, "width_mm"),
                dimension(definition, "height_mm"),
            );
            let size = match (&width, &height) {
                (Some(w), Some(h)) => format!(" {w} x {h} mm"),
                _ => String::new(),
            };
            let rect_mm = width
                .as_deref()
                .and_then(|w| w.parse::<f64>().ok())
                .zip(height.as_deref().and_then(|h| h.parse::<f64>().ok()))
                .map(|(w, h)| [w, h]);
            out.profiles.push(Choice {
                label: format!("{name}{size} ({})", short(&digest)),
                digest,
                rect_mm,
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
    /// The author recorded as `--actor-id` of every create command, when the window has one.
    pub actor: Option<String>,
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
            actor: None,
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
    /// The command of the selected form.
    pub fn prepare(&self, store: &Path) -> Result<Prepared, FormError> {
        let mut prepared = match self.active {
            ToolKind::Plate => self.plate.prepare(store),
            ToolKind::Member => self.member.prepare(store),
        }?;
        push_actor(&mut prepared.args, self.actor.as_deref());
        Ok(prepared)
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
    fn numeric_input_keeps_cli_expressions_and_uses_the_shared_engine_validation() {
        use beamcraft_engine::coordinate::{AxisLock, resolve};
        let mut form = MemberForm {
            profile: Some("profile".into()),
            start_input: "100,200,300".into(),
            end_input: "@300,400,50".into(),
            axis_lock: AxisLock::Y,
            ..MemberForm::default()
        };
        let prepared = form.prepare(Path::new("S")).unwrap();
        let option = |name| prepared.args.windows(2).find(|w| w[0] == name).unwrap()[1].clone();
        let start = resolve(&option("--start"), [0.; 3], AxisLock::None).unwrap();
        assert_eq!(
            resolve(
                &option("--end"),
                start,
                AxisLock::parse(&option("--axis-lock")).unwrap()
            )
            .unwrap(),
            [100., 600., 300.]
        );
        form.end_input = "@5<NaN".into();
        assert!(form.prepare(Path::new("S")).is_err());
        form.set_endpoints([0.; 3], [0., 0., 500.]);
        assert!(form.start_input.is_empty() && form.end_input.is_empty());
    }

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
    fn the_ghost_orientation_is_identity_for_an_empty_field_and_none_for_garbage() {
        // Empty is the identity the commit runs: prepare omits the flag then.
        assert_eq!(orientation_or_identity("  "), Some([0.0, 0.0, 0.0, 1.0]));
        // The decimal text parses to the nearest f64, which is the constant itself.
        let half = core::f64::consts::FRAC_1_SQRT_2;
        assert_eq!(
            orientation_or_identity("0,0,0.7071067811865476,0.7071067811865476"),
            Some([0.0, 0.0, half, half])
        );
        for bad in ["0,0,1", "0,0,1,2,3", "0,0,x,1", ",,,", "0, 0, 0"] {
            assert_eq!(orientation_or_identity(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn roll_and_orientation_extend_the_member_command_only_when_filled() {
        let mut form = MemberForm {
            profile: Some(DIGEST.into()),
            ..MemberForm::default()
        };
        // The planar default keeps today's exact argument list: no roll, no orientation.
        let args = form.prepare(store()).unwrap().args;
        assert!(!args.contains(&"--roll".to_owned()));
        assert!(!args.contains(&"--orientation-xyzw".to_owned()));
        form.roll = " 0.7853981633974483 ".into();
        form.orientation = "0,0,0.7071067811865476,0.7071067811865476".into();
        let args = form.prepare(store()).unwrap().args;
        assert_eq!(
            args.get(args.len() - 4..),
            Some(
                &[
                    "--roll".to_owned(),
                    "0.7853981633974483".into(),
                    "--orientation-xyzw".to_owned(),
                    "0,0,0.7071067811865476,0.7071067811865476".into()
                ][..]
            )
        );
        let error_of = |roll: &str, orientation: &str| {
            MemberForm {
                profile: Some(DIGEST.into()),
                roll: roll.into(),
                orientation: orientation.into(),
                ..MemberForm::default()
            }
            .prepare(store())
            .unwrap_err()
        };
        assert_eq!(error_of("x", "").field, "Roll");
        assert_eq!(error_of("", "0,0,1").field, "Orientation");
        assert_eq!(error_of("", "0,0,1,2,3").field, "Orientation");
    }

    #[test]
    fn the_viewport_clicks_fill_the_endpoints_and_the_place_state_remembers_the_start() {
        let mut place = BeamPlace::default();
        assert_eq!(place.start(), None);
        assert_eq!(place.click([0.0, 0.0, 0.0]), None);
        assert_eq!(place.start(), Some([0.0, 0.0, 0.0]));
        assert_eq!(
            place.click([4000.0, 0.0, 4000.0]),
            Some(([0.0, 0.0, 0.0], [4000.0, 0.0, 4000.0]))
        );
        assert_eq!(place.start(), None);
        place.reset();
        assert_eq!(place.start(), None);

        let mut form = MemberForm::default();
        form.set_endpoints([0.0, -0.25, 1e6], [4000.0, 0.0, 4000.0]);
        assert_eq!(
            form.start,
            ["0".to_owned(), "-0.25".to_owned(), "1000000".to_owned()]
        );
        assert_eq!(
            form.end,
            ["4000".to_owned(), "0".to_owned(), "4000".to_owned()]
        );
        form.profile = Some(DIGEST.into());
        let args = form.prepare(store()).unwrap().args;
        let at = |flag: &str| args.iter().position(|a| a == flag).unwrap() + 1;
        assert_eq!(args[at("--start")], "0,-0.25,1000000");
        assert_eq!(args[at("--end")], "4000,0,4000");
    }

    #[test]
    fn bad_numbers_are_refused_before_anything_runs() {
        for bad in ["", "  ", "abc", "1,5", "NaN", "inf", "-inf", "1e999"] {
            let mut form = plate();
            form.width = bad.into();
            let error = form.prepare(store()).unwrap_err();
            assert_eq!(error.field, "Width", "{bad:?}");
        }
        // A decimal comma parses to nothing: the message says what to use, not that the value
        // is infinite.
        let mut form = plate();
        form.thickness = "12,5".into();
        let error = form.prepare(store()).unwrap_err();
        assert!(error.message.contains("period"), "{}", error.message);
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
        // The engine counts bytes: 150 Cyrillic letters are 150 characters but 300 bytes.
        form.name = "ы".repeat(MAX_TEXT / 2 + 1);
        assert_eq!(form.prepare(store()).unwrap_err().field, "Name");
        assert!(form.prepare(store()).unwrap_err().message.contains("bytes"));
        form.name = "ы".repeat(100);
        assert!(form.prepare(store()).is_ok());
    }

    #[test]
    fn a_failed_definitions_refresh_is_reported_and_clears_stale_choices() {
        let choice = |digest: &str| Choice {
            digest: digest.into(),
            label: digest.into(),
            rect_mm: None,
        };
        let listed = Definitions {
            profiles: vec![choice("p")],
            materials: vec![choice("m")],
            ..Definitions::default()
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
        let decoded = beamcraft_json::decode(array.as_bytes(), &Limits::DEFAULT).unwrap();
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
        assert_eq!(created.revision_id, "r1");
        assert_eq!(created.manifest_digest, "d1");
        assert_eq!(created.sequence, 3);
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

    #[test]
    fn a_refusal_keeps_the_code_and_message() {
        let reply = r#"{"code":"INVALID_ARGUMENT","diagnostics":[],"message":"dimension is not positive","observed_context":null,"outcome":"rejected"}"#;
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
        // A07.2 definitions carry a stored display name; it leads the label.
        let reply = format!(
            r#"{{"definition_count":2,"definitions":[{{"density_kg_m3":7850,"digest":"{m}","display_name":"Structural steel","grade":"S355","kind":"material","used_by":[]}},{{"digest":"{DIGEST}","display_name":"Roof beams","height_mm":300,"kind":"rectangle","width_mm":200,"used_by":[]}}],"outcome":"definitions"}}"#,
            m = "36281e26dd4affda6b39d884fe8a8bb04deb71684215741c188dac0806710ba9"
        );
        let definitions = parse_definitions(Some(0), reply.as_bytes()).unwrap();
        assert_eq!(definitions.materials.len(), 1);
        assert_eq!(definitions.profiles.len(), 1);
        assert_eq!(definitions.profiles[0].digest, DIGEST);
        // The rectangle dimensions also feed the display-only ghost preview.
        assert_eq!(definitions.profiles[0].rect_mm, Some([200.0, 300.0]));
        assert_eq!(definitions.materials[0].rect_mm, None);
        assert_eq!(
            definitions.profiles[0].label,
            "Roof beams 200 x 300 mm (5d660ddd)"
        );
        assert_eq!(
            definitions.materials[0].label,
            "Structural steel 7850 kg/m3 (36281e26)"
        );
        // Definitions published before A07.2 have no display name: the designation or grade
        // and the kind still label them.
        let unnamed = format!(
            r#"{{"definition_count":2,"definitions":[{{"density_kg_m3":7850,"digest":"{m}","display_name":null,"grade":"S355","kind":"material","used_by":[]}},{{"digest":"{DIGEST}","display_name":null,"height_mm":300,"kind":"rectangle","width_mm":200,"used_by":[]}}],"outcome":"definitions"}}"#,
            m = "36281e26dd4affda6b39d884fe8a8bb04deb71684215741c188dac0806710ba9"
        );
        let definitions = parse_definitions(Some(0), unnamed.as_bytes()).unwrap();
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
        // No revision object in the reply: the head is unknown, not invented.
        assert_eq!(definitions.head, None);
        let with_head = r#"{"definitions":[],"outcome":"definitions","revision":{"revision_id":"r9","sequence":"9","manifest_digest":"abc"}}"#;
        let head = parse_definitions(Some(0), with_head.as_bytes())
            .unwrap()
            .head
            .unwrap();
        assert_eq!((head.sequence, head.manifest_digest.as_str()), (9, "abc"));
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
