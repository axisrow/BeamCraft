// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The single inventory of `beamcraft` commands.
//!
//! Every command the binary dispatches is listed here once, with its family, access, parameters and the
//! adapter that runs it. The binary routes a command word through [`route`], and the help
//! inventory and the `commands` command are rendered from this table, so a word that is not
//! listed here is not dispatched and a listed word cannot be left without a route. Add a command
//! here when it is added to its adapter.

use beamcraft_json::Json;
use std::ffi::OsStr;

/// The adapter in the binary that runs a command.
///
/// A route is a dispatch detail, not a family: `resolve-workspace` is documented with the
/// workspace commands but runs in its own adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    /// The store-level blob and revision commands.
    Generic,
    /// `commands`, which lists this registry.
    Commands,
    /// `export-view`, which writes a framed or raw byte stream.
    ExportView,
    /// The IFC export commands, whose output is rendered by the command itself.
    Export,
    /// The view snapshot commands.
    View,
    /// The named-view session commands.
    Views,
    /// The annotation commands.
    Annotation,
    /// The history inspection and compaction commands.
    History,
    /// The reference model commands.
    Reference,
    /// The assembly commands.
    Assembly,
    /// `rebase-workspace`.
    Rebase,
    /// `resolve-workspace`.
    Resolve,
    /// `compensate-workspace`.
    Compensate,
    /// The changeset commands.
    Changeset,
    /// The coordinator commands.
    Coordinator,
    /// The drawing commands.
    Drawing,
    /// The work plane and construction point commands.
    WorkPlane,
    /// The feature queries.
    Pick,
    /// The element filter query.
    Filters,
    /// The construction object commands.
    Construction,
    /// The weld commands.
    Weld,
    /// The bolt commands.
    Bolt,
    /// `export-bom`.
    Bom,
    /// `schedule-export`.
    Schedule,
    /// The library commands.
    Library,
    /// `fastener-geometry`.
    Fastener,
    /// `create-project` and `list-project`.
    Project,
    /// The grid commands.
    Grid,
    /// The level commands.
    Level,
    /// The F12 project descriptor and coordination commands.
    Coordination,
    /// The attribute commands.
    Attribute,
    /// The numbering commands.
    Numbering,
    /// The modelling, definition, edit and workspace commands.
    Plate,
}

/// What a command changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// Changes nothing persistent: it answers from the store, a library file or its own
    /// arguments, and is safe to repeat.
    Read,
    /// Changes something persistent: a revision in the store, a blob, the user settings or the
    /// recent-project list, a library file, or a requested output file.
    Write,
}

impl Access {
    /// The stable name printed by `commands`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
        }
    }
}

/// Whether a command accepts the five request identity options of the modelling commands:
/// `--op-id`, `--revision-id`, `--actor-id`, `--project-id` and `--workspace-id`, all optional
/// UUIDs that default to fresh ones.
///
/// A command that accepts some other set lists them as ordinary parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ids {
    /// The command takes none of them.
    None,
    /// The command takes all five.
    Common,
}

/// The identity options added to a command with [`Ids::Common`].
pub const COMMON_IDENTITY_OPTIONS: [&str; 5] = [
    "--op-id",
    "--revision-id",
    "--actor-id",
    "--project-id",
    "--workspace-id",
];

/// Whether a parameter must be given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Need {
    /// The command is refused without it, whatever else is given.
    Required,
    /// The command works without it. A rule that links it to other options (for example
    /// `--width` with `--length`) is in the family help and in the refusal.
    Optional,
    /// Exactly one parameter of this numbered group must be given.
    OneOf(u8),
}

/// One option of a command, `--name VALUE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Param {
    /// The option, with its leading `--`.
    pub name: &'static str,
    /// The shape of its value in the vocabulary of the help text (`ROOT`, `UUID`, `MM`,
    /// `X,Y,Z`, `top|middle|bottom`); empty for an option that takes no value.
    pub value: &'static str,
    /// Whether it must be given.
    pub need: Need,
}

/// A parameter the command is refused without.
const fn req(name: &'static str, value: &'static str) -> Param {
    Param {
        name,
        value,
        need: Need::Required,
    }
}

/// A parameter the command works without.
const fn opt(name: &'static str, value: &'static str) -> Param {
    Param {
        name,
        value,
        need: Need::Optional,
    }
}

/// A parameter of a group of which exactly one must be given.
const fn one(group: u8, name: &'static str, value: &'static str) -> Param {
    Param {
        name,
        value,
        need: Need::OneOf(group),
    }
}

/// One dispatched command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Command {
    /// The command word.
    pub name: &'static str,
    /// The command family; it matches the file under `src/help/` that documents the command.
    pub family: &'static str,
    /// The adapter that runs the command.
    pub route: Route,
    /// What the command changes.
    pub access: Access,
    /// Whether it takes the five common identity options.
    pub ids: Ids,
    /// The options it takes, apart from the common identity options.
    pub params: &'static [Param],
}

const fn entry(
    route: Route,
    family: &'static str,
    name: &'static str,
    access: Access,
    ids: Ids,
    params: &'static [Param],
) -> Command {
    Command {
        name,
        family,
        route,
        access,
        ids,
        params,
    }
}

