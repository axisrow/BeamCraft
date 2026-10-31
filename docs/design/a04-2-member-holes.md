# A04.2 member holes: engine and CLI

`AddHole`, `UpdateHole` and `RemoveHole` accept a member target (the F06 member-hole
amendment, accepted 2026-10-27): `add-hole` takes `--axis-point U,V,W --axis-direction
X,Y,Z --diameter D` plus an optional `--from F --to T` range, stored as an owned
`MemberHoleRecord` (operation id, per-member ordinal, member-frame axis point and
direction, extent, authoritative polygon with vertex 0 on `+Z_m`) on the member, replayed
from the base shape in ordinal order like the plate records. Plate holes keep their form;
the engine kind guards stay, and member records live in their own typed list on
`AuthoredMember`, so plate (A04) and member (A04.2) records never conflate under
`update-hole`/`remove-hole`; record identities remain one project-wide space. Storage is
the `steel:mhole:*` attribute set plus its own `steel:mhole:nextOrdinal`/
`retiredOperationIds` history (a third record family next to `steel:cut:*` and
`steel:hole:*`); the canonical instance group replays as one atomic unit, as for cuts.

Replay cuts each record with the kernel's certified through-hole operation, mapped into
the hole frame by an exact signed axis permutation (the member frame `u,v,w` of F06
section 1 to the hole frame x `+Z_m`, y `axis_direction x +Z_m`, z the direction), so an
axis-aligned straight rectangle-profile member is cut with bit-exact coordinates today:
geometry, volume and undo run end to end through the CLI, a hole that no longer fits
refuses with a typed diagnostic, and removal restores the gross volume.

Everything else is an explicit `UNSUPPORTED_GEOMETRY` refusal naming the kernel slice
G14.1 that lifts it: section profiles (I-beams, hollow walls), polybeam and curved paths,
a frame that is not axis-aligned (including effective roll), and ranged cutters. The
slice accepts only `through` round holes along the section directions `(+-1,0,0)` /
`(0,+-1,0)`; a polygon generator on a member, a direction with an axial component and a
member that cannot serve a stored record all refuse and change nothing.
