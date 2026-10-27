// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! V06.1 workspace publication, as pure logic: the Publish action is the argument list of the
//! CLI's `publish-workspace`, confirmed first, and the reply becomes a result or the engine's own
//! refusal. The window computes no verdict about publishability: a refusal (a moved published
//! head is `BASE_REVISION_MISMATCH`, an unchanged draft is `INVALID_ARGUMENT`) is only displayed
//! with its stable code and message. There is no merge, rebase or conflict resolution here
//! (C03 to C06 are deferred).
//!
//! The workspace does not record where the published store is, so the window needs `--published
//! ROOT` next to `--workspace DIR` to publish; without both, Publish is off and says why.

use crate::edit::sequence_of;
use crate::props::RevisionStamp;
use crate::tools::{ToolError, decode, rejection, text};
use beamcraft_json::Json;
use std::path::{Path, PathBuf};

/// The arguments of `publish-workspace` for a published store and a workspace directory.
pub fn publish_args(published: &Path, workspace: &Path) -> Vec<String> {
    vec![
        "publish-workspace".into(),
        "--store".into(),
        published.to_string_lossy().into_owned(),
        "--workspace-dir".into(),
        workspace.to_string_lossy().into_owned(),
    ]
}

/// The arguments of `inspect-workspace` for a published store and a workspace directory.
pub fn status_args(published: &Path, workspace: &Path) -> Vec<String> {
    vec![
        "inspect-workspace".into(),
        "--store".into(),
        published.to_string_lossy().into_owned(),
        "--workspace-dir".into(),
        workspace.to_string_lossy().into_owned(),
    ]
}

/// What an accepted publication reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Published {
    /// The published revision that holds the draft's content.
    pub revision: RevisionStamp,
    /// True when the CLI answered an already applied publication again.
    pub replayed: bool,
}

/// Reads the reply of `publish-workspace`: the published revision or the engine's refusal.
pub fn parse_published(exit_code: Option<i32>, stdout: &[u8]) -> Result<Published, ToolError> {
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
    let outcome = text(&reply, "outcome").unwrap_or_default();
    let replayed = outcome == "workspace_publish_replayed";
    if !replayed && outcome != "workspace_published" {
        return Err(ToolError::local(
            "UNREADABLE_REPLY",
            format!(
                "unexpected outcome \"{}\"",
                outcome.chars().take(40).collect::<String>()
            ),
        ));
    }
    let revision = reply
        .get("revision")
        .ok()
        .and_then(RevisionStamp::from_json)
        .ok_or_else(|| ToolError::local("UNREADABLE_REPLY", "the CLI reply lacks a field"))?;
    Ok(Published { revision, replayed })
}

/// What `inspect-workspace` reports, as the engine states it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// False when the published head moved since the workspace was forked (a publish is then
    /// refused by the engine).
    pub base_is_current: bool,
    /// Draft revisions not yet published.
    pub unsent_revisions: u64,
    /// The draft head's sequence number.
    pub draft_sequence: u64,
    /// The published head's sequence number.
    pub published_sequence: u64,
}

impl Status {
    /// A one-line summary for the panel.
    pub fn summary(&self) -> String {
        let base = if self.base_is_current {
            "the published head has not moved since the fork"
        } else {
            "the published head moved since the fork (a publish is refused; there is no merge)"
        };
        format!(
            "{} unsent draft revision(s); draft sequence {}, published sequence {}; {base}.",
            self.unsent_revisions, self.draft_sequence, self.published_sequence
        )
    }
}

/// Reads the reply of `inspect-workspace`.
pub fn parse_status(exit_code: Option<i32>, stdout: &[u8]) -> Result<Status, ToolError> {
    let reply = decode(stdout)?;
    if let Some(error) = rejection(&reply, exit_code) {
        return Err(error);
    }
    let unreadable = || ToolError::local("UNREADABLE_REPLY", "the CLI reply lacks a field");
    let workspace = reply.get("workspace").map_err(|_| unreadable())?;
    let sequence = |key: &str| {
        workspace
            .get(key)
            .ok()
            .and_then(sequence_of)
            .ok_or_else(unreadable)
    };
    let base_is_current = match workspace.get("base_is_current") {
        Ok(Json::Bool(flag)) => *flag,
        _ => return Err(unreadable()),
    };
    Ok(Status {
        base_is_current,
        unsent_revisions: text(workspace, "unsent_revisions")
            .and_then(|s| s.parse().ok())
            .ok_or_else(unreadable)?,
        draft_sequence: sequence("draft_head")?,
        published_sequence: sequence("published_head")?,
    })
}

