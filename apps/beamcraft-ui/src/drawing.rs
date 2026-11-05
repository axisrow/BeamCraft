// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D03 drawing document window: freshness, publication and version-bound export.
//!
//! The window reads a document through the same `inspect-drawing` command a user runs,
//! publishes its head as a new immutable version through `publish-drawing` with fresh
//! identities, lists the published versions through `inspect-drawing-versions` and exports
//! one exact version through `export-drawing`. The UI computes no engineering geometry:
//! every export reopens the version's own archived source inside the engine. Creating and
//! refreshing a document stay CLI commands for now, and the window says so instead of
//! hiding the gap.

use crate::command::{self, Output};
use crate::sheet::fresh_id;
use crate::stats::Stats;
use crate::tools::{ToolError, decode, rejection, text};
use beamcraft_json::Json;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::Receiver;

/// One reading of the document head, as the inspect reply reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    /// The revision every edit names as its expected head.
    pub revision_id: String,
    /// The drawing name.
    pub name: String,
    /// `draft` or `published`.
    pub state: String,
    /// The version this exact head was published as, if it was.
    pub version: Option<String>,
    /// The newest published version of the document.
    pub latest_published: Option<String>,
    /// `current`, `stale`, `unavailable` or `unknown`: how the live model store compares
    /// with the archived source of the head.
    pub freshness: String,
}

/// One published version as the versions reply reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct Version {
    /// The immutable version identity an export names.
    pub id: String,
    /// The model revision the version's archived source was captured from.
    pub source_revision_id: String,
}

/// Reads the `inspect-drawing` reply into the window's document mirror. `None` means the
/// reply is malformed for this window; nothing is guessed from partial fields. Without a
/// model store the freshness is `unknown`, never invented.
pub fn parse_document(reply: &Json) -> Option<Document> {
    let version = match reply.get("version_id") {
        Ok(Json::Null) | Err(_) => None,
        Ok(value) => Some(value.string().ok()?.to_owned()),
    };
    let latest_published = match reply.get("latest_published_version_id") {
        Ok(Json::Null) | Err(_) => None,
        Ok(value) => Some(value.string().ok()?.to_owned()),
    };
    let freshness = reply
        .get("freshness")
        .ok()
        .and_then(|value| value.string().ok())
        .map(str::to_owned)
        .unwrap_or_else(|| "unknown".to_owned());
    Some(Document {
        revision_id: text(reply, "revision_id")?,
        name: text(reply, "name")?,
        state: text(reply, "state")?,
        version,
        latest_published,
        freshness,
    })
}

/// Reads one entry of the `inspect-drawing-versions` list; `None` on a malformed entry.
fn version(reply: &Json) -> Option<Version> {
    Some(Version {
        id: text(reply, "version_id")?,
        source_revision_id: text(reply, "source_revision_id")?,
    })
}

/// Reads the `inspect-drawing-versions` reply into the published-version list, in journal
/// order. `None` means the reply is malformed; nothing is guessed from partial entries.
pub fn parse_versions(reply: &Json) -> Option<Vec<Version>> {
    reply
        .get("versions")
        .ok()?
        .array()
        .ok()?
        .iter()
        .map(version)
        .collect::<Option<Vec<_>>>()
}

/// The `inspect-drawing` arguments, with the model store when the window knows one: it
/// turns the freshness from `unknown` into `current`, `stale` or `unavailable`.
pub fn load_args(document_dir: &str, model_store: Option<&Path>) -> Vec<String> {
    let mut args = vec![
        "inspect-drawing".to_owned(),
        "--document-dir".to_owned(),
        document_dir.to_owned(),
    ];
    if let Some(store) = model_store {
        args.extend([
            "--model-store".to_owned(),
            store.to_string_lossy().into_owned(),
        ]);
    }
    args
}

