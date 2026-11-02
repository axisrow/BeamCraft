// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The BeamCraft application layer: the commands the apps run against a project.
//!
//! Each module turns one family of typed requests (create and edit plates, members, bolts and
//! welds; copies and arrays; definitions, grids and levels; assemblies, numbering, drawings,
//! schedules and exports; workspaces, history, publication, rebase and the coordinator) into
//! engine and discipline calls, layer edits and a host commit, and returns typed results or a
//! refusal. It parses no command line and prints nothing: the `beamcraft` CLI
//! (`apps/beamcraft-cli`) reads arguments, calls these modules and renders their results, and the
//! window can call the same modules. Only an app or a tool may depend on this crate (F08 kind
//! `service`). It was the library target of `beamcraft-cli` until #208 moved it here.

pub mod annotation;
pub mod array;
pub mod assembly;
pub mod attribute;
pub mod bend;
pub mod bolt;
pub mod bom;
pub mod changeset;
pub mod command_evidence;
pub mod construction;
pub mod coordination;
pub mod coordinator;
pub mod copy;
pub mod cut;
pub mod definition;
pub mod delete;
pub mod drawing;
pub mod edit;
pub mod export;
/// Shared rule parsing and the admitted head dataset of element filters (V14).
pub mod filters;
pub mod grid;
pub mod history;
pub mod history_compact;
pub mod hole;
pub mod level;
pub mod library;
pub mod loft;
pub mod member;
pub mod numbering;
pub mod numbering_approval;
pub mod plate;
pub mod plate_draft;
pub mod project;
mod publication;
pub mod reference;
mod reference_view;
pub mod registry;
pub mod scope;
pub mod source_file;
pub mod weld;
pub mod workspace;

/// Schedule template decoding and neutral rows (I05).
pub mod schedules;
