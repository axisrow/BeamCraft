// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Compose feature-owned help without changing command dispatch or parsing.

mod assembly;
mod blobs;
mod bolt;
mod bom;
mod changeset;
mod coordinator;
mod copy;
mod definitions;
mod delete;
mod drawing;
mod edit;
mod export;
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
mod start;
mod store;
mod view;
mod view_export;
mod weld;
mod workspace;

// Keep the established inventory order independently of the detailed section order.
const COMMAND_GROUPS: &[&[&str]] = &[
    blobs::COMMANDS,
    revision::COMMANDS,
    plate::COMMANDS,
    definitions::COMMANDS,
    library::COMMANDS,
    member::COMMANDS,
    edit::COMMANDS,
    delete::COMMANDS,
    copy::COMMANDS,
    workspace::COMMANDS,
    rebase::COMMANDS,
    reference::COMMANDS,
    assembly::COMMANDS,
    weld::COMMANDS,
    bolt::COMMANDS,
    level::COMMANDS,
    grid::COMMANDS,
    export::COMMANDS,
    numbering::COMMANDS,
    changeset::COMMANDS,
    coordinator::COMMANDS,
    history::COMMANDS,
    view::COMMANDS,
    pick::COMMANDS,
    view_export::COMMANDS,
    bom::COMMANDS,
    start::COMMANDS,
];

const SECTIONS: &[&str] = &[
    start::HELP,
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
    edit::HELP,
    delete::HELP,
    copy::HELP,
    assembly::HELP,
    weld::HELP,
    bolt::HELP,
    level::HELP,
    grid::HELP,
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
    pick::HELP,
    view_export::HELP,
    bom::HELP,
    store::HELP,
    drawing::DOCUMENTS,
];

/// Write the same help for no arguments, `--help` and `-h`.
pub(crate) fn print() {
    println!("{}", render());
}

fn render() -> String {
    let commands: Vec<_> = COMMAND_GROUPS
        .iter()
        .flat_map(|group| group.iter().copied())
        .collect();
    format!(
        "BeamCraft: {}\n{}",
        commands.join(" | "),
        SECTIONS.join("\n")
    )
}
