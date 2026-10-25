// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! U14 headless commands `history-stats` and `history-compact`.
//!
//! ```text
//! beamcraft history-stats   --store ROOT [--workspaces-root DIR]... [--workspace-dir DIR]...
//! beamcraft history-compact --store ROOT (--keep-head | --keep N) --dry-run
//!                           [--workspaces-root DIR]... [--workspace-dir DIR]...
//!                           [--ledger-tail N] [--orphan-grace-secs N]
//! beamcraft history-compact --store ROOT (--keep-head | --keep N) --yes
//!                           (--backup DIR | --no-backup)
//!                           (--workspaces-root DIR | --workspace-dir DIR | --no-workspaces)
//!                           [--ledger-tail N] [--orphan-grace-secs N]
//! ```
//!
//! `history-stats` and `--dry-run` change nothing. A real compaction deletes history for good, so
//! it is refused unless the operator confirms (`--yes`), chooses a backup (`--backup DIR`, copied
//! and verified under the exclusive store lease before the first change) or explicitly declines it
//! (`--no-backup`), and declares where the live personal workspaces are (or that there are none),
//! because a workspace's base revision must not be removed. The dry run lists exactly the
//! revisions whose undo points would be lost.

use beamcraft_cli::history_compact::{
    backup_store, collect_pins, hold_coordinator_lease, is_workspace_store,
};
use beamcraft_cli::plate::PlateError;
use beamcraft_host::compaction::{
    Blocker, Compacted, CompactionError, Keep, NoFault, Pin, PinStatus, Plan, Request, Safeguard,
    Stats, compact, plan, stats,
};
use beamcraft_host::lease::StoreLease;
use beamcraft_host::revision::RevisionStore;
use beamcraft_host::storage::BlobStore;
use beamcraft_json::Json;
use beamcraft_model::error::ErrorCode;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Revisions listed in a report (the totals are always exact).
const LIST_CAP: usize = 256;
/// Forgotten operation ids listed in a report.
const FORGOTTEN_CAP: usize = 64;
/// Default grace period for blobs no revision references.
const DEFAULT_GRACE_SECS: u64 = 3600;

struct Failure {
    error: PlateError,
    diagnostics: Vec<Json>,
}

type Run<T> = Result<T, Failure>;

impl From<PlateError> for Failure {
    fn from(error: PlateError) -> Self {
        Self {
            error,
            diagnostics: Vec::new(),
        }
    }
}

impl From<CompactionError> for Failure {
    fn from(error: CompactionError) -> Self {
        let diagnostics = match &error {
            CompactionError::Blocked(blockers) => blockers.iter().map(blocker_json).collect(),
            _ => Vec::new(),
        };
        Self {
            error: PlateError {
                code: error.code(),
                message: error.message(),
            },
            diagnostics,
        }
    }
}

fn invalid(message: &'static str) -> Failure {
    PlateError {
        code: ErrorCode::InvalidArgument,
        message,
    }
    .into()
}

/// Whether `command` is one of the history commands.
pub fn handles(command: &OsString) -> bool {
    command == "history-stats" || command == "history-compact"
}

/// Runs the command and returns the JSON result and the process exit code.
pub fn execute(args: &[OsString]) -> (Json, u8) {
    match run(args) {
        Ok(json) => (json, 0),
        Err(failure) => crate::report::report_with(Err(failure.error), failure.diagnostics),
    }
}

/// The refusal printed when a coordinator cannot take its store lease.
pub fn lease_refusal(error: PlateError) -> (Json, u8) {
    crate::report::report(Err(error))
}

/// Holds the store lease of a starting `coordinator-serve`; see
/// [`beamcraft_cli::history_compact::hold_coordinator_lease`].
pub fn coordinator_lease(args: &[OsString]) -> Result<Option<StoreLease>, (Json, u8)> {
    hold_coordinator_lease(args).map_err(lease_refusal)
}

const VALUE_OPTIONS: [&str; 7] = [
    "--store",
    "--keep",
    "--ledger-tail",
    "--orphan-grace-secs",
    "--backup",
    "--workspaces-root",
    "--workspace-dir",
];
const FLAGS: [&str; 5] = [
    "--keep-head",
    "--dry-run",
    "--yes",
    "--no-backup",
    "--no-workspaces",
];

struct Parsed<'a> {
    values: BTreeMap<&'a str, &'a str>,
    roots: Vec<&'a str>,
    dirs: Vec<&'a str>,
    flags: BTreeSet<&'a str>,
}

