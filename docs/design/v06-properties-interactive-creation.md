# V06 modelling properties and interactive creation

Issue #546, owner scope clarified on 2026-10-29. This updates the delivered native
inspector and creation tools on the existing engine/CLI paths.

## Properties

The selected part exposes its name, class, material, supported thickness and placement,
with Apply/Reset and a Reload action. A level is selected by name and elevation from the
same pinned `inspect-elements` response. Assignment/removal uses `update-element --level`
and changes no geometry, placement or quantity. A missing previously assigned level is
shown as unavailable and is not silently removed. Choices are bounded and validated.

Quantities retain current, stale and unavailable states; an unknown mass is not zero.
Stale-property protection and shared-definition impact notices remain. Commands, argument
lists, revision hashes and sequence numbers no longer appear in normal property, create,
move/copy/delete or publication confirmation flows. Those internal adapters still stage an
exact validated request before running the CLI; this is no longer a user-facing ceremony.

## Creation

Selecting Beam or Column starts point picking. Set the profile/material/class and pick
start/end; the second point creates one element through `create-member`. The tool remains
available for the next member after success. Exact endpoint fields and coordinate expressions
are optional precision input. The ghost uses the same resolved/snap-constrained endpoint
as the final click. An axis remains visible when no supported body preview is available.

Selecting Plate starts an unpublished spatial contour, with the active work plane or world XY
as the initial free-point fallback. Set thickness/material/class and pick vertices. The outline
follows the pointer. Type a length and press Enter to add a point in the aimed 3D direction;
O toggles orthogonal X/Y/Z. Enter without a length, right-click, wheel-click, Finish plate or clicking the
first vertex closes a contour with at least three vertices and runs `create-plate --contour`.
Backspace removes a typed digit or vertex; Reverse direction
explicitly reverses winding. Precision fields add a point in work-plane coordinates.
Interactive contours pass `--normalize-winding true`: the engine normalizes either local XY
traversal before evaluating the plate, retaining the first vertex. No intersection is repaired.
The CLI keeps its authored-winding default for existing scripts; the opt-in mode refuses corner
treatments whose vertex indexing would otherwise change. Real CLI coverage checks both traversals
of a concave contour in world XY and a vertical frame, and unchanged revisions on invalid outlines.

The existing engine admits contour topology, winding, thickness, supported geometry and
limits. The UI repairs no contour or intersection and caps draft vertices at the model limit.
For inferred spatial frames the engine chooses a consistent normal hemisphere, so reversing
the traversal does not flip the thickness side; an explicit working plane retains its normal.
A refusal leaves points available for correction. Cancel, Esc or switching tools discards
unfinished points and starts no command. Once saving has started it is an ordinary undoable
operation, with existing failure/unknown-outcome handling. Input is disabled during saving.
Work-plane changes discard points from the previous frame. Picking also works before the
first scene exists, so the first object can be created in an empty project.

Snap kinds come from the bottom toolbar; queries use the displayed snapshot and grid records.
Source masters and point kinds must both permit a candidate. The grid source never enables
nearest points by itself; [the switch matrix](v11-construction-snaps.md) defines every current combination.
Hidden grids are excluded. Spatial snaps use a cursor ray; the engine infers the contour plane
from non-collinear points and refuses non-planar contours without flattening them. The session
plane stays unchanged. No magnetic link is
created, and view state does not write a revision.

## Verification and boundaries

Native-adapter acceptance drives plate/member controllers through real CLI processes, including
empty-model creation, cancellation, a final point that starts saving, and UI text without raw
commands. Real-binary suites compare a concave plate on a rotated work plane with a direct CLI
request and independent expected volume; a crossing contour publishes nothing and retains its
draft. Property coverage verifies named level choices, reset, assignment/removal and unchanged
placement, mesh and quantities. Existing stale-property, publication and workspace checks remain.

Existing plate contour grips, changing a member's profile/axis through properties, advanced
column placement, new profile/material editors and new engineering capabilities are separate
work. Full global V11 snap-tool coverage remains its own task; this change uses the existing
coordinate query paths in these creation tools. No new dependency or shipped asset is added.
The UI, code and synthetic tests are contributor-original MPL-2.0; no commercial product's
layout, icons, screenshot or code was copied or traced.

## Selection outline

The cyan highlight follows the full evaluated body's face boundaries, including the diagonal
side of a triangular plate and any evaluated cut boundary. It never substitutes a world-aligned
bounding box. The engine's `query::feature::element_edges` removes duplicate shared edges and
excludes coplanar triangle diagonals, with a 65,536-edge cap. The CLI exposes the same result as
`element-edges --store ROOT --id UUIDV7`. The UI caches world segments by selected identity and
snapshot token and invalidates them on reload. Unsupported semantic bodies have no invented
outline. The overlay retains the existing through-model drawing behavior; it is not an occlusion
or silhouette query. CLI and native projection tests verify nine actual triangle-prism edges
without the absent fourth rectangle corner.

## Review hardening

A head that could not be checked disables property editing and Apply and is refused by
the property adapter itself. The first draft marker closes a contour within ten screen
points, regardless of engine vertex-snap switches or zoom. Right-click and wheel-click
finish a contour with three or more points; dragging the wheel remains navigation. In a
member tool these clicks end the tool and discard an unfinished first point.

Saving blocks tool changes/Esc so an in-flight command cannot erase its draft before an
unknown outcome is reported. Contour text observes the existing CLI's 4096-byte argument
budget before staging. Duplicate level/definition labels gain a unique identifier suffix
only when needed to tell choices apart. Failed member preview resolution clears the
visible preview and its coordinate cache. Stored quantity freshness is shown separately
from newly evaluated current quantities; a stale stored record is never labelled current.
