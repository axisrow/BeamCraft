# A12.1 plate thickness side

Contributor-original BeamCraft code and documentation, MPL-2.0.

A plate carries the new input `thickness_side`: `top`, `middle` or `bottom` (the F04 plate
amendment draft, section 1). The contour lies in the element-local plane `z = 0`; the side says
where the thickness `t` is laid off from that plane. `top` is the default and keeps every older
plate: absence of the input, of the patch field and of the stored `steel:plate:thicknessSide`
token all read as `top`, and the `top` evaluation path is bit-identical to A02. `middle` puts
half the material on each side of the contour plane, `bottom` lays it off in `-Z`. The side is
the axis of the element-local frame, so a rotated placement (P10) turns it with the plate.
Volume, area and mass do not depend on it; the mesh, its z band and the shape signature do.

`create-plate` takes `--thickness-side top|middle|bottom`; `update-element` patches it, and the
patch rebuilds the body like a thickness change. A copy carries the side of its source, an
instance shares it with its canonical part, and the schema descriptor gains the optional token
attribute. The engine lays the body off by translating the plain kernel mesh with the production
rigid map and re-certifying it with its own face table, normals and tolerance; hole cutters span
the laid-off band and plane-cut records replay on the laid-off body, so the same record removes
different material for a different side.

Numbering normalises the side away (owner decision 2026-10-23): the A08 facts are the outline,
dimensions and holes, which do not depend on the side, so two plates differing only in
`thickness_side` share one number. Plane-cut records are not part of the A08 key today, so no
key coordinate needs shifting in this slice; the gross signature also stays side-independent
because the gross volume does not depend on the side.

Tests: the engine checks the z ranges `0..t`, `-t/2..t/2` and `-t..0` from the mesh, equal
volumes, the side-blind default, a contour plate's laid-off body, and that a fixed plane cut
removes different material per side while a through hole pierces the laid-off band. The CLI end
to end creates the three sides, reads them back through a fresh process, checks the saved token
attribute and the saved mesh's z bands, re-lays a plate off with `update-element`, and previews
one numbering group for all three plates.
