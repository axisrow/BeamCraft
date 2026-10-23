# C04: semantic conflict detection

Scope: fork #12 / upstream #126, after C03. This specifies acceptance before the C04 package.
The implementation plan calls for detection and preservation of both sides. Choosing a winner,
merging a conflicted value or building a resolution UI is separate work.

## Contract

Use the engine's typed `rebase::Operation`, `Conflict` and `Decision`, exposed through the
explicit C03 API and CLI. The planner compares every original command in both verified intervals,
including intermediate writes subsequently reverted and unchanged objects read by a command.
Do not substitute final USD values, property names or layer-file overlap for semantic groups.

A pair conflicts when either command writes an entry the other reads or writes. Read/read
overlap is allowed. Element lifecycle, atomic shape inputs, placement, material references,
relationships, derived state and immutable definition dependencies use F04's existing groups.
Both directions matter: an accepted read can prevent a later draft write just as an accepted
write can invalidate a draft read. Equal final text still represents two distinct writes.

The complete diagnostic identifies both indexed operation IDs, full target kind/identity and
group. An accepted publication indexes its verified aggregate under the publication operation
ID; this can conservatively report a dependency retained from its source interval. It does not
pretend that the publication ID is an individual source command ID. Sort deterministically and
deduplicate access directions, without dropping distinct operation pairs. A report above 256
conflicts returns `LIMIT_EXCEEDED`, rather than a truncated
success or an apparently independent plan. Existing 4096 operation/entry limits apply unchanged.
Missing or invalid command evidence refuses; it never means that an operation read nothing.

On conflict, accepted head, personal draft head, revision manifests, USD layer blobs and workspace
metadata remain unchanged. An attempted operation ID may acquire its normal durable request
binding; a refusal cannot permit reuse with a different payload. Both original model states
remain inspectable after a process restart. No publication or server-wins policy is implicit.

## Executable acceptance

| Case | Required observable result |
|---|---|
| C04-01 same property | Different and identical final labels conflict with full target/group and both operation IDs; retry returns the same diagnostics |
| C04-02 atomic shape | Different shape-input properties still conflict on `shape_inputs`, including plate thickness versus contour changes |
| C04-03 dependency direction | Copy-source/change and change/copy-source conflict in both draft/accepted directions; unrelated copy sources remain independent |
| C04-04 delete/link | Draft weld linking a remotely deleted part and draft deletion of a remotely linked part both refuse; both original versions reopen intact |
| C04-05 intervening intent | A conflicting write later reverted still conflicts; the indexed command/publication identity is reported |
| C04-06 distinct identities | Two clients allocating the same element ID conflict on lifecycle; distinct new IDs remain independent even in the same USD file |
| C04-07 report integrity | Stable ordering, full IDs, no duplicate direction entries; exactly 256 conflicts are complete and 257 refuse; read/read is independent |
| C04-08 retained state | For every runtime refusal, verify both heads, metadata bytes and reopened authored content; no accepted or unsent edit disappears |

Existing C03 tests already exercise parts of this matrix. C04 adds missing semantic directions
and stronger preservation/diagnostic assertions; it need not introduce a redundant second planner.
Required workspace and platform checks still apply. A test-only C04 package is valid when the
existing runtime satisfies these observable requirements.
