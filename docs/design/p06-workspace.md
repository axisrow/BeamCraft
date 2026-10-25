# P06 personal workspace

Date: 2026-10-18. Status: **experimental vertical slice; P06 acceptance is limited as listed below.**
Branch `feat/p06-workspace`, stacked on `feat/p05-copy-element` (PR #67). Merge order:
#63, #64, #65, #66, #67, then this PR.

Plan row: P06 / M, depends on P05 and the revision store. Result: a user has a personal draft; edits
change only the draft; the published head moves only on an explicit publish. Acceptance: two
workspaces of one user are independent; a draft survives a process restart; publishing is atomic.

## What is implemented

```text
beamcraft create-workspace  --store PUBLISHED --workspace-dir DIR [--user-id UUID] [--workspace-id UUID]
beamcraft inspect-workspace  --store PUBLISHED --workspace-dir DIR
beamcraft publish-workspace --store PUBLISHED --workspace-dir DIR
```

| Area | What exists |
|---|---|
| `beamcraft_cli::workspace` | `create_workspace`, `status`, `publish_workspace`, `WorkspaceMeta` (`workspace.json`: workspace id, user id, base revision, base manifest, last published draft sequence). Atomic temp-and-rename write, bounded read. |
| `beamcraft_cli::plate::commit_verified` | Commits an existing complete layer set through U06/U07 with the content verifier; used by fork and publish. |
| CLI | The three commands above. Every other edit command works on a workspace with `--store DIR`. |

## Decisions

- A workspace is a directory holding its own revision store (blobs, head log, outcome ledger). The
  draft is never connected to the published root, so a half-written draft cannot affect it.
- `create-workspace` copies the published head's layer bytes as the first draft revision under the
  workspace's own id. Repeating it with the same identities resumes an interrupted fork; another
  user or workspace id for an existing directory is `DUPLICATE_ID`.
- `publish-workspace` takes no revision id: the published revision id and the operation are both the draft head's revision id, so an interrupted publish retried in a new process is the same request. It is a fast-forward: when the published head is still the base, the draft head's
  layers are committed as one new published revision through the same strict verifier. Otherwise
  it is `BASE_REVISION_MISMATCH` and nothing is written. A fresh fork is `INVALID_ARGUMENT`.
- Publish is atomic in the published store's single head update. It is idempotent: the published
  head whose operation is the draft head's revision is recognised, so a retry after a crash (or a
  repeat) is `workspace_publish_replayed` and moves nothing.
- The published store itself is refused as a workspace directory.

## Evidence

`apps/beamcraft-cli/tests/workspace.rs` (4 end-to-end tests): fork, edit with `update-element` and
`copy-element` in the draft, the published head unchanged until publish, status counts unsent
revisions, fast-forward publish and replay; two workspaces of one user are independent and the
second publisher gets `BASE_REVISION_MISMATCH` with its draft untouched; unsent work survives a
restart and a repeated create resumes; error codes (`NOT_FOUND`, `INVALID_ID`,
`INVALID_ARGUMENT`, `DUPLICATE_ID`).

## Not done

- No authentication or permission model: the user id is a recorded label.
- No merge or rebase of a draft over a moved published head; the user must fork again.
- One linear draft history, published as one revision (the intermediate draft revisions are not
  republished one by one).
- Layer processing is in-process (open decision LEW-03).
