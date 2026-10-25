# A07 section families, environments and profile libraries

Date: 2026-10-18. Status: **A07.1 implemented as an experimental slice (section 9); A07.2 implemented as
an experimental slice (section 10); A07.3 is implemented as an experimental slice (section 11). Owner decisions D1, D2 and D4 of section 7
are taken, and D3 is decided as a warning, not a refusal (section 10).** Branch
`feat/a07-section-library`, based on `origin/master` `d893f94`; A07.2 branch `feat/a07-2-libraries`,
based on `origin/master` `5c54c0a`; A07.3 branch `claude/determined-keller-3r153u`, based on
`origin/master` `fd6fde9`.

Plan row: A07, the I-section families named by the P02.2 note, widened by the owner on
2026-10-18. The owner's goals:

1. The user never types section dimensions to place a member. They choose "insert column",
   pick a designation (for example the name of a wide-flange shape) from the profile library of
   the active **environment**, and the engine and kernel do the rest.
2. An environment is a per-country (or per-company) set of libraries, selected in the
   application's settings.
3. Users can add their own sections: parametric shapes with their own dimensions, cold-formed
   (light-gauge, LSTK) shapes, and fully custom outlines.

This is the MVP scope item 1 of `ROADMAP.md`. The UI picker belongs to V05; this note covers the
engine, data and CLI parts that the picker will call (`AGENTS.md` section 3: engine API and CLI
first).

## 1. What exists today

* A member pins exactly one immutable profile definition by the full SHA-256 of its USD layer
  (`bim:profile`, P02/P03). The definition is a content-addressed prim
  `/World/Definitions/D_<sha256>`; equal content is one definition.
* The only profile kind is `RectangleProfile { width_mm, height_mm, section }` with optional
  declared `SectionProperties` (designation, standard, area, mass per metre, surface per metre).
* `kernel::straight_member` builds an 8-vertex box. G05 (`kernel::profile`) already admits any
  planar outer contour with cavities, triangulates it and extrudes it into a sealed mesh, with a
  flagged polygon approximation for curves. A04 introduced `engine::solid::SolidMesh` for bodies
  that are not boxes.

## 2. The model in one picture

```text
Environment (USD layer)         Libraries (USD layers)            Model (USD revision)
  env-example.usda                example-sections.usda              /World/Definitions/D_<sha>
    subLayers = [user,              EX-I300 = i_shape{...} --resolve-->  profile definition:
                 example]         user-sections.usda                       shape + dims + labels
                                    MY-C150 = lipped_c{...}          member --bim:profile--> D_<sha>
```

* **Libraries are inputs, not model state.** Resolving a designation produces a complete profile
  definition (shape family, every dimension, declared section numbers, designation and library
  provenance labels) which is pinned by content like any other definition. A model therefore
  never depends on the library files: changing or deleting a library never changes an existing
  member, and opening a model on another machine needs no library.
* Resolving the same entry twice gives the same bytes and therefore the same digest, so placing
  a hundred `W310x52` columns creates one definition.
* An entry edited later (a corrected flange thickness) is a different definition; existing
  members keep the old one until an explicit `UpdateElement` repins them. Detecting "library
  entry changed since placement" is a later report, not an automatic edit.

## 3. Section families (A07.1)

A `SectionShape` is a closed enumeration. Every family generates its outline in the section
plane (millimetres, the F06 member frame: width along the rolled `X_m`, height along `Y_m`,
centred on the axis by the rule of section 4) and is admitted by its own rules before the kernel
sees a point.

| Family | Parameters | Notes |
|---|---|---|
| `rectangle` | `h`, `b` | Existing; solid bar. Stays byte-compatible. |
| `i_shape` | `h`, `b`, `tw`, `tf`, `r` | Doubly symmetric I/H (wide flange, IPE/HE style). |
| `channel` | `h`, `b`, `tw`, `tf`, `r` | Hot-rolled parallel-flange channel. |
| `angle` | `h`, `b`, `t`, `r` | Equal or unequal leg. |
| `rhs` | `h`, `b`, `t`, `r_out` | Rectangular/square hollow section (one cavity). |
| `chs` | `d`, `t` | Circular hollow section (one cavity). |
| `lipped_c` | `h`, `b`, `c`, `t`, `r_in` | Cold-formed C with lips (LSTK). |
| `plain_c` | `h`, `b`, `t`, `r_in` | Cold-formed U/C without lips. |
| `lipped_z` | `h`, `b`, `c`, `t`, `r_in` | Cold-formed Z with lips. |
| `omega` | `h`, `b`, `c`, `t`, `r_in` | Cold-formed hat section. |
| `custom` | outer contour, cavities | Any user outline, admitted by G05 as is. |

