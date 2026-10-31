// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Compose feature-owned help without changing command dispatch or parsing. The command
//! inventory is rendered from the registry.

use beamcraft_cli::registry;

mod annotation;
mod assembly;
mod attribute;
mod blobs;
mod bolt;
mod bom;
mod changeset;
mod commands;
mod construction;
mod coordinator;
mod copy;
mod definitions;
mod delete;
mod drawing;
mod edit;
mod export;
mod filters;
mod grid;
mod history;
mod level;
mod library;
mod member;
mod numbering;
mod pick;
mod plate;
mod rebase;
mod reference;
mod revision;
mod schedules;
mod start;
mod store;
mod view;
mod view_export;
mod views;
mod weld;
mod work_plane;
mod workspace;

const SECTIONS: &[&str] = &[
    start::HELP,
    commands::HELP,
    blobs::HELP,
    drawing::PREVIEW,
    revision::CONTROL,
    revision::HELP,
    revision::RECOVERY,
    history::HELP,
    plate::HELP,
    definitions::HELP,
    library::HELP,
    member::HELP,
    work_plane::HELP,
    edit::HELP,
    delete::HELP,
    copy::HELP,
    assembly::HELP,
    weld::HELP,
    bolt::HELP,
    level::HELP,
    attribute::HELP,
    filters::HELP,
    grid::HELP,
    construction::HELP,
    annotation::HELP,
    grid::LINES,
    grid::UPDATE,
    grid::QUERIES,
    export::HELP,
    numbering::PREVIEW,
    numbering::APPROVAL,
    workspace::HELP,
    changeset::HELP,
    coordinator::HELP,
    workspace::UNDO,
    rebase::HELP,
    reference::HELP,
    view::HELP,
    views::HELP,
    pick::HELP,
    view_export::HELP,
    bom::HELP,
    schedules::HELP,
    store::HELP,
    drawing::DOCUMENTS,
];

/// Write the same help for no arguments, `--help` and `-h`.
pub(crate) fn print() {
    println!("{}", render());
}

fn render() -> String {
    let commands: Vec<_> = registry::COMMANDS.iter().map(|c| c.name).collect();
    format!(
        "BeamCraft: {}\n{}",
        commands.join(" | "),
        SECTIONS.join("\n")
    )
}
