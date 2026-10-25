// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! V06.1 properties editor, as pure logic: what the engine reports about the selected plate or
//! straight member (`inspect-elements`), whether that reading is still current, and the one
//! `beamcraft update-element` command an edit of the supported properties becomes.
//!
//! The UI computes no engineering value. Every number shown (mass, volume, dimensions) is the
//! engine's, and a value the reply does not carry is shown as unknown, never guessed. The only
//! checks made here are input hygiene on typed text (a finite decimal, and not beyond the
//! engine's published limits, [`DIMENSION_CAP_MM`] and [`COORDINATE_CAP_MM`]); the engine stays
//! the authority and its refusal (a stable code and message) is shown as it is.
//!
//! `update-element` supports a label, a material reference, a plate's thickness, an absolute
//! translation and a level. This editor offers the first four; plate width and length, a
//! member's profile and axis, and the level assignment have no editing path in this slice.

use crate::edit::sequence_of;
use crate::tools::{FormError, MAX_TEXT, ToolError, decode, rejection, text};
use beamcraft_json::Json;
use beamcraft_model::limits::{COORDINATE_CAP_MM, DIMENSION_CAP_MM};
use std::path::Path;

/// One accepted revision, as an engine reply names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionStamp {
    /// The revision's identity.
    pub revision_id: String,
    /// Its manifest digest, which every reading is pinned to.
    pub manifest_digest: String,
    /// Its sequence number.
    pub sequence: u64,
}

impl RevisionStamp {
    /// Reads the `revision` object of an engine reply.
    pub fn from_json(revision: &Json) -> Option<Self> {
        Some(Self {
            revision_id: text(revision, "revision_id")?,
            manifest_digest: text(revision, "manifest_digest")?,
            sequence: sequence_of(revision)?,
        })
    }

    /// A short label: the sequence and the start of the digest.
    pub fn label(&self) -> String {
        format!(
            "sequence {} ({}…)",
            self.sequence,
            self.manifest_digest
                .get(..12)
                .unwrap_or(&self.manifest_digest)
        )
    }
}

/// The quantities the engine returned for the element, each as reported (a missing one is
/// `None`: unknown, not zero).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Quantities {
    /// Net volume in mm3.
    pub volume_mm3: Option<f64>,
    /// The engine's status of that volume (`current` or another token).
    pub volume_status: Option<String>,
    /// Net mass in kg, as the engine's display value (the engine rounds it, the UI does not).
    pub mass_kg: Option<f64>,
    /// The engine's status of that mass (a mass needs a material with a density).
    pub mass_status: Option<String>,
    /// The density the mass was computed with, kg/m3.
    pub density_kg_m3: Option<f64>,
}

/// What `inspect-elements` reports about one element, read for the properties panel.
#[derive(Debug, Clone, PartialEq)]
pub struct Details {
    /// The element's id.
    pub id: String,
    /// `plate`, `beam` or `column`.
    pub kind: String,
    /// The revision the reading was taken from.
    pub revision: RevisionStamp,
    /// The element's label.
    pub name: String,
    /// The pinned material definition, `None` when the element has none.
    pub material: Option<String>,
    /// The material's display name, when the definition has one.
    pub material_name: Option<String>,
    /// Absolute placement translation in millimetres.
    pub translation_mm: [f64; 3],
    /// A plate's thickness in millimetres (`None` for a member).
    pub thickness_mm: Option<f64>,
    /// A plate's width and length in millimetres.
    pub plate_size_mm: Option<[f64; 2]>,
    /// A member's section width and height in millimetres.
    pub section_mm: Option<[f64; 2]>,
    /// A member's axis start and end in millimetres.
    pub axis_points_mm: Option<[[f64; 3]; 2]>,
    /// A member's profile name, when the definition has one.
    pub profile_name: Option<String>,
    /// The assigned level's id.
    pub level: Option<String>,
    /// The engine's quantities.
    pub quantities: Quantities,
}

fn unreadable() -> ToolError {
    ToolError::local("UNREADABLE_REPLY", "the CLI reply lacks a field")
}

fn floats<const N: usize>(json: &Json) -> Option<[f64; N]> {
    let items = json.array().ok()?;
    if items.len() != N {
        return None;
    }
    let mut out = [0.0; N];
    for (slot, item) in out.iter_mut().zip(items) {
        *slot = item.float().ok().filter(|v| v.is_finite())?;
    }
    Some(out)
}

fn optional_float(json: &Json, key: &str) -> Option<f64> {
    json.get(key).ok()?.float().ok().filter(|v| v.is_finite())
}

