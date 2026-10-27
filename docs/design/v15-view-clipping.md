# V15 non-destructive view clipping

Contributor-original Rust and synthetic fixtures, MPL-2.0.

`set-view-clipping --workspace-dir DIR --id UUID` patches explicit world half-spaces
(`--planes PX,PY,PZ,NX,NY,NZ;...|none`), work area, or `--depth-enabled true|false`.
At least one property is required. `clear-view-clipping` clears custom planes and work area
and disables construction-depth clipping. Both support W01's expected-file-digest CAS.
The visible side satisfies normal dot (worldPoint - planePoint) >= 0.

The engine admits at most eight user planes, normalized finite nonzero normals and bounded
points. Effective state additionally expands a work-area box into six half-spaces and, when
enabled, a constructed view's full depth band into two. The effective maximum is sixteen.
The pure read query conservatively removes only whole AABBs proven outside a half-space;
intersecting elements retain their complete local meshes. Actual triangle clipping belongs
to a display consumer using those same plane parameters. No engineering body, measurement,
quantity, numbering result or model token reads the clipped display mask.

`export-view --store PERSONAL_WORKSPACE --view UUID --out FILE|-` emits `BCVXFRM3`:
eight magic bytes, then four little-endian u32-length-prefixed JSON sections (definitions,
selected-element inspection, grids, presentation), then the original engineering bundle.
Unrequested sections are zero length. Presentation includes the verified model/snapshot pin,
personal-file version, resolved camera, effective planes and sorted visible element IDs.
Unresolved source views are refused; no stored-camera fallback is exported. FILE output stores
the same frame and reports presentation in its JSON summary. Existing exports without `--view`
retain their raw bundle/BCVXFRM1/BCVXFRM2 formats and ordinary summary shape. Native consumers
must explicitly implement V3 before requesting it; native UI integration is outside this scope.

Tests compare the framed engineering bundle byte-for-byte with the legacy export, check the
mask and expanded planes, clearing, file output, unchanged full model inspection and BOM,
and run the five existing export-view tests. Engine tests check depth/box half-spaces and
invalid plane admission. Rendering acceptance is not inferred from these headless results.
