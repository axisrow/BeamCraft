// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the bolt command family.

pub(super) const COMMANDS: &[&str] = &["create-bolt", "bolts", "bolt-groups"];

pub(super) const HELP: &str = concat!(
    "Bolt groups (A14.3, experimental, a connection element with no body): create-bolt --store ",
    "ROOT --name TEXT --parts ID1,ID2[,...] --fastener DESIGNATION [--fastener-library FILE] ",
    "--point1 X,Y,Z --point2 X,Y,Z --plane-normal X,Y,Z [--rotation DEGREES] [--on-plane MM] ",
    "[--depth MM] [--offset-start MM] (--spacings M1,M2,... | --count N --pitch M | --count N ",
    "--offset-end M) [--length-mode auto|fixed --fixed-length MM] [--washer-under-head true|false] ",
    "[--washer-under-nut true|false] [--hole-mode generate|none] [--hole-clearance MM] ",
    "[--new-id UUID] [--op-id UUID] [--revision-id UUID] [--actor-id UUID] [--project-id UUID] ",
    "[--workspace-id UUID]: one straight row of fasteners of one pinned definition (the bolt ",
    "amendment). The line runs from --point1 towards --point2; the plane normal's component ",
    "perpendicular to the line is the shank axis at zero rotation; the shank runs from the ",
    "bearing face towards the nut. With --fastener-library the resolved entry is pinned in the ",
    "same transaction (one undo unit); without it the designation resolves among the head's ",
    "pinned definitions. The length is the smallest standard length covering the grip, the ",
    "washers, the nut and the protrusion (auto), or --fixed-length; every listed part must be ",
    "crossed by at least one shank. hole_mode generate cuts holes (A14.4, refused yet); none ",
    "writes no part. bolt-groups --store ROOT (parts, digest, count, resolved ",
    "length, mode); bolts --store ROOT lists the A05 bolt groups stored earlier, readable as ",
    "annotations and upgraded by a later update that supplies a fastener. delete-element ",
    "refuses a connected part while a group connects it and deletes a group",
);
