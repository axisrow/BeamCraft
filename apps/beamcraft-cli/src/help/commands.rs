// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the commands inventory.

pub(super) const HELP: &str = concat!(
    "Commands: commands (the registry of every command with its ",
    "family, access (read changes nothing persistent; write changes the store, settings, ",
    "a library or an output file) and parameters, each with its value shape, whether it is ",
    "required and its one_of group (exactly one member of a group is required); the common ",
    "identity options are listed on the commands that take them; a rule that links optional ",
    "parameters is in the family help; needs no store and takes no options; ",
    "read-only)",
);
