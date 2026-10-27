# A12.2 plate corner treatment

Contributor-original BeamCraft code and documentation, MPL-2.0.

A contour vertex may carry a corner treatment (the F04 plate amendment draft, section 2): a
`chamfer` with one or two cut distances, or a `round` with a radius. The authored contour keeps
its sharp corner points; the treatment is an authored parameter, so a rounded corner stays
editable by its radius and is never a stored polygon. An empty treatment list is the default and
keeps every older plate unchanged: no attribute is stored, no signature or label moves, and the
evaluation paths of A02 and A12.1 are bit-identical.

`create-plate` takes `--corners "V=chamfer:D1[,D2];V=round:R"` (a single chamfer distance is the
symmetric cut; the typed command carries both distances explicitly). The evaluator derives the
treated outline in `f64` before the extrusion: a chamfer replaces the vertex with its two cut
points, a rounding cuts the corner with the arc of radius `r` tangent to both edges, faceted to a
chord deviation of at most 0.05 mm (the round-hole rule of F06 A-2 D-2, at least two segments,
exact tangent end points), and reports the largest kept deviation as `arc_deviation_mm`. The
kernel then validates the derived polygon with its exact predicates, as for any contour.

A treatment is refused (`INVALID_ARGUMENT`, field `corner`) on a vertex whose edges are collinear
or fold back onto each other, and when the lengths two neighbouring treatments consume from one
edge reach the edge length; convex and concave corners are both allowed. Values must be finite,
positive and within the F04 coordinate cap; each vertex is named at most once. A derived outline
that would need more points than the F04 contour cap is `LIMIT_EXCEEDED`. A treated origin
rectangle is no longer the G02 box: it evaluates through the contour path and carries the new
volume label `kernel-a12-corner-treatment-volume/1;tolerance-policy-3`, because the quantities
are those of the faceted body and an analytical area of the rounded shape is never claimed.

The five parallel attributes `steel:plate:cornerVertex/cornerKind/cornerD1Mm/cornerD2Mm/
cornerRadiusMm` are authored together or not at all, so copies, instances and rebases carry the
treatments, and a reopen in a fresh process reads the same parameters and the same mesh. Holes
and plane cuts replay on the treated outline, so a hole outside the treated plate is refused as
before. The A08 outline fact is now the evaluated outline: two plates whose treated outlines
differ share no part number, and without treatments the fact is the authored contour as before.
Quantity signatures gain the treatment block only for treated plates, so stored records of older
plates stay current. Corner editing commands (`SetPlateCorner` and the rest) are the A12.3 slice;
`UpdateElement` does not patch treatments yet.

Tests: the engine derives the amendment's area cases 2 to 5 (symmetric and asymmetric chamfer,
rounding within the reported deviation, a concave corner) and refuses the negative cases 1 to 3
plus the fit and point-cap bounds; the CLI end to end creates treated plates, checks the saved
volumes, deviations, corners and mesh digests through fresh processes, verifies every refusal
changes nothing, and shows copy, numbering, undo and redo carrying the treatments exactly.