/// The `publish-drawing` arguments: the loaded head as the expected base and fresh
/// operation, revision, actor and version identities for the immutable new version.
pub fn publish_args(
    document_dir: &str,
    expected_head: &str,
    op_id: &str,
    revision_id: &str,
    actor_id: &str,
    version_id: &str,
) -> Vec<String> {
    vec![
        "publish-drawing".to_owned(),
        "--document-dir".to_owned(),
        document_dir.to_owned(),
        "--expected-head".to_owned(),
        expected_head.to_owned(),
        "--op-id".to_owned(),
        op_id.to_owned(),
        "--revision-id".to_owned(),
        revision_id.to_owned(),
        "--actor-id".to_owned(),
        actor_id.to_owned(),
        "--version-id".to_owned(),
        version_id.to_owned(),
    ]
}

/// The `inspect-drawing-versions` arguments.
pub fn versions_args(document_dir: &str) -> Vec<String> {
    vec![
        "inspect-drawing-versions".to_owned(),
        "--document-dir".to_owned(),
        document_dir.to_owned(),
    ]
}

/// The `export-drawing` arguments: one exact published version, an SVG by default or a
/// print-ready PDF page, to a destination that must not exist yet.
pub fn export_args(document_dir: &str, version_id: &str, out: &str, pdf: bool) -> Vec<String> {
    let mut args = vec![
        "export-drawing".to_owned(),
        "--document-dir".to_owned(),
        document_dir.to_owned(),
        "--version-id".to_owned(),
        version_id.to_owned(),
        "--out".to_owned(),
        out.to_owned(),
    ];
    if pdf {
        args.extend(["--format".to_owned(), "pdf".to_owned()]);
    }
    args
}

/// What a finished background command was doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Job {
    /// `inspect-drawing`: replace the whole reading, then read the versions.
    Load,
    /// `publish-drawing`: a version was published; reload afterwards.
    Publish,
    /// `inspect-drawing-versions`: replace the version list.
    Versions,
    /// `export-drawing`: a file was written.
    Export,
}

/// The drawing document window state: one document directory at a time.
#[derive(Default)]
pub struct Manager {
    /// Whether the window is shown.
    pub open: bool,
    doc: Option<Document>,
    /// The published versions of the loaded document, in journal order.
    versions: Option<Vec<Version>>,
    /// The document directory text field.
    document_dir: String,
    /// The version identity the next export names, kept on the newest published version.
    version_field: String,
    /// The export destination text field.
    export_path: String,
    /// Whether the export writes PDF instead of SVG.
    export_pdf: bool,
    note: Option<Result<String, String>>,
    job: Option<(Job, Receiver<Result<Output, ToolError>>)>,
}

impl Manager {
    /// Shows the window and advances its background command, when open.
    pub fn window(
        &mut self,
        ctx: &egui::Context,
        cli: &Path,
        model_store: Option<&Path>,
        actor: Option<&str>,
        stats: &Arc<Stats>,
    ) {
        if !self.open {
            return;
        }
        self.poll(cli, model_store, stats);
        let mut open = self.open;
        egui::Window::new("Drawing document")
            .open(&mut open)
            .resizable(true)
            .show(ctx, |ui| self.body(ui, cli, model_store, actor, stats));
        self.open = open;
    }

