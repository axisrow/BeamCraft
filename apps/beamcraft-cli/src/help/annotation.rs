// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help for the D06 annotation commands.

pub(super) const HELP: &str = "Annotations (D06): annotation-create --store ROOT --name TEXT --kind dimension|mark|leader --new-id UUID and the usual command-context identity options. A dimension takes --from and --to, two qualified feature ids (as `pick` reports them); a mark takes --anchor and --attribute (the anchored element's user attribute whose value is the text); a leader takes --anchor, --text and --end X,Y,Z. Anchors are stored verbatim and are never re-numbered or re-bound: a lost feature stays unresolved. annotation-list --store ROOT returns the annotations with their binding state: current (every anchor resolves at the placed generation), stale (the model changed since placement; the address may name a different feature) or unresolved (element-missing, feature-missing or malformed-address). delete-element removes one.";
