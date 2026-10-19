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

pub(crate) fn number(field: &'static str, text: &str) -> Result<String, FormError> {
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

fn decode(stdout: &[u8]) -> Result<Json, ToolError> {
    json::decode(stdout, &message::read_limits())
        .map_err(|_| ToolError::local("UNREADABLE_REPLY", "the CLI reply is not valid JSON"))
}

pub(crate) fn text(json: &Json, key: &str) -> Option<String> {
    json.get(key)
        .ok()
        .and_then(|v| v.string().ok())
        .map(str::to_owned)
}

/// Decodes a reply and refuses it when the CLI reported a rejection: what every query and
/// change parser reads next. Even a zero exit status cannot turn a rejection into a success.
pub(crate) fn reply_json(exit_code: Option<i32>, stdout: &[u8]) -> Result<Json, ToolError> {
    let reply = decode(stdout)?;
    match rejection(&reply, exit_code) {
        Some(error) => Err(error),
        None => Ok(reply),
    }
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
        match self.active {
            ToolKind::Plate => self.plate.prepare(store),
            ToolKind::Member => self.member.prepare(store),
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