fn read_quantities(element: &Json) -> Quantities {
    let volume = element.get("quantities").and_then(|q| q.get("volume")).ok();
    let mass = element.get("quantities").and_then(|q| q.get("mass")).ok();
    Quantities {
        volume_mm3: volume.and_then(|v| optional_float(v, "mm3")),
        volume_status: volume.and_then(|v| text(v, "status")),
        mass_kg: mass.and_then(|m| optional_float(m, "kg_display")),
        mass_status: mass.and_then(|m| text(m, "status")),
        density_kg_m3: mass.and_then(|m| optional_float(m, "density_kg_m3")),
    }
}

/// Reads the reply of `inspect-elements` for one element. The editable fields (label, material,
/// translation, a plate's thickness) and the revision are required: without them the element's
/// properties are unknown and the reply is `UNREADABLE_REPLY`. Everything else is optional and
/// stays `None` when absent. An id the revision does not hold is `NOT_FOUND`.
pub fn parse_details(
    id: &str,
    exit_code: Option<i32>,
    stdout: &[u8],
) -> Result<Details, ToolError> {
    let reply = decode(stdout)?;
    if let Some(error) = rejection(&reply, exit_code) {
        return Err(error);
    }
    let revision = reply
        .get("revision")
        .ok()
        .and_then(RevisionStamp::from_json)
        .ok_or_else(unreadable)?;
    for key in ["plates", "members"] {
        let Ok(list) = reply.get(key).and_then(Json::array) else {
            continue;
        };
        let Some(element) = list
            .iter()
            .find(|e| text(e, "element_id").as_deref() == Some(id))
        else {
            continue;
        };
        let kind = text(element, "kind").ok_or_else(unreadable)?;
        let translation_mm = element
            .get("placement")
            .and_then(|p| p.get("translation_mm"))
            .ok()
            .and_then(floats::<3>)
            .ok_or_else(unreadable)?;
        let material = match element.get("material_digest").map_err(|_| unreadable())? {
            Json::Null => None,
            other => Some(other.string().map_err(|_| unreadable())?.to_owned()),
        };
        let dimensions = element.get("dimensions_mm").ok();
        let thickness_mm = match (kind.as_str(), dimensions) {
            ("plate", Some(d)) => Some(optional_float(d, "thickness").ok_or_else(unreadable)?),
            ("plate", None) => return Err(unreadable()),
            _ => None,
        };
        let plate_size_mm = dimensions
            .and_then(|d| Some([optional_float(d, "width")?, optional_float(d, "length")?]));
        let section = element.get("section_mm").ok();
        let section_mm =
            section.and_then(|s| Some([optional_float(s, "width")?, optional_float(s, "height")?]));
        let axis_points_mm = element
            .get("axis_points_mm")
            .and_then(Json::array)
            .ok()
            .and_then(|points| match points {
                [a, b] => Some([floats::<3>(a)?, floats::<3>(b)?]),
                _ => None,
            });
        return Ok(Details {
            id: id.to_owned(),
            kind,
            revision,
            name: text(element, "name").ok_or_else(unreadable)?,
            material,
            material_name: text(element, "material_name"),
            translation_mm,
            thickness_mm,
            plate_size_mm,
            section_mm,
            axis_points_mm,
            profile_name: text(element, "profile_name"),
            level: text(element, "level"),
            quantities: read_quantities(element),
        });
    }
    Err(ToolError::local(
        "NOT_FOUND",
        "the element is not a plate or straight member of the accepted revision",
    ))
}

fn show(value: f64) -> String {
    format!("{value}")
}

fn unknown() -> String {
    "unknown (not reported)".into()
}

impl Details {
    /// A plate takes a thickness edit.
    pub fn is_plate(&self) -> bool {
        self.kind == "plate"
    }

    /// The label, kind and translation the V05.2 edit panel keeps.
    pub fn info(&self) -> crate::edit::ElementInfo {
        crate::edit::ElementInfo {
            id: self.id.clone(),
            kind: self.kind.clone(),
            name: self.name.clone(),
            translation_mm: self.translation_mm,
        }
    }