fn parse<'a>(tail: &[&'a str], allowed: &[&str]) -> Run<Parsed<'a>> {
    let mut parsed = Parsed {
        values: BTreeMap::new(),
        roots: Vec::new(),
        dirs: Vec::new(),
        flags: BTreeSet::new(),
    };
    let mut at = 0usize;
    while let Some(key) = tail.get(at).copied() {
        at = at.saturating_add(1);
        if !allowed.contains(&key) {
            return Err(invalid("unknown option for this command"));
        }
        if FLAGS.contains(&key) {
            if !parsed.flags.insert(key) {
                return Err(invalid("repeated option"));
            }
            continue;
        }
        let value = tail
            .get(at)
            .copied()
            .ok_or_else(|| invalid("an option is missing its value"))?;
        at = at.saturating_add(1);
        match key {
            "--workspaces-root" => parsed.roots.push(value),
            "--workspace-dir" => parsed.dirs.push(value),
            _ => {
                if parsed.values.insert(key, value).is_some() {
                    return Err(invalid("repeated option"));
                }
            }
        }
    }
    if parsed.roots.len() > 16 || parsed.dirs.len() > 64 {
        return Err(invalid("too many workspace options"));
    }
    Ok(parsed)
}

fn run(args: &[OsString]) -> Run<Json> {
    let args: Vec<&str> = args
        .iter()
        .map(|a| {
            a.to_str()
                .filter(|s| s.len() <= 4096)
                .ok_or_else(|| invalid("argument is not bounded UTF-8"))
        })
        .collect::<Run<_>>()?;
    let (command, tail) = args
        .split_first()
        .ok_or_else(|| invalid("missing command"))?;
    if tail.len() > 64 {
        return Err(invalid("too many arguments"));
    }
    match *command {
        "history-stats" => {
            let parsed = parse(tail, &["--store", "--workspaces-root", "--workspace-dir"])?;
            history_stats(&parsed)
        }
        "history-compact" => {
            let parsed = parse(tail, &[VALUE_OPTIONS.as_slice(), FLAGS.as_slice()].concat())?;
            history_compact(&parsed)
        }
        _ => Err(invalid("unknown command")),
    }
}

fn open_store(parsed: &Parsed<'_>) -> Run<(PathBuf, RevisionStore)> {
    let root = parsed
        .values
        .get("--store")
        .ok_or_else(|| invalid("--store is required"))?;
    let blobs = BlobStore::new(root).map_err(|e| PlateError {
        code: e.code(),
        message: "store root is not usable",
    })?;
    Ok((PathBuf::from(root), RevisionStore::new(blobs)))
}

fn pins_of(store: &Path, parsed: &Parsed<'_>) -> Run<Vec<Pin>> {
    let roots: Vec<PathBuf> = parsed.roots.iter().map(PathBuf::from).collect();
    let dirs: Vec<PathBuf> = parsed.dirs.iter().map(PathBuf::from).collect();
    Ok(collect_pins(store, &roots, &dirs)?)
}

fn number(parsed: &Parsed<'_>, key: &str, default: u64, message: &'static str) -> Run<u64> {
    parsed.values.get(key).map_or(Ok(default), |text| {
        text.parse::<u64>().map_err(|_| invalid(message))
    })
}

