// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Native services above the portable BeamCraft domain crates.

pub mod changeset;
pub mod compaction;
pub mod control;
pub mod coordinator;
pub mod ids;
pub mod lease;
pub mod ledger;
pub mod output;
pub mod revision;
pub mod storage;
pub mod views;
/// The W01 views layer on disk: the camera-prim form of the view book the CLI and the window
/// persist beside a store.
pub mod views_layer;
