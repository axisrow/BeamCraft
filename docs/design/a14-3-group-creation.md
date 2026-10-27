# A14.3 fastener groups: creation (part 1)

Date: 2026-10-27. Status: **experimental slice; the A14 acceptance (holes in all parts,
conflicts as in A05) stays open.** `UpdateBoltGroup` is the named second part of this slice.
Branch `feat/a14-3-group-placement`, stacked on `feat/a14-2-fastener-geometry` (A14.2).

Plan row: A14.3 / M (slice of A14, issue axisrow/BeamCraft#194), contract
`docs/contracts/f04-bolt-fastener-amendment-draft-v0.1.md` sections 4 and 6. The slice grew
past the +2000-line PR rule, so it ships in two parts: this part is the group creation end to
end; the second part adds `UpdateBoltGroup` (the merged re-admission, `repin`, one undo unit per
edit) on top.

## What is implemented

```text
beamcraft create-bolt --store ROOT --name TEXT --parts ID1,ID2[,...]
                      --fastener DESIGNATION [--fastener-library FILE]
                      --point1 X,Y,Z --point2 X,Y,Z --plane-normal X,Y,Z
                      [--rotation DEGREES] [--on-plane MM] [--depth MM] [--offset-start MM]
                      (--spacings M1,M2,... | --count N --pitch M | --count N --offset-end M)
                      [--length-mode auto|fixed --fixed-length MM]
                      [--washer-under-head true|false] [--washer-under-nut true|false]
                      [--hole-mode generate|none] [--hole-clearance MM]
beamcraft bolt-groups --store ROOT
```

| Area | What exists |
|---|---|
| `beamcraft_model::command` | `CreateBoltGroup` (bolt amendment section 4): 2 to 1024 distinct parts, the pinned fastener digest, two points, the plane normal with a rotation in stored radians, `on_plane`/`depth`/`offset_start`, the spacings (the group size is `len + 1`, at most 512), `BoltLengthMode` auto/fixed, the washer switches defaulting from the definition, the hole mode and clearance. Admission with the F04 codes and a versioned content fingerprint. The A05 `CreateBolt` type stays for stored groups. |
| `beamcraft_engine::bolt_group` | The frame (`x` the unit line, `z` the plane normal's perpendicular component rotated about `x`, `y = z × x`), the fastener anchors at `offset_start + sum(spacings[..i])`, the slab test that every listed part is crossed by at least one shank (`GEO_OP_NO_EFFECT`), the washer resolution against the definition, the auto length (smallest standard length covering the conservative grip, the washers, the nut and the protrusion; largest grip decides) or the fixed length, and the footprint (`for_bolt_group`: the group's writes, each part's lifecycle read, the definition read — and written too when the adapter pins it in the same transaction). Conservative bounds, as A05. `hole_mode = generate` is refused until A14.4. |
| `beamcraft_layer_engine::authored` | `Shape::BoltGroup`: a `connection` of type `fastener_group` with `steel:boltGroup:*` attributes (line, plane, rotation, offsets, spacings, length mode and the resolved `lengthMm`, the washer switches, the hole mode and clearance), `bim:connects` over 2 to 1024 parts and `bim:fastener` pinning the definition. The structural read re-checks the bounds and tokens, the discriminator refuses mixed weld/bolt/group property sets, `connects()` admits 2 to 1024 distinct parts for a fastener group, and `link()` resolves `bim:fastener` against the fastener category (A14.1's `read_definition`). The mapping descriptor gains the group line. |
| `beamcraft_cli::bolt_group` | `create_bolt_group`: resolves the designation (from `--fastener-library`, pinned in the same transaction, or among the head's definitions as a member's profile is), runs the engine admission, authors the definition (when pinning) and the group in one revision — one undo unit — and verifies the observed writes against the footprint. Replay by `op_id` and content fingerprint. `bolt-groups` lists the stored groups with parts, digest, count and resolved length. |

The stored `lengthMm` is the resolved standard length: the fixed value or the auto choice, so a
reader knows the shank length without re-running the selection.

## Decisions

- **The slice splits rather than shrinks.** The storage surface (the connection discriminator,
  the `connects` bounds, the definition relationship, the index guards) is the cost of a second
  connection form; `UpdateBoltGroup` is a coherent second part on the same branch.
- **The auto length is conservative.** The grip is measured against the parts' world boxes, so a
  box may overcount an L-shaped part; the choice is documented in the module, as A05's reach.
- **`generate` is refused.** A14.4 turns it into operation records; until then the refusal keeps
  the group from writing parts it cannot regenerate.

## Limits

- `UpdateBoltGroup` (with `repin`) is the second part of this slice.
- Bolts through a rotated part need P10 (the amendment, section 8); the identity placement of
  the group element is unchanged from A05.
- Generated holes (A14.4) and member holes (G14) are refused with `UNSUPPORTED_GEOMETRY`.

## Tests

`crates/engine/src/bolt_group.rs`: the auto length covers grip + washer + nut + protrusion; the
fixed length must be standard; a parallel normal and an uncrossed part are refused; no standard
length long enough is `UNSUPPORTED_GEOMETRY`; generated holes are refused until A14.4; the
rotation turns the shank about the line; the offsets slide the row.
`apps/beamcraft-cli/tests/fastener_groups.rs` (end to end): the DIN entry pins in the group's
transaction (one revision after the plates, the digest the listing predicted), the auto length
is 45 mm for the case stack, the listing shows parts, count and mode; an unknown designation, an
uncrossed part, a generated hole and an impossible grip each change nothing; the same operation
replays.