fn obj(entries: Vec<(&str, Json)>) -> Json {
    Json::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

fn count(value: u64) -> Json {
    Json::uint(value)
}

fn blocker_json(blocker: &Blocker) -> Json {
    match blocker {
        Blocker::OpenIntents(n) => obj(vec![
            ("kind", Json::text("open_intents")),
            ("count", Json::uint(u64::try_from(*n).unwrap_or(u64::MAX))),
            ("action", Json::text("run revision-recover first")),
        ]),
        Blocker::StoreInUse => obj(vec![
            ("kind", Json::text("store_in_use")),
            (
                "action",
                Json::text("stop the coordinator or the other command that holds the store"),
            ),
        ]),
        Blocker::WorkspaceBase { label, sequence } => obj(vec![
            ("kind", Json::text("workspace_base")),
            ("workspace", Json::text(label.clone())),
            ("base_sequence", count(*sequence)),
            (
                "action",
                Json::text("keep more revisions or publish or discard that workspace"),
            ),
        ]),
    }
}

fn pins_json(pins: &[PinStatus]) -> Json {
    Json::Array(
        pins.iter()
            .map(|pin| {
                obj(vec![
                    ("workspace", Json::text(pin.label.clone())),
                    ("base_sequence", pin.sequence.map_or(Json::Null, Json::uint)),
                ])
            })
            .collect(),
    )
}

fn percent(hundredths: u64) -> Run<Json> {
    #[allow(clippy::cast_precision_loss)]
    let value = hundredths as f64 / 100.0;
    Json::number(value).map_err(|_| invalid("a percentage is not finite"))
}

fn stats_json(s: &Stats, workspaces_declared: bool) -> Run<Json> {
    let workspaces = obj(vec![
        ("declared", Json::Bool(workspaces_declared)),
        (
            "count",
            Json::uint(u64::try_from(s.pins.len()).unwrap_or(u64::MAX)),
        ),
        (
            "reachable_blobs",
            s.workspace_blob_count.map_or(Json::Null, Json::uint),
        ),
        (
            "reachable_bytes",
            s.workspace_bytes.map_or(Json::Null, Json::uint),
        ),
        ("workspaces", pins_json(&s.pins)),
    ]);
    Ok(obj(vec![
        ("outcome", Json::text("history_stats")),
        ("changes_nothing", Json::Bool(true)),
        ("revisions", count(s.revisions)),
        ("first_sequence", count(s.first_sequence)),
        ("head_sequence", count(s.head_sequence)),
        ("head_revision_id", Json::text(s.head_revision.as_str())),
        ("compacted", Json::Bool(s.compacted)),
        ("unreadable_revisions", count(s.unreadable_revisions)),
        (
            "blobs",
            obj(vec![
                ("count", count(s.blob_count)),
                ("bytes", count(s.blob_bytes)),
            ]),
        ),
        (
            "reachable_from_head",
            obj(vec![
                ("blobs", count(s.head_blob_count)),
                ("bytes", count(s.head_bytes)),
            ]),
        ),
        (
            "reachable_from_history",
            obj(vec![
                ("blobs", count(s.history_blob_count)),
                ("bytes", count(s.history_bytes)),
            ]),
        ),
        ("reachable_from_workspaces", workspaces),
        (
            "unreachable",
            obj(vec![
                ("blobs", count(s.unreachable_blob_count)),
                ("bytes", count(s.unreachable_bytes)),
                ("share_percent", percent(s.unreachable_share_hundredths)?),
            ]),
        ),
        ("missing_blobs", count(s.missing_blobs)),
        (
            "revision_log",
            obj(vec![
                ("bytes", count(s.revision_log_bytes)),
                ("leftover_records", count(s.leftover_records)),
            ]),
        ),
        (
            "ledger",
            obj(vec![
                ("bytes", count(s.ledger_bytes)),
                ("records", count(s.ledger_records)),
                ("open_intents", count(s.open_intents)),
            ]),
        ),
    ]))
}

fn history_stats(parsed: &Parsed<'_>) -> Run<Json> {
    let (root, store) = open_store(parsed)?;
    let declared = !parsed.roots.is_empty() || !parsed.dirs.is_empty();
    let pins = pins_of(&root, parsed)?;
    let measured = stats(&store, &pins)?;
    stats_json(&measured, declared)
}

fn plan_fields(plan: &Plan, declared: bool) -> Vec<(&'static str, Json)> {
    let listed: Vec<Json> = plan
        .removed
        .iter()
        .take(LIST_CAP)
        .map(|r| {
            obj(vec![
                ("sequence", count(r.sequence)),
                ("revision_id", Json::text(r.revision.as_str())),
                (
                    "op_id",
                    r.op.as_ref()
                        .map_or(Json::Null, |op| Json::text(op.as_str())),
                ),
                ("readable", Json::Bool(r.readable)),
            ])
        })
        .collect();
    let forgotten: Vec<Json> = plan
        .ledger
        .forgotten_ops
        .iter()
        .take(FORGOTTEN_CAP)
        .map(|op| Json::text(op.as_str()))
        .collect();
    let removed = u64::try_from(plan.removed.len()).unwrap_or(u64::MAX);
    vec![
        ("head_revision_id", Json::text(plan.head_revision.as_str())),
        ("head_sequence", count(plan.head_sequence)),
        ("first_sequence_before", count(plan.old_floor)),
        ("first_sequence_after", count(plan.new_floor)),
        ("revisions_retained", count(plan.retained)),
        ("revisions_removed", count(removed)),
        ("blobs_removed", count(plan.blobs_to_delete)),
        ("bytes_freed", count(plan.bytes_to_free)),
        ("record_files_removed", count(plan.records_to_remove)),
        ("blobs_before", count(plan.blob_count)),
        ("bytes_before", count(plan.blob_bytes)),
        (
            "unreferenced_blobs_kept_by_grace",
            count(plan.protected_orphans),
        ),
        (
            "undo_history_lost",
            obj(vec![
                ("revisions", count(removed)),
                ("oldest_restorable_sequence", count(plan.new_floor)),
                ("listed", Json::Array(listed)),
                (
                    "listed_count",
                    count(u64::try_from(plan.removed.len().min(LIST_CAP)).unwrap_or(u64::MAX)),
                ),
                (
                    "note",
                    Json::text(
                        "states before the oldest restorable sequence can no longer be opened or restored",
                    ),
                ),
            ]),
        ),
        (
            "ledger",
            obj(vec![
                ("bytes_before", count(plan.ledger.bytes_before)),
                ("bytes_after", count(plan.ledger.bytes_after)),
                ("rotated", Json::Bool(plan.ledger.rotated)),
                (
                    "operation_ids_forgotten",
                    count(u64::try_from(plan.ledger.forgotten_ops.len()).unwrap_or(u64::MAX)),
                ),
                ("forgotten_listed", Json::Array(forgotten)),
                ("tombstones_kept", count(plan.ledger.tombstones)),
            ]),
        ),
        (
            "workspaces",
            obj(vec![
                ("declared", Json::Bool(declared)),
                ("workspaces", pins_json(&plan.pins)),
            ]),
        ),
        (
            "blockers",
            Json::Array(plan.blockers.iter().map(blocker_json).collect()),
        ),
    ]
}

fn history_compact(parsed: &Parsed<'_>) -> Run<Json> {
    let keep = match (
        parsed.flags.contains("--keep-head"),
        parsed.values.get("--keep"),
    ) {
        (true, None) => Keep::Head,
        (false, Some(text)) => Keep::Last(
            text.parse::<u64>()
                .map_err(|_| invalid("--keep is a whole number"))?,
        ),
        _ => return Err(invalid("give exactly one of --keep-head and --keep N")),
    };
    let ledger_tail = number(
        parsed,
        "--ledger-tail",
        0,
        "--ledger-tail is a whole number",
    )?;
    let orphan_grace_secs = number(
        parsed,
        "--orphan-grace-secs",
        DEFAULT_GRACE_SECS,
        "--orphan-grace-secs is a whole number",
    )?;
    let dry_run = parsed.flags.contains("--dry-run");
    let yes = parsed.flags.contains("--yes");
    let no_backup = parsed.flags.contains("--no-backup");
    let backup = parsed.values.get("--backup").copied();
    let declared = !parsed.roots.is_empty() || !parsed.dirs.is_empty();
    let no_workspaces = parsed.flags.contains("--no-workspaces");
    if no_workspaces && declared {
        return Err(invalid(
            "--no-workspaces excludes --workspaces-root and --workspace-dir",
        ));
    }
    if dry_run && (yes || no_backup || backup.is_some()) {
        return Err(invalid(
            "--dry-run changes nothing and takes no --yes, --backup or --no-backup",
        ));
    }
    if !dry_run {
        if !yes {
            return Err(invalid(
                "refused: a compaction deletes history for good; run --dry-run first, then confirm with --yes",
            ));
        }
        if backup.is_some() == no_backup {
            return Err(invalid(
                "refused: give exactly one of --backup DIR and --no-backup",
            ));
        }
        if !declared && !no_workspaces {
            return Err(invalid(
                "refused: declare the live workspaces (--workspaces-root or --workspace-dir) or --no-workspaces",
            ));
        }
    }
    let (root, store) = open_store(parsed)?;
    if is_workspace_store(&root) {
        return Err(invalid(
            "a personal workspace's draft store is never compacted",
        ));
    }
    let pins = pins_of(&root, parsed)?;
    let request = Request {
        keep,
        pins: &pins,
        ledger_tail,
        orphan_grace_secs,
    };
    if dry_run {
        let planned = plan(&store, &request)?;
        let mut fields = vec![
            ("outcome", Json::text("compaction_dry_run")),
            ("changes_nothing", Json::Bool(true)),
            ("would_proceed", Json::Bool(planned.blockers.is_empty())),
        ];
        fields.extend(plan_fields(&planned, declared));
        return Ok(obj(fields));
    }
    let done = match backup {
        Some(directory) => {
            let destination = PathBuf::from(directory);
            let mut copy = || backup_store(&root, &destination);
            compact(
                &store,
                &request,
                true,
                Safeguard::Backup(&mut copy),
                &mut NoFault,
            )?
        }
        None => compact(&store, &request, true, Safeguard::NoBackup, &mut NoFault)?,
    };
    Ok(compacted_json(&done, declared, backup))
}

fn compacted_json(done: &Compacted, declared: bool, backup: Option<&str>) -> Json {
    let nothing = done.plan.removed.is_empty()
        && done.records_removed == 0
        && done.blobs_removed == 0
        && !done.plan.ledger.rotated;
    let mut fields = vec![
        ("outcome", Json::text("compacted")),
        ("changes_nothing", Json::Bool(false)),
        ("nothing_to_do", Json::Bool(nothing)),
        ("backup", backup.map_or(Json::Null, Json::text)),
        ("backup_declined", Json::Bool(backup.is_none())),
    ];
    fields.extend(plan_fields(&done.plan, declared));
    fields.extend([
        ("records_removed_now", count(done.records_removed)),
        ("blobs_removed_now", count(done.blobs_removed)),
        ("bytes_freed_now", count(done.bytes_freed)),
    ]);
    obj(fields)
}
