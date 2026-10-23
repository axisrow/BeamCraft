// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Headless CLI using the same native dispatcher as application callers.

mod revision;

use beamcraft_host::{
    control::{Cancellation, Control, HostError},
    storage::BlobStore,
};
use beamcraft_json::{Json, Limits};
use beamcraft_model::{ErrorCode, digest::ContentDigest};
use std::collections::BTreeMap;

mod assembly_cmd;
mod bolt_cmd;
mod bom_cmd;
mod changeset_cmd;
mod coordinator_cmd;
mod export_cmd;
mod history_cmd;
mod level_cmd;
mod library_cmd;
mod numbering_cmd;
mod pick_cmd;
mod plate_cmd;
mod reference_cmd;
mod view;
mod view_export;
mod weld_cmd;

fn invalid() -> HostError {
    HostError::logical(ErrorCode::InvalidArgument)
}
fn main() -> std::process::ExitCode {
    let args = std::env::args_os().skip(1).take(50).collect::<Vec<_>>();
    if args.len() == 1 && (args.first().is_some_and(|s| s == "--version" || s == "-V")) {
        println!("beamcraft {}", env!("CARGO_PKG_VERSION"));
        return std::process::ExitCode::SUCCESS;
    }
    if args.is_empty()
        || (args.len() == 1 && args.first().is_some_and(|s| s == "--help" || s == "-h"))
    {
        println!(
            "BeamCraft: blob-put | blob-verify | revision-commit | revision-open | revision-recover | revision-outcome | create-plate | plate-inspect | define-material | define-profile | definitions | library-list | library-show | library-add | create-member | update-element | delete-element | copy-element | workspace-create | workspace-status | workspace-publish | workspace-undo | workspace-redo | workspace-history | reference-attach | reference-move | reference-correct-units | reference-write | reference-inspect | create-assembly | assembly-attach | assembly-detach | assemblies | create-weld | welds | create-bolt | bolts | create-level | levels | export-ifc | ifc-guid | numbering-preview | numbering-approve | numbering-status | changeset-create | changeset-inspect | changeset-apply | coordinator-serve | coordinator-publish | coordinator-head | history-stats | history-compact | view-snapshot | view-changes | pick | feature-lookup | feature-list | snap | view-export | bom-export\nBlobs: blob-put --store ROOT --file FILE (stores the bytes under their SHA-256); blob-verify --store ROOT --digest SHA256\nControl (blob and revision commands): [--timeout-ms 1..120000] [--cancel-after-ms 0..120000]\nRevision: revision-commit --store ROOT --project-id UUID --workspace-id UUID --revision-id UUID --op-id UUID --generation U64 --base none|REVISION_UUID --schema SHA256 --root-layer NAME --layers ROLE:NAME:SHA256,... (ROLE authored|derived|external; blobs must be stored first); revision-open --store ROOT\nRecovery: revision-recover --store ROOT --min-age-secs U64 (0 asserts that no commit is running; closes unresolved intents from the root, removes abandoned temporary files, reports unreferenced blobs, deletes no blob); revision-outcome --store ROOT --op-id UUID (host-ledger QueryOutcome: committed | aborted | not recorded | OUTCOME_UNKNOWN)\nHistory (U14): history-stats --store ROOT [--workspaces-root DIR] [--workspace-dir DIR] (revisions, blob bytes in total, reachable from the head and from the workspaces, unreachable blobs, revision-log and ledger size; read-only); history-compact --store ROOT (--keep-head | --keep N) --dry-run (changes nothing; reports bytes freed, revisions removed and the undo points that would be lost); history-compact ... --yes (--backup DIR | --no-backup) (--workspaces-root DIR | --workspace-dir DIR | --no-workspaces) [--ledger-tail N] [--orphan-grace-secs N] (deletes history for good: refused without --yes, without a backup decision, with a live workspace based on a removed revision, while a coordinator holds the store or an unresolved intent exists)\nPlate (P01, committed USD revision): create-plate --store ROOT (--width MM --length MM | --contour X,Y;X,Y;...) --thickness MM [--name TEXT] [--translation X,Y,Z] [--material-digest SHA256] [--hole-rect X,Y,W,H[;...]] [--hole-round X,Y,D[;...]] [--hole-segments N] [--new-id UUID] [--op-id UUID] [--revision-id UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID] (--width and --length make the origin rectangle; --contour (A02.1) is any simple counter-clockwise polygon in the plane z = 0 given as X,Y points separated by semicolons, extruded as authored: clockwise, self-crossing or touching outlines are INVALID_ARGUMENT, nothing is reordered or repaired; the origin rectangle written as a contour is the same box; plate-inspect lists contour_points_mm and outline rectangle|contour, and width and length are the bounding-box extents of a contour; a retry of the same --op-id needs the same --new-id and arguments and is replayed); plate-inspect --store ROOT [--element UUID] (verified head revision, plates, meshes, snapshot; --element lists only that element, none when the head does not hold it, names it as element_filter and keeps the counts of the whole revision)\nDefinitions (P02, immutable USD assets pinned by full digest; a material without --density keeps an unknown density, so mass stays unknown): define-material --store ROOT [--density KG_M3] [--revision-id UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID]; define-profile --store ROOT --width MM --height MM [same ids] (rectangle section); define-profile --store ROOT --family i_shape|channel|angle|rhs|chs|lipped_c|plain_c|lipped_z|omega --h MM --b MM ... (A07.1: one --<parameter> per family parameter: i_shape/channel h,b,tw,tf,r; angle/rhs/plain_c h,b,t,r; chs d,t; lipped_c/lipped_z/omega h,b,c,t,r; r is the root, outer (rhs) or inner bend radius, 0 is sharp) [--designation TEXT] [--standard TEXT] [--area-mm2 F] [--mass-per-meter-kg F] [--surface-per-meter-m2 F] [same ids]; define-profile --store ROOT --family custom --outer X,Y;X,Y;... [--cavity X,Y;...|X,Y;...] (outer counter-clockwise, cavities clockwise, kept as authored); definitions --store ROOT (verified head: digest, kind, units, values, plates that pin it). A definition already in the head is not rewritten (definition_exists). create-plate --material-digest pins an existing material.\nNames and libraries (A07.2, experimental): every definition carries a readable display name that is part of its content (so another name is another digest): --name TEXT on define-material and define-profile, else the designation, else a generated one (S355 7850 kg/m3, I 200x100, RECT 200x300); definitions and plate-inspect list it (display_name, profile_name, material_name). create-member --profile NAME and create-member/create-plate --material NAME select a definition of the head by display name or designation/grade (exact; no match is NOT_FOUND; several matches are INVALID_ARGUMENT with every candidate in diagnostics, never a guess; not combined with the matching --...-digest). Library: a USD layer of named section entries (one file, 8 MiB at most, 4096 entries, no sublayers): library-list --library FILE [--family F] [--filter TEXT] [--offset N] [--limit 1..256] (paged, default 64; next_offset continues; entries then refused entries); definitions --store ROOT [--name TEXT] [--offset N] [--limit 1..256] lists the definitions a name selects, paged; an ambiguous name lists its first 16 candidates in diagnostics with the total; library-show --library FILE --designation NAME; define-profile --store ROOT --library FILE --designation NAME [--name TEXT] builds the definition from the entry with every value stored in it (an unknown designation is NOT_FOUND, never substituted; a duplicate designation refuses the library; a declared area that differs from the outline area by more than 1 percent is reported as declared_area_warning and never refuses). Environments (A07.3): --environment FILE replaces --library on library-list, library-show and define-profile: a USD layer whose customLayerData has bim:environment:id and bim:environment:libraries (string[], contained relative paths next to the file, strongest first, 1 to 32, no ..); the first library that declares a designation answers it, a refused entry is never replaced by a weaker library, and library-list reports the shadowed designations, paged on their own with --shadow-offset N (--limit of them; shadowed_count and shadowed_next_offset), while library-show reports the shadows of its designation only; --user-library FILE is consulted before every other library (its bim:library:id must differ from every other library of the lookup); a library that cannot be loaded refuses the environment. library-add --library FILE [--library-id ID [--title TEXT]] --designation NAME (--width MM --height MM | --family F --h .. | --family custom --outer ..) [--standard TEXT] [--area-mm2 F] [--mass-per-meter-kg F] [--surface-per-meter-m2 F] writes one admitted entry into the library file (--library-id creates a missing file; a .usdc destination or a binary USD library is refused, USDA text only; a replaced file keeps its permissions; a duplicate designation, a library with refused entries or a file that does not read back as intended is refused and nothing is written; the file is rendered again, so comments and unread attributes are not kept).\nMember (P03): create-member --store ROOT --start X,Y,Z --end X,Y,Z (--profile-digest SHA256 | --profile NAME) [--kind beam|column] [--roll RAD] [--material-digest SHA256 | --material NAME] [--name TEXT] [--translation X,Y,Z] [--new-id UUID] [--op-id UUID] [--revision-id UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID] (straight member with a pinned rectangle or A07.1 section profile, built from the detailed outline; plate-inspect lists plates and members)\nUpdate (P04, P08): update-element --store ROOT --id UUID [--name TEXT] [--material-digest SHA256|none] [--thickness MM] [--translation X,Y,Z] [--level UUID|none] [--op-id UUID] [--revision-id UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID] (typed patch; a rename rewrites only the authored layer, a material change only the quantity record, a thickness change the mesh, a move (--translation, absolute, rotation kept) only the placement without rebuilding the body; plate-inspect reports stored quantities as current, stale or missing)\nDelete (P08): delete-element --store ROOT --id UUID [--op-id UUID] [--revision-id UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID] (removes the whole element; DEPENDENCY_CONFLICT while another element links to it, no cascade; the definitions it pinned stay)\nCopy (P05): copy-element --store ROOT --id UUID [--name TEXT] [--offset X,Y,Z] [--new-id UUID] [--op-id UUID] [--revision-id UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID] (new element with the source's authored inputs, the same body and mesh digest; the source is only read; editing the copy later never changes the source)\nAssembly (A01): create-assembly --store ROOT --name TEXT [--new-id UUID] [--op-id UUID] [--revision-id UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID] (an element of kind assembly with its own UUID and no body); assembly-attach --store ROOT --id PART_UUID --assembly-id UUID [same ids] (authors the part's one bim:partOf record; plate, beam, column or nested assembly; DEPENDENCY_CONFLICT for a part of another assembly or a cycle); assembly-detach --store ROOT --id PART_UUID [same ids]; assemblies --store ROOT (verified head: assemblies, their parts and parent). delete-element refuses an assembly that still has parts\nWeld (A06, a connection element with no body): create-weld --store ROOT --name TEXT --main PART_UUID --secondary PART_UUID --size MM --location workshop|site --side arrow|other|both [--type fillet] [--new-id UUID] [--op-id UUID] [--revision-id UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID] (authors the weld's properties and its one bim:connects record towards two distinct plates, beams or columns, main part first; the parts are only read; no default for size, location or side); welds --store ROOT (verified head: welds, parts, properties). delete-element refuses a welded part while the weld connects it and deletes a weld\nBolt (A05, experimental, a connection element with no body): create-bolt --store ROOT --name TEXT --main PART_UUID --secondary PART_UUID --origin X,Y,Z --direction X,Y,Z --count N --pitch MM --diameter MM [--new-id UUID] [--op-id UUID] [--revision-id UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID] (a bolt group of --count bolts in a straight world line from --origin along the unit --direction with centre distance --pitch; authors the group's properties and its one bim:connects record towards two distinct plates, beams or columns, main part first; the parts are only read; every bolt position must lie within both parts' bounds; no default for any property); bolts --store ROOT (verified head: bolt groups, parts, properties). delete-element refuses a bolted part while the group connects it and deletes a bolt group\nLevel (D2, a storey element with no body): create-level --store ROOT --name TEXT --elevation MM [--new-id UUID] [--op-id UUID] [--revision-id UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID] (a level with its own UUID and a finite elevation within 1e9 mm; moves no element); levels --store ROOT (verified head: levels in ascending id order, their elevations and the elements assigned); assign with update-element --level UUID (one level per plate, beam or column; DANGLING_RELATIONSHIP for an unknown level) or --level none; delete-element refuses a level that elements use (DEPENDENCY_CONFLICT); copy-element keeps the source's level. Project (D1): the first revision records its --project-id (or a generated one) in the root layer; plate-inspect reports it as project_id_record; a later command naming another --project-id is INVALID_ARGUMENT\nExport (E02, IFC4 STEP of one committed revision; experimental, E01 contract 0.1.0): export-ifc --store ROOT --out FILE [--revision UUID] [--project-name TEXT] [--schema IFC4] [--format step] [--strict] [--overwrite] [--report FILE] (the head or an explicit revision: plates, beams and columns as IfcPlate, IfcBeam and IfcColumn with their kernel mesh as an IfcTriangulatedFaceSet, every other kind such as assemblies, welds and references as IfcBuildingElementProxy, levels as IfcBuildingStorey, the pinned material, published current quantities and the property sets BeamCraft_Element and BeamCraft_Declared; a deactivated element is not exported; stale, missing or unknown quantities are left out and listed, never zero; the file is created new unless --overwrite and is installed only after a complete write; the JSON report lists every element with its GlobalId, the findings and the losses; --strict refuses a plate, beam or column that would leave without mesh or placement; the ProjectId is read from the store and there is no --project-id; no IFC is ever read); ifc-guid --id UUID | --decode GLOBALID (the deterministic 22-character GlobalId of an ElementId, INVALID_ID for anything but a canonical UUIDv7, or the reverse)\nNumbering (A08, read-only preview; approving numbers is A09 below): numbering-preview --store ROOT [--plate-series PREFIX:START] [--beam-series PREFIX:START] [--column-series PREFIX:START] [--assembly-series PREFIX:START] [--tolerance MM] [--compare-name true|false] [--previous TOKEN] [--offset N] [--limit 1..256] (groups plates, beams and columns of the verified head that have the same kind, pinned profile, pinned material, lengths within the tolerance (default 0.001 mm), roll and holes; the oldest part is the reference and groups are numbered in the order of their references; defaults PL:1, B:1, C:1 and A:1 (assembly marks are PREFIX-NUMBER, grouped by the groups of all parts they hold); a position that an earlier group already takes is overlapping and its parts stay unnumbered; preview_token covers everything that changes the proposal, so --previous TOKEN reports stale true after an edit that would change the numbering (thickness, material, holes, a new or deleted part) and false after a move or a rename; nothing is written by the preview, which also reports the stored approval and, per group, whether the stored marks of its members are the proposed position)\nNumbering approval (A09, experimental): numbering-approve --store ROOT --preview-token TOKEN [the settings options of numbering-preview] [--op-id UUID] [--revision-id UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID] (evaluates the preview again against the head and refuses a TOKEN that is no longer the proposal with STALE_WORKSPACE, overlapping series and an empty model with INVALID_ARGUMENT; otherwise stores the mark of every part and assembly (series, number and the approval record) as authored data in the next revision, so equivalent parts share a mark; approving what is already stored writes nothing (numbering_approved_unchanged); a retry of the same --op-id is a replay; workspace-undo restores the earlier marks); numbering-status --store ROOT [--offset N] [--limit 1..256] (the approval stored in the head: not_approved, current or stale with the reason, and every stored mark; an edit of profile, material, dimensions, holes or outline, or a new, copied or deleted part makes it stale, a move or a view change does not)\nWorkspace (P06, a personal draft is a separate store directory; edit it with the usual commands and --store DIR): workspace-create --store PUBLISHED --workspace-dir DIR [--workspace-id UUID] [--user-id UUID] [--revision-id UUID] [--op-id UUID] (forks the published head); workspace-status --store PUBLISHED --workspace-dir DIR (base, unsent revisions, whether the published head moved); workspace-publish --store PUBLISHED --workspace-dir DIR (the published revision id is the draft head's id, so a retry is the same request; fast-forward only: BASE_REVISION_MISMATCH when the published head moved, no merge)\nChangeset (C01, the publication unit of a workspace): changeset-create --store PUBLISHED --workspace-dir DIR [--changeset-id UUID] [--revision-id UUID] [--actor-id UUID] (base, unsent operations, complete layer set with base digests, conservative footprint; stored as a blob in the workspace, addressed by digest; same ids on an unchanged draft give the same digest); changeset-inspect --workspace-dir DIR --changeset SHA256; changeset-apply --store PUBLISHED --workspace-dir DIR --changeset SHA256 --actor-id UUID (the trusted caller: PERMISSION_DENIED unless it is the changeset's actor; the changeset id is bound to its payload before publication, so the same payload replays and another payload is OP_ID_REUSE; fast-forward only, BASE_REVISION_MISMATCH otherwise)\nCoordinator (C02, off unless started; loopback only; one publish queue; tokens from files, at least 32 printable bytes, not group/world-readable on Unix): coordinator-serve --store PUBLISHED --workspaces-root DIR --client ACTOR_UUID:TOKEN_FILE [--client ...] [--port N] [--max-requests N] (prints a listening line with the address, then serves; the token decides the actor, so a client publishes only its own changesets); coordinator-publish --address 127.0.0.1:PORT --token-file FILE --workspace NAME --changeset SHA256 --expected-head REVISION_UUID (NAME is a directory under the workspaces root; the expected head must be the changeset's base; BASE_REVISION_MISMATCH with the current head when it moved; the same payload replays); coordinator-head --address 127.0.0.1:PORT --token-file FILE\nUndo (P07, durable undo and redo of the draft; each is one new verified revision with the restored layers): workspace-undo --workspace-dir DIR [--revision-id UUID] [--op-id UUID]; workspace-redo --workspace-dir DIR [same]; workspace-history --workspace-dir DIR (undo and redo units; an edit after an undo discards redo; INVALID_ARGUMENT when there is nothing to undo or redo)\nReference (R01, a foreign USD file attached read-only; its bytes are stored under their SHA-256 and never rewritten): reference-attach --store ROOT --source FILE [--name TEXT] [--translation X,Y,Z] [--rotation-z-deg DEG] [--new-id UUID] [--op-id UUID] [--revision-id UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID] (FILE is a local USDA or restricted USDC layer of at most 65536 bytes with a defaultPrim and no other assets; a URL or network path is PERMISSION_DENIED before anything is read; the placement belongs to the wrapper and is stored apart from the foreign content); reference-move --store ROOT --id UUID [--translation X,Y,Z] [--rotation-z-deg DEG] [same ids] (absolute placement of the wrapper only); reference-correct-units --store ROOT --id UUID (--meters-per-unit F --up-axis Y|Z | --clear true) [same ids] (the explicit units and up-axis correction of the wrapper; the source declares what it declares and is never edited, and without a correction it is taken as stored); reference-write --store ROOT --id UUID (--target source | --prim /DefaultPrim/Path [--attribute NAME]) (always FOREIGN_READ_ONLY: the source layer and every foreign prim are read-only, including overrides in owned layers); reference-inspect --store ROOT (verified head: wrappers, pinned source, declared and explicit units, units status, foreign prims, external layers with their digests)\nView (synthetic kernel plate scene, no store): view-snapshot --plates 1..500 [--hide KIND,...] [--representation full|simplified] [--stand-in-simplified true|false] [--select ID,...] [--revision SHA256] [--generation U64]; view-changes --plates 1..500 --edit move-first|resize-first|add-one|remove-last,... [--since TOKEN] [--stand-in-simplified true|false]\nPick (V04, engine queries over the verified head's view snapshot; read-only; FeatureIds fid1:<element>:plate|member:<role> exist for box bodies only, others are listed as skipped; an index is never an address): pick --store ROOT (--origin X,Y,Z --direction X,Y,Z [--t-max MM] [--include-backfaces true|false] | --point X,Y,Z) --targets face,edge,vertex [--radius MM] [--max-hits 1..64]; feature-lookup --store ROOT --feature-id FID (kind, owner, local and world geometry); feature-list --store ROOT --id UUID (all qualified semantic box features at the exact model pin; unsupported bodies refuse); snap --store ROOT --point X,Y,Z [--radius MM] [--kinds vertex,axis_end,axis] [--max-hits 1..64] (box vertices and member axis start, end and nearest point)\nView export (V03, input of the native viewport): view-export --store ROOT --out FILE|- [--with-definitions true|false] [--with-element UUID] (the verified head's pinned view snapshot as one engine bundle; - writes the raw bundle to standard output, a FILE is created new and a JSON summary printed; --with-definitions true and --with-element, only with --out -, frame replies of the same inspection before the bundle: BCVXFRM1, then for the definitions and then the plate-inspect --element reply a little-endian u32 length and the reply as that command prints it, 0 for one not asked, then the bundle)\nBOM (E03): bom-export --store ROOT --out ELEMENTS_CSV [--summary SUMMARY_CSV] (the verified head's bill of materials as two CSV files: one row per plate, beam and column and one summary group per kind, profile and material; the mark is the element's approved numbering mark (A09), or the element id for an element without one; a mass without a density stays an empty cell and is counted in mass_unknown, never zero; both files are reserved before anything is written, so a refusal on either path leaves nothing on disk)\nUSD is read and written in this process by the pure-Rust layer engine under its size and depth budgets; no other process is started. revision-commit verifies blobs but not USD content; create-plate verifies the USD content (U03 index, structural check, kernel re-evaluation)."
        );
        return std::process::ExitCode::SUCCESS;
    }
    if args.first().is_some_and(|c| c == "view-export") {
        match view_export::execute(&args) {
            view_export::Output::Raw(bytes) => {
                use std::io::Write;
                let mut out = std::io::stdout().lock();
                if out.write_all(&bytes).and_then(|()| out.flush()).is_err() {
                    return std::process::ExitCode::from(3);
                }
                return std::process::ExitCode::SUCCESS;
            }
            view_export::Output::Framed { head, bundle } => {
                use std::io::Write;
                let mut out = std::io::stdout().lock();
                let written = out
                    .write_all(&head)
                    .and_then(|()| out.write_all(&bundle))
                    .and_then(|()| out.flush());
                if written.is_err() {
                    return std::process::ExitCode::from(3);
                }
                return std::process::ExitCode::SUCCESS;
            }
            view_export::Output::Json(json, exit) => return print_json(&json, exit),
        }
    }
    if args.first().is_some_and(export_cmd::handles) {
        // The report lists every element, so it can exceed the control-message bounds that
        // `print_json` enforces; it is rendered by the command itself.
        return export_cmd::run_and_print(&args);
    }
    // A serving coordinator holds its store for as long as it runs, so a compaction cannot start
    // under it (U14).
    let _serving = match history_cmd::coordinator_lease(&args) {
        Ok(lease) => lease,
        Err((json, exit)) => return print_json(&json, exit),
    };
    let (json, exit) = if args
        .first()
        .is_some_and(|c| c == "view-snapshot" || c == "view-changes")
    {
        view::execute(&args)
    } else if args.first().is_some_and(history_cmd::handles) {
        history_cmd::execute(&args)
    } else if args.first().is_some_and(|c| reference_cmd::handles(c)) {
        reference_cmd::execute(&args)
    } else if args.first().is_some_and(|c| {
        c == "create-assembly"
            || c == "assembly-attach"
            || c == "assembly-detach"
            || c == "assemblies"
    }) {
        assembly_cmd::execute(&args)
    } else if args.first().is_some_and(|c| {
        c == "changeset-create" || c == "changeset-inspect" || c == "changeset-apply"
    }) {
        changeset_cmd::execute(&args)
    } else if args.first().is_some_and(coordinator_cmd::handles) {
        coordinator_cmd::execute(&args)
    } else if args.first().is_some_and(pick_cmd::handles) {
        pick_cmd::execute(&args)
    } else if args.first().is_some_and(weld_cmd::handles) {
        weld_cmd::execute(&args)
    } else if args.first().is_some_and(bolt_cmd::handles) {
        bolt_cmd::execute(&args)
    } else if args.first().is_some_and(bom_cmd::handles) {
        bom_cmd::execute(&args)
    } else if args.first().is_some_and(library_cmd::handles) {
        library_cmd::execute(&args)
    } else if args.first().is_some_and(level_cmd::handles) {
        level_cmd::execute(&args)
    } else if args.first().is_some_and(numbering_cmd::handles) {
        numbering_cmd::execute(&args)
    } else if args.first().is_some_and(|c| {
        c == "create-plate"
            || c == "plate-inspect"
            || c == "define-material"
            || c == "define-profile"
            || c == "definitions"
            || c == "create-member"
            || c == "update-element"
            || c == "delete-element"
            || c == "copy-element"
            || c == "workspace-create"
            || c == "workspace-status"
            || c == "workspace-publish"
            || c == "workspace-undo"
            || c == "workspace-redo"
            || c == "workspace-history"
    }) {
        plate_cmd::execute(&args)
    } else {
        match run(args) {
            Ok(j) => (j, 0),
            Err((e, observed)) => (
                Json::object([
                    (
                        "outcome",
                        Json::text(if e.code == ErrorCode::OutcomeUnknown {
                            "outcome_unknown"
                        } else if e.code == ErrorCode::Cancelled {
                            "cancelled"
                        } else {
                            "rejected"
                        }),
                    ),
                    ("code", Json::text(e.code.as_str())),
                    ("message", Json::text(e.message())),
                    ("observed_context", observed),
                    ("diagnostics", Json::Array(vec![])),
                ]),
                e.exit_code(),
            ),
        }
    };
    print_json(&json, exit)
}
fn print_json(json: &Json, exit: u8) -> std::process::ExitCode {
    match json.encode(&Limits::DEFAULT) {
        Ok(bytes) => {
            use std::io::Write;
            if std::io::stdout().lock().write_all(&bytes).is_err() {
                return std::process::ExitCode::from(3);
            }
        }
        Err(_) => {
            println!("{{\"outcome\":\"rejected\",\"code\":\"LIMIT_EXCEEDED\"}}");
            return std::process::ExitCode::from(2);
        }
    }
    std::process::ExitCode::from(exit)
}
fn run(args: Vec<std::ffi::OsString>) -> Result<Json, (HostError, Json)> {
    let result = (|| -> Result<Json, HostError> {
        if args.len() > 49 {
            return Err(invalid());
        }
        let args = args
            .into_iter()
            .map(|s| s.into_string().map_err(|_| invalid()))
            .collect::<Result<Vec<_>, _>>()?;
        if args.iter().any(|s| s.len() > 4096) {
            return Err(invalid());
        }
        let command = args.first().ok_or_else(invalid)?;
        let tail = args.get(1..).ok_or_else(invalid)?;
        if !tail.len().is_multiple_of(2) {
            return Err(invalid());
        }
        let mut options = BTreeMap::new();
        for pair in tail.as_chunks::<2>().0 {
            let [key, value] = pair;
            if !matches!(
                key.as_str(),
                "--store"
                    | "--project-id"
                    | "--workspace-id"
                    | "--generation"
                    | "--file"
                    | "--digest"
                    | "--timeout-ms"
                    | "--cancel-after-ms"
                    | "--op-id"
                    | "--revision-id"
                    | "--base"
                    | "--schema"
                    | "--root-layer"
                    | "--layers"
                    | "--min-age-secs"
            ) || options.insert(key.as_str(), value.as_str()).is_some()
            {
                return Err(invalid());
            }
        }
        let get = |k: &str| options.get(k).copied().ok_or_else(invalid);
        let timeout = options
            .get("--timeout-ms")
            .map(|s| s.parse::<u64>().map_err(|_| invalid()))
            .transpose()?;
        let cancellation = Cancellation::default();
        if let Some(ms) = options.get("--cancel-after-ms") {
            let ms = ms.parse::<u64>().map_err(|_| invalid())?;
            if ms > 120000 {
                return Err(invalid());
            }
            if ms == 0 {
                cancellation.cancel();
            } else {
                let token = cancellation.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(ms));
                    token.cancel();
                });
            }
        }
        let control = Control::new(timeout, cancellation)?;
        control.remaining()?;
        let store = BlobStore::new(get("--store")?).map_err(|e| HostError::logical(e.code()))?;
        let parse_digest = |s: &str| s.parse::<ContentDigest>().map_err(|_| invalid());
        if command == "blob-put" {
            let bytes = read_file(get("--file")?, 32 * 1024 * 1024)?;
            let (digest, _) = store
                .publish(&bytes)
                .map_err(|e| HostError::logical(e.code()))?;
            control.remaining()?;
            return Ok(Json::object([
                ("outcome", Json::text("blob_stored")),
                ("digest", Json::text(digest.to_string())),
            ]));
        }
        if command == "blob-verify" {
            let digest = parse_digest(get("--digest")?)?;
            store
                .verify(&digest)
                .map_err(|e| HostError::logical(e.code()))?;
            control.remaining()?;
            return Ok(Json::object([
                ("outcome", Json::text("blob_verified")),
                ("digest", Json::text(digest.to_string())),
            ]));
        }
        if command == "revision-commit" {
            // The deadline and cancellation are checked before and during the commit (not
            // after the root switch: a timeout would hide a committed revision). A retry with
            // the same op id returns the revision.
            return revision::commit(&store, &options, &control);
        }
        if command == "revision-open" {
            let result = revision::open(&store)?;
            control.remaining()?;
            return Ok(result);
        }
        // Recovery and outcome queries only restate what the root proves; a deadline after the
        // work would hide a result that is already durable, so none is checked afterwards.
        if command == "revision-recover" {
            return revision::recover(&store, &options);
        }
        if command == "revision-outcome" {
            return revision::outcome(&store, &options);
        }
        Err(HostError::logical(ErrorCode::UnsupportedCommand))
    })();
    // These commands carry no workspace context to report.
    result.map_err(|e| (e, Json::Null))
}
fn read_file(path: &str, cap: usize) -> Result<Vec<u8>, HostError> {
    beamcraft_cli::source_file::read_file(path, cap).map_err(HostError::logical)
}
