# P13 circular-path members

The model command `CreateCurvedBeam` wraps the shared bounded member inputs, with exactly
three element-local points: start, on-arc, end. They determine the circle, plane and travel
side; they stay authored parameters and are never replaced by sampled points. Its request
fingerprint has its own domain, so a three-point polybeam cannot replay a curved command.

## API and CLI

Engine `member::create_curved_beam` prepares the complete result without external state.
CLI `create-curved-beam --store ROOT --points "X,Y,Z;X,Y,Z;X,Y,Z" --profile-digest SHA256`
uses the existing member name/class/material/placement/identity options. Coordinates are
absolute element-local millimetres, as in P12; no new expression parser is introduced.

Alternatively supply all of `--start X,Y,Z --end X,Y,Z --radius MM --normal X,Y,Z`.
`member::arc_from_radius` constructs the **minor arc** with the on-arc point on the
`normal × (end-start)` side of its chord. The normal must be nonzero and perpendicular
to the chord; negating it selects the other side. Radius must be at least half the chord.
Major arcs are specified by three points. Mixing the two input forms is refused.

The existing `update-element --axis-point "START;ON;END"` replaces all three arc points.
Non-default P14 section placement is currently supported only by straight members;
circular members explicitly refuse it instead of ignoring offsets or extending their length.
The arc kind remains explicit: replacing them with two or four points refuses atomically.
Use independent creation for a different path family. No new roll rule is invented;
sections are parallel-transported by G12 and authored roll on an arc is refused.

## Quantities and representation

G12 returns the true circular path length from its existing circle solve (`R × angle`),
independently of its chord tessellation. Uncut engineering quantities retain P-Mass:
effective section area times **true arc length**, with catalogue/computed area provenance
and density rules unchanged. A replayed cut uses the certified cut-body volume, as before.
The distinct algorithm label makes obsolete derived records stale.

The mesh is G12's certified sweep with a requested **0.05 mm path-surface chord bound**.
`arc_deviation_mm` reports the achieved bound for transport of the section's detailed
polygon outline; it does not claim that the original section's own rounded corners have
that same bound. `axis_length_mm` reports the true path length. Geometry consumers use
the certified mesh; its faceted volume is checked separately against analytical references.
The existing radius, dimension, mesh-vertex/triangle and segment caps remain enforced;
an unreachable deviation is refused, never silently coarsened. No cross-platform byte
identity is claimed for trigonometric functions.

## Storage, history and compatibility

Owned member USD uses optional token `steel:member:pathType = "arc"` with exactly three
`bim:axisPointsMm`. Missing token retains the old straight/polyline interpretation.
Unknown tokens and an authored roll on an arc refuse. This extends the schema descriptor
and changes its pin; unsupported old pins refuse explicitly without rewriting the store.
The native validator, authoring/capture, rebase shape group, P11 shared-property resolver,
independent copy, decoder, body cache and quantity signatures all include the path kind.
Existing straight/polyline request and quantity fingerprints retain their encoding.

One accepted creation or point replacement publishes one revision and is one undo unit.
Creation retries read immutable evidence and the exact accepted revision, including after
later edits or deletion; uncertain outcomes request recovery. Bad input publishes nothing.
Supply both `--op-id` and `--new-id` for an exact CLI retry.

## Integration limits

This PR reuses open P12 PR #257 and existing CI fix #262, and includes current master's D08.
It does not recreate their implementations. Merge those prerequisites before this slice.
The CLI currently refuses numbering when circular members are present: the existing A08
member facts compare only length and roll and cannot establish equal curved shape. The
planned M2/new-geometry integration must provide path-aware numbering facts; reporting
that two different bends are equal would be incorrect. BOM and mesh-based export use the
true length/quantities and certified body. No UI or complete M5 gate is claimed here.

## Provenance and checks

Original Rust and synthetic analytical fixtures, MPL-2.0, using elementary circle geometry
and the existing G12 construction. No additional dependencies or third-party assets.
Engine and real CLI tests cover analytical length/volume, achieved deviation, explicit
path identity, cache/signature separation, radius/plane admission, immutable retries,
copy/instance/reopen, invalid input without publication, point replacement and undo/redo.
Native tests cover token persistence, unknown tokens, invalid point count/roll and pin
refusal. Actual command results are recorded in the PR, not inferred from this note.