/// Every command, in the order the help inventory lists them.
pub const COMMANDS: &[Command] = &[
    entry(
        Route::Generic,
        "blobs",
        "put-blob",
        Access::Write,
        Ids::None,
        &[
            req("--store", "ROOT"),
            req("--file", "FILE"),
            opt("--timeout-ms", "U64"),
            opt("--cancel-after-ms", "U64"),
        ],
    ),
    entry(
        Route::Generic,
        "blobs",
        "verify-blob",
        Access::Read,
        Ids::None,
        &[
            req("--store", "ROOT"),
            req("--digest", "SHA256"),
            opt("--timeout-ms", "U64"),
            opt("--cancel-after-ms", "U64"),
        ],
    ),
    entry(
        Route::Generic,
        "revision",
        "commit-revision",
        Access::Write,
        Ids::None,
        &[
            req("--store", "ROOT"),
            req("--project-id", "UUID"),
            req("--workspace-id", "UUID"),
            req("--revision-id", "UUID"),
            req("--op-id", "UUID"),
            req("--generation", "U64"),
            req("--base", "none|UUID"),
            req("--schema", "SHA256"),
            req("--root-layer", "TEXT"),
            req("--layers", "ROLE:NAME:SHA256,..."),
            opt("--timeout-ms", "U64"),
            opt("--cancel-after-ms", "U64"),
        ],
    ),
    entry(
        Route::Generic,
        "revision",
        "open-revision",
        Access::Read,
        Ids::None,
        &[
            req("--store", "ROOT"),
            opt("--timeout-ms", "U64"),
            opt("--cancel-after-ms", "U64"),
        ],
    ),
    entry(
        Route::Generic,
        "revision",
        "recover-revision",
        Access::Write,
        Ids::None,
        &[
            req("--store", "ROOT"),
            req("--min-age-secs", "U64"),
            opt("--timeout-ms", "U64"),
            opt("--cancel-after-ms", "U64"),
        ],
    ),
    entry(
        Route::Generic,
        "revision",
        "inspect-revision-outcome",
        Access::Read,
        Ids::None,
        &[
            req("--store", "ROOT"),
            req("--op-id", "UUID"),
            opt("--timeout-ms", "U64"),
            opt("--cancel-after-ms", "U64"),
        ],
    ),
    entry(
        Route::Plate,
        "plate",
        "create-plate",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--thickness", "MM"),
            opt("--width", "MM"),
            opt("--length", "MM"),
            opt("--contour", "X,Y[,Z];X,Y[,Z];..."),
            opt("--normalize-winding", "true|false"),
            opt("--thickness-side", "top|middle|bottom"),
            opt("--corners", "V=chamfer:D1[,D2];V=round:R;..."),
            opt("--name", "TEXT"),
            opt("--class", "INT32"),
            opt("--plane", "OX,OY,OZ,XX,XY,XZ,YX,YY,YZ"),
            opt("--translation", "X,Y,Z"),
            opt("--orientation-xyzw", "X,Y,Z,W"),
            opt("--origin-at-grid", "GRID:LABEL,LABEL"),
            opt("--z", "MM"),
            opt("--level", "UUID"),
            opt("--offset", "MM"),
            opt("--material-digest", "SHA256"),
            opt("--material", "TEXT"),
            opt("--hole-rect", "X,Y,W,H;X,Y,W,H;..."),
            opt("--hole-round", "X,Y,D;X,Y,D;..."),
            opt("--hole-segments", "N"),
            opt("--new-id", "UUID"),
        ],
    ),
    entry(
        Route::Plate,
        "plate",
        "create-lofted-plate",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--start", "X,Y,Z;X,Y,Z;..."),
            req("--end", "X,Y,Z;X,Y,Z;..."),
            opt("--name", "TEXT"),
            opt("--class", "INT32"),
            opt("--translation", "X,Y,Z"),
            opt("--orientation-xyzw", "X,Y,Z,W"),
            opt("--material-digest", "SHA256"),
            opt("--new-id", "UUID"),
        ],
    ),
    entry(
        Route::Plate,
        "plate",
        "inspect-lofted-plates",
        Access::Read,
        Ids::None,
        &[req("--store", "ROOT")],
    ),
    entry(
        Route::Plate,
        "plate",
        "add-hole",
        Access::Write,
        Ids::None,
        &[
            req("--store", "ROOT"),
            req("--id", "UUID"),
            one(1, "--polygon", "X,Y;X,Y;..."),
            one(1, "--hole-rect", "X,Y,W,H"),
            one(1, "--hole-round", "X,Y,D"),
            opt("--axis-point", "X,Y,Z"),
            opt("--axis-direction", "X,Y,Z"),
            opt("--diameter", "MM"),
            opt("--from", "MM"),
            opt("--to", "MM"),
            opt("--operation-id", "UUID"),
            opt("--revision-id", "UUID"),
            opt("--actor-id", "UUID"),
            opt("--project-id", "UUID"),
            opt("--workspace-id", "UUID"),
        ],
    ),
    entry(
        Route::Plate,
        "plate",
        "update-hole",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--id", "UUID"),
            req("--operation-id", "UUID"),
            one(1, "--polygon", "X,Y;X,Y;..."),
            one(1, "--hole-rect", "X,Y,W,H"),
            one(1, "--hole-round", "X,Y,D"),
            opt("--axis-point", "X,Y,Z"),
            opt("--axis-direction", "X,Y,Z"),
            opt("--diameter", "MM"),
            opt("--from", "MM"),
            opt("--to", "MM"),
        ],
    ),
    entry(
        Route::Plate,
        "plate",
        "remove-hole",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--id", "UUID"),
            req("--operation-id", "UUID"),
        ],
    ),
    entry(
        Route::Plate,
        "plate",
        "inspect-elements",
        Access::Read,
        Ids::None,
        &[req("--store", "ROOT"), opt("--element", "UUID")],
    ),
    entry(
        Route::Plate,
        "definitions",
        "define-material",
        Access::Write,
        Ids::None,
        &[
            req("--store", "ROOT"),
            opt("--density", "N"),
            opt("--grade", "TEXT"),
            opt("--standard", "TEXT"),
            opt("--name", "TEXT"),
            opt("--revision-id", "UUID"),
            opt("--actor-id", "UUID"),
            opt("--project-id", "UUID"),
            opt("--workspace-id", "UUID"),
        ],
    ),
    entry(
        Route::Plate,
        "definitions",
        "define-profile",
        Access::Write,
        Ids::None,
        &[
            req("--store", "ROOT"),
            opt("--width", "MM"),
            opt("--height", "MM"),
            opt(
                "--family",
                "i_shape|channel|angle|rhs|chs|lipped_c|plain_c|lipped_z|omega|custom",
            ),
            opt("--h", "MM"),
            opt("--b", "MM"),
            opt("--tw", "MM"),
            opt("--tf", "MM"),
            opt("--t", "MM"),
            opt("--r", "MM"),
            opt("--c", "MM"),
            opt("--d", "MM"),
            opt("--outer", "X,Y;X,Y;..."),
            opt("--cavity", "X,Y;...|X,Y;..."),
            opt("--library", "FILE"),
            opt("--environment", "FILE"),
            opt("--user-library", "FILE"),
            opt("--designation", "TEXT"),
            opt("--standard", "TEXT"),
            opt("--area-mm2", "N"),
            opt("--mass-per-meter-kg", "N"),
            opt("--surface-per-meter-m2", "N"),
            opt("--name", "TEXT"),
            opt("--revision-id", "UUID"),
            opt("--actor-id", "UUID"),
            opt("--project-id", "UUID"),
            opt("--workspace-id", "UUID"),
        ],
    ),
    entry(
        Route::Plate,
        "definitions",
        "define-fastener",
        Access::Write,
        Ids::None,
        &[
            req("--store", "ROOT"),
            req("--fastener-library", "FILE"),
            req("--designation", "TEXT"),
            opt("--name", "TEXT"),
            opt("--revision-id", "UUID"),
            opt("--actor-id", "UUID"),
            opt("--project-id", "UUID"),
            opt("--workspace-id", "UUID"),
        ],
    ),
    entry(
        Route::Plate,
        "definitions",
        "definitions",
        Access::Read,
        Ids::None,
        &[
            req("--store", "ROOT"),
            opt("--name", "TEXT"),
            opt("--offset", "N"),
            opt("--limit", "N"),
        ],
    ),
    entry(
        Route::Library,
        "library",
        "list-library",
        Access::Read,
        Ids::None,
        &[
            one(1, "--library", "FILE"),
            one(1, "--environment", "FILE"),
            one(1, "--fastener-library", "FILE"),
            opt("--user-library", "FILE"),
            opt("--family", "TEXT"),
            opt("--filter", "TEXT"),
            opt("--offset", "N"),
            opt("--limit", "N"),
            opt("--shadow-offset", "N"),
        ],
    ),
    entry(
        Route::Library,
        "library",
        "show-library",
        Access::Read,
        Ids::None,
        &[
            req("--designation", "TEXT"),
            one(1, "--library", "FILE"),
            one(1, "--environment", "FILE"),
            one(1, "--fastener-library", "FILE"),
            opt("--user-library", "FILE"),
        ],
    ),
    entry(
        Route::Library,
        "library",
        "add-library",
        Access::Write,
        Ids::None,
        &[
            req("--library", "FILE"),
            req("--designation", "TEXT"),
            opt("--library-id", "TEXT"),
            opt("--title", "TEXT"),
            opt("--width", "MM"),
            opt("--height", "MM"),
            opt(
                "--family",
                "i_shape|channel|angle|rhs|chs|lipped_c|plain_c|lipped_z|omega|custom",
            ),
            opt("--h", "MM"),
            opt("--b", "MM"),
            opt("--tw", "MM"),
            opt("--tf", "MM"),
            opt("--t", "MM"),
            opt("--r", "MM"),
            opt("--c", "MM"),
            opt("--d", "MM"),
            opt("--outer", "X,Y;X,Y;..."),
            opt("--cavity", "X,Y;X,Y;...|X,Y;X,Y;..."),
            opt("--standard", "TEXT"),
            opt("--area-mm2", "N"),
            opt("--mass-per-meter-kg", "N"),
            opt("--surface-per-meter-m2", "N"),
        ],
    ),
    entry(
        Route::Plate,
        "member",
        "create-member",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            one(1, "--profile-digest", "SHA256"),
            one(1, "--profile", "TEXT"),
            one(2, "--start", "X,Y,Z"),
            one(2, "--start-at-grid", "GRID:LABEL,LABEL"),
            one(3, "--end", "X,Y,Z"),
            one(3, "--end-at-grid", "GRID:LABEL,LABEL"),
            opt("--kind", "beam|column"),
            opt("--axis-lock", "none|x|y|z"),
            opt("--roll", "RAD"),
            opt("--material-digest", "SHA256"),
            opt("--material", "TEXT"),
            opt("--name", "TEXT"),
            opt("--class", "INT32"),
            opt("--plane", "OX,OY,OZ,XX,XY,XZ,YX,YY,YZ"),
            opt("--translation", "X,Y,Z"),
            opt("--orientation-xyzw", "X,Y,Z,W"),
            opt("--z", "MM"),
            opt("--level", "UUID"),
            opt("--offset", "MM"),
            opt("--section-plane", "left|centre|right"),
            opt("--section-plane-offset", "MM"),
            opt("--section-depth", "front|centre|back"),
            opt("--section-depth-offset", "MM"),
            opt("--start-offset", "MM"),
            opt("--end-offset", "MM"),
            opt("--new-id", "UUID"),
        ],
    ),
    entry(
        Route::Plate,
        "member",
        "create-polybeam",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--points", "X,Y,Z;X,Y,Z;..."),
            one(1, "--profile-digest", "SHA256"),
            one(1, "--profile", "TEXT"),
            opt("--kind", "beam|column"),
            opt("--material-digest", "SHA256"),
            opt("--material", "TEXT"),
            opt("--name", "TEXT"),
            opt("--class", "INT32"),
            opt("--translation", "X,Y,Z"),
            opt("--orientation-xyzw", "X,Y,Z,W"),
            opt("--new-id", "UUID"),
        ],
    ),
    entry(
        Route::Plate,
        "member",
        "create-curved-beam",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            one(1, "--profile-digest", "SHA256"),
            one(1, "--profile", "TEXT"),
            opt("--points", "X,Y,Z;X,Y,Z;..."),
            opt("--start", "X,Y,Z"),
            opt("--end", "X,Y,Z"),
            opt("--radius", "MM"),
            opt("--normal", "X,Y,Z"),
            opt("--kind", "beam|column"),
            opt("--material-digest", "SHA256"),
            opt("--material", "TEXT"),
            opt("--name", "TEXT"),
            opt("--class", "INT32"),
            opt("--translation", "X,Y,Z"),
            opt("--orientation-xyzw", "X,Y,Z,W"),
            opt("--new-id", "UUID"),
        ],
    ),
    entry(
        Route::WorkPlane,
        "work_plane",
        "work-plane",
        Access::Read,
        Ids::None,
        &[
            one(1, "--plane", "OX,OY,OZ,XX,XY,XZ,YX,YY,YZ"),
            one(1, "--parallel", "xy|xz|yz"),
            one(1, "--points", "OX,OY,OZ,PX,PY,PZ,QX,QY,QZ"),
            one(1, "--vertical", "AX,AY,AZ,BX,BY,BZ"),
            one(1, "--store", "ROOT"),
            opt("--offset", "MM"),
            opt("--origin", "X,Y,Z"),
            opt("--direction", "X,Y,Z"),
        ],
    ),
    entry(
        Route::WorkPlane,
        "work_plane",
        "resolve-construction-point",
        Access::Read,
        Ids::None,
        &[
            req("--reference", "X,Y,Z"),
            one(1, "--toward", "X,Y,Z"),
            one(1, "--origin", "X,Y,Z"),
            opt("--direction", "X,Y,Z"),
            opt("--distance", "MM"),
            opt("--ortho", "true|false"),
            opt("--plane", "OX,OY,OZ,XX,XY,XZ,YX,YY,YZ"),
        ],
    ),
    entry(
        Route::Plate,
        "edit",
        "update-element",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--id", "UUID"),
            opt("--name", "TEXT"),
            opt("--class", "INT32"),
            opt("--profile-digest", "SHA256"),
            opt("--profile", "TEXT"),
            opt("--material-digest", "SHA256|none"),
            opt("--thickness", "MM"),
            opt("--thickness-side", "top|middle|bottom"),
            opt("--corners", "V=chamfer:D1[,D2];V=round:R|none"),
            opt("--axis-point", "X,Y,Z;X,Y,Z;..."),
            opt("--translation", "X,Y,Z"),
            opt("--orientation-xyzw", "X,Y,Z,W|none"),
            opt("--level", "UUID|none"),
            opt("--rotate-about", "X,Y,Z"),
            opt("--rotate-axis", "X,Y,Z"),
            opt("--rotate-angle", "RAD"),
            opt("--mirror-point", "X,Y,Z"),
            opt("--mirror-normal", "X,Y,Z"),
            opt("--move-from", "X,Y,Z"),
            opt("--move-to", "X,Y,Z"),
        ],
    ),
    entry(
        Route::Plate,
        "edit",
        "move-plate-vertex",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--id", "UUID"),
            req("--vertex", "N"),
            req("--to", "X,Y"),
        ],
    ),
    entry(
        Route::Plate,
        "edit",
        "move-plate-edge",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--id", "UUID"),
            req("--edge", "N"),
            req("--delta", "DX,DY"),
        ],
    ),
    entry(
        Route::Plate,
        "edit",
        "insert-plate-vertex",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--id", "UUID"),
            req("--edge", "N"),
            req("--point", "X,Y"),
        ],
    ),
    entry(
        Route::Plate,
        "edit",
        "delete-plate-vertex",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--id", "UUID"),
            req("--vertex", "N"),
        ],
    ),
    entry(
        Route::Plate,
        "edit",
        "set-plate-corner",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--id", "UUID"),
            req("--vertex", "N"),
            req("--corner", "chamfer:D1[,D2]|round:R|sharp"),
        ],
    ),
    entry(
        Route::Plate,
        "edit",
        "cut-by-plane",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--id", "UUID"),
            req("--operation-id", "UUID"),
            req("--point", "X,Y,Z"),
            req("--normal", "X,Y,Z"),
            req("--keep", "positive|negative"),
        ],
    ),
    entry(
        Route::Plate,
        "edit",
        "update-cut",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--id", "UUID"),
            req("--operation-id", "UUID"),
            req("--point", "X,Y,Z"),
            req("--normal", "X,Y,Z"),
            req("--keep", "positive|negative"),
        ],
    ),
    entry(
        Route::Plate,
        "edit",
        "remove-cut",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--id", "UUID"),
            req("--operation-id", "UUID"),
        ],
    ),
    entry(
        Route::Plate,
        "edit",
        "cut-by-line",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--id", "UUID"),
            req("--operation-id", "UUID"),
            req("--point-a", "X,Y,Z"),
            req("--point-b", "X,Y,Z"),
            req("--axis", "X,Y,Z"),
            req("--keep", "positive|negative"),
        ],
    ),
    entry(
        Route::Plate,
        "edit",
        "cut-by-polygon",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--id", "UUID"),
            req("--operation-id", "UUID"),
            req("--polygon", "X,Y;X,Y;..."),
        ],
    ),
    entry(
        Route::Plate,
        "edit",
        "cut-by-part",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--id", "UUID"),
            req("--operation-id", "UUID"),
            req("--tool", "UUID"),
        ],
    ),
    entry(
        Route::Plate,
        "edit",
        "fit-end-to-part",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--id", "UUID"),
            req("--operation-id", "UUID"),
            req("--tool", "UUID"),
        ],
    ),
    entry(
        Route::Plate,
        "edit",
        "chamfer-part",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--id", "UUID"),
            req("--operation-id", "UUID"),
            req("--vertex-index", "N"),
            req("--first-distance", "MM"),
            req("--second-distance", "MM"),
        ],
    ),
    entry(
        Route::Plate,
        "edit",
        "split-part",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--id", "UUID"),
            req("--new-id", "UUID"),
            req("--new-name", "TEXT"),
            req("--operation-id", "UUID"),
            req("--point", "X,Y,Z"),
            req("--normal", "X,Y,Z"),
            req("--keep", "positive|negative"),
        ],
    ),
    entry(
        Route::Plate,
        "edit",
        "combine-parts",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--survivor", "UUID"),
            req("--consumed", "UUID"),
            req("--operation-id", "UUID"),
        ],
    ),
    entry(
        Route::Plate,
        "edit",
        "bend-plate",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--id", "UUID"),
            req("--operation-id", "UUID"),
            req("--kind", "fold|roll"),
            req("--point-a", "X,Y,Z"),
            req("--point-b", "X,Y,Z"),
            req("--radius", "MM"),
            opt("--radius-b", "MM"),
            opt("--angle-rad", "RAD"),
        ],
    ),
    entry(
        Route::Plate,
        "delete",
        "delete-element",
        Access::Write,
        Ids::Common,
        &[req("--store", "ROOT"), req("--id", "UUID")],
    ),
    entry(
        Route::Plate,
        "copy",
        "copy-element",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--id", "UUID"),
            opt("--name", "TEXT"),
            opt("--offset", "X,Y,Z"),
            opt("--copy-from", "X,Y,Z"),
            opt("--copy-to", "X,Y,Z"),
            opt("--new-id", "UUID"),
            opt("--new-axis-ids", "UUID,UUID,..."),
        ],
    ),
    entry(
        Route::Plate,
        "copy",
        "copy-instance",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--id", "UUID"),
            opt("--name", "TEXT"),
            opt("--offset", "X,Y,Z"),
            opt("--new-id", "UUID"),
        ],
    ),
    entry(
        Route::Plate,
        "copy",
        "array-linear",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--id", "UUID"),
            req("--count", "N"),
            req("--direction", "X,Y,Z"),
            req("--spacing", "MM"),
            opt("--name", "TEXT"),
        ],
    ),
    entry(
        Route::Plate,
        "copy",
        "array-polar",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--id", "UUID"),
            req("--count", "N"),
            req("--point", "X,Y,Z"),
            req("--axis", "X,Y,Z"),
            req("--angle", "RAD"),
            opt("--name", "TEXT"),
        ],
    ),
    entry(
        Route::Plate,
        "workspace",
        "create-workspace",
        Access::Write,
        Ids::None,
        &[
            req("--store", "ROOT"),
            req("--workspace-dir", "DIR"),
            opt("--workspace-id", "UUID"),
            opt("--user-id", "UUID"),
            opt("--revision-id", "UUID"),
            opt("--op-id", "UUID"),
            opt("--scope-code", "CODE"),
            opt("--scope-section", "CODE"),
            opt("--scope-zone", "CODE"),
            opt("--scope-shared", "true|false"),
        ],
    ),
    entry(
        Route::Plate,
        "workspace",
        "inspect-workspace",
        Access::Read,
        Ids::None,
        &[req("--store", "ROOT"), req("--workspace-dir", "DIR")],
    ),
    entry(
        Route::Plate,
        "workspace",
        "publish-workspace",
        Access::Write,
        Ids::None,
        &[req("--store", "ROOT"), req("--workspace-dir", "DIR")],
    ),
    entry(
        Route::Resolve,
        "workspace",
        "resolve-workspace",
        Access::Write,
        Ids::None,
        &[
            req("--workspace-dir", "DIR"),
            req("--op-id", "UUID"),
            req("--discard", "UUID"),
            req("--expected-draft", "UUID"),
            opt("--actor-id", "UUID"),
        ],
    ),
    entry(
        Route::Plate,
        "workspace",
        "undo-workspace",
        Access::Write,
        Ids::None,
        &[
            req("--workspace-dir", "DIR"),
            opt("--revision-id", "UUID"),
            opt("--op-id", "UUID"),
        ],
    ),
    entry(
        Route::Plate,
        "workspace",
        "redo-workspace",
        Access::Write,
        Ids::None,
        &[
            req("--workspace-dir", "DIR"),
            opt("--revision-id", "UUID"),
            opt("--op-id", "UUID"),
        ],
    ),
    entry(
        Route::Plate,
        "workspace",
        "inspect-workspace-history",
        Access::Read,
        Ids::None,
        &[req("--workspace-dir", "DIR")],
    ),
    entry(
        Route::Rebase,
        "rebase",
        "rebase-workspace",
        Access::Write,
        Ids::None,
        &[
            req("--store", "ROOT"),
            req("--workspace-dir", "DIR"),
            req("--op-id", "UUID"),
            req("--expected-head", "UUID"),
            req("--expected-draft", "UUID"),
            opt("--actor-id", "UUID"),
        ],
    ),
    entry(
        Route::Compensate,
        "rebase",
        "compensate-workspace",
        Access::Write,
        Ids::None,
        &[
            req("--store", "ROOT"),
            req("--workspace-dir", "DIR"),
            req("--target-revision", "UUID"),
            req("--op-id", "UUID"),
            req("--expected-head", "UUID"),
            opt("--actor-id", "UUID"),
        ],
    ),
    entry(
        Route::Reference,
        "reference",
        "attach-reference",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--source", "FILE"),
            opt("--name", "TEXT"),
            opt("--translation", "X,Y,Z"),
            opt("--rotation-z-deg", "DEG"),
            opt("--new-id", "UUID"),
        ],
    ),
    entry(
        Route::Reference,
        "reference",
        "move-reference",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--id", "UUID"),
            opt("--translation", "X,Y,Z"),
            opt("--rotation-z-deg", "DEG"),
        ],
    ),
    entry(
        Route::Reference,
        "reference",
        "correct-reference-units",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--id", "UUID"),
            opt("--meters-per-unit", "F"),
            opt("--up-axis", "Y|Z"),
            opt("--clear", "true"),
        ],
    ),
    entry(
        Route::Reference,
        "reference",
        "write-reference",
        Access::Read,
        Ids::None,
        &[
            req("--store", "ROOT"),
            req("--id", "UUID"),
            one(1, "--target", "source"),
            one(1, "--prim", "/DefaultPrim/Path"),
            opt("--attribute", "NAME"),
        ],
    ),
    entry(
        Route::Reference,
        "reference",
        "inspect-reference",
        Access::Read,
        Ids::None,
        &[req("--store", "ROOT")],
    ),
    entry(
        Route::Reference,
        "reference",
        "measure-reference",
        Access::Read,
        Ids::None,
        &[req("--store", "ROOT"), opt("--id", "UUID")],
    ),
    entry(
        Route::Assembly,
        "assembly",
        "create-assembly",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--name", "TEXT"),
            opt("--new-id", "UUID"),
        ],
    ),
    entry(
        Route::Assembly,
        "assembly",
        "attach-to-assembly",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--id", "UUID"),
            req("--assembly-id", "UUID"),
        ],
    ),
    entry(
        Route::Assembly,
        "assembly",
        "detach-from-assembly",
        Access::Write,
        Ids::Common,
        &[req("--store", "ROOT"), req("--id", "UUID")],
    ),
    entry(
        Route::Assembly,
        "assembly",
        "set-main-part",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--id", "UUID"),
            req("--assembly-id", "UUID"),
        ],
    ),
    entry(
        Route::Assembly,
        "assembly",
        "explode-assembly",
        Access::Write,
        Ids::Common,
        &[req("--store", "ROOT"), req("--id", "UUID")],
    ),
    entry(
        Route::Assembly,
        "assembly",
        "assemblies",
        Access::Read,
        Ids::None,
        &[req("--store", "ROOT")],
    ),
    entry(
        Route::Weld,
        "weld",
        "create-weld",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--name", "TEXT"),
            req("--main", "UUID"),
            req("--secondary", "UUID"),
            req("--size", "MM"),
            req("--location", "workshop|site"),
            req("--side", "arrow|other|both"),
            opt("--type", "fillet"),
            opt("--path", "X,Y,Z;X,Y,Z;..."),
            opt("--all-around", "true|false"),
            opt("--contour", "none|flush|concave|convex"),
            opt("--new-id", "UUID"),
        ],
    ),
    entry(
        Route::Weld,
        "weld",
        "welds",
        Access::Read,
        Ids::None,
        &[req("--store", "ROOT")],
    ),
    entry(
        Route::Bolt,
        "bolt",
        "create-bolt-group",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--name", "TEXT"),
            req("--parts", "UUID,UUID,..."),
            req("--fastener-library", "FILE"),
            req("--designation", "TEXT"),
            req("--point1", "X,Y,Z"),
            req("--point2", "X,Y,Z"),
            req("--plane-normal", "X,Y,Z"),
            opt("--rotation-deg", "DEG"),
            opt("--on-plane", "MM"),
            opt("--depth", "MM"),
            req("--offset-start", "MM"),
            opt("--spacings", "MM,MM,..."),
            opt("--count", "N"),
            opt("--pitch", "MM"),
            opt("--offset-end", "MM"),
            opt("--washer-under-head", "true|false"),
            opt("--washer-under-nut", "true|false"),
            opt("--length-mode", "auto|fixed"),
            opt("--length", "MM"),
            opt("--hole-mode", "generate|none"),
            opt("--hole-clearance", "MM"),
            opt("--new-id", "UUID"),
        ],
    ),
    entry(
        Route::Bolt,
        "bolt",
        "update-bolt-group",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--target", "UUID"),
            opt("--name", "TEXT"),
            opt("--parts", "UUID,UUID,..."),
            opt("--fastener-library", "FILE"),
            opt("--designation", "TEXT"),
            opt("--point1", "X,Y,Z"),
            opt("--point2", "X,Y,Z"),
            opt("--plane-normal", "X,Y,Z"),
            opt("--rotation-deg", "DEG"),
            opt("--on-plane", "MM"),
            opt("--depth", "MM"),
            opt("--offset-start", "MM"),
            opt("--spacings", "MM,MM,..."),
            opt("--count", "N"),
            opt("--pitch", "MM"),
            opt("--offset-end", "MM"),
            opt("--washer-under-head", "true|false"),
            opt("--washer-under-nut", "true|false"),
            opt("--length-mode", "auto|fixed"),
            opt("--length", "MM"),
            opt("--hole-mode", "generate|none"),
            opt("--hole-clearance", "MM"),
            opt("--repin", "true"),
        ],
    ),
    entry(
        Route::Bolt,
        "bolt",
        "bolts",
        Access::Read,
        Ids::None,
        &[req("--store", "ROOT")],
    ),
    entry(
        Route::Fastener,
        "bolt",
        "fastener-geometry",
        Access::Read,
        Ids::None,
        &[
            req("--fastener-library", "FILE"),
            req("--designation", "TEXT"),
            req("--length", "MM"),
            opt("--count", "N"),
            opt("--density", "KG_M3"),
            opt("--washer-under-head", "true|false"),
            opt("--washer-under-nut", "true|false"),
        ],
    ),
    entry(
        Route::Level,
        "level",
        "create-level",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--name", "TEXT"),
            req("--elevation", "MM"),
            opt("--new-id", "UUID"),
        ],
    ),
    entry(
        Route::Level,
        "level",
        "levels",
        Access::Read,
        Ids::None,
        &[req("--store", "ROOT")],
    ),
    entry(
        Route::Coordination,
        "coordination",
        "set-project-metadata",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            opt("--code", "CODE"),
            opt("--name", "TEXT"),
            opt("--organization", "TEXT"),
        ],
    ),
    entry(
        Route::Coordination,
        "coordination",
        "define-section",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--code", "CODE"),
            req("--module", "TOKENS"),
            opt("--section-id", "UUID"),
            opt("--label", "TEXT"),
        ],
    ),
    entry(
        Route::Coordination,
        "coordination",
        "define-zone",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--code", "CODE"),
            opt("--zone-id", "UUID"),
            opt("--label", "TEXT"),
        ],
    ),
    entry(
        Route::Coordination,
        "coordination",
        "show-project",
        Access::Read,
        Ids::Common,
        &[req("--store", "ROOT")],
    ),
    entry(
        Route::Coordination,
        "coordination",
        "connect-project",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--source-store", "ROOT"),
            req("--source-project", "UUID"),
            req("--source-revision", "UUID"),
            req("--label", "TEXT"),
            opt("--scope", "whole|CODE"),
            opt("--connection-id", "UUID"),
            opt("--translation", "X,Y,Z"),
            opt("--orientation", "X,Y,Z,W"),
        ],
    ),
    entry(
        Route::Coordination,
        "coordination",
        "update-connection",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--connection", "UUID"),
            req("--expected-revision", "UUID"),
            req("--source-store", "ROOT"),
            req("--source-project", "UUID"),
            req("--source-revision", "UUID"),
            opt("--scope", "whole|CODE"),
            opt("--label", "TEXT"),
            opt("--translation", "X,Y,Z"),
            opt("--orientation", "X,Y,Z,W"),
        ],
    ),
    entry(
        Route::Coordination,
        "coordination",
        "remove-connection",
        Access::Write,
        Ids::Common,
        &[req("--store", "ROOT"), req("--connection", "UUID")],
    ),
    entry(
        Route::Coordination,
        "coordination",
        "resolve-connection",
        Access::Read,
        Ids::Common,
        &[req("--store", "ROOT"), req("--connection", "UUID")],
    ),
    entry(
        Route::Attribute,
        "attribute",
        "attribute-define",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--name", "TEXT"),
            req("--type", "text|number|integer|boolean|choice"),
            opt("--unit", "TEXT"),
            opt("--options", "A,B,C"),
            opt("--set", "TEXT"),
        ],
    ),
    entry(
        Route::Attribute,
        "attribute",
        "attribute-undefine",
        Access::Write,
        Ids::Common,
        &[req("--store", "ROOT"), req("--name", "TEXT")],
    ),
    entry(
        Route::Attribute,
        "attribute",
        "attribute-set",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--element", "UUID"),
            req("--name", "TEXT"),
            req("--value", "TEXT"),
        ],
    ),
    entry(
        Route::Attribute,
        "attribute",
        "attribute-clear",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--element", "UUID"),
            req("--name", "TEXT"),
        ],
    ),
    entry(
        Route::Attribute,
        "attribute",
        "attribute-get",
        Access::Read,
        Ids::None,
        &[req("--store", "ROOT"), req("--element", "UUID")],
    ),
    entry(
        Route::Attribute,
        "attribute",
        "attributes",
        Access::Read,
        Ids::None,
        &[req("--store", "ROOT")],
    ),
    entry(
        Route::Filters,
        "filters",
        "query-elements",
        Access::Read,
        Ids::None,
        &[
            req("--store", "ROOT"),
            req("--rule", "FILE"),
            opt("--limit", "N"),
            opt("--after", "UUID"),
            opt("--expected-model", "SHA256|none"),
        ],
    ),
    entry(
        Route::Grid,
        "grid",
        "create-grid",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--name", "TEXT"),
            opt("--new-id", "UUID"),
            opt("--origin", "X,Y"),
            opt("--rotation", "DEG"),
            opt("--u-expression", "TEXT"),
            opt("--u-labels", "TEXT"),
            opt("--u-scheme", "numeric|alpha"),
            opt("--u-label-start", "TEXT"),
            opt("--u-label-step", "N"),
            opt("--u-label-order", "ascending|descending"),
            opt("--u-axis-ids", "UUID,UUID,..."),
            opt("--u-span", "MM,MM"),
            opt("--u-from", "X,Y"),
            opt("--u-to", "X,Y"),
            opt("--u-label", "TEXT"),
            opt("--u-axis-id", "UUID"),
            opt("--v-expression", "TEXT"),
            opt("--v-labels", "TEXT"),
            opt("--v-scheme", "numeric|alpha"),
            opt("--v-label-start", "TEXT"),
            opt("--v-label-step", "N"),
            opt("--v-label-order", "ascending|descending"),
            opt("--v-axis-ids", "UUID,UUID,..."),
            opt("--v-span", "MM,MM"),
            opt("--v-from", "X,Y"),
            opt("--v-to", "X,Y"),
            opt("--v-label", "TEXT"),
            opt("--v-axis-id", "UUID"),
        ],
    ),
    entry(
        Route::Grid,
        "grid",
        "update-grid",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--id", "UUID"),
            opt("--name", "TEXT"),
            opt("--origin", "X,Y"),
            opt("--rotation", "DEG"),
            opt("--fit-spans", "true|false"),
            opt("--u-expression", "TEXT"),
            opt("--u-labels", "TEXT"),
            opt("--u-new-axis-ids", "UUID,UUID,..."),
            opt("--u-span", "MM,MM"),
            opt("--v-expression", "TEXT"),
            opt("--v-labels", "TEXT"),
            opt("--v-new-axis-ids", "UUID,UUID,..."),
            opt("--v-span", "MM,MM"),
        ],
    ),
    entry(
        Route::Grid,
        "grid",
        "list-grids",
        Access::Read,
        Ids::None,
        &[req("--store", "ROOT")],
    ),
    entry(
        Route::Grid,
        "grid",
        "resolve-grid-point",
        Access::Read,
        Ids::None,
        &[
            req("--store", "ROOT"),
            req("--grid", "UUID|TEXT"),
            req("--labels", "U,V"),
        ],
    ),
    entry(
        Route::Grid,
        "grid",
        "resolve-grid-plane",
        Access::Read,
        Ids::None,
        &[
            req("--store", "ROOT"),
            req("--grid", "UUID|TEXT"),
            req("--label", "TEXT"),
        ],
    ),
    entry(
        Route::Grid,
        "grid",
        "draw-grid-line",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--id", "UUID"),
            req("--family", "u|v"),
            req("--label", "TEXT"),
            req("--from", "X,Y"),
            req("--to", "X,Y"),
            opt("--new-axis-id", "UUID"),
        ],
    ),
    entry(
        Route::Grid,
        "grid",
        "reshape-grid-line",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--id", "UUID"),
            req("--axis-id", "UUID"),
            req("--from", "X,Y"),
            req("--to", "X,Y"),
        ],
    ),
    entry(
        Route::Grid,
        "grid",
        "remove-grid-line",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--id", "UUID"),
            req("--axis-id", "UUID"),
        ],
    ),
    entry(
        Route::Grid,
        "grid",
        "relabel-grid-line",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--id", "UUID"),
            opt("--axis-id", "UUID"),
            opt("--label", "TEXT"),
            opt("--family", "u|v"),
            opt("--labels", "TEXT"),
        ],
    ),
    entry(
        Route::Construction,
        "construction",
        "create-construction",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--name", "TEXT"),
            req("--kind", "point|line|circle|plane"),
            req("--a", "X,Y,Z"),
            opt("--b", "X,Y,Z"),
            opt("--radius", "MM"),
            opt("--new-id", "UUID"),
        ],
    ),
    entry(
        Route::Construction,
        "construction",
        "list-construction",
        Access::Read,
        Ids::None,
        &[req("--store", "ROOT")],
    ),
    entry(
        Route::Export,
        "export",
        "export-ifc",
        Access::Write,
        Ids::None,
        &[
            req("--store", "ROOT"),
            req("--out", "FILE"),
            opt("--revision", "UUID"),
            opt("--project-name", "TEXT"),
            opt("--schema", "IFC4"),
            opt("--format", "step"),
            opt("--report", "FILE"),
            opt("--strict", ""),
            opt("--overwrite", ""),
        ],
    ),
    entry(
        Route::Export,
        "export",
        "ifc-guid",
        Access::Read,
        Ids::None,
        &[one(1, "--id", "UUID"), one(1, "--decode", "GLOBALID")],
    ),
    entry(
        Route::Numbering,
        "numbering",
        "preview-numbering",
        Access::Read,
        Ids::None,
        &[
            req("--store", "ROOT"),
            opt("--plate-series", "PREFIX:START"),
            opt("--beam-series", "PREFIX:START"),
            opt("--column-series", "PREFIX:START"),
            opt("--assembly-series", "PREFIX:START"),
            opt("--tolerance", "MM"),
            opt("--compare-name", "true|false"),
            opt("--previous", "SHA256"),
            opt("--offset", "N"),
            opt("--limit", "N"),
        ],
    ),
    entry(
        Route::Numbering,
        "numbering",
        "approve-numbering",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--preview-token", "SHA256"),
            opt("--plate-series", "PREFIX:START"),
            opt("--beam-series", "PREFIX:START"),
            opt("--column-series", "PREFIX:START"),
            opt("--assembly-series", "PREFIX:START"),
            opt("--tolerance", "MM"),
            opt("--compare-name", "true|false"),
        ],
    ),
    entry(
        Route::Numbering,
        "numbering",
        "inspect-numbering",
        Access::Read,
        Ids::None,
        &[
            req("--store", "ROOT"),
            opt("--offset", "N"),
            opt("--limit", "N"),
        ],
    ),
    entry(
        Route::Changeset,
        "changeset",
        "create-changeset",
        Access::Write,
        Ids::None,
        &[
            req("--store", "ROOT"),
            req("--workspace-dir", "DIR"),
            opt("--changeset-id", "UUID"),
            opt("--revision-id", "UUID"),
            opt("--actor-id", "UUID"),
        ],
    ),
    entry(
        Route::Changeset,
        "changeset",
        "inspect-changeset",
        Access::Read,
        Ids::None,
        &[req("--workspace-dir", "DIR"), req("--changeset", "SHA256")],
    ),
    entry(
        Route::Changeset,
        "changeset",
        "apply-changeset",
        Access::Write,
        Ids::None,
        &[
            req("--store", "ROOT"),
            req("--workspace-dir", "DIR"),
            req("--changeset", "SHA256"),
            req("--actor-id", "UUID"),
        ],
    ),
    entry(
        Route::Coordinator,
        "coordinator",
        "serve-coordinator",
        Access::Write,
        Ids::None,
        &[
            req("--store", "ROOT"),
            req("--workspaces-root", "DIR"),
            req("--client", "ACTOR_UUID:TOKEN_FILE"),
            opt("--port", "N"),
            opt("--max-requests", "N"),
        ],
    ),
    entry(
        Route::Coordinator,
        "coordinator",
        "publish-changeset",
        Access::Write,
        Ids::None,
        &[
            req("--address", "IP:PORT"),
            req("--token-file", "FILE"),
            req("--workspace", "NAME"),
            req("--changeset", "SHA256"),
            req("--expected-head", "UUID"),
        ],
    ),
    entry(
        Route::Coordinator,
        "coordinator",
        "coordinator-compensate",
        Access::Write,
        Ids::None,
        &[
            req("--address", "IP:PORT"),
            req("--token-file", "FILE"),
            req("--workspace", "NAME"),
            req("--op-id", "UUID"),
            req("--target-revision", "UUID"),
            req("--expected-head", "UUID"),
        ],
    ),
    entry(
        Route::Coordinator,
        "coordinator",
        "coordinator-rebase",
        Access::Write,
        Ids::None,
        &[
            req("--address", "IP:PORT"),
            req("--token-file", "FILE"),
            req("--workspace", "NAME"),
            req("--op-id", "UUID"),
            req("--expected-head", "UUID"),
            req("--expected-draft", "UUID"),
        ],
    ),
    entry(
        Route::Coordinator,
        "coordinator",
        "inspect-coordinator-head",
        Access::Read,
        Ids::None,
        &[req("--address", "IP:PORT"), req("--token-file", "FILE")],
    ),
    entry(
        Route::Views,
        "views",
        "list-views",
        Access::Read,
        Ids::None,
        &[req("--file", "FILE"), opt("--store", "ROOT")],
    ),
    entry(
        Route::Views,
        "views",
        "save-view",
        Access::Write,
        Ids::None,
        &[
            req("--file", "FILE"),
            req("--name", "TEXT"),
            opt("--store", "ROOT"),
            opt("--target", "X,Y,Z"),
            opt("--yaw", "RAD"),
            opt("--pitch", "RAD"),
            opt("--distance", "MM"),
            opt("--ortho", "true|false"),
            opt("--work-area", "MM,MM,MM,MM,MM,MM"),
            opt("--plane-grid", "SELECTOR"),
            opt("--plane-line", "LABEL"),
            opt("--plane-reverse", "true|false"),
            opt("--plane-level", "true|false"),
            opt("--from", "X,Y,Z"),
            opt("--to", "X,Y,Z"),
            opt("--centre", "X,Y,Z"),
        ],
    ),
    entry(
        Route::Views,
        "views",
        "remove-view",
        Access::Write,
        Ids::None,
        &[req("--file", "FILE"), req("--name", "TEXT")],
    ),
    entry(
        Route::Views,
        "views",
        "apply-view",
        Access::Read,
        Ids::None,
        &[req("--file", "FILE"), req("--name", "TEXT")],
    ),
    entry(
        Route::Annotation,
        "annotation",
        "annotation-create",
        Access::Write,
        Ids::Common,
        &[
            req("--store", "ROOT"),
            req("--name", "TEXT"),
            req("--kind", "dimension|mark|leader"),
            req("--new-id", "UUID"),
            opt("--from", "FEATURE_ID"),
            opt("--to", "FEATURE_ID"),
            opt("--anchor", "FEATURE_ID"),
            opt("--attribute", "TEXT"),
            opt("--text", "TEXT"),
            opt("--end", "X,Y,Z"),
        ],
    ),
    entry(
        Route::Annotation,
        "annotation",
        "annotation-list",
        Access::Read,
        Ids::None,
        &[req("--store", "ROOT")],
    ),
    entry(
        Route::History,
        "history",
        "inspect-history",
        Access::Read,
        Ids::None,
        &[
            req("--store", "ROOT"),
            opt("--workspaces-root", "DIR"),
            opt("--workspace-dir", "DIR"),
        ],
    ),
    entry(
        Route::History,
        "history",
        "compact-history",
        Access::Write,
        Ids::None,
        &[
            req("--store", "ROOT"),
            one(1, "--keep-head", ""),
            one(1, "--keep", "N"),
            opt("--dry-run", ""),
            opt("--yes", ""),
            opt("--backup", "DIR"),
            opt("--no-backup", ""),
            opt("--workspaces-root", "DIR"),
            opt("--workspace-dir", "DIR"),
            opt("--no-workspaces", ""),
            opt("--ledger-tail", "N"),
            opt("--orphan-grace-secs", "N"),
        ],
    ),
    entry(
        Route::View,
        "view",
        "snapshot-view",
        Access::Read,
        Ids::None,
        &[
            req("--plates", "N"),
            opt("--hide", "KIND,..."),
            opt("--representation", "full|simplified"),
            opt("--stand-in-simplified", "true|false"),
            opt("--select", "UUID,..."),
            opt("--revision", "SHA256"),
            opt("--generation", "U64"),
        ],
    ),
    entry(
        Route::View,
        "view",
        "inspect-view-changes",
        Access::Read,
        Ids::None,
        &[
            req("--plates", "N"),
            req("--edit", "move-first|resize-first|add-one|remove-last,..."),
            opt("--since", "TOKEN"),
            opt("--stand-in-simplified", "true|false"),
            opt("--revision", "SHA256"),
            opt("--generation", "U64"),
        ],
    ),
    entry(
        Route::View,
        "view",
        "clip-view",
        Access::Read,
        Ids::None,
        &[
            req("--plates", "N"),
            opt("--cut", "X,Y,Z,X,Y,Z;..."),
            opt("--area", "MM,MM,MM,MM,MM,MM"),
            opt("--stand-in-simplified", "true|false"),
            opt("--revision", "SHA256"),
            opt("--generation", "U64"),
        ],
    ),
    entry(
        Route::Pick,
        "pick",
        "pick",
        Access::Read,
        Ids::None,
        &[
            req("--store", "ROOT"),
            opt("--origin", "X,Y,Z"),
            opt("--direction", "X,Y,Z"),
            opt("--t-max", "MM"),
            opt("--include-backfaces", "true|false"),
            opt("--point", "X,Y,Z"),
            req("--targets", "face,edge,vertex"),
            opt("--radius", "MM"),
            opt("--max-hits", "N"),
            opt("--exclude", "KIND,..."),
        ],
    ),
    entry(
        Route::Pick,
        "pick",
        "snap",
        Access::Read,
        Ids::None,
        &[
            req("--store", "ROOT"),
            req("--point", "X,Y,Z"),
            opt("--radius", "MM"),
            opt("--kinds", "KIND,..."),
            opt("--max-hits", "N"),
            opt("--plane", "OX,OY,OZ,XX,XY,XZ,YX,YY,YZ"),
            opt("--origin", "X,Y,Z"),
            opt("--direction", "X,Y,Z"),
            opt("--aperture", "F"),
            opt("--cursor-radius", "MM"),
            opt("--reference-snaps", "true|false"),
            opt("--geometry-snaps", "true|false"),
            opt("--grid-snaps", "true|false"),
            opt("--construction-snaps", "true|false"),
            opt("--grids", "UUID,..."),
            opt("--nearest", "true|false"),
        ],
    ),
    entry(
        Route::Pick,
        "pick",
        "lookup-feature",
        Access::Read,
        Ids::None,
        &[req("--store", "ROOT"), req("--feature-id", "TEXT")],
    ),
    entry(
        Route::Pick,
        "pick",
        "list-feature",
        Access::Read,
        Ids::None,
        &[req("--store", "ROOT"), req("--id", "UUID")],
    ),
    entry(
        Route::Pick,
        "pick",
        "element-edges",
        Access::Read,
        Ids::None,
        &[req("--store", "ROOT"), req("--id", "UUID")],
    ),
    entry(
        Route::ExportView,
        "view_export",
        "export-view",
        Access::Write,
        Ids::None,
        &[
            req("--store", "ROOT"),
            req("--out", "FILE"),
            opt("--with-definitions", "true|false"),
            opt("--with-element", "UUID"),
            opt("--with-grids", "true|false"),
        ],
    ),
    entry(
        Route::Bom,
        "bom",
        "export-bom",
        Access::Write,
        Ids::None,
        &[
            req("--store", "ROOT"),
            req("--out", "FILE"),
            opt("--summary", "FILE"),
        ],
    ),
    entry(
        Route::Schedule,
        "schedules",
        "schedule-export",
        Access::Write,
        Ids::None,
        &[
            req("--store", "ROOT"),
            req("--template", "FILE"),
            req("--out", "FILE"),
            opt("--summary", "FILE"),
            opt("--html", "FILE"),
        ],
    ),
    entry(
        Route::Drawing,
        "drawing",
        "preview-drawing",
        Access::Write,
        Ids::None,
        &[
            req("--store", "ROOT"),
            req("--id", "UUID"),
            req("--drawing-id", "UUID"),
            req("--view-id", "UUID"),
            req("--dimension-id", "UUID"),
            req("--anchor-a", "FEATURE_ID"),
            req("--anchor-b", "FEATURE_ID"),
            opt("--name", "TEXT"),
            opt("--plane", "front|top|right"),
            opt("--scale", "N"),
            opt("--origin", "X,Y"),
            opt("--offset", "MM"),
            opt("--decimals", "0..255"),
            opt("--intent-out", "FILE"),
        ],
    ),
    entry(
        Route::Drawing,
        "drawing",
        "create-drawing",
        Access::Write,
        Ids::None,
        &[
            req("--store", "ROOT"),
            req("--document-dir", "DIR"),
            req("--op-id", "UUID"),
            req("--revision-id", "UUID"),
            req("--actor-id", "UUID"),
            req("--id", "UUID"),
            req("--drawing-id", "UUID"),
            req("--view-id", "UUID"),
            req("--dimension-id", "UUID"),
            req("--anchor-a", "FEATURE_ID"),
            req("--anchor-b", "FEATURE_ID"),
            opt("--name", "TEXT"),
            opt("--view-title", "TEXT"),
            opt("--plane", "front|top|right"),
            opt("--scale", "N"),
            opt("--origin", "X,Y"),
            opt("--offset", "MM"),
            opt("--decimals", "0..255"),
        ],
    ),
    entry(
        Route::Drawing,
        "drawing",
        "refresh-drawing",
        Access::Write,
        Ids::None,
        &[
            req("--store", "ROOT"),
            req("--document-dir", "DIR"),
            req("--expected-head", "UUID"),
            req("--op-id", "UUID"),
            req("--revision-id", "UUID"),
            req("--actor-id", "UUID"),
            req("--id", "UUID"),
            req("--drawing-id", "UUID"),
            req("--view-id", "UUID"),
            req("--dimension-id", "UUID"),
            req("--anchor-a", "FEATURE_ID"),
            req("--anchor-b", "FEATURE_ID"),
            opt("--name", "TEXT"),
            opt("--view-title", "TEXT"),
            opt("--plane", "front|top|right"),
            opt("--scale", "N"),
            opt("--origin", "X,Y"),
            opt("--offset", "MM"),
            opt("--decimals", "0..255"),
        ],
    ),
    entry(
        Route::Drawing,
        "drawing",
        "edit-drafting",
        Access::Write,
        Ids::None,
        &[
            req("--document-dir", "DIR"),
            req("--expected-head", "UUID"),
            req("--op-id", "UUID"),
            req("--revision-id", "UUID"),
            req("--actor-id", "UUID"),
            opt("--drafting", "UUID:kind:X,Y,..."),
        ],
    ),
    entry(
        Route::Drawing,
        "drawing",
        "edit-views",
        Access::Write,
        Ids::None,
        &[
            req("--document-dir", "DIR"),
            req("--expected-head", "UUID"),
            req("--op-id", "UUID"),
            req("--revision-id", "UUID"),
            req("--actor-id", "UUID"),
            req("--placed-view", "SPEC"),
            opt("--placed-section", "SPEC"),
            opt("--placed-detail", "SPEC"),
        ],
    ),
    entry(
        Route::Drawing,
        "drawing",
        "edit-sheet",
        Access::Write,
        Ids::None,
        &[
            req("--document-dir", "DIR"),
            req("--expected-head", "UUID"),
            req("--op-id", "UUID"),
            req("--revision-id", "UUID"),
            req("--actor-id", "UUID"),
            opt("--sheet-size", "SIZE"),
            opt("--sheet-frame", "true|false"),
            opt("--sheet-project", "TEXT"),
            opt("--sheet-author", "TEXT"),
            opt("--sheet-remove", "true"),
        ],
    ),
    entry(
        Route::Drawing,
        "drawing",
        "publish-drawing",
        Access::Write,
        Ids::None,
        &[
            req("--document-dir", "DIR"),
            req("--expected-head", "UUID"),
            req("--op-id", "UUID"),
            req("--revision-id", "UUID"),
            req("--actor-id", "UUID"),
            req("--version-id", "UUID"),
        ],
    ),
    entry(
        Route::Drawing,
        "drawing",
        "inspect-drawing",
        Access::Read,
        Ids::None,
        &[req("--document-dir", "DIR"), opt("--model-store", "ROOT")],
    ),
    entry(
        Route::Drawing,
        "drawing",
        "inspect-drawing-versions",
        Access::Read,
        Ids::None,
        &[req("--document-dir", "DIR")],
    ),
    entry(
        Route::Drawing,
        "drawing",
        "export-drawing",
        Access::Write,
        Ids::None,
        &[
            req("--document-dir", "DIR"),
            req("--version-id", "UUID"),
            req("--out", "FILE"),
            opt("--format", "svg|pdf"),
            opt("--sheet", "a4|a3|a2|a1|a0"),
        ],
    ),
    entry(
        Route::Project,
        "start",
        "create-project",
        Access::Write,
        Ids::Common,
        &[
            req("--parent", "DIR"),
            req("--name", "TEXT"),
            opt("--unique-name", "true|false"),
            opt("--config-dir", "DIR"),
            opt("--environment", "ID"),
            opt("--code", "CODE"),
            opt("--sections", "LIST"),
        ],
    ),
    entry(
        Route::Project,
        "start",
        "list-project",
        Access::Read,
        Ids::None,
        &[req("--dir", "DIR")],
    ),
    entry(
        Route::Commands,
        "commands",
        "commands",
        Access::Read,
        Ids::None,
        &[],
    ),
];

