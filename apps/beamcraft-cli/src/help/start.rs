// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the start command family.

pub(super) const HELP: &str = concat!(
    "Start (projects; --config-dir DIR replaces the user configuration directory, ",
    "BEAMCRAFT_CONFIG_DIR or the platform default): create-project ",
    "(--parent DIR --name NAME [--unique-name true|false] | --here) [--environment ID] [--op-id UUID] [--revision-id UUID] [--actor-id UUID] ",
    "[--project-id UUID] [--workspace-id UUID] (makes the folder DIR/NAME, refused when it ",
    "exists, and commits an empty first revision; --here initializes the empty current directory ",
    "and uses its folder name; the author is --actor-id, else the settings ",
    "profile; the project is remembered as recent; --unique-name true reserves the first free ",
    "NAME, NAME 1, NAME 2, up to 10000 candidates; --environment ID copies that environment into ",
    "the project folder (environment/), so a later --profile NAME resolves against it without any ",
    "option and defines the profile in the model on first use (definitions lists the rest as ",
    "environment_profiles; environment_presets lists the part presets the environment declares, ",
    "the default name, class, material, profile or thickness of a new beam, column or plate; environment_view_filters and environment_view_presets list the view filters and view presets it declares, which a view can be filled from), and publishes the default materials Steel_Undefined (density 0, a known ",
    "zero mass for auxiliary parts) and Steel (7850 kg/m3), then the starter grids and levels its ",
    "template declares (bim:environment:template; blank: one 3 x 3 grid at 6000 and the level ",
    "+0.000), one revision each, counted in seeded.grids and seeded.levels; a template with any ",
    "refused entry refuses the creation; an unknown or unreadable environment is ",
    "refused and the new files are rolled back; without --environment the active environment of the ",
    "settings is seeded instead (the start window's choice; a sectioned project seeds nothing), and ",
    "seeded.environment_source says option when the command line chose and settings when the ",
    "settings did; a settings file that cannot be read refuses a creation that needs it)); ",
    "list-project --dir DIR (the store folders ",
    "directly inside DIR by name, read-only; created_ms is the ProjectId timestamp, ",
    "modified_ms the head revision id timestamp)",
);
