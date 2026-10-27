// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the start command family.

pub(super) const COMMANDS: &[&str] = &[
    "show-settings",
    "set-settings",
    "list-environment",
    "create-project",
    "suggest-project-name",
    "list-project",
    "list-recent",
    "add-recent",
];

pub(super) const HELP: &str = concat!(
    "Start (settings of this user, environments and projects; --config-dir DIR replaces the ",
    "user configuration directory, BEAMCRAFT_CONFIG_DIR or the platform default): ",
    "show-settings; set-settings [--name TEXT --email TEXT] [--environment ID|none] ",
    "[--skip-launcher true|false] [--models-dir DIR|none] (the first profile gets an author ",
    "UUIDv7 that later changes keep); list-environment (environments/<name>/environment.usda ",
    "next to the program, then in the configuration directory; the first of an id wins, later ",
    "ones are shadowed, unreadable ones are listed with their refusal); create-project ",
    "--parent DIR --name NAME [--unique-name true|false] [--op-id UUID] [--revision-id UUID] [--actor-id UUID] ",
    "[--project-id UUID] [--workspace-id UUID] (makes the folder DIR/NAME, refused when it ",
    "exists, and commits an empty first revision; the author is --actor-id, else the settings ",
    "profile; the project is remembered as recent; --unique-name true reserves the first free ",
    "NAME, NAME 1, NAME 2, up to 10000 candidates); suggest-project-name --parent DIR --name NAME ",
    "(read-only name suggestion); list-project --dir DIR (the store folders ",
    "directly inside DIR by name, read-only; created_ms is the ProjectId timestamp, ",
    "modified_ms the head revision id timestamp); list-recent (removes confirmed missing ",
    "project folders from the saved recent list; unreadable stores remain listed); add-recent --store DIR",
);
