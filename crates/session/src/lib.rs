// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Platform state for the applications: what the command line and the window both need and that
//! is neither storage nor a discipline.
//!
//! - [`settings`]: the per-user settings file (profile, theme, models folder, recent list).
//! - [`project`]: project folders as people see them: names, listing, reading a head.
//! - [`environment`]: the environments a session can start with, and their admission.
//! - [`recent`]: the recent-project list over the settings and the project folders.
//! - [`pipeline`]: the layer names, the base a revision is authored on, the project claim and the
//!   revision summary every command shares.
//! - [`foreign`]: one read-only, self-contained USD layer from outside the project, opened under
//!   fixed budgets (a library or an environment file).
//!
//! The crate is discipline-agnostic: nothing here knows about steel, concrete or pipes. It sits
//! one layer above `beamcraft-host`, which stays disk-only, and both applications call it
//! in-process. Every function is bounded and returns a typed error instead of panicking.

pub mod environment;
pub mod foreign;
pub mod pipeline;
pub mod project;
pub mod recent;
mod refusal;
pub mod settings;

pub use refusal::Refusal;