    fn body(
        &mut self,
        ui: &mut egui::Ui,
        cli: &Path,
        model_store: Option<&Path>,
        actor: Option<&str>,
        stats: &Arc<Stats>,
    ) {
        ui.horizontal(|ui| {
            ui.label("Document");
            ui.add(
                egui::TextEdit::singleline(&mut self.document_dir)
                    .desired_width(280.0)
                    .hint_text("path to the document directory"),
            );
            if ui
                .add_enabled(
                    self.job.is_none() && !self.document_dir.is_empty(),
                    egui::Button::new("Load"),
                )
                .clicked()
            {
                self.note = None;
                self.start(
                    cli,
                    stats,
                    Job::Load,
                    load_args(&self.document_dir, model_store),
                );
            }
        });
        let Some(doc) = self.doc.clone() else {
            ui.label(
                "Load a drawing document directory to publish a version or export one. \
                 Creating and refreshing a document stay CLI commands.",
            );
            self.note_view(ui);
            return;
        };
        ui.separator();
        ui.label(format!(
            "{} — {}, freshness {}",
            doc.name, doc.state, doc.freshness
        ));
        match doc.latest_published.as_deref() {
            Some(version) => ui.label(format!("Newest published version {version}.")),
            None => ui.label("Never published."),
        };
        if ui
            .add_enabled(
                self.job.is_none(),
                egui::Button::new("Publish as a new version"),
            )
            .clicked()
        {
            self.note = None;
            self.publish(cli, actor, stats, &doc);
        }
        ui.separator();
        ui.label("Published versions:");
        egui::ScrollArea::vertical()
            .max_height(90.0)
            .show(ui, |ui| match &self.versions {
                None => {
                    ui.label("Not read yet.");
                }
                Some(versions) if versions.is_empty() => {
                    ui.label("No published versions yet.");
                }
                Some(versions) => {
                    for entry in versions {
                        ui.label(format!(
                            "{} — source {}",
                            entry.id, entry.source_revision_id
                        ));
                    }
                }
            });
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Version");
            ui.add(
                egui::TextEdit::singleline(&mut self.version_field)
                    .desired_width(280.0)
                    .hint_text("a published version identity"),
            );
        });
        ui.horizontal(|ui| {
            ui.label("File");
            ui.add(
                egui::TextEdit::singleline(&mut self.export_path)
                    .desired_width(240.0)
                    .hint_text("new destination file"),
            );
            ui.checkbox(&mut self.export_pdf, "PDF");
            if ui
                .add_enabled(
                    self.job.is_none()
                        && !self.version_field.is_empty()
                        && !self.export_path.is_empty(),
                    egui::Button::new("Export"),
                )
                .clicked()
            {
                self.note = None;
                let args = export_args(
                    &self.document_dir,
                    &self.version_field,
                    &self.export_path,
                    self.export_pdf,
                );
                self.start(cli, stats, Job::Export, args);
            }
        });
        self.note_view(ui);
    }

    fn note_view(&mut self, ui: &mut egui::Ui) {
        match &self.note {
            Some(Ok(message)) => {
                ui.label(message);
            }
            Some(Err(message)) => {
                ui.colored_label(egui::Color32::from_rgb(230, 80, 60), message);
            }
            None => {}
        }
    }

    /// Publishes the loaded head through `publish-drawing` with fresh identities; the actor
    /// is the window's author when it has one, otherwise a fresh identity.
    fn publish(&mut self, cli: &Path, actor: Option<&str>, stats: &Arc<Stats>, doc: &Document) {
        let missing = || "no identity could be generated".to_owned();
        let Some(op) = fresh_id() else {
            self.note = Some(Err(missing()));
            return;
        };
        let Some(revision) = fresh_id() else {
            self.note = Some(Err(missing()));
            return;
        };
        let Some(version) = fresh_id() else {
            self.note = Some(Err(missing()));
            return;
        };
        let actor = actor
            .map(str::to_owned)
            .or_else(fresh_id)
            .unwrap_or_default();
        let args = publish_args(
            &self.document_dir,
            &doc.revision_id,
            &op,
            &revision,
            &actor,
            &version,
        );
        self.start(cli, stats, Job::Publish, args);
    }

    /// Spawns one CLI command on a worker thread.
    fn start(&mut self, cli: &Path, stats: &Arc<Stats>, job: Job, args: Vec<String>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let cli = cli.to_path_buf();
        let stats = Arc::clone(stats);
        let spawned = std::thread::Builder::new()
            .name("beamcraft-drawing".into())
            .spawn(move || {
                let _ = tx.send(command::run(&cli, &args, command::DEADLINE, &stats));
            });
        if spawned.is_ok() {
            self.job = Some((job, rx));
        } else {
            self.note = Some(Err("the command thread could not be started".to_owned()));
        }
    }

    fn poll(&mut self, cli: &Path, model_store: Option<&Path>, stats: &Arc<Stats>) {
        let Some((job, rx)) = &self.job else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Err(ToolError::local(
                "CLI_FAILED",
                "the command thread ended without a result",
            )),
        };
        let job = *job;
        self.job = None;
        let parsed = result.and_then(|output| {
            let reply = decode(&output.stdout)?;
            rejection(&reply, output.code).map_or(Ok(reply), Err)
        });
        match (job, parsed) {
            (Job::Load, Ok(reply)) => match parse_document(&reply) {
                Some(doc) => {
                    self.version_field = doc.latest_published.clone().unwrap_or_default();
                    self.doc = Some(doc);
                    self.note = None;
                    // The document moved: read the versions so the export names a real one.
                    if !self.document_dir.is_empty() {
                        self.start(cli, stats, Job::Versions, versions_args(&self.document_dir));
                    }
                }
                None => {
                    self.note = Some(Err(
                        "the inspect reply is malformed for the drawing window".to_owned()
                    ));
                }
            },
            (Job::Versions, Ok(reply)) => match parse_versions(&reply) {
                Some(versions) => {
                    self.versions = Some(versions);
                }
                None => {
                    self.note = Some(Err(
                        "the versions reply is malformed for the drawing window".to_owned(),
                    ));
                }
            },
            (Job::Publish, Ok(reply)) => match parse_document(&reply) {
                Some(published) => {
                    let note = match published.version.as_deref() {
                        Some(version) => Ok(format!("Published version {version}.")),
                        None => Ok("Published.".to_owned()),
                    };
                    self.note = Some(note);
                    // The head moved: read it again so the next publish names the new base.
                    if !self.document_dir.is_empty() {
                        self.start(
                            cli,
                            stats,
                            Job::Load,
                            load_args(&self.document_dir, model_store),
                        );
                    }
                }
                None => {
                    self.note = Some(Err(
                        "the publish reply is malformed for the drawing window".to_owned()
                    ));
                }
            },
            (Job::Export, Ok(reply)) => {
                let format = text(&reply, "format").unwrap_or_else(|| "svg".to_owned());
                let bytes = reply
                    .get("bytes")
                    .ok()
                    .and_then(|value| value.float().ok())
                    .unwrap_or(0.0);
                self.note = Some(Ok(format!("Exported {format}, {bytes} bytes.")));
            }
            (_, Err(error)) => {
                self.note = Some(Err(error.to_string()));
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const REPLY: &str = r#"{
        "revision_id": "01900000-0000-7000-8000-000000000001",
        "name": "Member drawing",
        "view": {"id": "p", "plane": "front", "scale_denominator": 100, "paper_origin_mm": [0.0, 0.0], "title": null},
        "sheet": null,
        "state": "published",
        "version_id": "01900000-0000-7000-8000-00000000000a",
        "latest_published_version_id": "01900000-0000-7000-8000-00000000000a",
        "source_revision_id": "01900000-0000-7000-8000-000000000002",
        "source_manifest": "abc123",
        "request_digest": "def456",
        "placed_views": [],
        "placed_sections": [],
        "placed_details": [],
        "layers": [],
        "visible_drafting": [],
        "drafting": [],
        "freshness": "stale"
    }"#;

    #[test]
    fn the_inspect_reply_reads_into_the_whole_mirror_or_nothing() {
        let reply =
            beamcraft_json::decode(REPLY.as_bytes(), &beamcraft_json::Limits::DEFAULT).unwrap();
        let doc = parse_document(&reply).unwrap();
        assert_eq!(doc.revision_id.len(), 36);
        assert_eq!(doc.name, "Member drawing");
        assert_eq!(doc.state, "published");
        assert_eq!(
            doc.version.as_deref(),
            Some("01900000-0000-7000-8000-00000000000a")
        );
        assert_eq!(doc.latest_published, doc.version);
        assert_eq!(doc.freshness, "stale");
        // A head that was never published reads with versions absent, not an error, and a
        // reply without a model store reads freshness `unknown`, never invented.
        let draft = beamcraft_json::decode(
            br#"{"revision_id":"r","name":"n","state":"draft","version_id":null,"latest_published_version_id":null}"#,
            &beamcraft_json::Limits::DEFAULT,
        )
        .unwrap();
        let doc = parse_document(&draft).unwrap();
        assert_eq!(doc.version, None);
        assert_eq!(doc.latest_published, None);
        assert_eq!(doc.freshness, "unknown");
        // A malformed reply is refused, never half-read.
        let broken = beamcraft_json::decode(
            br#"{"name":"n","state":"draft","version_id":null,"latest_published_version_id":null}"#,
            &beamcraft_json::Limits::DEFAULT,
        )
        .unwrap();
        assert!(parse_document(&broken).is_none());
    }

    #[test]
    fn the_versions_reply_reads_into_a_list_or_nothing() {
        let reply = beamcraft_json::decode(
            br#"{"outcome":"drawing_versions","versions":[
                {"version_id":"v1","source_revision_id":"s1"},
                {"version_id":"v2","source_revision_id":"s2"}]}"#,
            &beamcraft_json::Limits::DEFAULT,
        )
        .unwrap();
        let versions = parse_versions(&reply).unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[1].id, "v2");
        assert_eq!(versions[1].source_revision_id, "s2");
        // An empty list is a real answer.
        let empty = beamcraft_json::decode(
            br#"{"outcome":"drawing_versions","versions":[]}"#,
            &beamcraft_json::Limits::DEFAULT,
        )
        .unwrap();
        assert!(parse_versions(&empty).unwrap().is_empty());
        // A malformed entry is refused, never half-read.
        let broken = beamcraft_json::decode(
            br#"{"outcome":"drawing_versions","versions":[{"version_id":"v1"}]}"#,
            &beamcraft_json::Limits::DEFAULT,
        )
        .unwrap();
        assert!(parse_versions(&broken).is_none());
    }

    #[test]
    fn the_commands_are_the_ones_a_user_types() {
        assert_eq!(
            load_args("/docs/one", None),
            ["inspect-drawing", "--document-dir", "/docs/one"]
        );
        assert_eq!(
            load_args("/docs/one", Some(Path::new("/model"))),
            [
                "inspect-drawing",
                "--document-dir",
                "/docs/one",
                "--model-store",
                "/model"
            ]
        );
        assert_eq!(
            publish_args("/docs/one", "head", "op", "rev", "actor", "ver"),
            [
                "publish-drawing",
                "--document-dir",
                "/docs/one",
                "--expected-head",
                "head",
                "--op-id",
                "op",
                "--revision-id",
                "rev",
                "--actor-id",
                "actor",
                "--version-id",
                "ver"
            ]
        );
        assert_eq!(
            versions_args("/docs/one"),
            ["inspect-drawing-versions", "--document-dir", "/docs/one"]
        );
        // SVG is the command's own default; PDF names the format explicitly.
        assert_eq!(
            export_args("/docs/one", "ver", "/out.svg", false),
            [
                "export-drawing",
                "--document-dir",
                "/docs/one",
                "--version-id",
                "ver",
                "--out",
                "/out.svg"
            ]
        );
        assert_eq!(
            export_args("/docs/one", "ver", "/out.pdf", true),
            [
                "export-drawing",
                "--document-dir",
                "/docs/one",
                "--version-id",
                "ver",
                "--out",
                "/out.pdf",
                "--format",
                "pdf"
            ]
        );
    }
}
