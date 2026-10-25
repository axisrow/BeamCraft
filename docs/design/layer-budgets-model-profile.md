# Layer-engine budgets: the model profile

Date: 2026-10-18. Status: **implemented; owner request of 2026-10-18 ("raise the USD layer
limits").** Branch `feat/layer-budgets`, based on `origin/master` `7e7f067` (A07.1 merged).

## Problem

`Budgets::default()` held the U01 first-slice ceilings: 64 KiB per layer, 256 KiB per stage, 256
prims, 4096 values and 1024 scalars per array. They were sized for the N01 spike's small hostile
corpus ("the cheap 64 KiB corpus ... establish no production safety bound", F11; LEW-03 asks for
a reviewed budget profile) and were never meant for a project. Because a revision's whole derived
layer is one USDA text, they bounded a model to **7 detailed 300x150 I-sections or 36 rectangle
members** (A07.1 probe), and a circular hollow section above about 110 mm exceeded the array cap.

## Decision

`Budgets` gains two named profiles, and the default ceiling becomes the model profile:

| Budget | First slice (`Budgets::FIRST_SLICE`) | Model (`Budgets::MODEL`, default) | Bound by |
|---|---|---|---|
| `layer_bytes` | 64 KiB | 32 MiB | one layer is one host blob (`host::storage::MAX_BLOB_BYTES`) |
| `total_bytes` | 256 KiB | 64 MiB | host content verification (`host::revision::MAX_CONTENT_VERIFY_BYTES`) |
| `prims` | 256 | 262,144 | 4 prims per element at the engine's 65,536-element view cap |
| `values` | 4,096 | 4,194,304 | 64 fields per element at that cap |
| `array_items` | 1,024 | 32,768 | one mesh at the engine blob caps (8,192 triangles = 24,576 indices) |
| `decoded_bytes` | 4 MiB | 256 MiB | 4x the stage (binary expansion allowance) |
| `snapshot_bytes` | 1 MiB | 512 MiB | 8x the stage (typed snapshot accounting allowance, not measured at the cap) |
| `layers`, `depth`, `page_items` | 8, 16, 64 | unchanged | fixed shallow project layer stack |

> 2026-10-21 amendment: the engine mesh caps rose with F06 section 6 (decision on #141), and the
> model `array_items` grew with the same bound: one element mesh at the new caps is 524,288
> triangles = 1,572,864 indices per array (262,144 vertices = 786,432 scalars). Every other row
> and the first-slice profile are unchanged.

* Every new value is taken from a limit that already binds the same data further along (host
  blobs and verification, engine view caps), so no stage accepts what a later one refuses.
* Ceilings stay ceilings: `Budgets::validate` still refuses any value above the default, every cap
  is still charged before allocation, and callers may tighten.
* The first-slice profile stays for small untrusted inputs that are not a project stage: the R01
  reference source inspection uses it explicitly, so its behaviour is unchanged. The hostile-input
  tests of the layer engine (index paging, USDC declared counts, foreign inspection) run on it too,
  because they test that a cap holds, not its size; new tests check the model profile's own caps
  at the boundary and plus one (array at 1,572,864 and 1,572,865 entries, a layer of 32 MiB + 1
  byte, a raised budget refused).

## Measurements (release build, Linux container, one synthetic 300x150 I-section per member)

| Members | Derived layer | `create-member` (one edit) |
|---|---|---|
| 20 | | 0.50 s |
| 40 | | 0.96 s |
| 60 | 509,650 bytes (8.5 KB per member) | 1.48 s |

On the 60-member store: `open-revision` 10 ms, `inspect-elements` 0.60 s, `export-view` 0.63 s.
One `kernel::section::section_member` build of that section (64 vertices, 124 triangles, G06
certification included) takes 8.8 ms.

**Capacity now.** The layer text bound is about 3,900 such members per revision (32 MiB / 8.5 KB);
rectangle members need less.

**The next bottleneck is time, not bytes.** Every command re-evaluates and re-certifies every
element of the model with the kernel (about 25 ms per detailed member per edit, measured above),
so an edit on a 2,000-member model would take about 50 s, close to the CLI's 60 s request
deadline. That cost is the publish-time verifier and the committed-state evaluation, not USD
parsing (`open-revision` is 10 ms). The fix is a separate task: verify only the elements whose
inputs changed since the already verified base revision, and make G06 certification cheaper.
Storing meshes as blobs outside the derived layer text would further raise the byte bound.

## Not changed

Host limits (32 MiB blob, 64 MiB verification, 256 MiB revision), engine view caps, the CLI
deadline and R01's 64 KiB source limit.

## Evidence

`cargo test -p beamcraft-layer-engine` (new `the_model_profile_is_the_default_ceiling_and_its_caps_hold_at_plus_one`,
the existing hostile-input tests on the first-slice profile, the authored input bound one point
above the array cap, 524,289 points since the 2026-10-21 amendment),
`cargo test -p beamcraft-cli --test sections`
(`more_detailed_members_fit_one_revision_than_the_first_slice_allowed`: 12 detailed I-sections in
one revision through the real binary), and the probe above.
