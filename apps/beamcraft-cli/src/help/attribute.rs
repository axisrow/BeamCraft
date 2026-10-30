// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the attribute command family (I04).

pub(super) const COMMANDS: &[&str] = &[
    "attribute-define",
    "attribute-undefine",
    "attribute-set",
    "attribute-clear",
    "attribute-get",
    "attributes",
];

pub(super) const HELP: &str = concat!(
    "Attributes (I04, user attributes): attribute-define --store ROOT --name NAME --type ",
    "text|number|integer|boolean|choice [--unit TEXT] [--options a,b,c] [--set NAME] ",
    "(a name is 1 to 64 ASCII letters, digits or underscores; a choice needs 1 to 64 unique ",
    "options; only a choice carries options; --set groups the definition); attribute-undefine ",
    "--store ROOT --name NAME (keeps every stored value: they become unknown attributes, ",
    "shown, never dropped); attribute-set --store ROOT --element UUID --name NAME --value V ",
    "(the value is parsed and checked against the definition; any element kind carries ",
    "values); attribute-clear --store ROOT --element UUID --name NAME (unknown attributes ",
    "clear by name too); attribute-get --store ROOT --element UUID (defined values with their ",
    "type and unit, then unknown ones marked defined=false); attributes --store ROOT (the ",
    "definitions with their sets); copies do not carry values yet"
);
