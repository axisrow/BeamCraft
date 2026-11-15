// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help for the Q03 recorded-input commands.

pub(super) const HELP: &str = "Recorded inputs (Q03): record-create --store ROOT --name TEXT --kind joint_intent --scope UUID,UUID,... --connection UUID:weld:UUID (repeated per declared connection, the kind is weld or contact) --new-id UUID and the usual command-context identity options declares the joint intent: the scope names the joint's elements verbatim, and a declared connection is judged by the connectivity rules of check-model. record-create --kind gap_requirement --gap-a UUID --gap-b UUID --gap-min MM --gap-max MM --gap-tolerance MM records the permitted local gap of one declared weld connection; with no recorded gap requirement a declared weld connection's local gap stays unknown (no_supported_rule). Declared ids are stored verbatim and may point at elements that stop existing: the check reports target_unresolved, nothing auto-retargets. record-list --store ROOT returns the records with their values. delete-element removes one.";
