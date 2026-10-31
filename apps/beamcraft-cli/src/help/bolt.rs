// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the bolt command family.

pub(super) const HELP: &str = concat!(
    "Bolt groups (experimental). create-bolt-group --store ROOT --name TEXT --parts ",
    "UUID,UUID[,UUID...] --fastener-library FILE --designation TEXT --point1 X,Y,Z --point2 ",
    "X,Y,Z --plane-normal X,Y,Z [--rotation-deg DEG] [--on-plane MM] [--depth MM] ",
    "--offset-start MM (--spacings a,b,c | --count N --pitch MM | --count N --offset-end MM) ",
    "[--washer-under-head true|false] [--washer-under-nut true|false] [--length-mode ",
    "auto|fixed --length MM] [--hole-mode generate|none] [--hole-clearance MM] [--new-id UUID] ",
    "(A14.3: one row of identical fasteners of the pinned definition between 2 to 1024 parts; ",
    "the definition is pinned in the same transaction; the lengths list of the definition ",
    "decides --length-mode auto. A14.4: --hole-mode generate (a bolt's default) writes one ",
    "hole record per crossing fastener on every connected plate, of diameter nominal + ",
    "--hole-clearance (the definition's clearance when absent); a beam or column part is ",
    "refused; --hole-mode none (a screw's default) writes no part). update-bolt-group --store ",
    "ROOT --target GROUP_UUID with any of the create-bolt-group inputs (absent keeps the ",
    "stored value), or --repin true with --fastener-library FILE --designation TEXT to follow ",
    "the current library; one atomic edit and one undo unit; the holes are regenerated; ",
    "upgrades an A05 group. The A05 create-bolt token is retired: stored annotation groups ",
    "stay readable and upgradable. bolts --store ROOT (verified head: A05 groups as ",
    "annotations and A14.3 groups with their records). delete-element refuses a bolted part ",
    "while a group connects it and deletes a group with its generated holes. ",
    "fastener-geometry --fastener-library FILE --designation TEXT --length MM [--count N] ",
    "[--density KG_M3] [--washer-under-head true|false] [--washer-under-nut true|false] ",
    "(A14.2: one prototype of the definition as separate closed meshes with per-component ",
    "counts, volumes and the mass with its source; reads the library, never a store)",
);
