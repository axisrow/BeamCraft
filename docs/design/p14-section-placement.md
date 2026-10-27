# P14 section placement about the member axis

Contributor-original BeamCraft code and documentation, MPL-2.0.

A straight member's section (beam or column) can now sit anywhere about its axis. `create-member`
authors an optional `SectionPlacement` (P14): where the section's bounding-box centre sits in the
rolled element-local section plane and where the solid physically starts and ends along the axis.
The axis points stay the modelling line; the placement moves the material.

## What is authored

* `plane` (`left|centre|right`) plus `plane_offset_mm`: a shift along the rolled local `X_m`
  (the section width of the F06 section 4 frame). `left` puts the bounding box in the `-X_m`
  half (`-w .. 0`), `right` in `+X_m` (`0 .. +w`), `centre` straddles the axis; the offset adds
  to the half shift.
* `depth` (`front|centre|back`) plus `depth_offset_mm`: the same along the rolled local `Y_m`
  (the section height). The names are frame-relative, never world directions: a rotated
  placement or a roll turns them with the member.
* `start_offset_mm`, `end_offset_mm`: the solid spans `p1 + start*u .. p2 + end*u` along the
  unit axis `u`, so its length is the axis length moved by `end - start` and must stay positive
  (`GEO_AXIS_TOO_SHORT`, field `section_placement`, otherwise).
* Rotation about the axis stays the existing `roll_rad`; it composes with the position the same
  way it composes with the frame. No second rotation input exists.

The default (every field absent) is the centred, unshifted body: the kernel keeps the P03 vertex
expressions bit for bit, so mesh bytes, digests, quantities and signatures of a member authored
without the fields are unchanged — the acceptance "the default placement unchanged". A member
authored before the field existed reads as the default (`steel:member:*` attributes all absent).

## Where it lives

`SectionPlacement` (model) → `CreateStraightMember.section_placement: Option<..>` (fingerprint
`CreateStraightMember/v5`) → `AuthoredMember` → the kernel builders (`StraightMember`,
`SectionMember`) move the box origin in the local section plane and extrude over the net length;
volumes are `effective area x solid length`. The body stays element-local: the rigid placement
(Xform) still maps it to the scene, unchanged for bounds, features, picking, IFC and the
snapshot. Columns share the whole path; the kind never enters the geometry.

Storage is six optional USD attributes, all six or none (schema descriptor extended;
`absent = centred, unshifted`): `steel:member:planePosition|planeOffsetMm|depthPosition|
depthOffsetMm|startOffsetMm|endOffsetMm`. Older pre-alpha stores are refused without migration
(the descriptor digest changes), the standing pre-alpha policy. Copies and instance copies
author the same records; body-cache keys (`Member/v4`) and the P-Mass gross signature
(`MemberGross/v2`) include the placement.

## Not done (not claimed)

* No `update-element` patch and no `Change::` variant changes the placement of a committed
  member; it is a create-time input like the roll today.
* Per-end section positions (different plane/depth/rotation at each end, tapered or twisted
  bodies) are ruled-surface territory (G13), not a rigid extrusion; both ends share one
  section position, only the axis offsets differ per end.
* The offset caps are the model admission (1e6 mm) re-checked by the kernel; there is no
  minimum solid length beyond positivity.
