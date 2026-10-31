# F04: model and command contract, version 0.1.0

Status: **frozen 0.1.0** (2026-10-20). Changes only by a new version.

This contract fixes the domain semantics of the model and its commands: identities, units and
limits, authored fields, derived results, the mutation envelope, the command surface, footprints
and errors. The geometry contract (F06) defines how shapes are evaluated; this contract defines
what is authored and how it may change. JSON examples are control messages and acceptance
descriptions, never a second saved model format.

## 1. Authority and ownership

USD contains the authoritative authored parameters, placement, operations and relationships.
Published revisions also include the corresponding derived geometry and quantities. Rust owns
engineering validation, commands, rights, dependency tracking and transactions; a USD adapter
handles composition and serialization. UI and CLI call the same engine API.

An owned element, an immutable definition and a foreign reference are different targets. Mutation
commands address owned elements by ID. The engine derives ownership from validated composition
provenance, not from an `owner` string submitted by a caller. Unknown ownership is not write
permission. Edits and overrides of foreign prims are forbidden even in our own layers. Own
reference wrappers and measurement annotations have separate identities; they never make their
foreign source writable.

## 2. Identity, revision and paths

| Type | Representation and invariant |
|------|------------------------------|
| ElementId, ProjectId, WorkspaceId, OpId | Full UUIDv7, canonical lowercase `8-4-4-4-12` form; validate version 7 and RFC variant |
| ActorId | Opaque identity from authenticated execution context; a request claim must match that context |
| RevisionId | Full UUIDv7 identifying one immutable accepted project revision; never a shortened timestamp or array index |
| ContentDigest | SHA-256 of exact published bytes, all 64 lowercase hexadecimal digits |
| WorkspaceGeneration | Unsigned 64-bit counter; logical JSON form is a canonical decimal string, without leading zeros except `"0"` |
| FeatureId | Opaque stable identifier of a face, edge or vertex within a versioned geometry result (F06) |

