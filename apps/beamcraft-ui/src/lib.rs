// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! V03: the native BeamCraft window and viewport.
//!
//! V05.1 adds create tools (a plate, a straight member): the window is a command client that
//! runs the CLI's `create-plate` / `create-member` in a separate process and then reloads the
//! accepted revision; see [`tools`] (pure parameter mapping and reply parsing) and [`command`].
//!
//! V06.1 adds a properties panel for the selected plate or straight member (what the engine
//! reports, labelled current, stale or unknown; edits are the CLI's `update-element`) and a
//! Publish action (`publish-workspace`); see [`props`] and [`publish`].
//!
//! A launch without arguments opens a per-user default workspace created on demand
//! ([`source::default_workspace_at`]), so a double-clicked app bundle launches.
//!
//! The viewport shows the accepted (published) revision of a store. Its input is the engine's
//! pinned view snapshot, produced by the headless CLI (`beamcraft export-view --out -`) in a
//! separate process and rebuilt here with [`beamcraft_engine::ViewSnapshot::from_bundle`], which
//! re-validates every blob, digest and the token. The renderer computes no engineering geometry:
//! it decodes the engine's element-local meshes, derives display-only flat normals, and draws
//! each distinct mesh once with one instance per element placement.
//!
//! Units: the model is in millimetres (right-handed, Z up). The scene is re-centred on the
//! engineering bounds in f64 before conversion to f32, so large world coordinates keep their
//! precision on the GPU.

pub mod app;
pub mod appearance;
pub mod camera;
pub mod colour_sets;
pub mod command;
pub mod creation;
pub mod cut_plane;
pub mod dialog;
pub mod edit;
pub mod grid;
pub mod metrics;
pub mod perf;
pub mod preview;
pub mod props;
pub mod publish;
pub mod render;
pub mod scene;
pub mod selection;
pub mod sheet;
pub mod shell;
pub mod shortcuts;
pub mod source;
pub mod start;
pub mod stats;
pub mod theme;
pub mod tools;
pub mod views;
pub mod work_plane;

/// A bounded viewport failure; adapter absence is reported, never replaced by a fake result.
#[derive(Debug)]
pub enum Error {
    /// A caller argument or camera input is out of range or not finite.
    Input(&'static str),
    /// The engine refused the snapshot or one of its meshes.
    View(beamcraft_engine::ViewError),
    /// The CLI process failed or answered with a rejection.
    Cli(String),
    /// The store has no published revision yet: a normal empty state, not a failure.
    NoRevision,
    /// No native graphics adapter exists.
    NoAdapter,
    /// Graphics device, validation, mapping or completion failure.
    Graphics(String),
    /// A bounded CPU allocation could not be reserved.
    Allocation,
    /// A file or process operation failed.
    Io(std::io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Input(what) => write!(f, "invalid input: {what}"),
            Self::View(e) => write!(f, "view snapshot refused: {}", e.code().as_str()),
            Self::Cli(message) => write!(f, "beamcraft CLI: {message}"),
            Self::NoRevision => f.write_str("the store has no revision yet"),
            Self::NoAdapter => f.write_str("no native graphics adapter"),
            Self::Graphics(message) => write!(f, "graphics: {message}"),
            Self::Allocation => f.write_str("allocation refused"),
            Self::Io(e) => write!(f, "I/O: {}", e.kind()),
        }
    }
}

impl std::error::Error for Error {}

impl From<beamcraft_engine::ViewError> for Error {
    fn from(error: beamcraft_engine::ViewError) -> Self {
        Self::View(error)
    }
}

/// Fallibly parses face 0 with the parser epaint uses at font init, which panics on failure.
pub fn check_font(bytes: &[u8]) -> Result<(), Error> {
    skrifa::FontRef::from_index(bytes, 0)
        .map(|_| ())
        .map_err(|_| Error::Input("font file does not parse"))
}

/// JSON string escaping without a serialization dependency.
pub fn json_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