/// Where a publication is in its confirm, run, result cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Nothing asked.
    Idle,
    /// The confirm dialog is open; nothing has run.
    Confirming,
    /// `publish-workspace` is running.
    Running,
}

/// The Publish action's state machine. Publishing runs only after the dialog is confirmed.
#[derive(Debug, Clone)]
pub struct Publisher {
    published: Option<PathBuf>,
    workspace: Option<PathBuf>,
    phase: Phase,
    /// The result of the last publication or the last status read.
    pub note: Option<Result<String, String>>,
    /// The last status the engine reported.
    pub status: Option<Status>,
}

impl Publisher {
    /// A publisher for the window's workspace directory and the published store, when given.
    pub fn new(published: Option<PathBuf>, workspace: Option<PathBuf>) -> Self {
        Self {
            published,
            workspace,
            phase: Phase::Idle,
            note: None,
            status: None,
        }
    }

    /// The phase.
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// The published store and workspace directory, or why publishing is not available.
    pub fn target(&self) -> Result<(&Path, &Path), &'static str> {
        match (&self.published, &self.workspace) {
            (Some(published), Some(workspace)) => Ok((published, workspace)),
            (_, None) => Err(
                "Publishing needs a personal workspace: open the window with --workspace DIR --published ROOT.",
            ),
            (None, Some(_)) => Err(
                "Publishing needs the published store the workspace was forked from: add --published ROOT.",
            ),
        }
    }

    /// Opens the confirm dialog. Refused when publishing is not available or one is in progress.
    pub fn request(&mut self) -> Result<(), &'static str> {
        self.target()?;
        if self.phase != Phase::Idle {
            return Err("a publication is already in progress");
        }
        self.phase = Phase::Confirming;
        Ok(())
    }

    /// Closes the dialog without running anything.
    pub fn cancel(&mut self) {
        if self.phase == Phase::Confirming {
            self.phase = Phase::Idle;
        }
    }

    /// The command the dialog would run, for display.
    pub fn args(&self) -> Option<Vec<String>> {
        let (published, workspace) = self.target().ok()?;
        Some(publish_args(published, workspace))
    }

    /// Confirms the dialog: returns the command to run and moves to `Running`. Only a confirmed
    /// dialog runs anything.
    pub fn confirm(&mut self) -> Option<Vec<String>> {
        if self.phase != Phase::Confirming {
            return None;
        }
        let args = self.args()?;
        self.phase = Phase::Running;
        self.note = None;
        Some(args)
    }

    /// Ends the run and records its result or the engine's refusal.
    pub fn finish(&mut self, result: Result<Published, ToolError>) {
        self.phase = Phase::Idle;
        self.note = Some(match result {
            Ok(published) if published.replayed => Ok(format!(
                "Already published: revision {} holds the draft (nothing was written).",
                published.revision.label()
            )),
            Ok(published) => Ok(format!(
                "Published the workspace as revision {}.",
                published.revision.label()
            )),
            Err(error) => Err(format!(
                "Publication refused, nothing was published: {error}"
            )),
        });
    }

    /// Records the outcome of a status read.
    pub fn set_status(&mut self, result: Result<Status, ToolError>) {
        match result {
            Ok(status) => self.status = Some(status),
            Err(error) => {
                self.status = None;
                self.note = Some(Err(format!("workspace status: {error}")));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn both() -> Publisher {
        Publisher::new(Some("P".into()), Some("W".into()))
    }

    fn published_reply(outcome: &str) -> Vec<u8> {
        format!(
            "{{\"outcome\":\"{outcome}\",\"revision\":{{\"revision_id\":\"r7\",\"sequence\":\"7\",\"manifest_digest\":\"0123456789abcdef\"}}}}"
        )
        .into_bytes()
    }

    #[test]
    fn the_command_is_workspace_publish_with_both_directories() {
        assert_eq!(
            publish_args(Path::new("P"), Path::new("W")),
            ["publish-workspace", "--store", "P", "--workspace-dir", "W"]
        );
        assert_eq!(
            status_args(Path::new("P"), Path::new("W")).first().unwrap(),
            "inspect-workspace"
        );
    }

    #[test]
    fn publishing_needs_a_workspace_and_its_published_store() {
        let mut none = Publisher::new(None, None);
        assert!(none.request().unwrap_err().contains("--workspace DIR"));
        let mut only_workspace = Publisher::new(None, Some("W".into()));
        assert!(
            only_workspace
                .request()
                .unwrap_err()
                .contains("--published")
        );
        let mut only_store = Publisher::new(Some("P".into()), None);
        assert!(only_store.request().is_err());
        assert_eq!(only_store.phase(), Phase::Idle);
        assert!(only_store.confirm().is_none());
    }

    #[test]
    fn only_a_confirmed_dialog_runs_and_cancel_runs_nothing() {
        let mut p = both();
        assert!(p.confirm().is_none(), "nothing runs without a dialog");
        p.request().unwrap();
        assert_eq!(p.phase(), Phase::Confirming);
        assert!(p.request().is_err(), "one dialog at a time");
        p.cancel();
        assert_eq!(p.phase(), Phase::Idle);
        assert!(p.confirm().is_none());
        p.request().unwrap();
        let args = p.confirm().unwrap();
        assert_eq!(args.first().unwrap(), "publish-workspace");
        assert_eq!(p.phase(), Phase::Running);
        assert!(p.confirm().is_none(), "no second run while running");
        assert!(p.request().is_err());
        p.cancel();
        assert_eq!(p.phase(), Phase::Running, "cancel does not stop a run");
        p.finish(parse_published(
            Some(0),
            &published_reply("workspace_published"),
        ));
        assert_eq!(p.phase(), Phase::Idle);
        assert!(p.note.clone().unwrap().unwrap().starts_with("Published"));
    }

    #[test]
    fn a_published_reply_is_read_and_a_forged_outcome_is_not() {
        let done = parse_published(Some(0), &published_reply("workspace_published")).unwrap();
        assert_eq!(done.revision.sequence, 7);
        assert!(!done.replayed);
        let again =
            parse_published(Some(0), &published_reply("workspace_publish_replayed")).unwrap();
        assert!(again.replayed);
        let forged = parse_published(Some(0), &published_reply("element_updated")).unwrap_err();
        assert_eq!(forged.code, "UNREADABLE_REPLY");
        assert_eq!(
            parse_published(Some(0), b"{\"outcome\":\"workspace_published\"}")
                .unwrap_err()
                .code,
            "UNREADABLE_REPLY"
        );
        assert_eq!(
            parse_published(Some(1), b"oops").unwrap_err().code,
            "CLI_FAILED"
        );
    }

    #[test]
    fn a_refusal_shows_the_message_without_the_code_and_changes_nothing_else() {
        let refusal = br#"{"outcome":"rejected","code":"BASE_REVISION_MISMATCH","message":"the published head moved since the workspace was forked"}"#;
        let error = parse_published(Some(2), refusal).unwrap_err();
        assert_eq!(error.code, "BASE_REVISION_MISMATCH");
        // A zero exit cannot hide a rejection.
        assert_eq!(
            parse_published(Some(0), refusal).unwrap_err().code,
            "BASE_REVISION_MISMATCH"
        );
        let mut p = both();
        p.request().unwrap();
        p.confirm().unwrap();
        p.finish(Err(error));
        assert_eq!(p.phase(), Phase::Idle);
        let shown = p.note.clone().unwrap().unwrap_err();
        assert!(
            shown.contains("the published head moved") && shown.contains("nothing was published")
        );
        assert!(!shown.contains("BASE_REVISION_MISMATCH"));
        // The dialog can be opened again: there is no conflict resolution to run instead.
        assert!(p.request().is_ok());
    }

    #[test]
    fn the_status_is_read_as_reported() {
        let reply = br#"{"outcome":"workspace_status","workspace":{"base_is_current":false,"unsent_revisions":"2","draft_head":{"revision_id":"d","sequence":"5","manifest_digest":"x"},"published_head":{"revision_id":"p","sequence":"4","manifest_digest":"y"}}}"#;
        let status = parse_status(Some(0), reply).unwrap();
        assert_eq!(
            status,
            Status {
                base_is_current: false,
                unsent_revisions: 2,
                draft_sequence: 5,
                published_sequence: 4
            }
        );
        assert!(status.summary().contains("moved since the fork"));
        assert_eq!(
            parse_status(Some(0), br#"{"workspace":{"base_is_current":true}}"#)
                .unwrap_err()
                .code,
            "UNREADABLE_REPLY"
        );
        let mut p = both();
        p.set_status(Ok(status));
        assert!(p.status.is_some());
        p.set_status(Err(ToolError::local("NOT_FOUND", "x")));
        assert!(p.status.is_none() && p.note.is_some());
    }
}
