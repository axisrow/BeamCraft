# T02 construction objects

Contributor-original BeamCraft code and documentation, MPL-2.0.

Construction points, finite line segments, circles and infinite planes are ordinary owned
`helper` elements under `/World/Helpers/HLP_<uuid>`. Their immutable category is distinct from
parts. `bim:helper:kind` is an explicit token, and `bim:helper:parameters` is a bounded double
array. Parameters are world coordinates in millimetres:

| Kind | Parameters |
|---|---|
| point | X, Y, Z |
| line | start X, Y, Z, end X, Y, Z |
| circle | center X, Y, Z, normal X, Y, Z, radius |
| plane | origin X, Y, Z, normal X, Y, Z |

Normals need not be unit length but must have length at least 1e-12. Coordinates are finite
and bounded by 1e9 mm; segment length and circle radius are bounded by 1e6 mm, with a 0.001 mm
minimum. A circle's conservative center-plus-radius bounds must fit the coordinate cap.
One model contains at most 1024 helpers. Labels follow the existing model label rules.
The stored placement is the identity; helper coordinates have no hidden transformation.

`HelperGeometry` admits the shape in the model crate. `engine::helper` admits creation against
the pinned model's used identities and computes exact construction snaps. The USD writer
rechecks admission and refuses material links, solid Geometry meshes, numbering marks and
part/level assignments on helpers. Helpers are excluded from `Inspection.elements` and the
plate/member collections used by BOM and numbering. Creating a helper adds no engineering
mesh, quantity or definition and changes no existing part key.

## Headless interface

```
beamcraft create-helper --store ROOT --name Datum --kind point --parameters 10,20,30
beamcraft create-helper --store ROOT --name Baseline --kind line --parameters 0,0,0,1000,0,0
beamcraft create-helper --store ROOT --name Circle --kind circle --parameters 0,0,0,0,0,1,250
beamcraft create-helper --store ROOT --name Plane --kind plane --parameters 0,0,100,0,0,1
beamcraft helpers --store ROOT
beamcraft snap-helper --store ROOT --point 11,20,30 --tolerance 5
beamcraft snap --store ROOT --point 11,20,30 --radius 5 --kinds helper
beamcraft delete-element --store ROOT --id HELPER_UUID
```

Creation uses the common command identities, footprint validation, strict publication, replay
ledger and workspace undo unit. Deletion uses the existing `DeleteElement` engine footprint and
layer writer. Reusing an operation with different creation inputs is refused. Helpers are read
from the exact revision used by a snap query. `snap-helper` returns its revision pin and a
qualified helper/role/point result; the combined `snap` returns a `helper` candidate with the
helper's stable element ID, no engineering FeatureId, and the query snapshot token.

The engine projects onto finite segments and planes and uses the analytic circle circumference,
with its center also available as a snap. Ties in the nearest helper query use identity order.
Combined queries use the existing bounded distance/kind/identity ordering and result truncation.
A point directly on a circle's normal axis has no unique circumference snap; its center remains
available. These queries never use display tessellation.

## Native interface

The Helpers tool uses the same CLI command with Preview, Commit and Cancel. Inputs are explicitly
world coordinates. Helper metadata travels in the existing framed refresh beside definitions,
from the same verified inspection as the mesh bundle; no extra process is started. The UI admits
all records again. It only uses helper snaps or markers while that metadata's manifest pin
matches the displayed snapshot. Failed or stale refreshes cannot snap to a newer hidden model.

Helpers are cyan view overlays: crosses for points/centers, segments for lines, 64 display
segments per circle, and a 1000 mm square marker for a plane. Plane snapping remains infinite.
Display bounds include the markers, giving helper-only scenes a floating origin and Fit framing;
the snapshot's engineering bounds remain unchanged. Overlays are visible through solids and do
not contribute pick triangles or measurements. The create inspector has a helper snap switch
and a world-distance radius. Member clicks and ghost endpoints use the same helper snap API;
with a work plane active, off-plane candidates are refused. Deletion is available headlessly.

The schema pin changes; older pre-alpha stores are refused rather than migrated, following the
repository's existing schema compatibility policy. No dependency, asset or external data is added.
