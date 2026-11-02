# A14.1 fastener definitions and libraries

Date: 2026-10-27. Status: **experimental slice; A14 acceptance (holes in all parts, conflicts as
in A05) stays open and is claimed by A14.3 and A14.4.**
Branch `feat/a14-bolt-groups`, based on the master of 2026-10-27.

Plan row: A14.1 / M (slice of A14, issue axisrow/BeamCraft#194), contract
`docs/contracts/f04-bolt-fastener-amendment-draft-v0.1.md` section 2. Depends on A07 (the library
format and the digest pinning) and nothing else; no environments (decided O-B2).

## What is implemented

A fastener definition is the data of one size of one standard: head, optional nut and washer,
standard hole clearance and standard lengths, in millimetres. It follows the profile mechanism of
A07 exactly:

```text
beamcraft list-library  --fastener-library FILE [--filter TEXT] [--offset N --limit N]
beamcraft show-library  --fastener-library FILE --designation TEXT
beamcraft define-fastener --store ROOT --fastener-library FILE --designation TEXT [--name TEXT]
beamcraft definitions --store ROOT
```

| Area | What exists |
|---|---|
| `beamcraft_model::fastener` | `FastenerSpec` (bolt amendment section 2): `kind` (`bolt`/`screw`), `head_kind` (`hex`), nominal diameter, head across flats and height, optional nut across flats and height, optional `WasherSpec` (outer, inner, thickness), `hole_clearance_mm` (zero is a fitted hole), strictly ascending `lengths_mm`, `thread_protrusion_mm`, optional declared `mass_each_kg`, the `washer_under_head`/`washer_under_nut`/`hole_mode` defaults, and the `designation`/`standard`/`grade`/`display_name` labels. Admission with the F04 codes: finiteness, the dimension cap, the geometric floor, then the definition's own rules — a bolt carries a nut, a screw none; a washer switch is only true for a washer the definition carries (under a nut only for a bolt); head and nut clear the shank; the washer hole passes the shank and stays inside the washer; lengths ascend strictly; nothing is inferred from a designation. |
| `beamcraft_layer_engine::authored` | `Definition::Fastener { spec }`: a definition of kind `fastener` at `/World/Definitions/Fasteners/FST_<sha256>` with `steel:fastener:*` attributes, canonical bytes and digest like every pinned definition. `read_definition` re-reads and `Resource::new` re-admits, so a committed fastener definition is verified like a profile. The mapping descriptor gains the fastener line; pre-alpha stores are not migrated (the standing rule). |
| `beamcraft_commands::library` | `load_fastener`/`load_fastener_bytes`: one fastener library file in the A07 format — same layer budgets, same `customLayerData` identity (`bim:library:id`, optional `title`/`version`/`source`/`licence`), entries under `/Library/Fasteners` carrying `steel:fastener:*` — sharing the profile loader's opening, admission, refused-entry reporting and `Library`/`Lookup` types. A refused entry is reported and skipped; a duplicate designation refuses the whole library; an unknown designation is `NOT_FOUND`, never a near match. `add-library` refuses fastener records: a fastener library is shipped or maintained by hand, and the shipped files are the provenance record of their values. |
| `beamcraft_cli` | `define-fastener`: resolves exactly one entry of `--fastener-library` and publishes it as an immutable definition (the `define` adapter operation, one revision, idempotent — the same content is `definition_exists`). The definition stores every value itself, so the model never needs the library file again; changing or deleting a library changes no pinned definition. `DefinitionRecord::Fastener` lists, decodes and re-validates with the other definitions. |

## Shipped libraries

- `assets/libraries/synthetic-fasteners.usda` — three contributor-original synthetic entries
  (`SYN-BOLT-12x50`, `SYN-BOLT-16x70-BARE`, `SYN-SCREW-10x30`) covering a washered bolt, a bare
  bolt and a screw with `hole_mode = none`; every value invented.
- `assets/libraries/din-fasteners.usda` — the one European DIN set of the amendment: `M12`,
  `M16`, `M20` hex bolts with their nuts (DIN EN ISO 4032 dimensions) and washers (DIN EN ISO
  7089), clearances from the EN 1090-2 standard hole series (13/18/22 mm holes), entered as our
  own data. Which standards make the first set was open (O-B2a); this set is recorded in
  `ATTRIBUTION.md` and can be replaced without touching a mechanism.

## Decisions

- **Pinning is the P02 adapter operation, not a new F04 command.** As `define-profile` did for
  profiles, `define-fastener` publishes the resolved entry; A14.3's `CreateBoltGroup` then
  resolves by designation and pins in its transaction (the amendment's command table), and can
  reuse the same `DefinitionRecord::Fastener`.
- **No hand-authored fastener values on the command line.** A definition has fifteen values and
  the amendment names the library as their source; an explicit-value form would duplicate the
  library format for no caller. Adding one later is a new command surface, not a change.
- **`add-library` does not write fastener entries.** The writer re-renders a whole library; for
  fasteners the shipped file is the provenance record, so maintenance is editing that file.

## Limits

- A group does not exist yet: A14.3 adds `CreateBoltGroup`/`UpdateBoltGroup` and consumes the
  pinned digest; A14.2 builds the meshes from the same `FastenerSpec`.
- One library per lookup, no environments (O-B2); oversize/slotted/blind holes are post-MVP
  (O-B3) and have no representation here.
- The nut and washer dimensions travel inside the bolt definition, keyed by the bolt standard's
  designation; a separate nut or washer catalogue is not modelled.

## Tests

`beamcraft-cli/src/library.rs`: a fastener entry resolves to a complete named definition; a bad
entry is reported and the others load; a fastener record is refused by `add-entry`; the profile
loader never sees a fastener entry. `tests/fastener_libraries.rs` (end to end through the
binary): the shipped libraries list and show with their recorded source and licence;
`define-fastener` pins the digest the listing predicted, survives deleting the library file, is
idempotent, is listed by `definitions` and selectable by name in a fresh process; unknown
designations and a refused entry change nothing; every shipped entry pins its predicted digest.