/// The adapter that runs `word`, or `None` when it is not a registered command.
///
/// The binary falls back to the store-level path for an unregistered word, which refuses it.
pub fn route(word: &OsStr) -> Option<Route> {
    let word = word.to_str()?;
    COMMANDS.iter().find(|c| c.name == word).map(|c| c.route)
}

/// One parameter in usage notation: `--name VALUE`, or `--name` for an option without a value.
fn usage_item(param: &Param) -> String {
    if param.value.is_empty() {
        param.name.to_owned()
    } else {
        format!("{} {}", param.name, param.value)
    }
}

/// The usage items of a command: required options, then each one-of group as `(--a X | --b Y)`,
/// then optional options in brackets, then the common identity options when it takes them.
fn usage_items(command: &Command) -> Vec<String> {
    let mut items: Vec<String> = command
        .params
        .iter()
        .filter(|p| p.need == Need::Required)
        .map(usage_item)
        .collect();
    let mut groups: Vec<u8> = command
        .params
        .iter()
        .filter_map(|p| match p.need {
            Need::OneOf(group) => Some(group),
            _ => None,
        })
        .collect();
    groups.sort_unstable();
    groups.dedup();
    for group in groups {
        let members: Vec<String> = command
            .params
            .iter()
            .filter(|p| p.need == Need::OneOf(group))
            .map(usage_item)
            .collect();
        items.push(format!("({})", members.join(" | ")));
    }
    items.extend(
        command
            .params
            .iter()
            .filter(|p| p.need == Need::Optional)
            .map(|p| format!("[{}]", usage_item(p))),
    );
    if command.ids == Ids::Common {
        items.extend(
            COMMON_IDENTITY_OPTIONS
                .iter()
                .map(|name| format!("[{name} UUID]")),
        );
    }
    items
}

