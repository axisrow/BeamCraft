// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! BeamCraft command-line interface library: the P01 plate adapter and the P02 definition adapter
//! shared by the `beamcraft` binary and its tests. Nothing depends on this package; it is the
//! application layer.

pub mod assembly;
pub mod changeset;
pub mod coordinator;
pub mod copy;
pub mod definition;
pub mod delete;
pub mod edit;
pub mod export;
pub mod history;
pub mod level;
pub mod library;
pub mod member;
pub mod numbering;
pub mod plate;
pub mod reference;
pub mod source_file;
pub mod weld;
pub mod workspace;