UUID layout follows [RFC 9562 section 5.7](https://www.rfc-editor.org/rfc/rfc9562.html#section-5.7).
Creation checks actual uniqueness. UUID clock order gives neither authority nor conflict priority.
Same-millisecond elements keep different complete IDs.

**Prim paths.** A path holds only an element's category and its identity, never anything that can
change.

| Category | Path |
|----------|------|
| Part (plate, beam, column) | `/World/Steel/Parts/PRT_<uuid>` |
| Assembly | `/World/Steel/Assemblies/ASM_<uuid>` |
| Connection (weld, bolt group) | `/World/Steel/Connections/CON_<uuid>` |
| Level | `/World/Levels/LVL_<uuid>` |
| Read-only reference | `/World/References/REF_<uuid>` |
| Material definition | `/World/Definitions/Materials/MAT_<sha256>` |
| Profile definition | `/World/Definitions/Profiles/PRF_<sha256>` |

`<uuid>` is all 32 hex digits of the UUIDv7 without hyphens; `<sha256>` all 64 hex digits of the
definition's content digest. `bim:id` is the full hyphenated ID; `bim:name` is a label, shown as
`displayName`. Rename does not change ID or path, and neither does a change of assembly, level or
mark, which are relationships and attributes. Paths are adapter and index results, not
caller-selected mutation destinations; elements are never reparented. Discipline folders such as
`/Steel` sit under `/World`; definitions, levels and references are shared outside them.

A revision ID and a content digest have different roles. Each immutable revision has one validated
layer and asset manifest; reusing its ID for different content is an integrity error. Publication
switches the root atomically and verifies complete blob bytes. Commands never accept shortened
digests, absolute asset paths or remote resource URIs instead of a ContentDigest. The host resolves
authorized project-relative resources with containment checks.

## 3. Units, types and limits

Lengths and coordinates use finite f64 millimetres, angles use radians, and the coordinate system
is right-handed with Z up. Volume is m3, density kg/m3 and mass kg. The USD adapter authors
`metersPerUnit=0.001` and `upAxis=Z` explicitly. [USD stage linear
units](https://openusd.org/release/api/group___usd_geom_linear_units__group.html) are an IO concern.

Logical messages follow [RFC 8259](https://www.rfc-editor.org/rfc/rfc8259.html): reject duplicate
object keys, invalid UTF-8 and non-JSON NaN/Infinity literals. Numeric fields accept numbers only,
not booleans or numeric strings. Reject f64 overflow or underflow that changes a nonzero submitted
number to infinity or zero. Never silently round or clamp engineering inputs. Ordinary nearest-f64
conversion is the numeric representation; no extra dimension rounding is allowed. Positive and
negative zero are equivalent for signatures; only their sign is normalized for that purpose.

| Input | Constraint |
|-------|------------|
| Encoded logical command | At most 1 MiB before parsing; nesting depth at most 16 |
| Single string | At most 4096 UTF-8 bytes unless a narrower field cap applies |
| Element label | 1–256 UTF-8 bytes; reject control characters U+0000–001F and U+007F |
| Plate thickness and edges, member-axis length | Greater than zero and at most 1,000,000 mm |
| Minimum nonzero dimension | Plate thickness and edges, section dimensions and member-axis length are at least `eps_geom` (below), inclusive; a value above zero and below it is `INVALID_ARGUMENT` |
| Section dimensions | At most 1,000,000 mm; above is `LIMIT_EXCEEDED` |
| Local size L / contour radius D | Plate L = D <= 2,000,000 mm; member L = max(axis length, section half-diagonal) <= 1,000,000 mm |
| Plate contour count / roll angle | 3–1024 points; `abs(roll_rad) <= 6.283185307179586` rad (2*pi) |
| Coordinates and translation components | Absolute value at most 1,000,000,000 mm |
| Material density, when authored | At least zero and at most 1,000,000 kg/m3; absence is unknown (amended 2026-10-30, see [the density amendment](f04-material-density-amendment-v0.1.md): zero was "greater than zero") |
| Element relationship targets | At most 1024; valid pinned targets, no forbidden cycles |
| Derived command footprint entries | At most 4096; fail before publishing if the footprint cannot be bounded |

These caps are original operating limits, not vendor tolerances or promises of kernel accuracy.
The transport enforces byte, depth and string bounds before allocation and never widens them.

**Geometric length floor.** `eps_geom = max(0.001 mm, 2^-22 * L)`, where `L` is an element-local
body size in mm, evaluated before placement: for a plate `L = D`, the maximum distance from contour
point p0 to any contour point; for a member `L = max(ell, H)`, where `ell` is the axis length and
`H` the section half-diagonal. World coordinates, translation, thickness (for a plate) and roll
never enter it.

Evaluation uses binary64 round-to-nearest-even at every written operation, without reassociation
or fused multiply-add. `norm(v) = sqrt((v_x*v_x + v_y*v_y) + v_z*v_z)`. Subtract submitted
element-local point components first, then evaluate each norm in that order. For a plate scan
`norm(p_i - p0)` in ascending point index, starting with D = 0 and keeping the larger value with
`>`; L = D. For a member `ell = norm(p2 - p1)`, `hw = width_mm/2`, `hh = height_mm/2`,
`H = sqrt(hw*hw + hh*hh)`, L = max(ell, H). A section dimension that is not greater than zero
contributes 0 to H and is rejected as `INVALID_ARGUMENT` (`GEO_NOT_POSITIVE`). Multiply L by the
exact binary64 constant `2^-22 = 1/4194304`, then take max with binary64(0.001 mm).

Finiteness and caps are checked before the floor. An excessive finite L is `LIMIT_EXCEEDED`
(`GEO_ABOVE_LIMIT`). Non-finite submitted input is `NON_FINITE_NUMBER` (`GEO_NON_FINITE`). A
non-finite derived L from finite capped input is numeric uncertainty: `UNSUPPORTED_GEOMETRY`
(`GEO_NUMERIC_UNCERTAIN`), with no floor, partial geometry or key. L is never clamped and the
absolute floor never replaces a failed scale.

**Placement** is a rigid pose: `translation_mm` (three numbers) and `orientation_xyzw` (four
quaternion numbers). Orientation components are finite in [-1, 1]; the squared norm is within 1e-6
of one. Scale, shear, reflection and position presets are unsupported. Generic foreign transforms
stay read-only.

**Quaternion convention.**

- Component order is `(x, y, z, w)`: `q = x*i + y*j + z*k + w`, with Hamilton's rule `i*j = k`.
  `[0, 0, 0, 1]` is the identity.
- The rotation is **active** and acts on element-local coordinates: a local point `v` becomes
  `R(q) * v`, the proper rotation of `v' = q v q^-1`. For a unit axis `u` and angle `a` the
  quaternion is `(u*sin(a/2), cos(a/2))`, counter-clockwise seen from the tip of `u`.
- `world = R(q) * local + translation_mm`: rotate about the local origin, then translate.
- The accepted quaternion is normalized (`q / |q|`) before use; authored components are stored as
  submitted.
- `q` and `-q` denote the same rotation and are both accepted. The canonical sign for signatures
  belongs to the geometry keys (F06).

Example: `q = (0, 0, 1/sqrt(2), 1/sqrt(2))` is +90 degrees about +Z and maps `(3000, 0, 0)` to
`(0, 3000, 0)`; with `translation_mm = [100, 200, 300]` the world point is `(100, 3200, 300)`
(`F04-POS-10`).

**Member axis.** The two `axis_points_mm` give a length greater than zero. Identical points are
`INVALID_ARGUMENT`, like any "must be positive" violation; a length above 1,000,000 mm is
`LIMIT_EXCEEDED`; a non-finite coordinate is `NON_FINITE_NUMBER` or `MALFORMED_COMMAND`.
`UNSUPPORTED_GEOMETRY` is reserved for a well-formed shape outside the supported scope (a curved
member, a scaled transform). A nonzero axis below `eps_geom` is also `INVALID_ARGUMENT`.

### Element-local frames

These fix where the authored dimensions sit in the element-local frame, before placement.

**Plate.** `contour_points_mm` is an ordered array of 3 to 1024 distinct three-component points in
element-local millimetres. Closure is implicit from the last point to the first; the first point is
not repeated. Each coordinate is at most 1,000,000,000 mm in absolute value; thickness and each edge
are in [eps_geom, 1,000,000] mm; D is at most 2,000,000 mm.

- Frame: origin `p0`; `X_p = unit(p1 - p0)`. The area vector `N = sum((p_i - p0) x (p_(i+1) - p0))`
  is summed in ascending index order including the closing edge; exact signs and zero tests use
  the rational values of the submitted binary64 coordinates, only normalization rounds. A zero `N`
  is invalid. The largest absolute component of N (ties: Z, then Y, then X) must be positive, so a
  clockwise contour is `INVALID_ARGUMENT`; points are never reordered. `Z_p = unit(N)`,
  `Y_p = Z_p x X_p`.
- Planarity: every point lies within `eps_plane = eps_geom + 1e-6*D` of the plane through p0 with
  normal Z_p. No point is silently moved.
- Solid: the contour extruded `thickness_mm` along +Z_p; the contour is the bottom face (body z = 0),
  the top face is at z = thickness.
- Evaluation: a simple polygon lying in the plane `z = 0` is evaluated as that prism. A valid
  contour outside `z = 0`, or within the planarity band but not exactly planar, is accepted
  authored input with geometry status `unsupported` (`UNSUPPORTED_GEOMETRY`); it is not invalid.
- Validation order: types and count, caps before allocation, finiteness, coordinate, dimension and
  L caps, eps_geom, thickness and edge minima, repeated points, zero N, planarity, winding, then
  simplicity. Simplicity drops the dominant coordinate of N (same tie rule) and tests non-adjacent
  closed segments in ascending index-pair order; any intersection, touch, overlap, nonconsecutive
  duplicate or adjacent backtracking is invalid. Adjacent edges share only their endpoint. An
  ambiguous numeric certificate fails as unsupported, never as a guessed valid solid.
- Codes: fewer than three points, repeated consecutive points, subminimum edges, degeneracy,
  non-planarity, clockwise winding or self-intersection are `INVALID_ARGUMENT`; non-finite numbers
  `NON_FINITE_NUMBER` (bare NaN stays malformed JSON); caps `LIMIT_EXCEEDED`.

**Member.** For axis points `p1 -> p2`, `Z_m = unit(p2 - p1)`. The section lies in the local XY
plane, with the centre of its bounding box on the axis, its width along `X_m` and its height along
`Y_m`. The end faces are perpendicular to the axis through `p1` and `p2`; there are no extensions.

**Roll.** The command carries no "up" vector. `Y_m` is element-local +Z projected onto the plane
normal to `Z_m` and normalized, so a horizontal member's height points up. If the axis is parallel
to element-local Z (a column, including an anti-parallel axis), `Y_m` is element-local +Y projected
the same way; an exactly vertical axis keeps `Y_m = +Y`. Then `X_m = normalize(Y_m x Z_m)` and
`Y_m = Z_m x X_m`. Optional `roll_rad` (finite, default 0) then rotates the section about +Z_m:
`X_r = cos(r)*X_m + sin(r)*Y_m`, `Y_r = -sin(r)*X_m + cos(r)*Y_m`. Roll belongs to `shape_inputs`,
never placement; it is stored as authored, without modulo reduction. Above 2*pi is
`LIMIT_EXCEEDED`. A plate roll patch is `UNSUPPORTED_PROPERTY`; an omitted create roll means zero,
an omitted patch roll keeps the old value. The "parallel" tolerance is the geometry contract's.
The default frame is discontinuous at that threshold by design: a member tilted just past it may
get a section turned by 90 or 180 degrees, and the user sets `roll_rad` when another orientation is
wanted. The rule stays simple and deterministic.

**Reserved section anchor.** "Centred on the axis" is the default anchor. A future optional
`section_anchor` places the section by one of nine points (top, middle, bottom by left, centre,
right) plus offsets `dx`, `dy` in millimetres along `X_m`, `Y_m`. Only the name is reserved: a
request carrying it is `UNSUPPORTED_PROPERTY`.

| Example | Frame | Local bounds (mm) |
|---------|-------|-------------------|
| Plate 1000 x 500 x 10, identity pose | local X, Y, Z carry width, length, thickness | x in [0, 1000], y in [0, 500], z in [0, 10] |
| Horizontal member `p1 = (0, 0, 0)`, `p2 = (4000, 0, 0)`, `w = 200`, `h = 300` | `Z_m = (1, 0, 0)`, `Y_m = (0, 0, 1)`, `X_m = (0, 1, 0)` | x in [0, 4000], y in [-100, 100], z in [-150, 150] |
| Column `p1 = (0, 0, 0)`, `p2 = (0, 0, 3000)`, `w = 200`, `h = 300` | `Z_m = (0, 0, 1)`, `Y_m = (0, 1, 0)`, `X_m = (1, 0, 0)` | x in [-100, 100], y in [-150, 150], z in [0, 3000] |

These examples are cases `F04-POS-14` to `F04-POS-16`.

## 4. Authored model fields

| Field / USD mapping | Semantics |
|---------------------|-----------|
| `bim:id` | Immutable owned element identity |
| `bim:name` | Explicit caller label; no environment-dependent hidden name |
| `bim:elementKind` | `plate`, `beam`, `column`, `assembly`, `connection`, `level`, `reference`; independent of USD model kind |
| `bim:geometryMode` | `parametric` for create and update; other loaded modes are preserved and their edits reported unsupported |
| `bim:schemaVersion` | Explicit `0.1.0` on owned elements |
| `bim:createdBy` | Creation actor from trusted context; does not authorize future edits |
| `bim:revision` | Last accepted authored revision; absent on a never-published draft element |
| Plate contour / thickness | Explicit `steel:plate:contourPointsMm` and `steel:plate:thicknessMm`; no schema fallback supplies missing input |
| Member axis / roll | Explicit two `bim:axisPointsMm` points; optional `steel:member:rollRad` |
| Profile / material | `bim:profile` / `bim:material` relationships to pinned immutable definitions |
| Placement | Standard USD Xform operations on the element root, in authored layers |
| Engineering operations | Owned operation records (below). A create command carries no operations |

Profile and material definitions are immutable and content-addressed: they carry their full digest
and explicit units and schema version. A profile is a rectangle, one of the parametric section
families or a custom outline with cavities. A member's profile is required; material is optional.
Missing material or density never selects a guessed grade or density. Physical material and display
appearance are separate references and signatures.

Catalogue lookup, inferred defaults, position presets, curved members and direct-shape authoring are
outside this contract. Unknown USD namespaces, attributes and relationships are preserved when
editing supported fields. Unsupported schema versions may be inspected and preserved, but not
mutated without an explicit migration.

### Operation records

An engineering operation of an element is an owned **operation record**:

| Field | Rule |
|-------|------|
| `operation_id` | Full UUIDv7 of the record, unused in the project (`DUPLICATE_ID`) |
| `kind` | `hole` or `plane_cut` |
| `ordinal` | Assigned by the engine from a per-element counter in creation order; never reused, renumbered or caller supplied. It names the operation's FeatureIds (`cut:f.h<ordinal>...`) |
| parameters | `hole`: the clockwise polygon in plate-local XY, 3 to 1024 points, finite, within the coordinate cap; optional generator parameters (`round`: centre and diameter; `rectangle`: corner, width, height) are kept for editing, and the polygon is authoritative. `plane_cut`: a point and a normal in element-local coordinates and the kept side (`positive` or `negative`) |

The records of one element are ordered by ordinal and always replayed from the base shape in that
order; no stored intermediate mesh can go stale. A round hole is stored as its regular `N`-gon.
There is no designed limit on the number of operations; the limits are the mesh caps and the
kernel's implementation capacity.

**Capacity before the kernel.** Before an operation runs the kernel, the engine checks the request
against the kernel's implementation limits (hole and edge counts, triangle and certification budget
estimates). Above them the command is refused with `LIMIT_EXCEEDED`, diagnostic `field` token
`kernel_capacity` (`GEO_KERNEL_CAPACITY`). It is never worded as invalid geometry, changes nothing
and publishes nothing. A kernel budget refusal after the pre-check maps to the same result.

**Geometry refusals.** `GEO_OP_UNSUPPORTED` is `UNSUPPORTED_GEOMETRY`; `GEO_OP_NO_EFFECT` and an
invalid hole polygon (`GEO_CONTOUR_INVALID`) are `INVALID_ARGUMENT`.

### Element validity state

A part wrongly authored by the user (for example contour points in the wrong order, so the polygon
is clockwise or self-intersecting) can be kept as a **degenerate** element without derived data;
when the user repairs it, geometry and quantities are computed as for a new element.

| Topic | Rule |
|-------|------|
| State | Every owned element has validity `valid` or `degenerate`. A degenerate element keeps its inputs exactly as submitted and carries up to 32 diagnostics (code and field token). Only `valid` has derived data |
| Derived data | A degenerate element has no mesh, shape key, FeatureIds, volume, area, bounds or mass; every quantity is `unknown` with reason `shape_degenerate` |
| Which defects qualify | Only a geometric validity failure of well-typed, finite, in-cap inputs: a clockwise, self-intersecting, repeated-point, zero-area or non-planar contour, a dimension or axis below its minimum, a zero-length axis, an operation record that no longer fits after an edit. Malformed commands, non-finite numbers, cap violations, unknown targets, permission and revision failures are always refused |
| Creation | `CreatePlate` and `CreateStraightMember` take `on_invalid_shape`: `reject` (default) refuses with `INVALID_ARGUMENT` and creates nothing; `keep_degenerate` creates the element as degenerate. Interactive tools pass `keep_degenerate` |
| Update | `UpdateElement` takes the same argument. A patch on a degenerate element that leaves it degenerate is accepted, so a user can repair step by step; a patch that makes it valid evaluates geometry and quantities in the same transaction. Rename, placement and material patches are always allowed. A patch never applies a valid half |
| Operations on it | `AddHole`, `UpdateHole` and `CutByPlane` on a degenerate target are `DEPENDENCY_UNAVAILABLE`; `RemoveHole` of an existing record is allowed |
| Repair | An ordinary edit. The result equals creating the repaired element. A repair that makes an existing operation record unfit keeps the element degenerate with a diagnostic naming the record; no record is dropped silently |
| Queries | `InspectElement` shows validity, diagnostics, inputs and `unknown` quantities. Snapshots report it with null keys and no mesh. A viewport may draw the outline as an error marker; it cannot be picked and has no FeatureId |
| Numbering, IFC, BOM | No part number, no equivalence group; excluded from IFC export and from BOM and mass totals, which list it with `unknown` quantities, never zero |
| Never degenerate | A contour with fewer than three points is always refused with `INVALID_ARGUMENT`: a create creates nothing, an update applies nothing, whatever `on_invalid_shape` says |
| Publication | Publishing a workspace that contains a degenerate element is refused with `INVALID_ARGUMENT`. The diagnostics name every degenerate element (ElementId and label) with its reasons, so the UI can select the part and show what is wrong |
| Undo | Creating a degenerate element, a failed-state edit and a repair are each one undo unit; undo restores the exact previous state and validity |
| Other commands | `CopyElement` copies it as degenerate; `DeleteElement` works as usual |

A valid contour that the geometry cannot evaluate (for example outside `z = 0`) stays `valid` with
geometry `unsupported`.

## 5. Derived results and signatures

Each quantity has its own record: value (if any), unit, status, input signature and algorithm
version. Status is `current`, `stale`, `unknown` or `unsupported`. `current` requires a supported
computation over matching authored inputs and dependencies. `stale` may keep a clearly marked
previous value; consumers never treat it as current. `unknown` and `unsupported` have no value. A
declared external value is a separate record. No bounding-box volume, approximate mesh or missing
density is promoted to exact mass or volume. A volume from a certified closed shell is `current`
and carries its enclosure width.

| Change | Dependencies and invalidation |
|--------|-------------------------------|
| Rename | Label only; shape and quantities kept |
| Placement | Local shape kept; world bounds and measurements invalidated |
| Contour, thickness, axis, roll, profile or operation | Shape and its dependent quantities and features invalidated |
| Material or density | Shape kept; mass invalidated; appearance changes only if another display reference is selected |
| Camera or selection | No authored or engineering change |

Appearance is the element's resolved display reference, not its physical material: a density
change alone invalidates mass only. Key preimages and their numeric encoding are defined by the
geometry contract. A blob digest identifies saved bytes; it is not the signature of normalized
engineering inputs. No dimension rounding is used to manufacture shared hashes.

Example: a 1000 x 500 x 10 mm plate has volume 0.005 m3; with density 1000 kg/m3 its mass is 5 kg.

## 6. Logical mutation envelope

The envelope is shared by the engine API, the CLI and the host.

| Field | Rule |
|-------|------|
| `contract_version` | Exactly `0.1.0`; an unsupported version is a diagnostic, not best-effort parsing |
| `project_id`, `workspace_id` | Full IDs selecting an authorized project and owned draft |
| `op_id` | Full UUIDv7 for one atomic logical command |
| `actor_id` | Must match trusted caller context; never a credential |
| `base_revision` | Revision pinned by the selected workspace |
| `expected_generation` | Exact decimal WorkspaceGeneration precondition |
| `command` | Closed command name from section 7 |
| `arguments` | Typed command-specific object; unknown envelope or argument fields are rejected |

Draft execution compares `base_revision` with the workspace base, not with the latest published
head, so drafts work offline. Publication additionally checks the current accepted head; a changed
head needs explicit rebase or conflict handling, never silent draft loss.

Within a workspace, retrying an accepted op_id with identical canonical request content returns the
recorded result without another generation increment. Different content under that op_id is
`OP_ID_REUSE`. Authentication and authorization still apply to a replay. Canonical request content
is project, workspace, actor, base, generation, command and all arguments, without transport
metadata.

## 7. Command surface

| Command / arguments | Atomic behaviour |
|---------------------|------------------|
| `CreatePlate`: new_id, name, contour_points_mm, thickness_mm, placement, optional material_digest, optional on_invalid_shape | Validate contour, thickness and an unused ID; author one parametric plate |
| `CreateStraightMember`: new_id, element_kind, name, axis_points_mm, profile_digest, placement, optional roll_rad, optional material_digest, optional on_invalid_shape | Validate a nonzero axis and a pinned profile; beam or column is an explicit kind |
| `UpdateElement`: target_id, changes | Typed patch of name, contour_points_mm, thickness_mm, axis_points_mm, roll_rad, profile_digest, material_digest, placement; no raw USD attribute or path setter |
| `CopyElement`: source_id, new_id, name | An independent element with a fresh ID; immutable definitions and shape shared until an edit |
| `DeleteElement`: target_id | Refused while incoming dependencies remain; no implicit cascade, no foreign-source mutation |
| `AddHole`: target_id, operation_id, hole (`polygon_points_mm`, or a `round` or `rectangle` generator) | On a valid plate: validate a clockwise simple polygon and an unused operation_id, assign the next ordinal, check capacity, replay the base shape and all records; on success append the record and update derived data in one transaction |
| `UpdateHole`: target_id, operation_id, changes | Replace the parameters of one hole record, keeping its ordinal and operation_id; the whole replay succeeds or nothing changes |
| `RemoveHole`: target_id, operation_id | Remove one hole record; its ordinal is retired and its FeatureIds become dangling; the rest is replayed |
| `CutByPlane`: target_id, operation_id, plane (`point_mm`, `normal`), `keep` (`positive` or `negative`) | Add one `plane_cut` record to a valid target; a tangent or coplanar plane, several section loops or a plane that misses the body is refused and changes nothing |

Plate width and length are not command or patch fields; they are derived for a rectangle only.
`changes` is nonempty, every property is supported by the kind, and all changes form one
transaction. `material_digest=null` clears a material; absence leaves it unchanged. Clearing a
required profile is invalid. `bim:id`, paths, meshes, quantities, actor, revision and numbering
approval are never patched through this API.

Each operation command is one atomic command and one undo unit. `UpdateElement` of a contour or
thickness replays every operation record and is refused (or, with `keep_degenerate`, kept
degenerate) if one no longer fits. `CopyElement` copies the records with fresh operation_ids and the
same ordinals. `AddHole` targets plates; a hole in a beam flange or web is `UNSUPPORTED_GEOMETRY`
until the kernel supports it. Update and removal of a plane cut follow `UpdateHole` and
`RemoveHole`.

`CopyElement` takes one source. Owned operation records remap to fresh IDs; shared immutable
references stay shared. FeatureIds of the copy are addressed through its new ElementId.
Relationships outside the supported copy set are a diagnostic, never an incomplete copy. Copying a
foreign source, inventing a density or keeping a live reference to the source is forbidden.

`InspectElement` returns a revision- and generation-stamped view: ownership, explicit inputs,
relationships and per-quantity status; it never reconstructs missing input. Preview validates and
computes without publishing or advancing the generation. `CommitWorkspace` publishes a complete USD
revision.

### Naming

Every name of the command surface follows one style:

| Thing | Style | Examples |
|-------|-------|----------|
| Logical command (engine API, envelope `command`) | PascalCase, verb first, then the object | `CreatePlate`, `UpdateElement`, `AttachToAssembly`, `CutByPlane` |
| CLI command | kebab-case of the logical command, same words in the same order | `create-plate`, `update-element`, `attach-to-assembly`, `cut-by-plane` |
| Argument and option | snake_case in the envelope, kebab-case on the CLI | `contour_points_mm` / `--contour-points-mm`, `on_invalid_shape` |
| Units in a name | Suffix with the unit | `thickness_mm`, `roll_rad`, `density_kg_m3` |
| Error code | UPPER_SNAKE_CASE | `INVALID_ARGUMENT`, `DEPENDENCY_CONFLICT` |
| Geometry diagnostic | `GEO_` + UPPER_SNAKE_CASE | `GEO_CONTOUR_INVALID` |
| USD attribute | `namespace:group:lowerCamelCase`, unit suffix in the name | `steel:plate:thicknessMm`, `bim:densityKgM3` |
| Prim | Section 2 | `PRT_<uuid>` |

A read command uses the verb that says what it returns: `Inspect`, `List`, `Preview`, `Export`
(`InspectElement`, `ListDefinitions`, `PreviewNumbering`, `ExportIfc`). A new command never
introduces a second name for an existing action.

## 8. Footprints, transactions and errors

The engine derives read and write sets from command semantics and validated dependencies; caller
claims cannot narrow them. Each entry names an owned ID or immutable digest plus a semantic group:
label, shape_inputs, placement, material_ref, relationship, lifecycle or derived signature.

| Operation | Minimum footprint |
|-----------|-------------------|
| Create | Read ID absence and pinned definitions; write new lifecycle, complete authored fields and relationships |
| Update contour, thickness, axis, roll or profile | Read the complete shape-input group and definitions; write that group and invalidate dependent results |
| Update material | Read the geometry-result signature and the new definition; write material_ref and invalidate mass and appearance |
| Update placement | Read local shape and placement; write placement and invalidate world measurements |
| Copy | Read source lifecycle, full authored state, copy-set relationships and definitions; write the new lifecycle and remapped state |
| Delete | Read lifecycle and the complete incoming-relationship index at the expected generation; write lifecycle only if every dependency permits deletion |
| Add, update or remove hole; cut by plane | Read target lifecycle and validity, the shape-input group, all operation records and definitions; write the records and the shape-input group, invalidate shape, quantities, features and mass, re-evaluate derived data. A capacity refusal writes nothing |
| Change of validity | Read and write lifecycle (validity) and the shape-input group; derived data are written or cleared with the state |

Different property names alone never prove independence. If an index or required payload is
unavailable, the result is `DEPENDENCY_UNAVAILABLE`; no empty link set or current result is assumed.

Validation order: bounded decode, version and types -> trusted context and authorized workspace ->
existing op_id replay -> workspace, base and generation -> ownership, ID and relationships ->
complete footprint -> engineering validation -> apply to an isolated draft -> validate the
resulting composition -> record draft and journal. All state checks and the commit run under the
host's workspace serialization; publication is serialized with an expected-head check.

An accepted mutation advances the generation once and is one undo unit, whatever the number of
changed properties. A rejection leaves authored state, derived state and generation unchanged.
Cancellation before the apply point discards the work; after it, the recorded outcome or
`OUTCOME_UNKNOWN` is reported, and a timeout is not proof of rollback.

| Code | Meaning |
|------|---------|
| `MALFORMED_COMMAND` | Invalid encoding or JSON, duplicate keys or structurally invalid envelope |
| `UNSUPPORTED_VERSION`, `UNSUPPORTED_COMMAND`, `UNSUPPORTED_PROPERTY` | Version, operation or property outside this contract |
| `INVALID_ID`, `DUPLICATE_ID`, `NOT_FOUND` | Invalid full identity, already-owned creation ID, or missing target or definition |
| `INVALID_ARGUMENT`, `NON_FINITE_NUMBER`, `LIMIT_EXCEEDED` | Bad field, type or range; non-finite f64; bounded-input cap violation |
| `PERMISSION_DENIED`, `FOREIGN_READ_ONLY` | Untrusted or mismatched caller; attempted foreign edit or override |
| `BASE_REVISION_MISMATCH`, `STALE_WORKSPACE`, `OP_ID_REUSE` | Failed revision or generation precondition; changed request under an accepted op_id |
| `DANGLING_RELATIONSHIP`, `DEPENDENCY_CONFLICT`, `DEPENDENCY_UNAVAILABLE` | Invalid link; forbidden delete or change; incomplete validation inputs |
| `UNSUPPORTED_GEOMETRY`, `INTEGRITY_ERROR` | Shape or transform outside scope; revision or blob bytes inconsistent with their identity |
| `CANCELLED`, `OUTCOME_UNKNOWN`, `BACKEND_UNAVAILABLE` | Cancelled before commit; uncertain committed outcome; required backend unavailable |

**Operation-command failures.** `GEO_OP_UNSUPPORTED` and a body outside the kernel's supported
class are `UNSUPPORTED_GEOMETRY`; `GEO_OP_NO_EFFECT`, an invalid hole polygon and a plane that
leaves the body unchanged or empty are `INVALID_ARGUMENT`; `GEO_BUFFER_LIMIT`, `GEO_TOPOLOGY_LIMIT`
and `GEO_KERNEL_CAPACITY` (field `kernel_capacity`) are `LIMIT_EXCEEDED`.

Errors return code, safe message, field and target diagnostics and the observed workspace token when
known; never credentials, private paths or client data. Multiple failures follow the validation
order; tests never depend on which untrusted collection entry is visited first.

## 9. Out of scope

- Automatic environment, default or catalogue resolution: definitions are pinned explicitly.
- Position presets: member placement is an explicit rigid pose plus a local axis.
- Vendor quantity tables: quantities use geometric volume or declared section area and explicit
  density; an unknown density stays unknown.
- Edits of foreign data: annotations on a foreign source are separate owned objects.
- Numbering and part equivalence.

## 10. Acceptance cases

The [command cases](../../tests/fixtures/contracts/f04-model-command-cases-v0.1.json) are the
positive and negative acceptance descriptions: 69 logical cases (30 positive, 39 negative) and
seven malformed or non-finite raw JSON cases. They run through the real API -> CLI -> USD ->
independent reader path; `apps/beamcraft-cli/tests/f04_cases.rs` runs them against the engine.
Member cases bind an actual validated profile digest.

**Comparison rule.** Quantities pass when `|actual - expected| <= 1e-6*|expected|`; lengths and
coordinates when `|actual - expected| <= 0.001 mm + 1e-6*|expected|`; dimensionless frame values
use absolute 1e-6 at zero and relative 1e-6 otherwise. Identities, strings, booleans, counts,
generations and codes compare exactly. This is a test rule only, separate from geometric decisions.

Non-finite quaternion and translation components follow the raw-JSON rules: a bare `NaN` is
`MALFORMED_COMMAND`, an overflowing literal such as `1e400` is `NON_FINITE_NUMBER`.

## 11. Version

This is version **0.1.0**. A change to it is a new version or a recorded correction; implicit changes
through code or reviews are not allowed. Earlier stores with another prim layout are not read. The CLI
renaming to the naming rule is [issue #144](https://github.com/Apibim-com/BeamCraft/issues/144).