/// Lays `words` out in lines of at most `width` columns, continuation lines indented.
fn wrap(words: &[String], width: usize) -> String {
    let mut out = String::new();
    let mut line = 0;
    for word in words {
        if line == 0 {
            line = word.len();
        } else if line + 1 + word.len() > width {
            out.push_str("\n    ");
            line = 4 + word.len();
        } else {
            out.push(' ');
            line += 1 + word.len();
        }
        out.push_str(word);
    }
    out
}

/// The command reference as Markdown, generated from the registry: for every command its family,
/// access and one usage line (required options bare, optional ones in brackets, one-of groups
/// in parentheses). It is committed as `apps/beamcraft-cli/COMMANDS.md`, and a test fails when the
/// committed file differs from this text.
pub fn reference_markdown() -> String {
    let mut out = String::from(
        "# BeamCraft CLI command reference\n\n\
         Generated from `src/registry.rs`; do not edit. Regenerate it with\n\
         `BEAMCRAFT_BLESS=1 cargo test -p beamcraft-cli --test cli registry::the_reference_is_current`.\n\n\
         A command is `beamcraft COMMAND --option VALUE ...`. In a usage line an option written bare is\n\
         required, `[--option VALUE]` is optional, and `(--a X | --b Y)` means exactly one of them is\n\
         required. A rule that links optional options (for example `--width` with `--length`) is in\n\
         the family help, `beamcraft --help`, and in the refusal. `read` commands change nothing\n\
         persistent; `write` commands change the store, the settings, a library or an output file.\n",
    );
    let mut family = "";
    for command in COMMANDS {
        if command.family != family {
            family = command.family;
            out.push_str(&format!("\n## {family}\n"));
        }
        let mut words = vec![command.name.to_owned()];
        words.extend(usage_items(command));
        out.push_str(&format!(
            "\n### {}\n\n{}.\n\n```text\n{}\n```\n",
            command.name,
            command.access.as_str(),
            wrap(&words, 98)
        ));
    }
    out
}

