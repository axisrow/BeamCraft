// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the bolt command family.

pub(super) const HELP: &str = concat!(
    "Bolt (A05, experimental, a connection element with no body): create-bolt --store ROOT ",
    "--name TEXT --main PART_UUID --secondary PART_UUID --origin X,Y,Z --direction X,Y,Z ",
    "--count N --pitch MM --diameter MM [--new-id UUID] [--op-id UUID] [--revision-id UUID] ",
    "[--actor-id UUID] [--project-id UUID] [--workspace-id UUID] (a bolt group of --count ",
    "bolts in a straight world line from --origin along the unit --direction with centre ",
    "distance --pitch; authors the group's properties and its one bim:connects record towards ",
    "two distinct plates, beams or columns, main part first; the parts are only read; every ",
    "bolt position must lie within both parts' bounds; no default for any property); bolts ",
    "--store ROOT (verified head: bolt groups, parts, properties). delete-element refuses a ",
    "bolted part while the group connects it and deletes a bolt group. fastener-geometry ",
    "--fastener-library FILE --designation TEXT --length MM [--count N] [--density KG_M3] ",
    "[--washer-under-head true|false] [--washer-under-nut true|false] (A14.2: one prototype of ",
    "the definition as separate closed meshes — head, shank, optional nut and washers, round ",
    "parts to the 0.05 mm chord rule — with per-component counts, volumes and the mass with ",
    "its source declared or computed; reads the library, never a store)",
);
