// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! BeamCraft command-line interface library: the P01 plate adapter and the P02 definition adapter
//! shared by the `beamcraft` binary and its tests. Nothing depends on this package; it is the
//! application layer.

pub mod assembly;
pub mod bolt;
pub mod bom;
pub mod changeset;
pub mod command_evidence;
pub mod coordinator;
pub mod copy;
pub mod cut;
pub mod definition;
pub mod delete;
pub mod drawing;
pub mod edit;
pub mod export;
pub mod grid;
pub mod history;
pub mod history_compact;
pub mod hole;
pub mod level;
pub mod library;
pub mod member;
pub mod numbering;
pub mod numbering_approval;
pub mod plate;
pub mod plate_draft;
pub mod project;
mod publication;
pub mod reference;
mod reference_view;
pub mod source_file;
pub mod weld;
pub mod workspace;