/// One option as the `commands` result prints it.
fn param_json(param: &Param) -> Json {
    let (required, group) = match param.need {
        Need::Required => (true, Json::Null),
        Need::Optional => (false, Json::Null),
        Need::OneOf(group) => (false, Json::uint(u64::from(group))),
    };
    Json::object([
        ("name", Json::text(param.name)),
        ("value", Json::text(param.value)),
        ("required", Json::Bool(required)),
        ("one_of", group),
    ])
}

/// Every option the command takes: its own, then the common identity options when it has them.
fn parameters(command: &Command) -> Vec<Json> {
    let identity = match command.ids {
        Ids::None => &[][..],
        Ids::Common => &COMMON_IDENTITY_OPTIONS[..],
    };
    command
        .params
        .iter()
        .map(param_json)
        .chain(identity.iter().map(|name| param_json(&opt(name, "UUID"))))
        .collect()
}

/// The `commands` result: the registry with every command's family, access and parameters.
pub fn listing() -> Json {
    let commands = COMMANDS
        .iter()
        .map(|c| {
            Json::object([
                ("name", Json::text(c.name)),
                ("family", Json::text(c.family)),
                ("access", Json::text(c.access.as_str())),
                ("parameters", Json::Array(parameters(c))),
            ])
        })
        .collect();
    Json::object([
        ("outcome", Json::text("commands_listed")),
        ("commands", Json::Array(commands)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::ffi::OsStr;

    #[test]
    fn names_are_unique_and_command_words() {
        let mut seen = BTreeSet::new();
        for c in COMMANDS {
            assert!(seen.insert(c.name), "duplicate command: {}", c.name);
            assert!(
                !c.name.is_empty() && c.name.bytes().all(|b| b.is_ascii_lowercase() || b == b'-'),
                "not a command word: {}",
                c.name
            );
        }
    }

    #[test]
    fn every_registered_word_has_its_route_and_others_have_none() {
        for c in COMMANDS {
            assert_eq!(route(OsStr::new(c.name)), Some(c.route), "{}", c.name);
        }
        assert_eq!(route(OsStr::new("no-such-command")), None);
        assert_eq!(route(OsStr::new("")), None);
    }

    #[test]
    fn parameters_are_well_formed() {
        for c in COMMANDS {
            let mut seen = BTreeSet::new();
            let identity = match c.ids {
                Ids::None => &[][..],
                Ids::Common => &COMMON_IDENTITY_OPTIONS[..],
            };
            for name in identity {
                assert!(seen.insert(*name));
            }
            for p in c.params {
                assert!(
                    p.name.len() > 2
                        && p.name.starts_with("--")
                        && p.name[2..]
                            .bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
                    "{}: not an option: {}",
                    c.name,
                    p.name
                );
                assert!(
                    seen.insert(p.name),
                    "{}: {} is listed twice or is a common identity option",
                    c.name,
                    p.name
                );
                assert!(
                    !p.value.contains(char::is_whitespace),
                    "{}: {} value has whitespace",
                    c.name,
                    p.name
                );
            }
        }
    }

    #[test]
    fn a_one_of_group_has_alternatives() {
        for c in COMMANDS {
            let groups: BTreeSet<u8> = c
                .params
                .iter()
                .filter_map(|p| match p.need {
                    Need::OneOf(g) => Some(g),
                    _ => None,
                })
                .collect();
            for g in groups {
                let members = c.params.iter().filter(|p| p.need == Need::OneOf(g)).count();
                assert!(members >= 2, "{}: group {g} has one member", c.name);
            }
        }
    }
}