    /// The read-only rows of the panel: a label and the engine's value, or `unknown` for one the
    /// engine did not report. Mass and volume are shown exactly as returned, with their status.
    pub fn read_only_rows(&self) -> Vec<(String, String)> {
        let mut rows = Vec::new();
        let size = |v: Option<[f64; 2]>| {
            v.map_or_else(unknown, |[a, b]| format!("{} x {} mm", show(a), show(b)))
        };
        if self.is_plate() {
            rows.push(("Width x length".into(), size(self.plate_size_mm)));
        } else {
            rows.push((
                "Profile".into(),
                self.profile_name.clone().unwrap_or_else(unknown),
            ));
            rows.push(("Section".into(), size(self.section_mm)));
            rows.push((
                "Axis".into(),
                self.axis_points_mm
                    .map_or_else(unknown, |[[ax, ay, az], [bx, by, bz]]| {
                        format!(
                            "{},{},{} to {},{},{} mm",
                            show(ax),
                            show(ay),
                            show(az),
                            show(bx),
                            show(by),
                            show(bz)
                        )
                    }),
            ));
        }
        rows.push((
            "Material".into(),
            match (&self.material, &self.material_name) {
                (None, _) => "none".into(),
                (Some(_), Some(name)) => name.clone(),
                (Some(digest), None) => format!("unnamed definition {}", short(digest)),
            },
        ));
        rows.push((
            "Level".into(),
            self.level.clone().unwrap_or_else(|| "none".into()),
        ));
        let q = &self.quantities;
        let with_status = |value: Option<f64>, unit: &str, status: &Option<String>| match value {
            Some(v) => match status.as_deref() {
                Some("current") => format!("{} {unit}", show(v)),
                Some(other) => format!("{} {unit} (engine status: {other})", show(v)),
                None => format!("{} {unit} (engine status not reported)", show(v)),
            },
            None => match status.as_deref() {
                Some(other) => format!("not available (engine status: {other})"),
                None => unknown(),
            },
        };
        rows.push((
            "Volume".into(),
            with_status(q.volume_mm3, "mm3", &q.volume_status),
        ));
        rows.push(("Mass".into(), with_status(q.mass_kg, "kg", &q.mass_status)));
        rows.push((
            "Density".into(),
            q.density_kg_m3
                .map_or_else(unknown, |d| format!("{} kg/m3", show(d))),
        ));
        rows
    }
}

fn short(digest: &str) -> &str {
    digest.get(..8).unwrap_or(digest)
}

/// Whether the details are still the head's, as far as the window knows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// Read from the head revision (or from one newer than the last head the window saw).
    Current,
    /// Read from a revision the head has since moved past or away from.
    Stale {
        /// The revision the details were read from.
        read: RevisionStamp,
        /// The head the window has seen since.
        head: RevisionStamp,
    },
    /// The properties could not be read; nothing is shown as a value.
    Unknown {
        /// Why, in the engine's or the runner's words.
        reason: String,
    },
}

impl State {
    /// A one-line label for the panel.
    pub fn label(&self) -> String {
        match self {
            Self::Current => "current".into(),
            Self::Stale { read, head } => format!(
                "stale: read from revision {}, the head is {}",
                read.label(),
                head.label()
            ),
            Self::Unknown { reason } => format!("unknown: {reason}"),
        }
    }
}

/// Why a failed reading leaves the properties unknown.
fn unknown_reason(error: &ToolError) -> String {
    let what = match error.code.as_str() {
        "NOT_FOUND" => {
            "the element is not a plate or straight member of the accepted revision; the \
             properties editor supports only those"
        }
        "UNREADABLE_REPLY" => {
            "the engine reply was too large for the control-message limit or unreadable"
        }
        "CLI_FAILED" if error.message.contains("size limit") => {
            "the engine reply exceeds the size limit (a very large model)"
        }
        _ => "the engine could not report the properties",
    };
    format!("{what} ({error})")
}

/// The editable fields, as typed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Form {
    /// `--name`.
    pub name: String,
    /// `--material-digest` (`None` is the engine's explicit `none`).
    pub material: Option<String>,
    /// `--thickness` (plates only), mm.
    pub thickness: String,
    /// `--translation`, mm, absolute.
    pub translation: [String; 3],
}

impl Form {
    /// The form as the engine reports the element: nothing is guessed or rounded.
    pub fn of(details: &Details) -> Self {
        Self {
            name: details.name.clone(),
            material: details.material.clone(),
            thickness: details.thickness_mm.map(show).unwrap_or_default(),
            translation: details.translation_mm.map(show),
        }
    }
}

/// One `update-element` command, ready to run: only the fields that differ from the reading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropertyEdit {
    /// The arguments exactly as passed to the CLI, starting with the command name.
    pub args: Vec<String>,
    /// The labels of the changed properties, for the status line.
    pub changed: Vec<&'static str>,
}

