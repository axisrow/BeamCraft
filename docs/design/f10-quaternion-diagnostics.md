# F10 quaternion diagnostics

Non-finite placement quaternion components report `NON_FINITE_NUMBER` with domain rule
`DV-ATTR-003`, as required by the F10 domain vocabulary. This applies both to
`Placement::validate_orientation` (including `UpdateElement` argument admission) and to
quaternion encoding in `UpdateElement::content_fingerprint` when called directly.

The error field remains `placement.orientation_xyzw`. Finite quaternion admission and accepted
command fingerprints are unchanged. The existing engine/CLI diagnostic propagation consumes the
model rule ID. A regression test checks NaN and both infinities in each quaternion component
through all three entry points. This is a focused follow-up to #404, not a new F10 acceptance claim.

Contributor-original MPL-2.0. No dependencies or assets are added.
