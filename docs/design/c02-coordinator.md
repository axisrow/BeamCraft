# C02 local coordinator

Date: 2026-10-18. Status: **experimental vertical slice; C02 acceptance is limited as listed below.**
Branch `feat/c02-coordinator`, based on `origin/master` `7b1f2bf` (C01 merged).

Plan row: C02 / M, depends on C01 and U07. Result: an explicit loopback server with token
authentication, one publish queue and an expected head. Acceptance: off by default; refused
without a token; a parallel commit never overwrites the accepted head.

## What is implemented

```text
beamcraft serve-coordinator --store PUBLISHED --workspaces-root DIR --client ACTOR_UUID:TOKEN_FILE [--client ...]
                            [--port N] [--max-requests N]
beamcraft publish-changeset --address 127.0.0.1:PORT --token-file FILE --workspace NAME --changeset SHA256
                              --expected-head REVISION_UUID
beamcraft inspect-coordinator-head --address 127.0.0.1:PORT --token-file FILE
```

| Area | What exists |
|---|---|
| `beamcraft_host::coordinator` | `Token` (32 to 256 printable bytes from a file; on Unix the file must not be group/world accessible; never printed; constant-time comparison), `Client` (actor and token), `Coordinator::bind` (127.0.0.1 only, 1 to 16 clients with distinct tokens and actors) and `serve` (bounded frames, deadlines, at most 16 connections, a queue of at most 64 requests, one handler thread), `Handler`, `Request` (the authenticated actor, the op and the remaining fields), `call` (loopback addresses only). |
| `beamcraft_cli::coordinator` | `PublishHandler`: `head`, and `publish` (named workspace inside the root, expected head equal to the changeset's base, C01 receipt with the authenticated actor as the trusted caller, `BASE_REVISION_MISMATCH` with the current head). |
| CLI | `serve-coordinator` prints a `listening` line with the address, serves, and prints a final `stopped` object; `publish-changeset` and `inspect-coordinator-head` print the coordinator's answer with matching exit codes. |

## Decisions

- **Off by default, explicit and loopback.** Nothing starts a coordinator except
  `serve-coordinator`. It binds `127.0.0.1` (port 0 picks a free one), closes any connection whose
  peer is not loopback, and its clients refuse a non-loopback address before any I/O.
- **No token, no coordinator.** At least one client is required to start. Tokens come from files,
  never from the command line (where other local users could read them). The coordinator never
  generates a secret: the repository has no audited entropy source (`host::ids` documents that its
  random bytes are not cryptographic), so the user creates the token file, for example with an
  operating-system password generator.
- **The token is the actor.** C01 took the trusted caller from `--actor-id`. Through the
  coordinator the authenticated client's actor is the trusted caller and a request cannot name
  one, so a token publishes only changesets authored by its own actor (`PERMISSION_DENIED`
  otherwise). Direct `apply-changeset` on a store the user can write still trusts `--actor-id`.
- **One queue.** Every authenticated request goes through one bounded FIFO queue to one handler
  thread, so publications of one coordinator are strictly sequential. Reading and authenticating
  a request happen before it joins the queue, under one absolute deadline for the whole frame
  (each read gets only the time that remains), so a client that trickles bytes cannot stall the
  queue or hold a connection slot. A full queue or too many connections is `LIMIT_EXCEEDED`; a request dropped because the
  coordinator stopped is `BACKEND_UNAVAILABLE` (not handled); no answer within 120 s is
  `OUTCOME_UNKNOWN`, labelled `outcome_unknown`, not `rejected`. On shutdown the coordinator joins
  every connection thread before returning, so the last answer is written before the process
  exits.
- **Current head is information.** After C01 decided, the head is re-read for the answer and
  marked `known` or `unavailable` (`current_head_status`); a failed re-read never turns a
  publication or a refusal into a different answer.
- **Expected head.** A publication names the head the client expects; it must be the changeset's
  base (`INVALID_ARGUMENT` otherwise). Inside the queue, C01's receipt publishes fast-forward only,
  so a moved head is `BASE_REVISION_MISMATCH` and the answer carries the current head for a later
  rebase (C03). A publication that already happened replays whatever the head is now. Two
  concurrent publications from one base therefore accept exactly one, and a commit to the store
  from outside the coordinator is still caught by the store's own expected-base check (U06), so an
  accepted head is never overwritten.
- **Workspaces by name.** A request names a workspace, never a path: one directory component of
  `A-Z a-z 0-9 - _ .`, not starting with a dot, whose canonical path stays inside the canonical
  workspaces root (`PERMISSION_DENIED` for a symbolic link that leaves it, `NOT_FOUND` for a missing
  one).
- **Wire.** One request and one answer per connection, each a 4-byte big-endian length (at most
  65,536) and a JSON object read with the bounded protocol decoder; requests carry
  `protocol: beamcraft-coordinator/1`. Unknown protocol, operation and fields are
  `UNSUPPORTED_VERSION`, `UNSUPPORTED_COMMAND` and `UNSUPPORTED_PROPERTY`.

## Requirement coverage

The architecture's rule that "coordination and control servers require explicit opt-in, loopback
binding and token authentication" is implemented for the publish coordinator. C01's open point
("`--actor-id` stands for the trusted context until C02 adds tokens") is closed for publications
through the coordinator.

Provenance: the design follows the repository's architecture, F04 and C01; no external product material was used. No third-party dependency is added (`std::net` only).

## Evidence

- `apps/beamcraft-cli/tests/coordinator.rs` (3 end-to-end tests with a real server process):
  refusal to start without a client, with a short token or (Unix) a group/world-readable token file;
  a wrong token and a non-loopback address refused; two concurrent publications from one base
  accept exactly one, the other gets `BASE_REVISION_MISMATCH` with the winner as current head, the
  winner replays, a stale expected head is refused, and the server stops after its request budget
  with the head unchanged; a stranger's token cannot publish another actor's changeset; `..`,
  missing and (Unix) symlinked workspace names are refused with nothing written.
- `crates/host/src/coordinator.rs` unit tests: token rules, binding rules, loopback-only client,
  authentication, the actor taken from the token, one-at-a-time handling in order, oversized,
  malformed and wrong-protocol frames.

## Not done

- No TLS, no remote access, no user accounts or token rotation; one token per actor, loaded at
  start.
- No rebase (C03), semantic conflict detection (C04) or offline two-client harness (C05).
- No persistent queue: a stopped coordinator drops waiting requests (answered `BACKEND_UNAVAILABLE`).
- The handler runs C01's receipt synchronously; a very slow store blocks the queue up to the
  120-second answer deadline.
- Windows access control on the token file is not checked (only the Unix mode is).
