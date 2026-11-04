// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Compose feature-owned help without changing command dispatch or parsing. The command
//! inventory is rendered from the registry.

use beamcraft_commands::registry;

mod annotation;
mod assembly;
mod attribute;
mod blobs;
mod bolt;
mod bom;
mod changeset;
mod commands;
mod construction;
mod coordination;
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
mod session;
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
    coordination::HELP,
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
    session::HELP,
];

/// Write the same help for no arguments, `--help` and `-h`.
pub(crate) fn print() {
    println!("{}", render());
}

/// The sections that document one registry family, for a `command --help`; the whole
/// book when the family has no sections of its own.
pub(crate) fn for_family(family: &str) -> String {
    let sections: &[&str] = match family {
        "annotation" => &[annotation::HELP],
        "assembly" => &[assembly::HELP],
        "attribute" => &[attribute::HELP],
        "blobs" => &[blobs::HELP],
        "bolt" => &[bolt::HELP],
        "bom" => &[bom::HELP],
        "changeset" => &[changeset::HELP],
        "commands" => &[commands::HELP],
        "construction" => &[construction::HELP],
        "coordination" => &[coordination::HELP],
        "coordinator" => &[coordinator::HELP],
        "copy" => &[copy::HELP],
        "definitions" => &[definitions::HELP],
        "delete" => &[delete::HELP],
        "drawing" => &[drawing::PREVIEW, drawing::DOCUMENTS],
        "edit" => &[edit::HELP],
        "export" => &[export::HELP],
        "filters" => &[filters::HELP],
        "grid" => &[grid::LINES, grid::UPDATE, grid::QUERIES],
        "history" => &[history::HELP],
        "level" => &[level::HELP],
        "library" => &[library::HELP],
        "member" => &[member::HELP],
        "numbering" => &[numbering::PREVIEW, numbering::APPROVAL],
        "pick" => &[pick::HELP],
        "plate" => &[plate::HELP],
        "rebase" => &[rebase::HELP],
        "reference" => &[reference::HELP],
        "revision" => &[revision::CONTROL, revision::HELP, revision::RECOVERY],
        "schedules" => &[schedules::HELP],
        "session" => &[session::HELP],
        "start" => &[start::HELP],
        "store" => &[store::HELP],
        "view" => &[view::HELP],
        "view_export" => &[view_export::HELP],
        "views" => &[views::HELP],
        "weld" => &[weld::HELP],
        "workspace" => &[workspace::HELP, workspace::UNDO],
        "work_plane" => &[work_plane::HELP],
        _ => &[],
    };
    if sections.is_empty() {
        return render();
    }
    sections.join("\n")
}

fn render() -> String {
    let commands: Vec<_> = registry::COMMANDS.iter().map(|c| c.name).collect();
    format!(
        "BeamCraft: {}\n{}",
        commands.join(" | "),
        SECTIONS.join("\n")
    )
}