impl PropertyEdit {
    /// The argument list, one JSON string per line (not a shell command line).
    pub fn argument_list(&self) -> String {
        self.args
            .iter()
            .map(|a| crate::json_string(a))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn refuse(field: &'static str, message: impl Into<String>) -> FormError {
    FormError {
        field,
        message: message.into(),
    }
}

/// A typed decimal: trimmed, finite and within `limit` in absolute value. The text is forwarded
/// as typed (the CLI parses it the same way).
fn bounded(field: &'static str, typed: &str, limit: f64) -> Result<(String, f64), FormError> {
    let typed = typed.trim();
    if typed.is_empty() || typed.len() > 32 {
        return Err(refuse(field, "enter a number"));
    }
    let value = typed
        .parse::<f64>()
        .map_err(|_| refuse(field, format!("\"{typed}\" is not a number")))?;
    if !value.is_finite() {
        return Err(refuse(field, format!("\"{typed}\" is not a finite number")));
    }
    if value.abs() > limit {
        return Err(refuse(
            field,
            format!("\"{typed}\" is beyond the limit of {limit} mm"),
        ));
    }
    Ok((typed.to_owned(), value))
}

/// The properties panel's state machine: the reading, its currency, the form, and the preview,
/// commit and cancel of one `update-element`.
#[derive(Debug, Clone, Default)]
pub struct Properties {
    id: Option<String>,
    read: Option<Details>,
    failure: Option<String>,
    head: Option<RevisionStamp>,
    /// The editable fields, as typed.
    pub form: Form,
    /// The author recorded as `--actor-id` of a property edit, when the window has one.
    pub actor: Option<String>,
    baseline: Form,
    staged: Option<PropertyEdit>,
    busy: bool,
}

impl Properties {
    /// The selected element's id.
    pub fn selected(&self) -> Option<&str> {
        self.id.as_deref()
    }

    /// The reading, when it could be taken.
    pub fn details(&self) -> Option<&Details> {
        self.read.as_ref()
    }

    /// Takes the outcome of a details lookup for `id`: the form is reset to what the engine
    /// reported, a staged edit is dropped, and a failed lookup leaves the properties unknown.
    pub fn load(&mut self, id: &str, result: Result<Details, ToolError>) {
        self.id = Some(id.to_owned());
        self.staged = None;
        match result {
            Ok(details) => {
                self.baseline = Form::of(&details);
                self.form = self.baseline.clone();
                self.read = Some(details);
                self.failure = None;
            }
            Err(error) => {
                self.baseline = Form::default();
                self.form = Form::default();
                self.read = None;
                self.failure = Some(unknown_reason(&error));
            }
        }
    }

    /// Clears the selection, the reading and any staged edit.
    pub fn clear(&mut self) {
        let head = self.head.take();
        *self = Self {
            head,
            busy: self.busy,
            ..Self::default()
        };
    }

    /// Records the head the window last saw (`None`: it could not be read).
    pub fn observe_head(&mut self, head: Option<RevisionStamp>) {
        self.head = head;
    }

    /// Whether the head is known, so that a `Current` state has been checked against it.
    pub fn head_known(&self) -> bool {
        self.head.is_some()
    }

    /// The reading's state, `None` when nothing is selected. A reading is stale when its
    /// revision differs from the head's and is not newer than it; a reading newer than the
    /// last head seen (the view has not reloaded yet) is current.
    pub fn state(&self) -> Option<State> {
        self.id.as_ref()?;
        let Some(read) = &self.read else {
            return Some(State::Unknown {
                reason: self
                    .failure
                    .clone()
                    .unwrap_or_else(|| "the properties were not read".into()),
            });
        };
        Some(match &self.head {
            Some(head)
                if head.manifest_digest != read.revision.manifest_digest
                    && read.revision.sequence <= head.sequence =>
            {
                State::Stale {
                    read: read.revision.clone(),
                    head: head.clone(),
                }
            }
            _ => State::Current,
        })
    }

    /// The `update-element` command for the form: only the fields that differ from the reading.
    /// Refused while the reading is stale or unknown, when nothing changed, and for a typed value
    /// that is not a finite number within the engine's limits. Nothing is run.
    pub fn prepare(&self, store: &Path) -> Result<PropertyEdit, FormError> {
        let id = self
            .id
            .as_ref()
            .ok_or_else(|| refuse("Selection", "click an element first"))?;
        match self.state() {
            Some(State::Current) => {}
            Some(State::Stale { read, head }) => {
                return Err(refuse(
                    "Element",
                    format!(
                        "the properties were read from revision {} but the head is {}: re-read them first",
                        read.label(),
                        head.label()
                    ),
                ));
            }
            Some(State::Unknown { reason }) => {
                return Err(refuse(
                    "Element",
                    format!("the properties are unknown, nothing can be edited: {reason}"),
                ));
            }
            None => return Err(refuse("Selection", "click an element first")),
        }
        let details = self
            .read
            .as_ref()
            .ok_or_else(|| refuse("Element", "the properties were not read"))?;
        let mut args = vec![
            "update-element".to_owned(),
            "--store".into(),
            store.to_string_lossy().into_owned(),
            "--id".into(),
            id.clone(),
        ];
        let mut changed = Vec::new();
        if self.form.name != self.baseline.name {
            let name = self.form.name.trim();
            if name.is_empty() {
                return Err(refuse("Name", "enter a label"));
            }
            if name.chars().count() > MAX_TEXT {
                return Err(refuse("Name", format!("at most {MAX_TEXT} characters")));
            }
            args.extend(["--name".into(), name.to_owned()]);
            changed.push("name");
        }
        if self.form.material != self.baseline.material {
            args.extend([
                "--material-digest".into(),
                self.form.material.clone().unwrap_or_else(|| "none".into()),
            ]);
            changed.push("material");
        }
        if details.is_plate() && self.form.thickness != self.baseline.thickness {
            let (typed, value) = bounded("Thickness", &self.form.thickness, DIMENSION_CAP_MM)?;
            if value <= 0.0 {
                return Err(refuse("Thickness", "must be greater than zero"));
            }
            args.extend(["--thickness".into(), typed]);
            changed.push("thickness");
        }
        if self.form.translation != self.baseline.translation {
            let [x, y, z] = &self.form.translation;
            let point = [x, y, z]
                .map(|part| bounded("Position", part, COORDINATE_CAP_MM).map(|(typed, _)| typed));
            let [x, y, z] = point;
            args.extend(["--translation".into(), format!("{},{},{}", x?, y?, z?)]);
            changed.push("position");
        }
        if changed.is_empty() {
            return Err(refuse("Properties", "nothing was changed"));
        }
        crate::tools::push_actor(&mut args, self.actor.as_deref());
        Ok(PropertyEdit { args, changed })
    }

    /// Stages the command for review; nothing is run or written.
    pub fn preview(&mut self, store: &Path) -> Result<&PropertyEdit, FormError> {
        let edit = self.prepare(store)?;
        Ok(self.staged.insert(edit))
    }

    /// The staged command, if any.
    pub fn staged(&self) -> Option<&PropertyEdit> {
        self.staged.as_ref()
    }

    /// True while a commit runs.
    pub fn busy(&self) -> bool {
        self.busy
    }

    /// True when a command is staged, the reading is still current and the form still produces it.
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

    /// Takes the staged command to run and marks the panel busy.
    pub fn begin_commit(&mut self, store: &Path) -> Option<PropertyEdit> {
        if !self.can_commit(store) {
            return None;
        }
        self.busy = true;
        self.staged.take()
    }

    /// Ends the running commit.
    pub fn finish(&mut self) {
        self.busy = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "019a7bf0-0000-7000-8000-000000000001";
    const BEAM: &str = "019a7bf0-0000-7000-8000-000000000002";
    const OTHER: &str = "019a7bf0-0000-7000-8000-000000000009";

    fn stamp(sequence: u64, digest: &str) -> String {
        format!(
            "{{\"revision_id\":\"r{sequence}\",\"sequence\":\"{sequence}\",\"manifest_digest\":\"{digest}\"}}"
        )
    }

    fn reply(sequence: u64, digest: &str) -> Vec<u8> {
        format!(
            "{{\"outcome\":\"plate_inspection\",\"revision\":{},\
\"plates\":[{{\"element_id\":\"{ID}\",\"kind\":\"plate\",\"name\":\"P1\",\
\"material_digest\":\"abcdef0123456789\",\"material_name\":\"S355\",\
\"dimensions_mm\":{{\"width\":1000.0,\"length\":500.0,\"thickness\":10.0}},\
\"placement\":{{\"translation_mm\":[100.0,-20.5,0.0]}},\
\"quantities\":{{\"volume\":{{\"mm3\":5000000.0,\"status\":\"current\"}},\
\"mass\":{{\"kg\":39.25,\"kg_display\":39.3,\"density_kg_m3\":7850.0,\"status\":\"current\"}}}},\
\"level\":null}}],\
\"members\":[{{\"element_id\":\"{BEAM}\",\"kind\":\"beam\",\"name\":\"B1\",\
\"material_digest\":null,\"profile_name\":\"R200x300\",\
\"section_mm\":{{\"width\":200.0,\"height\":300.0}},\
\"axis_points_mm\":[[0.0,0.0,0.0],[3000.0,0.0,0.0]],\
\"placement\":{{\"translation_mm\":[0.0,0.0,0.0]}},\
\"quantities\":{{\"volume\":{{\"mm3\":1.0,\"status\":\"current\"}},\
\"mass\":{{\"kg\":null,\"kg_display\":null,\"density_kg_m3\":null,\"status\":\"no_material\"}}}}}}]}}",
            stamp(sequence, digest)
        )
        .into_bytes()
    }

    fn details(id: &str, sequence: u64, digest: &str) -> Details {
        parse_details(id, Some(0), &reply(sequence, digest)).unwrap()
    }

    fn head(sequence: u64, digest: &str) -> RevisionStamp {
        RevisionStamp {
            revision_id: format!("r{sequence}"),
            manifest_digest: digest.into(),
            sequence,
        }
    }

    fn loaded() -> Properties {
        let mut p = Properties::default();
        p.load(ID, Ok(details(ID, 3, "d3")));
        p.observe_head(Some(head(3, "d3")));
        p
    }

    fn store() -> &'static Path {
        Path::new("S")
    }

    #[test]
    fn the_details_of_a_plate_and_a_member_are_read_as_reported() {
        let plate = details(ID, 3, "d3");
        assert_eq!(plate.kind, "plate");
        assert_eq!(plate.thickness_mm, Some(10.0));
        assert_eq!(plate.translation_mm, [100.0, -20.5, 0.0]);
        assert_eq!(plate.revision.sequence, 3);
        assert_eq!(plate.quantities.mass_kg, Some(39.3));
        let rows = plate.read_only_rows();
        let row = |label: &str| {
            rows.iter()
                .find(|(l, _)| l == label)
                .map(|(_, v)| v.clone())
                .unwrap()
        };
        assert_eq!(row("Width x length"), "1000 x 500 mm");
        assert_eq!(row("Mass"), "39.3 kg");
        assert_eq!(row("Volume"), "5000000 mm3");
        assert_eq!(row("Material"), "S355");
        assert_eq!(row("Level"), "none");
        let beam = details(BEAM, 3, "d3");
        assert!(!beam.is_plate());
        assert_eq!(beam.thickness_mm, None);
        let rows = beam.read_only_rows();
        assert!(rows.contains(&("Profile".into(), "R200x300".into())));
        assert!(rows.contains(&("Axis".into(), "0,0,0 to 3000,0,0 mm".into())));
        assert!(rows.contains(&("Material".into(), "none".into())));
    }

    #[test]
    fn a_long_axis_value_is_reported_in_full_for_the_panel() {
        let mut beam = details(BEAM, 3, "d3");
        beam.axis_points_mm = Some([
            [12345.678, 23456.789, 34567.891],
            [12645.678, 23756.789, 34867.891],
        ]);
        // The exact expected value from issue 65; the UI reports it, never rounds or clips it.
        assert!(beam.read_only_rows().contains(&(
            "Axis".into(),
            "12345.678,23456.789,34567.891 to 12645.678,23756.789,34867.891 mm".into()
        )));
    }

    #[test]
    fn a_value_the_engine_did_not_report_is_labelled_unknown_never_guessed() {
        let beam = details(BEAM, 3, "d3");
        let rows = beam.read_only_rows();
        // The mass has no value and the engine said why; the UI invents no number.
        assert!(rows.contains(&(
            "Mass".into(),
            "not available (engine status: no_material)".into()
        )));
        assert!(rows.contains(&("Density".into(), "unknown (not reported)".into())));
        let mut bare = details(ID, 3, "d3");
        bare.quantities = Quantities::default();
        bare.plate_size_mm = None;
        let rows = bare.read_only_rows();
        assert!(rows.contains(&("Mass".into(), "unknown (not reported)".into())));
        assert!(rows.contains(&("Width x length".into(), "unknown (not reported)".into())));
        // A mass whose status is not current says so.
        bare.quantities.mass_kg = Some(1.5);
        bare.quantities.mass_status = Some("stale".into());
        let rows = bare.read_only_rows();
        assert!(rows.contains(&("Mass".into(), "1.5 kg (engine status: stale)".into())));
    }

    #[test]
    fn a_reply_without_an_editable_field_is_unreadable_and_a_missing_element_is_not_found() {
        let broken = String::from_utf8(reply(3, "d3"))
            .unwrap()
            .replace("\"name\":\"P1\",", "");
        assert_eq!(
            parse_details(ID, Some(0), broken.as_bytes())
                .unwrap_err()
                .code,
            "UNREADABLE_REPLY"
        );
        let no_thickness = String::from_utf8(reply(3, "d3"))
            .unwrap()
            .replace("\"thickness\":10.0", "\"x\":1.0");
        assert_eq!(
            parse_details(ID, Some(0), no_thickness.as_bytes())
                .unwrap_err()
                .code,
            "UNREADABLE_REPLY"
        );
        let no_revision = String::from_utf8(reply(3, "d3"))
            .unwrap()
            .replace("\"revision\"", "\"rev\"");
        assert_eq!(
            parse_details(ID, Some(0), no_revision.as_bytes())
                .unwrap_err()
                .code,
            "UNREADABLE_REPLY"
        );
        assert_eq!(
            parse_details(OTHER, Some(0), &reply(3, "d3"))
                .unwrap_err()
                .code,
            "NOT_FOUND"
        );
        assert_eq!(
            parse_details(ID, Some(0), b"not json").unwrap_err().code,
            "UNREADABLE_REPLY"
        );
        let refused = br#"{"outcome":"rejected","code":"BACKEND_UNAVAILABLE","message":"no"}"#;
        assert_eq!(
            parse_details(ID, Some(2), refused).unwrap_err().code,
            "BACKEND_UNAVAILABLE"
        );
    }

    #[test]
    fn the_state_is_current_stale_or_unknown() {
        let mut p = loaded();
        assert_eq!(p.state(), Some(State::Current));
        assert!(p.head_known());
        // The head moved on: the reading is stale and says which revisions.
        p.observe_head(Some(head(5, "d5")));
        let state = p.state().unwrap();
        assert!(
            matches!(&state, State::Stale { read, head } if read.sequence == 3 && head.sequence == 5)
        );
        assert!(
            state
                .label()
                .starts_with("stale: read from revision sequence 3")
        );
        // A reading newer than the last head the view saw is not stale.
        p.observe_head(Some(head(2, "d2")));
        assert_eq!(p.state(), Some(State::Current));
        // Same sequence, different digest: not the same revision, so not current.
        p.observe_head(Some(head(3, "other")));
        assert!(matches!(p.state(), Some(State::Stale { .. })));
        // An unknown head is not a reason to call the reading stale.
        p.observe_head(None);
        assert_eq!(p.state(), Some(State::Current));
        assert!(!p.head_known());
        // A reading that failed is unknown, with the reason.
        let mut q = Properties::default();
        assert_eq!(q.state(), None);
        let too_big = ToolError {
            code: "CLI_FAILED".into(),
            message: "the CLI reply exceeds the size limit".into(),
        };
        q.load(ID, Err(too_big));
        let Some(State::Unknown { reason }) = q.state() else {
            panic!("expected unknown");
        };
        assert!(reason.contains("size limit") && reason.contains("CLI_FAILED"));
        assert!(q.details().is_none());
        assert_eq!(q.form, Form::default());
        let unsupported = ToolError {
            code: "NOT_FOUND".into(),
            message: "x".into(),
        };
        q.load(ID, Err(unsupported));
        assert!(q.state().unwrap().label().contains("supports only those"));
        q.clear();
        assert_eq!(q.state(), None);
    }

    #[test]
    fn the_command_carries_only_the_changed_fields() {
        let mut p = loaded();
        assert_eq!(p.prepare(store()).unwrap_err().field, "Properties");
        p.form.name = "  Gusset  ".into();
        p.form.thickness = "12.5".into();
        p.form.translation = ["100".into(), "-20.5".into(), "300".into()];
        p.form.material = None;
        let edit = p.prepare(store()).unwrap();
        assert_eq!(
            edit.args,
            [
                "update-element",
                "--store",
                "S",
                "--id",
                ID,
                "--name",
                "Gusset",
                "--material-digest",
                "none",
                "--thickness",
                "12.5",
                "--translation",
                "100,-20.5,300"
            ]
        );
        assert_eq!(edit.changed, ["name", "material", "thickness", "position"]);
        // One field alone.
        let mut p = loaded();
        p.form.material = Some("feedface".into());
        assert_eq!(
            p.prepare(store()).unwrap().args.get(5..).unwrap(),
            ["--material-digest", "feedface"]
        );
        // A member takes no thickness.
        let mut m = Properties::default();
        m.load(BEAM, Ok(details(BEAM, 3, "d3")));
        m.form.thickness = "5".into();
        assert_eq!(m.prepare(store()).unwrap_err().field, "Properties");
    }

    #[test]
    fn typed_values_that_are_not_numbers_or_are_beyond_the_limits_are_refused() {
        let thickness = |typed: &str| {
            let mut p = loaded();
            p.form.thickness = typed.into();
            p.prepare(store())
        };
        for bad in [
            "", "  ", "abc", "1,5", "NaN", "nan", "inf", "-inf", "1e999", "0", "-3", "-0",
            "1000001", "1e9", "0x10",
        ] {
            let error = thickness(bad).unwrap_err();
            assert_eq!(error.field, "Thickness", "{bad:?}: {error}");
        }
        assert!(thickness("1000000").is_ok());
        assert!(thickness("0.001").is_ok());
        let position = |x: &str, y: &str| {
            let mut p = loaded();
            p.form.translation = [x.into(), y.into(), "0".into()];
            p.prepare(store())
        };
        for bad in ["", "NaN", "inf", "1e999", "1000000001", "-1e12", "x"] {
            assert_eq!(position(bad, "0").unwrap_err().field, "Position", "{bad:?}");
            assert_eq!(position("0", bad).unwrap_err().field, "Position", "{bad:?}");
        }
        // A negative coordinate is fine; the cap is on the magnitude.
        assert!(position("-5000", "1e9").is_ok());
        let name = |typed: &str| {
            let mut p = loaded();
            p.form.name = typed.into();
            p.prepare(store())
        };
        assert_eq!(name("   ").unwrap_err().field, "Name");
        assert_eq!(name(&"x".repeat(MAX_TEXT + 1)).unwrap_err().field, "Name");
        assert!(name(&"x".repeat(MAX_TEXT)).is_ok());
        // An over-long typed number never reaches the command.
        assert!(thickness(&"1".repeat(40)).is_err());
    }

    #[test]
    fn a_stale_or_unknown_reading_is_never_edited() {
        let mut p = loaded();
        p.form.name = "N".into();
        p.observe_head(Some(head(4, "d4")));
        let error = p.prepare(store()).unwrap_err();
        assert_eq!(error.field, "Element");
        assert!(error.message.contains("re-read"));
        assert!(p.preview(store()).is_err());
        let mut q = Properties::default();
        q.load(ID, Err(ToolError::local("UNREADABLE_REPLY", "x")));
        let error = q.prepare(store()).unwrap_err();
        assert!(error.message.contains("unknown"));
        assert_eq!(
            Properties::default().prepare(store()).unwrap_err().field,
            "Selection"
        );
    }

    #[test]
    fn preview_commit_and_cancel_follow_the_staged_command() {
        let mut p = loaded();
        p.form.name = "N".into();
        assert!(!p.can_commit(store()));
        p.preview(store()).unwrap();
        assert!(p.can_commit(store()));
        // Editing after the preview disables Commit until it is previewed again.
        p.form.name = "M".into();
        assert!(!p.can_commit(store()));
        assert!(p.begin_commit(store()).is_none());
        p.preview(store()).unwrap();
        // The head moving after the preview also disables Commit.
        p.observe_head(Some(head(9, "d9")));
        assert!(!p.can_commit(store()));
        p.observe_head(Some(head(3, "d3")));
        assert!(p.can_commit(store()));
        // A different store is a different command.
        assert!(!p.can_commit(Path::new("T")));
        let edit = p.begin_commit(store()).unwrap();
        assert!(edit.argument_list().contains("\"--name\""));
        assert!(p.busy() && p.staged().is_none());
        p.preview(store()).unwrap();
        assert!(!p.can_commit(store()), "not while busy");
        p.finish();
        assert!(p.can_commit(store()));
        p.cancel();
        assert!(p.staged().is_none() && !p.can_commit(store()));
        // A new reading drops the staged command and resets the form.
        p.preview(store()).unwrap();
        p.load(ID, Ok(details(ID, 4, "d4")));
        assert!(p.staged().is_none());
        assert_eq!(p.form.name, "P1");
    }

    #[test]
    fn the_form_is_filled_from_the_engine_reading_exactly() {
        let form = Form::of(&details(ID, 3, "d3"));
        assert_eq!(form.name, "P1");
        assert_eq!(form.material.as_deref(), Some("abcdef0123456789"));
        assert_eq!(form.thickness, "10");
        assert_eq!(form.translation, ["100", "-20.5", "0"]);
        assert_eq!(Form::of(&details(BEAM, 3, "d3")).thickness, "");
    }
}
