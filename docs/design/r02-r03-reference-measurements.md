# R02–R03 foreign-reference measurements and updates

Status: the R02 engine seam, CLI read command, and R03 explicit source update/comparison are
implemented experimentally. Persisted W03 result lifecycle remains follow-up work. Neither K3 nor
the R02/R03 plan rows are accepted by this note alone.

## R02 result contract

A measurement is BeamCraft-owned data over one exact foreign source pin and one wrapper revision.
It never authors a property or annotation into the referenced layer. Its identity includes the
source digest and version, wrapper identity, and a signature of the transformed triangle buffers.
Placement or units correction therefore changes the input signature; a source update changes the
digest/version and cannot silently reuse an old result.

The first supported scope is triangle-shell surface area and enclosed volume. `reference-measure`
extracts triangle-only USD mesh arrays from the verified pinned snapshot, then applies
`ReferenceRecord::source_point_to_world`, including the explicit units interpretation and wrapper
placement. Mesh prims with authored transform ops, non-triangle faces, malformed arrays, and
geometry outside the G06 caps are reported unsupported with no values. This initial adapter does
not claim arbitrary USD transform-stack evaluation or polygon triangulation.

G06 shell certification gates both values. A certified shell returns outward-rounded enclosures
in mm² and mm³, labelled as mesh results. Open, inward, non-manifold, self-intersecting, or
numerically uncertain shells return no area or volume. Foreign mesh volume/area never becomes a
current F06 engineering quantity; density never produces mass. No tolerance is caller-selected:
G06's versioned geometric policy decides certification, and F04 section 10's test comparison is
not a product tolerance.

The engine seam is `beamcraft_engine::reference_measure::measure_reference_mesh`. It takes a
bounded, already validated `TriMesh`, transforms its vertices using the R01 record, re-certifies
the transformed shell, and returns the source pin, input signature, shell status, enclosures and
`unsupported` engineering-quantity status. The CLI returns a result for every supported foreign
mesh, with source path/digest/version, input signature, enclosures, and explicit null mass. These
results are presently read-only command output; saved W03 result objects remain follow-up work.

## R03 explicit source update

`reference-update` takes a new local source path and the expected current source digest. It reads
and validates the new USD snapshot under R01's byte and structure budgets, then commits a new
wrapper revision with incremented source version and the new digest. It does not modify foreign
content, resolve network paths, or update on file-system change. Existing measurements remain
immutable and become stale because their digest/version no longer match; a units correction is
cleared and must be explicitly applied to the new pin. The command reports added/removed prims,
changed typed properties, and changed triangle buffers for supported static USDA. Binary USD and
unsupported mesh geometry produce explicit unsupported comparison status, never equality. Replay
returns the recorded update without reapplying it.

## Evidence before acceptance

Required end-to-end fixtures include a closed triangle shell with independent area/volume values,
metres and Y-up correction, wrapper rotation/translation, open and self-intersecting shells,
malformed and over-budget arrays, and a changed source that preserves the old result while making
it stale. Tests must prove the foreign layer bytes remain identical and the model is not modified
by measurement. CLI source update and comparison tests now cover changed triangle geometry,
retained old source bytes, version increment, cleared units correction, and exact replay. W03
lifecycle coverage and broader measurement fixtures remain outstanding, so R02/R03 stay open.
