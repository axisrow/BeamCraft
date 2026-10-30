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

Selecting Plate starts an unpublished contour on the active work plane, or world XY. Set
thickness/material/class and pick vertices. The outline follows the pointer. Enter,
right-click, wheel-click, Finish plate or clicking the first vertex closes a contour with at least three
vertices and runs `create-plate --contour`. Backspace removes a vertex; Reverse direction
explicitly reverses winding. Precision fields add a point in work-plane coordinates.

The existing engine admits contour topology, winding, thickness, supported geometry and
limits. The UI repairs no contour or intersection and caps draft vertices at the model limit.
A refusal leaves points available for correction. Cancel, Esc or switching tools discards
unfinished points and starts no command. Once saving has started it is an ordinary undoable
operation, with existing failure/unknown-outcome handling. Input is disabled during saving.
Work-plane changes discard points from the previous frame. Picking also works before the
first scene exists, so the first object can be created in an empty project.

Snap kinds come from the bottom toolbar; queries use the displayed snapshot and grid records.
Hidden grids are excluded. Plate candidates must lie on the work plane. No magnetic link is
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
