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

The replay cuts each record with the kernel's certified section-aware member-hole
operation (G14.1, with the directed cutters of G14.2), mapped into the hole frame by an
exact signed axis permutation, so a member whose frame axes are the coordinate axes is
cut with bit-exact coordinates. Straight members of every profile family are served —
the rectangle and the A07.1 sections, faceted, sloped and curved walls included — with
any section-plane hole direction and a `through` or a `range` extent (a range ending
inside a wall is the kernel's blind refusal). Geometry, volume and undo run end to end
through the CLI, a hole that no longer fits refuses with a typed diagnostic, and removal
restores the gross volume.

Everything else refuses with `UNSUPPORTED_GEOMETRY` and writes nothing: a curved member
or polybeam, a frame that is not the coordinate axes (a roll or a skew), a direction
with an axial component, a polygon generator on a member, and a record that does not
fit strictly inside a wall face, clear of its edges and the other holes. The replay
runs in ordinal order, so a refusal keeps the member's last valid state.
