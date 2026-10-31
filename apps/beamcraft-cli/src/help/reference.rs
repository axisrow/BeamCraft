// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the reference command family.

pub(super) const HELP: &str = concat!(
    "Reference (R01, a foreign USD file attached read-only; its bytes are stored under their ",
    "SHA-256 and never rewritten): attach-reference --store ROOT --source FILE [--name TEXT] ",
    "[--translation X,Y,Z] [--rotation-z-deg DEG] [--new-id UUID] [--op-id UUID] ",
    "[--revision-id UUID] [--actor-id UUID] [--project-id UUID] [--workspace-id UUID] (FILE ",
    "is a local USDA or USDC layer of at most 33554432 bytes (32 MiB), within the ",
    "model-profile stage budgets, with a defaultPrim; values and arcs it uses that cannot be ",
    "held, other assets included, are left out and never resolved; a URL or network path as ",
    "FILE is PERMISSION_DENIED before anything is read; the ",
    "placement belongs to the wrapper and is stored apart from the foreign content); ",
    "move-reference --store ROOT --id UUID [--translation X,Y,Z] [--rotation-z-deg DEG] [same ",
    "ids] (absolute placement of the wrapper only); correct-reference-units --store ROOT --id ",
    "UUID (--meters-per-unit F --up-axis Y|Z | --clear true) [same ids] (the explicit units ",
    "and up-axis correction of the wrapper; the source declares what it declares and is never ",
    "edited, and without a correction it is taken as stored); write-reference --store ROOT ",
    "--id UUID (--target source | --prim /DefaultPrim/Path [--attribute NAME]) (always ",
    "FOREIGN_READ_ONLY: the source layer and every foreign prim are read-only, including ",
    "overrides in owned layers); inspect-reference --store ROOT (verified head: wrappers, ",
    "pinned source, declared and explicit units, units status, foreign prims, external layers ",
    "with their digests); measure-reference --store ROOT [--id UUID] (R02, experimental: the ",
    "kernel measurement of every wrapper's composed foreign surface, or of --id alone; the ",
    "explicit units correction and the full placement are applied; volume, area and centroid ",
    "only for a kernel-certified closed shell, as outward-rounded enclosures; an open, ",
    "self-intersecting, inward, degenerate or unsupported surface reports its status without ",
    "values, and mass is never invented; read-only: no revision, identity or byte is written, ",
    "and the same head always answers with the same numbers)",
);
