// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the start command family.

pub(super) const HELP: &str = concat!(
    "Start (projects; --config-dir DIR replaces the user configuration directory, ",
    "BEAMCRAFT_CONFIG_DIR or the platform default): create-project ",
    "--parent DIR --name NAME [--unique-name true|false] [--environment ID] [--op-id UUID] [--revision-id UUID] [--actor-id UUID] ",
    "[--project-id UUID] [--workspace-id UUID] (makes the folder DIR/NAME, refused when it ",
    "exists, and commits an empty first revision; the author is --actor-id, else the settings ",
    "profile; the project is remembered as recent; --unique-name true reserves the first free ",
    "NAME, NAME 1, NAME 2, up to 10000 candidates; --environment ID copies that environment into ",
    "the project folder (environment/), so a later --profile NAME resolves against it without any ",
    "option and defines the profile in the model on first use (definitions lists the rest as ",
    "environment_profiles), and publishes the default materials Steel_Undefined (density 0, a known ",
    "zero mass for auxiliary parts) and Steel (7850 kg/m3); an unknown or unreadable environment is ",
    "refused and the folder is removed again); list-project --dir DIR (the store folders ",
    "directly inside DIR by name, read-only; created_ms is the ProjectId timestamp, ",
    "modified_ms the head revision id timestamp)",
);
