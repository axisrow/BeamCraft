// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Every real-binary integration suite of the CLI, linked into one test binary.
//!
//! Each suite keeps its own file and becomes a module of the same name, so a suite runs with
//! `cargo test -p beamcraft-cli --test cli -- <suite>::` and a single case with
//! `cargo test -p beamcraft-cli --test cli -- --exact <suite>::<case>`. A new suite file is
//! listed here; `macos_icon` checks the desktop app's icon and stays its own target.

mod support;

mod a13;
mod add_hole;
mod alpha_scenario;
mod assemblies;
mod bolts;
mod bom;
mod changeset;
mod changeset_base;
mod command_evidence;
mod contour_plates;
mod coordinator;
mod copy;
mod curved_members;
mod cut_removals;
mod cut_updates;
mod cuts;
mod definitions;
mod drawing_codec;
mod environments;
mod export_ifc;
mod f04_cases;
mod f06_cases;
mod f06_member_cases;
mod f10_vocabulary;
mod fastener_libraries;
mod grid_geometry;
mod grid_lines;
mod grid_queries;
mod grid_updates;
mod grids;
mod help;
mod history_compact;
mod hole_removals;
mod hole_updates;
mod holes;
mod hostile_usd;
mod independent_reader;
mod inspection_cache;
mod instance_group;
mod levels;
mod libraries;
mod members;
mod numbering;
mod numbering_approval;
mod offline_clients;
mod part_classes;
mod pick;
mod plate;
mod plate_corners;
mod plate_placement;
mod pmass;
mod polybeam;
mod prim_naming;
mod project_claim;
mod publication_evidence;
mod publication_guard;
mod rebase;
mod rebase_receipt;
mod recovery;
mod reference;
mod reference_measure;
mod reference_reader;
mod relocate;
mod resolve;
mod revision;
mod sections;
mod start;
mod undo;
mod update;
mod version;
mod view_export;
mod view_snapshot;
mod welds;
mod work_planes;
mod workspace;
mod workspace_state;
