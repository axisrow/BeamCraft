---
description: Snapshot a BeamCraft scene or export a view bundle for the native viewport
argument-hint: [snapshot or export, and where]
---

Produce the BeamCraft view as described: $ARGUMENTS

Read and follow `${CLAUDE_PLUGIN_ROOT}/skills/beamcraft-view/SKILL.md` first - it carries the
exact CLI invocations, output shapes and captured evidence. Then:

1. Build the `beamcraft` binary if it is not built (the skill shows the command).
2. `view-snapshot` for a synthetic scene without a store; `view-export` for the verified head of
   a store as a bundle file.
3. Quote only captured JSON as evidence; for an export, confirm the file exists with the
   reported `bytes` (the destination is never overwritten).