Rules:

* Admission per family: finite, inside the F04 dimension cap, above the geometric floor, and the
  family's own consistency (`2*tf < h`, `tw < b`, `r` fits between web and flange, `2*t < d`,
  lips shorter than the flange, ...). A violation is `INVALID_ARGUMENT` naming the parameter.
* **Two representations (owner decision D2).** Every member body is generated twice from the one
  definition: a **detailed** outline with every root and corner radius, and a **simplified**
  outline with sharp corners. This is the full/simplified pair of `docs/architecture.md` ("Part
  representations and view filters"): both are stored in the revision, identical content once,
  the detailed one is the default geometry an ordinary USD reader sees, and a view setting of the
  application chooses which one to load, so a large model can be viewed light. A section without
  radii has one outline referenced by both.
* Radii in the detailed outline are arcs approximated by a deterministic polygon with its vertices
  on the arc and a sagitta bound of `max(0.25 mm, 0.5 % of the radius)` (section 9), so that geometry is flagged `Approximated` (G05 rule; F06 0.1.0 has no exact
  curves). The simplified outline is flagged not exact. Snapping and the mesh use the detailed
  one only, and the two are never summed (section 5). Volume and mass do **not** read the polygon
  any more: they use the exact section area (P-Mass, section 5).
* The point budget is the G05 one (1024 per contour, 2048 total); the generator chooses the arc
  segment count from the radius and the sagitta bound and refuses (`LIMIT_EXCEEDED`), never
  truncates.
* `custom` is stored as authored points; nothing is reordered, merged or repaired (F04 P-1).
* A new family is a new enumeration value and a new schema descriptor line, never a free-form
  string interpreted at runtime.

## 4. Kernel and engine (A07.1)

* `kernel::section_member(input)`: admits the generated outline with G05, extrudes it along
  `+Z` by the axis length and places it with the `geom::MemberFrame` basis (the same frame, roll
  and minima as `straight_member`). The result is a `SolidMesh`; the rectangle keeps
  `straight_member` so existing digests do not change.
* The derived layer stores the detailed mesh as the default geometry and the simplified mesh as
  a second representation (USD `proxy` purpose is the candidate encoding, as `docs/architecture.md`
  proposes); identical blobs are stored once. Each representation has its own mesh digest, and
  the publish-time verifier rebuilds both. The view snapshot (V02) gains the representation
  choice; the UI setting itself is V05/V06.
* Insertion point: the section's reference point is the centre of its bounding box, as for the
  rectangle today. Offsets (top-of-steel, flange edge) are a later placement option.
* `engine::member` resolves `ProfileInfo` as the full `SectionShape` instead of
  `width_mm`/`height_mm`, and the publish-time verifier re-generates and re-extrudes every member.
* Layer engine: `Definition::Profile { shape, section, source }` beside the existing
  `RectangleProfile`, written as `steel:profile:family`, one `double` attribute per parameter
  (`steel:profile:hMm`, ...) and, for `custom`, the same flat point and count arrays as A04
  holes. The schema descriptor gains these lines; the previous descriptors stay accepted.
* CLI: `define-profile --family i_shape --h 310 --b 165 --tw 5.8 --tf 9.7 --r 8.9 ...` for
  explicit definitions (tests, scripting, user-defined sections).

## 5. Section numbers and mass

* The library entry may declare area, mass per metre and surface per metre (P02.2 fields). They
  are what a BOM reads, and they are part of the definition's identity.
* **Member quantities (P-Mass, owner decision of 2026-10-18).** The member mass is
  `area x axis length x density`. The area has two origins and there is never "no area":
  `catalogue` is the area the pinned profile definition declares (a library entry or
  `--area-mm2`); `computed` is the EXACT area of the section from its parameters (closed form
  with true circular arcs; the shoelace area of a custom outline; `width x height` of a
  rectangle). The polygon of the detailed outline is used for the mesh only. See
  [p-mass-section-area.md](p-mass-section-area.md).
* A declared area is compared with the exact area (not with the polygon). A declared area that
  differs from the exact area by more than 1 percent is reported as a warning and never refused
  (owner decision D3, section 7 and section 10): catalogue values are authoritative. The
  rectangle keeps its refusing P02.2 check.
* No density, grade or mass is ever guessed from a designation.

## 6. Environments and libraries (A07.2, A07.3)

**File format (owner decision D1): USD.** A library is a separate USD layer, written and read by
the same layer engine and with the same `steel:profile:*` attributes as a model definition, so
there is one mapping, one admission and one validator. Each entry is a prim under
`/Library/Profiles/` whose name is derived from the designation and which carries the
designation itself as a label:

```usda
#usda 1.0
(
    customLayerData = {
        string "bim:library:id" = "example"
        string "bim:library:title" = "Synthetic example sections"
        string "bim:library:version" = "1"
        string "bim:library:source" = "Contributor-original synthetic values for tests"
        string "bim:library:licence" = "MPL-2.0"
    }
)
def Scope "Library" {
    def Scope "Profiles" {
        def Scope "EX_I300" {
            string steel:profile:designation = "EX-I300"
            token  steel:profile:family = "i_shape"
            double steel:profile:hMm = 300
            double steel:profile:bMm = 150
            double steel:profile:twMm = 7
            double steel:profile:tfMm = 10
            double steel:profile:rMm = 15
            double steel:profile:massPerMeterKg = 36   # optional declared number
        }
    }
}
```

National libraries (AISC, ICHA, EN, ...) are produced separately as their own `.usd` files with
their own provenance; the repository ships only the mechanism and one synthetic test library
that follows no country's standard.

**Environment.** (Implemented in A07.3 with one change: the order is an explicit list in
`customLayerData`, not `subLayers`; see section 11.) An environment is also a small USD layer: its `subLayers` list the libraries in
order of strength, and its `customLayerData` carries its id and title. USD sublayer strength is
exactly the lookup order (stronger first), so no second precedence rule is invented:

```usda
#usda 1.0
(
    customLayerData = { string "bim:environment:id" = "example" }
    subLayers = [@./user-sections.usda@, @./example-sections.usda@]
)
```

**Lookup order.** Project libraries, then the user's own library, then the environment's
libraries in order. The first library that has the designation wins; a duplicate inside one
library is refused when the library is loaded, and a shadowed designation is reported. Sublayer
paths are local and contained (the U01 resolution rules); a network path is refused before any
I/O, as for R01 references.

**Where files live.** The application data directory holds `environments/` and `libraries/`;
the active environment is a user setting. The CLI takes `--environment FILE` explicitly, never a
hidden global, so tests and scripts are reproducible.

**Untrusted input.** A library file is untrusted (`AGENTS.md` section 4): it is opened with the
U01 bounded read (size, prim and stage budgets), every entry is admitted by its family rules, a
bad entry is reported with its file, prim and designation and never stops the other entries from
loading, and nothing in a library entry can name a path, URL or executable.

**Commands (headless first).**

```text
beamcraft list-library   --environment FILE [--family F] [--filter TEXT]
beamcraft show-library   --environment FILE --designation NAME
beamcraft create-member  --store ROOT --environment FILE --profile NAME --start .. --end ..
```

`create-member --profile NAME` resolves the entry, defines the profile if it is not in the model
yet (content-addressed, so it is idempotent) and pins it, in one revision.

**User sections (A07.3).** A user library is the same format in the user's data directory.
`add-library` writes an admitted entry to it (parametric, cold-formed or `custom` outline), so
the UI "new profile" dialog of V05 calls the same API.

## 7. Owner decisions needed

* **D1. Library data source and format — decided 2026-10-18.** Libraries are separate `.usd`
  files made outside this change. For tests the repository ships one synthetic library that
  follows no country's standard. Any real table later records its source and licence
  (`AGENTS.md` section 1.2) and may live outside the open repository.
* **D2. Radii — decided 2026-10-18.** Store both: a detailed representation with every radius
  and a simplified one without, chosen by an application view setting (section 3).
* **D3. Declared-area tolerance** for families with radii (section 5). **Decided by the owner,
  2026-10-18: a warning, not a refusal.** Catalogue values are authoritative; they are not
  mathematically derived from the nominal dimensions, and rounded radii and published roundings
  legitimately differ from the kernel's polygon. The warning threshold is 1 percent (section 10).
* **D4. Contract amendment — decided 2026-10-18 (applied, section 9).** F04 and F06 0.1.0 name the rectangle as the only initial profile.
  A07.1 needs an amendment (F04 section 4 profile kinds, F06 section 2 member input) recorded
  before or with the implementation, as A04 did for holes.

## 8. Work split

| Sub-task | Result | Depends on |
|---|---|---|
| A07.1 | Families of section 3 (hot-rolled and cold-formed) and `custom`, detailed and simplified outlines, kernel `section_member`, layer-engine definition, engine, `define-profile --family`, end-to-end test | D4 |
| A07.2 | USD library and environment layers, loader, `list-library`, `show-library`, `create-member --profile NAME`, synthetic test library, end-to-end test | A07.1 |
| A07.3 | Environment files and lookup order, user library, `add-library` for parametric, cold-formed and custom sections (section 11) | A07.2 |
| V05 | UI: insert member, pick from the library; new-profile dialog | A07.2, A07.3 |

Each sub-task is its own PR. Completing a sub-task does not complete A07.

## 9. A07.1 as implemented

```text
beamcraft define-profile --store ROOT --family i_shape --h 300 --b 150 --tw 7 --tf 10 --r 15
                         [--designation TEXT] [--standard TEXT] [--area-mm2 F] [--mass-per-meter-kg F]
beamcraft define-profile --store ROOT --family custom --outer "X,Y;..." [--cavity "X,Y;...|..."]
beamcraft create-member  --store ROOT --start X,Y,Z --end X,Y,Z --profile-digest SHA256 ...
```

| Area | What exists |
|---|---|
| `beamcraft_model::section` | `SectionFamily` (9 families, tokens, parameter order), `ParametricSection` (admission: finite, caps, floor, family consistency, all `INVALID_ARGUMENT` naming the parameter; `without_radius`), `CustomSection` (counts and coordinate caps only), `SectionShape` (`token`, `validate`, `extent_mm`); `definition::SectionProfileSpec`. |
| `beamcraft_kernel::section` | `outline(shape, Detailed or Simplified)`: corner lists per family, quarter arcs centred at `P - r*d1 + r*d2` on the G05 deterministic angular grid, CHS as two circles; `section_member`: G05 extrusion along the axis length, `RigidTransform` of the F06 member frame, G06 certification of one shell; `SectionFailure::f04_code`. |
| `beamcraft_layer_engine::authored` | `Definition::SectionProfile`: `bim:definitionKind` is the family token or `custom`, one `steel:profile:<parameter>Mm` per parameter, flat `outerPointsMm`/`cavityPointsMm` + `cavityVertexCounts` for custom outlines; a member's `bim:profile` may target any profile kind; descriptor amended, the A06 descriptor stays accepted. |
| `beamcraft_engine::member` | `ProfileInfo::{Rectangle, Section}`; a rectangle keeps the P03 box and its bytes, a section is the kernel's detailed outline (`SECTION_VOLUME_ALGORITHM`); `MemberBody` keeps the resolved profile and its volume; the member shape signature of a rectangle is unchanged. |
| `beamcraft_cli` | `define-profile --family`, section definitions decoded, listed and pinned; `definitions` and member JSON show `parameters_mm` or the outline. |

Deviations and limits, recorded rather than hidden:

* **Contract (D4) — decided 2026-10-18.** The owner lifted the first-scope rectangle-only rule:
  F04 (amendments log, P-2) and F06 (amendment A-1, section 2) now admit the A07.1 families and
  custom outlines, additively.
* **Simplified body not stored yet.** The kernel builds both outlines, but the derived layer still
  stores one mesh per element. Storing the simplified mesh as the second representation and the
  view setting that chooses it follow with V02/V05 work.
* **Declared area (D3).** For section families, a declared area was admitted as a label-like value
  and not compared with the outline area; A07.2 now reports a warning when it differs (section 10).
* **Volume status.** A section member's volume is `current`. Since P-Mass it is the effective
  section area (declared, else the exact area) times the axis length; the stored mesh is still the
  detailed polygon, whose arcs are approximated (flagged in the kernel result, as the A04 round
  holes are), so the mesh volume and the quantity volume can differ by the arc approximation.
* **Layer budgets.** The U01 first-slice ceilings bounded a revision to 7 detailed I-sections; they
  were replaced by the model profile ([layer budgets](layer-budgets-model-profile.md)).
* No placement offsets, no rotated placement (P03 rule), no member holes or cuts.

Evidence: `cargo test -p beamcraft-model section`, `cargo test -p beamcraft-kernel --test section`
(closed-form areas of every family in both representations, bounding-box centring, member bounds
in the F06 frame, divergence-theorem volume, determinism, custom and refusal codes), the
layer-engine round trip and tamper test, the engine unit test, and `cargo test -p beamcraft-cli
--test sections` (real binary: define, idempotent redefine, build, reopen in a fresh process,
world bounds, `definitions`, saved text, cold-formed, CHS and custom members in one revision,
refusal codes).

## 10. A07.2 as implemented (experimental slice)

Branch `feat/a07-2-libraries`, based on `origin/master` `5c54c0a`.

```text
beamcraft list-library   --library FILE [--family F] [--filter TEXT]
beamcraft show-library   --library FILE --designation NAME
beamcraft define-profile --store ROOT --library FILE --designation NAME [--name TEXT]
beamcraft define-profile|define-material ... [--name TEXT]
beamcraft create-member  --store ROOT --start .. --end .. (--profile-digest SHA256 | --profile NAME)
                         [--material-digest SHA256 | --material NAME]
beamcraft create-plate   --store ROOT ... [--material-digest SHA256 | --material NAME]
```

| Area | What exists |
|---|---|
| `beamcraft_cli::library` | `load(path)` / `load_bytes`: bounded local read (8 MiB, no network or device path), the layer engine opens the file as one read-only foreign layer under library budgets (1 layer, 8,208 prims, 8,192 array items, 64 MiB decoded), metres per unit 0.001 and Z up are required, sublayers and external references cannot be resolved (the engine is given no other asset). Root `customLayerData` must carry `bim:library:id`; `title`, `version`, `source` and `licence` are reported when present, never inferred. Entries are the direct children of `/Library/Profiles`: `steel:profile:designation`, `steel:profile:family` (`rectangle`, one of the nine families, `custom`), one `steel:profile:<parameter>Mm` per parameter (rectangle: `h` is the height, `b` the width), optional `standard`, `areaMm2`, `massPerMeterKg`, `surfacePerMeterM2`, and for `custom` `outerPointsMm`, `cavityPointsMm`, `cavityVertexCounts`. Each entry is admitted by the same rules as a hand-published definition. |
| Failure policy | A duplicate designation refuses the whole library (`INVALID_ARGUMENT`); more than 4,096 entries, more than 8 MiB or a stage over the budgets is `LIMIT_EXCEEDED`; unreadable USD, other units, a missing `bim:library:id` is `INVALID_ARGUMENT` (`UNSUPPORTED_PROPERTY` for an encoding the static read profile cannot decode, `PERMISSION_DENIED` for a network path or an uncontained sublayer name); a bad entry is reported in `rejected` (prim, designation, code) and the others load. A designation whose entry was refused reports that entry's code, never `NOT_FOUND` and never another entry. |
| Lookup | Exact, case-sensitive designation. An unknown designation is `NOT_FOUND`; nothing is substituted or matched by prefix. |
| `define-profile --library` | The definition takes every value from the entry and stores them itself (designation, standard, declared numbers, display name = designation unless `--name`), so it never depends on the library file later: the tests delete the library before placing a member. The library provenance (id, version, licence) is reported by `list-library` and `show-library` but is **not** stored in the definition (the design note of section 2 foresaw provenance labels; they would be new definition fields and a new digest input, and are left to a later slice). `show-library` prints the digest that `define-profile --library` would pin. Combining `--library` with explicit dimensions, `--family`, `--standard` or declared numbers is `INVALID_ARGUMENT`. |
| Display names | `Definition::{Material, RectangleProfile, SectionProfile}` carry an optional readable name, written as USD `displayName` metadata on the definition prim. `define-material` and `define-profile` take `--name TEXT`; the default is the designation, else a generated name: `S355 7850 kg/m3` (grade and density, only what is declared; `Material (density unknown)` otherwise), `RECT 200x300`, `I 200x100`, `RHS 100x60x4`, `CHS 114x5`, `Custom 8 points`. `definitions` lists `display_name`; `inspect-elements` lists `profile_name` and `material_name` of each element (null for a definition without a name). JSON objects are printed with sorted keys by the shared encoder, so "name first" is not a property of the text; the name is the first thing a person looks for, not the first key. |
| Selection by name | `create-member --profile NAME`, `create-member|create-plate --material NAME` resolve against the **head's** definitions (verified and re-admitted like every read) by display name or by designation (profiles) or grade (materials); profiles and materials are separate namespaces. No match is `NOT_FOUND`; more than one distinct definition is `INVALID_ARGUMENT` with the candidates (digest, kind, display name, names; the first 16 and the total, see the review follow-up) in `diagnostics` and nothing published or guessed; a name and its digest option together are `INVALID_ARGUMENT`. The digest stays the identity: the command pins the resolved digest. |
| Declared area (D3) | Owner decision of 2026-10-18: **a warning, never a refusal**; the reason is that catalogue values are not mathematically derived and rounded radii differ legitimately. `engine::member::declared_section_area_warning` compares a declared area with the **exact** area of the section (`SectionShape::exact_area_mm2`, closed form with true arcs; since P-Mass, no longer the polygon the kernel extrudes); a relative gap above 1 percent (`model::limits::DECLARED_SECTION_AREA_TOLERANCE`) gives a `DeclaredAreaWarning` and the definition is created with its declared value. The CLI reports `declared_area_warning` (`declared_mm2`, `kernel_mm2` and its alias `exact_mm2` (both the exact area), `relative_difference`, `threshold`, `threshold_percent`; null when it agrees or no area is declared) in the `definition` of `define-profile`, in each entry of `list-library` and `show-library`, and in `definitions` (computed when listed, so stores need no change). A library entry with such an area still loads. Malformed values (non-finite, not above zero, above the cap) are still refused by the F04 rules. The rectangle keeps its refusing 0.1 percent P02.2 check (not widened). Mass uses the declared area when present (P-Mass); the declared mass per metre is still not read. |

### Digest and schema effect (honest record)

* **The display name is part of the definition content.** It is stored on the prim and the
  definition digest is the hash of the canonical isolated definition layer, so a different name is a
  different digest, and the CLI now names every new definition by default. Consequently
  publishing the same values as an old unnamed definition creates a second definition; and an
  unnamed definition (no `displayName`) keeps its old bytes and digest, so every definition
  published before A07.2 verifies unchanged. Names are labels only: they never select a number
  (a grade does not select a density, a name does not select a section).
* **Schema descriptor.** One optional line (`definition displayName=prim metadata string(...)`)
  was added after the D1/D2 additions (level, project record), so the schema digest changes. The
  digest of the D1/D2 descriptor of `origin/master` `85fe294`
  (`b799ae17004c22a5c2eaf7512367f919aec4b82b9b4b223166310286e1414b5b`, the one every store
  written before A07.2 carries) is accepted next to the current one: it is derived at run time by
  removing that one line. The A07.1 digest (`0311df71b2f7a4bd7b19e87ca7daef1884e989bd099bca24ee165de8e6bc310c`)
  and the earlier ones stay accepted through their descriptors. Unit tests pin the D1/D2 and A07.1
  digests and check that every previously published pin (A07.1, D1/D2, A06, A01, R01, P02.2, U09)
  is still accepted. The next edit of an older store rewrites the pin to the current digest.
* **Existing stores and tests.** Checked with the real pre-change binary (built from `origin/master`
  `5c54c0a`): a store made by it (material with grade, I-section with designation, a plate and a
  member) was listed, inspected and edited by the new binary; its unnamed definitions are selected
  by their grade and designation; the new binary then published a named definition of the same values
  as a second definition. **Forward compatibility is not kept:** the old binary refuses a store
  that holds a named definition (`INTEGRITY_ERROR`, it recomputes the digest without the name).
  Tests that compared a CLI-published digest with a hand-built `Resource` (`tests/definitions.rs`: the
  material and the rectangle) now build the `Resource` with the default display name, which is the
  explanation of the changed expected digest; every other digest check compares the CLI with the
  CLI or with `show-library`. `Definition::Material` gained a field, so the layer-engine and CLI
  fixtures that build it gained `display_name: None`.
* The U02 descriptor is shared with the layer-engine work of other branches; the change is one
  line in `SCHEMA_DESCRIPTOR`, one constant and one branch in `pin_accepted`, so a later rebase has one
  place to reconcile.

### Assumptions and limits, recorded rather than hidden

* **D3 is decided (owner, 2026-10-18): a warning.** The 1 percent threshold (the note of section 5
  gave no number) is the point above which the warning is reported; it covers the rounding of
  a published area. Since P-Mass the reference is the exact area, so the arc polygon no longer
  contributes to the gap. The declared mass per metre will feed the bill of materials in a
  follow-up.
* **(Superseded by A07.3, section 11.)** **One library file only.** `--library FILE` names it explicitly; the project, user and
  environment lookup order, `--environment`, environment layers (sublayers of libraries) and shadow
  reports of section 6 are **design only**. A07.3 (user library, `add-library`) is not started.
  A library is read only; this slice writes no library (the shipped one is hand-authored text).
* The prim name of an entry is not required to match the designation (the designation attribute is
  the identity); an inactive entry is reported as rejected.
* The library provenance is not stored in definitions (see above). Resolving by name uses
  the head only, not the library.
* The layer engine runs in the CLI process (no LEW-03 containment), as for every other command; a
  hostile library is bounded by the budgets, not isolated.
* `used_by` in `definitions` still lists plates only (members pinning a profile are not listed there).
* The synthetic library `assets/libraries/synthetic-sections.usda` has nine invented entries
  (`SYN-*`, MPL-2.0, listed in `ATTRIBUTION.md`); it follows no standard and contains no national data.
  The declared areas of its three entries that carry one (5,153, 2,000 and 2,400 mm2) agree
  with the exact area, so none warns.

Evidence: `cargo test -p beamcraft-cli --test libraries` (real binary: list and filter, show,
self-contained definition with the library deleted, digest equal to the predicted one, every
shipped entry builds a member, unknown designation `NOT_FOUND` with nothing published, duplicate,
oversized, garbage, fake binary, sublayer, wrong units, missing id, directory, missing file and
URL/UNC paths, a bad-entry mix, the 4,096/4,097 entry bound, option conflicts, default names, name
as part of the digest, ambiguous names with candidates, name and digest together),
`beamcraft_cli` unit tests of the loader, `cargo test -p beamcraft-model definition`,
`cargo test -p beamcraft-layer-engine pin_tests`.

### Review follow-up (PR #93)

* **Default names stay valid labels.** A generated material name that would exceed the 256-byte label
  limit (a 256-byte grade plus the density suffix) falls back to the grade alone, then the density
  alone; the grade and density themselves are stored unchanged.
* **Listings are paged.** `list-library` and `definitions` take `--offset` (default 0) and `--limit`
  (default 64, 1 to 256) and report `entry_count`/`definition_count` (all matches), `rejected_count`,
  `offset`, `page_size` and `next_offset` (null on the last page). `list-library` pages one position space:
  the matching entries, then the refused ones. Each page is also checked against the real encoder
  (1 MiB of text, the parser's token scratch): the page is halved until it encodes, so a page may be
  shorter than `--limit` (never empty while items remain), and one entry that alone is too large is
  `LIMIT_EXCEEDED`. `definitions --name NAME` lists the definitions that a name selects.
* **Ambiguity responses are capped.** The `diagnostics` of an ambiguous name list the first 16
  candidates and, when there are more, a final object with `candidate_count`, `listed` and the
  `definitions --name` command that pages the rest. The code stays `INVALID_ARGUMENT`.
* **Retries by name replay.** For an `--op-id` the host ledger has already committed, a name that
  still describes the definition the head's element was bound to resolves to that definition
  (`beamcraft_cli::plate::accepted_binding`), before any name is resolved against the head; the
  engine's content fingerprint then replays an identical request and returns `OP_ID_REUSE` for a changed one
  (another name, another size). A name that does not describe the original binding resolves
  normally, so an ambiguous or unknown name is still `INVALID_ARGUMENT` or `NOT_FOUND`, not a
  replay. Limit: as for every replay in this adapter, the original binding is read from the head's
  element, so an element deleted since is not replayed.

## 11. A07.3 as implemented (experimental slice)

Branch `claude/determined-keller-3r153u`, based on `origin/master` `fd6fde9`.

```text
beamcraft list-library   (--library FILE | --environment FILE) [--user-library FILE] [--family F] [--filter TEXT] [--offset N] [--limit N]
beamcraft show-library   (--library FILE | --environment FILE) [--user-library FILE] --designation NAME
beamcraft define-profile --store ROOT (--library FILE | --environment FILE) [--user-library FILE] --designation NAME [--name TEXT]
beamcraft add-library    --library FILE [--library-id ID [--title TEXT]] --designation NAME
                         (--width MM --height MM | --family F --h .. | --family custom --outer .. [--cavity ..])
                         [--standard TEXT] [--area-mm2 F] [--mass-per-meter-kg F] [--surface-per-meter-m2 F]
```

| Area | What exists |
|---|---|
| Environment file | A USD layer in millimetres, Z up, read by the same bounded loader as a library (one self-contained layer, 1 MiB at most). `customLayerData` carries `bim:environment:id`, an optional `bim:environment:title` and `bim:environment:libraries`: a `string[]` of 1 to 32 relative paths, **strongest first**, resolved against the directory of the environment file. A path that is absolute, has a drive or scheme (`:`), a backslash, an empty, `.` or `..` segment or a control character is `PERMISSION_DENIED` (lexical containment, as for R01 sublayer names); more than 32 is `LIMIT_EXCEEDED`; an empty list, a non-string item or a repeated path is `INVALID_ARGUMENT`. |
| Lookup | `beamcraft_cli::library::Lookup` holds the libraries in order: the optional `--user-library`, then the environment's libraries (or the single `--library`). The first library that declares a designation answers it. A **refused** entry of a stronger library answers with its refusal; a weaker library is never consulted for that designation, so a corrupted override cannot silently turn into another section. `list-library` lists the effective entries (one per designation, designation order, each with its `library_id`), the refused entries of every library, `libraries` (path, id, counts) and `shadowed` (designation, used and shadowed library id). The shadow records are paged on their own with `--shadow-offset` and `--limit`, and the reply carries `shadowed_count` and `shadowed_next_offset` (null on the last page), because a few overlapping libraries of thousands of entries make tens of thousands of records that must not decide whether the entries fit the 1 MiB output budget. `show-library` names the library and path that answer and reports only the shadow records of the designation it shows. |
| Whole-library failures | A listed library that cannot be loaded (missing file, duplicate designation, over a budget, wrong units) refuses the whole environment with that library's code, because a silently missing library would change which section a designation selects. Two libraries of one lookup with the same `bim:library:id` (inside an environment, or the user library against a single `--library` or an environment library) are `INVALID_ARGUMENT`, so the library id of an entry and of a shadow record is never ambiguous. `--library` and `--environment` are alternatives; `--user-library` needs one of them; with none the command is `INVALID_ARGUMENT`. |
| `define-profile` | `--environment` (and `--user-library`) take the place of `--library`; the definition takes every value from the answering entry and stores them itself, exactly as in A07.2, so the model never depends on any library file. Explicit dimensions do not combine with them. |
| `add-library` | Adds one profile entry (rectangle, any of the nine families, or a custom outline with cavities) to the library file named by `--library`, through the same option parsing and the same F04 admission as `define-profile` (`profile_record`). A missing file is created only when `--library-id` is given (`--title` optional); an identity for a file that exists, a duplicate designation, a library that holds refused entries (a rewrite would lose them), an invalid section, a material, or more than 4096 entries is refused. The designation and the display name of the entry are the one given designation. The prim name is the designation with every character outside `[A-Za-z0-9_]` replaced by `_` (prefixed `E_` when it would start with a digit or be empty) and made unique with a numeric suffix; existing prim names are kept. |
| Writer | The whole library is rendered again, entries in designation order, from what the loader understood. The text is read back with the loader before anything is written, and the identity, the optional metadata and every entry must come back equal (`INTEGRITY_ERROR` otherwise, nothing written). A `.usdc` destination, and an existing library whose content is binary USD, are refused (`INVALID_ARGUMENT`): the writer produces USDA text only, and the read-back check cannot see the encoding of the destination. The file is written to a temporary file beside it, synced and installed atomically, and a replacement keeps the permissions of the file it replaces (a private `0600` library stays private): a new file is hard-linked into place (it never replaces a file that appeared meanwhile) and an existing one is renamed over. Numbers are written in Rust's shortest round-trip form, so values read back bit for bit. |

### Deviations and limits, recorded rather than hidden

* **The order is a list, not `subLayers` (deviation from section 6).** The layer engine does not
  expose composition arcs (root metadata excludes `subLayers`), and composing the libraries as USD
  sublayers would compose same-path prims across libraries, attribute by attribute, instead of
  letting the strongest library answer a designation as a whole. An explicit ordered `string[]` in
  `customLayerData` keeps the "strongest first" meaning, needs no engine change and stays an
  ordinary USD layer that any USD reader opens. If the owner prefers real sublayers, only
  `parse_environment` changes.
* **No hidden global, no settings yet.** The active environment and the user library are command
  line options. An application data directory, the "active environment" setting and the lookup of
  a project-level library are V06 work; the UI picker (V05) will call the same functions.
* **`add-library` rewrites the file.** Comments, `doc` text and attributes the loader does not
  read are not kept, and two writers of one file at once are not supported (the last rename
  wins). Refusing a library with refused entries and the read-back check keep it from losing or
  changing anything the loader understood; an edited-by-hand library with extra content should be
  kept under version control. There is no `library-remove` or edit command.
* **Library provenance is still not stored in definitions** (section 10), so "this member's section
  came from library X, version Y" is not recorded; and `create-member --profile NAME` still
  resolves against the model's definitions, not a library: define the profile first with
  `define-profile --environment`, then place members by name. One command for both is a later
  convenience.
* **Containment is lexical.** A listed path cannot name a parent directory or an absolute path, but a
  symbolic link inside the environment directory is followed, as for any `--library` file.
* The layer engine runs in the CLI process (no LEW-03 containment); a hostile library or
  environment is bounded by the budgets, not isolated.

Evidence: `cargo test -p beamcraft-cli --test environments` (real binary: ordered lookup and shadow
report, the digest predicted by `show-library` equals the pinned one, user library first, a refused
stronger entry not replaced by a weaker one, hostile environments (escaping, absolute, drive,
scheme, backslash, empty and repeated paths, 33 libraries, no libraries, numbers, missing file,
duplicate library ids, sublayers, other units, directory, URL, a library used as an environment),
`add-library` creating a library and adding every family, a rectangle, custom outlines with a
cavity and awkward characters in the designation, the digest equal to an explicit definition with
the same name, a member placed with the library deleted, and every refusal leaving the file byte
for byte unchanged with no temporary file left), and `cargo test -p beamcraft-cli --lib library`
(lookup, containment, prim names, render and read back, environment parsing).

## Provenance

Section families and their parameters are generic engineering geometry described in public
standards (EN 10365, ASTM A6, EN 10162 for cold-formed sections) and textbooks; no dimension
table, catalogue file or software material is used. The environment and library concept is the
owner's requirement of 2026-10-18; the USD layout of libraries and environments is original to
this note and uses only standard USD layer metadata, sublayers and prims.
