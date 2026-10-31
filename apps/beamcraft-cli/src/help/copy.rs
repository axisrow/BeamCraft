// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Help owned by the copy command family.

pub(super) const HELP: &str = concat!(
    "Copy (P05): copy-element --store ROOT --id UUID [--name TEXT] [--offset X,Y,Z] [--new-id ",
    "UUID] [--op-id UUID] [--revision-id UUID] [--actor-id UUID] [--project-id UUID] ",
    "[--workspace-id UUID] (new element with the source's authored inputs, the same body and ",
    "mesh digest; the source is only read; editing the copy later never changes the source; ",
    "--copy-from X,Y,Z --copy-to X,Y,Z is the two-point form of the offset)\n",
    "Instance (P11): copy-instance takes the same part options as copy-element. Shape, profile, ",
    "material and operations share the canonical source definition; edits affect the whole group. ",
    "Identity, placement, name, level and other relationships stay per instance. Deleting a ",
    "definition owner with linked instances is refused; delete its instances first.\n",
    "Arrays (T03): array-linear --store ROOT --id UUID --count N --direction X,Y,Z --spacing MM ",
    "[--name TEXT] and array-polar --store ROOT --id UUID --count N --point X,Y,Z --axis X,Y,Z --angle RAD ",
    "(N independent copies of the source at the stepped placements, created in one revision so ",
    "one undo step takes every copy back; each copy mints fresh identities and operation records ",
    "and keeps the source's level; the source is only read)\n",
);
