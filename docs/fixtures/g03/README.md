# G03 original kernel fixtures, set version 1

Contributor-original, BeamCraft contributors, 2026-10-16, SPDX `MPL-2.0`.
Base: `d3c20ddd4a2588662823f6e56f42935d1cf6bf5b` (G02 PR #39).

These 56 descriptions exercise the real `beamcraft-kernel` rectangular primitive.
They are test descriptions, not saved models, a profile catalogue or a new engine contract.
No fixture, dimension, expectation or implementation comes from another kernel, commercial
detailing product, vendor catalogue, observation or client model.

| Support | Count | Meaning |
|---------|-------|---------|
| `supported` | 22 | Origin rectangles evaluated by the public kernel API |
| `invalid` | 26 | Real calls must reject with the declared F06 diagnostic list and status |
| `unverified` | 4 | Large translations: exact ideal references only; no available kernel adapter |
| `unsupported` | 4 | Reserved hole, cavity, thin-wall and nonrectangle descriptions; no execution |

Large-coordinate cases remain unconfirmed, separate from successful origin cases. The G02 API
accepts dimensions only. The harness never translates kernel vertices and calls that a kernel
placement test. No rotation, quaternion, member, CLI, USD or UI feature is added. A thin solid
rectangular plate at the accepted minimum is supported; a wall surrounding a void remains reserved.

## Files, format and provenance

The existing `ATTRIBUTION.md` entry covering `docs/fixtures/**` covers these original analytical
values and documentation. This dated record and the manifest specify the G03 files, author,
version, licence and use within that existing path attribution. All new non-code assets stay
inside that attributed path; harness and writer source are code under their SPDX notices.

| Path | Author and source | Use |
|------|-------------------|-----|
| `parameters/*.tsv` | BeamCraft contributors; original synthetic choices, set 1 | 56 bounded input descriptions |
| `manifest.tsv` | Same; set 1; exact-byte SHA-256 | Stable IDs, file paths, byte lengths, provenance, support and plan references |
| `expectations.tsv` | Same; exact rational evaluation of the formulas below | Separate outcomes, diagnostics, volumes, bounds and topology counts |
| This README | Same; version 1, frozen F06 0.1.0 and original mathematical derivations | Schema, provenance, scope and reproduction instructions |

All files use UTF-8 without BOM and LF. TSV means literal tab separators, one case per record,
with leading `#` comment lines. The format is deliberately independent of the F07 JSON decoder:
F07 uses older caps, arbitrary plate rotations and an explicit-up member convention. No F07
fixture changes or implicit reinterpretation are made here.

`parameters` fields: `id`, `mode`, `width`, `length`, `thickness`, `origin_x`, `origin_y`,
`origin_z`. Units are mm. `rectangle` means an axis-aligned positive extrusion; `placeholder`
reserves a future case only. The origin is a translation, never part of the local-size scale.
Special spellings `nan`, `inf`, `-inf` represent direct f64 API probes, not JSON tokens. Decimal
tokens convert to nearest binary64; `-0.0` intentionally preserves its sign as input.

Manifest columns are stated in its header. Each ID is `g03.plate.<stable-name>`; a semantic change
requires a new ID and fixture-set version. Every entry has author, SPDX licence, support and plan.
The harness checks all 56 byte lengths and full digests, unique IDs/paths, one-to-one expectation
coverage, schema sizes, support counts and absence of unlisted parameter files.

Expectation fields: ID, outcome, ordered diagnostics, exact volume, minimum bounds, maximum bounds,
counts, sources. Diagnostics are comma-separated `Stage:field:GEO_CODE`. Counts are ordered
`vertices,topological_edges,faces,triangles,mesh_edges,shells,euler`. `-` means no applicable value;
`not_run` cannot pass as a successful kernel result. Unsupported placeholders carry no invented
numerical values or diagnostics for an API that does not exist.

## Derivations and citations for every expectation

`F06:N` in a row cites section N of the frozen
[F06 geometry contract](../../contracts/f06-geometry-contract-v0.1.md); the following table resolves
the mathematical formula labels in the same row. The normative order is F06, including when older
G04 design examples retain superseded tolerances. No production output is used as an expectation.

| Expected value / source label | Derivation and authoritative source |
|-------------------------------|-------------------------------------|
| `MATH:box-product`, volume | The Cartesian product of intervals of lengths a,b,c has volume abc: integrate 1 over each interval, giving a*b*c. F06 section 2 requires the binary64 result `(a*b)*c`, then division by exactly 1e9 for m3. |
| `MATH:cartesian-bounds`, bounds | A point belongs to `[o_x,o_x+a] x [o_y,o_y+b] x [o_z,o_z+c]`. Independent minima/maxima are the interval endpoints. F06 sections 2/7 define computed bounds and spaces. |
| `MATH:euler`, counts | A box has 2 endpoint choices on each of 3 axes: 8 vertices; each of 3 edge directions has 4 endpoint combinations: 12 edges; 3 axis directions have 2 faces: 6 faces. Each quad splits into 2 triangles: 12 triangles and 6 additional diagonals, so 18 mesh edges. Both Euler sums give 2: 8-12+6 and 8-18+12. There is one connected shell. F06 sections 6/10 fix these counts. |
| Status and diagnostics | F06 sections 2/3/4: finite input first, submitted upper caps second, then local scale, positivity and inclusive minima. First failing stage only, every failure in input order. Nonfinite maps to `NON_FINITE_NUMBER`, above-cap to `LIMIT_EXCEEDED`, nonpositive/subminimum to `INVALID_ARGUMENT`. |
| Minimum decisions | F06 section 4: for positive origin edges D=sqrt((a*a+b*b)+0); eps=max(binary64(0.001),D/4194304). Thickness never enters D. A nonpositive prospective edge contributes zero before range rejection. Upper caps precede this work. |
| Reserved support/outcome | F06 sections 1/2 exclude operations and nonrectangle evaluation. Plan reservations: hole A04/G09; cavity and thin wall A02/G05/G09; nonrectangle A02/G09. No new diagnostic contract is invented. |

Rational values use the **exact binary64 inputs**: for example `0.001` means the dyadic rational
of its nearest binary64 value. Integer arithmetic on that exact rational multiplies lengths or
adds interval endpoints. Numerator/denominator strings retain
all bits. The Rust oracle converts these rational references only for comparison. The separate
F06 operation-order test compares f64 bits directly. In particular the `decimal` fixture can
detect reassociation of the volume products.

For unverified translations, stored bounds are **ideal rational endpoints**. F06 instead scans
rounded vertices for reported bounds. The harness only sanity-checks the references after f64
rounding; it makes no kernel placement or full-domain accuracy claim. Minimum cubes at 1e9 mm
remain subject to the open rounded-mesh issue in F06 erratum E-1 and G09. All plates stay unrotated.

Threshold cases use independent 3-4-5 triangles: (3072,4096) gives D=5120 and eps=5/4096;
(300000,400000) gives D=500000 and eps=500000/4194304. Both floors are exact dyadics.
`nextafter` selects the adjacent representable float below/above each floor and the dimension cap.
Cases also include all absolute minima, all upper caps, huge positive/negative finite values,
subnormal thickness, signed zero, every-axis NaN, both infinities, first-stage precedence and
same-stage range aggregation. Choosing these numbers requires no kernel execution.

## Independent harness

`crates/kernel/tests/g03_fixtures.rs` is an automatically discovered Cargo integration test.
It calls only the public rectangular builder for actual output. Its oracle imports no production
math helpers, tolerance constants, mesh tables, topology checks or builder internals. Existing
model SHA-256 is used only to validate fixture bytes.

The oracle recomputes scalar admission and analytic references, then reads the actual kernel
mesh. It infers each face from its constant coordinate plane and outward sign, collects directed
triangle-edge incidence in an independent ordered map, and distinguishes box edges from face
diagonals through adjacent face roles. It checks two opposite uses of every edge, all vertices
used, one connected shell, both Euler sums, every analytic corner, bounds and positive volume.

The signed volume check independently sums centred tetrahedral determinants divided by 6,
as specified by F06 section 6 (divergence theorem); triangle areas come from half the cross-product
norm, compared with the six rectangular face areas `2*((a*b+a*c)+b*c)` from F06 section 2.
This path shares no kernel vector or self-check code. Volume, area and mesh comparisons use
relative 1e-6 under CR-TOL-001; bounds of origin primitives, counts, decisions, diagnostics,
canonical zero and normative arithmetic comparisons are exact. No test epsilon controls admission.

Nine corruptions of each of 22 actual outputs must fail (198 checks): deleted/reversed/duplicate
triangle, out-of-range index, duplicate corner, wrong face, doubled volume, NaN coordinate and
negative-zero coordinate. Failure messages name the stable fixture ID. Reversing triangle order
together with its face mapping must still pass. This is bounded fixture QA of boxes, not a
general mesh validator or an accuracy certificate for arbitrary solids.

## Reproduction

```sh
cargo test -p beamcraft-kernel --test kernel -- g03_fixtures::
cargo test --workspace
```

The committed exact rational references and code are reviewable independently.
The Rust harness runs in existing workspace CI with no new dependencies or Cargo registration.
The fixture parameters, manifest and expectations were produced by a one-time generator that the
project retired with its Python toolchain (issue #178); the committed files are the frozen
evidence and git history keeps the generator.

This task does not implement a user-facing feature and makes no engine/CLI E2E claim. Native
results do not prove WASM or cross-platform conformance, K0, G05 or G09 acceptance. No kernel defect
has been found by these fixtures; unsupported adapters and numeric gates are recorded limitations.
