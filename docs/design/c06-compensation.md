# C06: compensate a published operation

Scope: fork #12 / upstream #126, after C05. This specifies acceptance before the engine/API/CLI
implementation. Snapshot undo of a personal draft remains P06; it cannot undo accepted state.
The proposed runtime's supported publication forms, refusal boundaries and executable mapping
are in [`c03-c06-acceptance.md`](c03-c06-acceptance.md).

## Operation and proof

Compensation is a new typed operation with its own request and result identities. The caller
identifies one original published operation, its verified before/after pins and the current
expected draft/accepted context. The application verifies publication membership and exact
source history rather than trusting a caller's before-image or arbitrary inverse USD text.
An original operation embedded in a published changeset requires its retained source proof;
an unavailable/compacted proof refuses instead of guessing an inverse from final state.

The CLI's `--original-revision` identifies one accepted publication. A C01/C02 changeset is one
published operation, even when it contains several source commands and administrative rebase
records. Verify its complete contiguous source interval and exact aggregate dependencies, then
compensate its accepted delta as one unit. This API does not select just its last source command.
The inverse before-image is the actual accepted base pinned by that payload, independently of
the older personal prefix used to bind/verify source history. Otherwise undoing a rebased client's
change could also undo another client's already accepted opinion. Original single-command proof
pins/request binding stay unchanged. Direct publication still requires proof that its complete
delta/dependencies are accounted for by one terminal command; unavailable intervals refuse.

The engine derives the inverse from the original command's supported semantic groups and
verified before/after images. Apply only those groups to the current model. All intervening
accepted operations, including reads and reverted writes, participate in current precondition
checks. A later operation by another user that touches or depends on an affected group prevents
automatic compensation. Do not treat the same current value as proof that nobody changed it.

For lifecycle changes, require that identity and relationships still permit the inverse. Removing
a created part must not strand a later weld, assembly or other link. Restoring a deleted part
must not replace an existing element or alias its path. Immutable definitions and foreign layers
keep their existing rules. A command unsupported by the inverse adapter returns an explicit
refusal; unsupported compensation is never a full-layer or full-snapshot restore.

Re-evaluate affected geometry, quantities and numbering freshness with the ordinary engine and
strict USD verifier. Commit one new personal revision against the exact expected draft, preserving
unrelated later opinions. Publish only on a subsequent explicit C01/C02 request with the current
accepted-base precondition. Use bounded evidence/history and the existing metadata/publication
leases; interruption or a competing draft CAS must retain the valid winner.

Request binding includes the original operation/proof pins, actor, workspace and explicit expected
heads. A same-request retry returns its historical compensating result without creating another
revision, including after later edits. A different request under that ID is `OP_ID_REUSE`.
The original accepted operation remains in immutable history. Compensation never rewrites it.

## Executable acceptance

| Case | Required observable result |
|---|---|
| C06-01 accepted rename | Publish A's rename; compensate it in A's workspace; accepted state stays unchanged until explicit publication; final label equals the verified before-image |
| C06-02 preserve B | B changes another part or a disjoint group of the same part; compensate A; B's name/placement/geometry/quantity remains exactly present |
| C06-03 refuse B overlap | B changes the same group, even to an equal or subsequently reverted value; compensation refuses, preserving both heads and metadata |
| C06-04 indirect dependencies | B copies or links a source affected by the inverse; current preconditions refuse rather than corrupting B's result |
| C06-05 lifecycle | Compensate a supported create/delete only with free identity and valid relationship preconditions; retained links and ID/path collisions refuse |
| C06-06 proof and actor | Wrong actor/project/schema, unpublished operation, missing/truncated/narrowed proof and unsupported inverse refuse before candidate commit |
| C06-07 replay/crash/race | Same request replays its historical result after a later edit; changed request refuses; interrupted commit recovers once; a competing head never gets overwritten |
| C06-08 final reader | After explicit publication, independent reading agrees with both the inverse and retained later edits; immutable original history remains readable |

The headless engine API precedes its CLI adapter and real end-to-end tests. Named slices may
separate proof, inverse planning, storage/CLI and acceptance, each under the issue's 1000-line
limit. A supported inverse slice must list its exact command coverage and cannot close parent
#12 until the required C03–C06 acceptance and platform checks are complete.
