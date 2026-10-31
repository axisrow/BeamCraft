// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the F12 coordination command family.

pub(super) const HELP: &str = concat!(
    "Coordination (F12, project descriptor and pinned connections): set-project-metadata ",
    "--store ROOT [--code CODE] [--name TEXT] [--organization TEXT] (the explicit shared ",
    "command for human project metadata; the first set names the code and name pair); ",
    "define-section --store ROOT --code CODE --module TOKENS [--section-id UUID] [--label ",
    "TEXT] and define-zone --store ROOT --code CODE [--zone-id UUID] [--label TEXT] ",
    "(registries of the F10 0.2 record shapes, keyed by stable identity; a repeated id ",
    "renames its record, a repeated code is DUPLICATE_ID, labels grant no authority); ",
    "show-project --store ROOT (the descriptor, both registries and every connection with its ",
    "resolved pins); connect-project --store ROOT --source-store ROOT --source-project UUID ",
    "--source-revision UUID --label TEXT [--scope whole|CODE] [--connection-id UUID] ",
    "[--translation X,Y,Z --orientation X,Y,Z,W] (pins one exact source revision; the source ",
    "is verified completely and its layers are copied into this store as external entries, so ",
    "the coordination revision reopens without the source; a text such as `latest` is not an ",
    "identity and is refused); update-connection --store ROOT --connection UUID ",
    "--expected-revision UUID --source-store ROOT --source-project UUID --source-revision UUID ",
    "[--scope|--label|--translation|--orientation] (the explicit update: a stale expected pin ",
    "is DEPENDENCY_CONFLICT, a moved head is BASE_REVISION_MISMATCH, a retry of the same ",
    "operation replays); remove-connection --store ROOT --connection UUID (older retained ",
    "revisions keep their pins); resolve-connection --store ROOT --connection UUID (pinned ",
    "source placements mapped by the connection's rigid transform into this frame; a missing ",
    "placement is an explicit unknown alignment, DEPENDENCY_UNAVAILABLE)"
);
