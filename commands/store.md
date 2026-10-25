---
description: Open, verify and inspect a BeamCraft store through the beamcraft-cli
argument-hint: [store path, or what to check]
---

Open and verify the BeamCraft store as described: $ARGUMENTS

Read and follow `${CLAUDE_PLUGIN_ROOT}/skills/beamcraft-store/SKILL.md` first - it carries the
exact CLI invocations, output shapes, refusal codes and captured evidence. Then:

1. Build the `beamcraft` binary if it is not built (the skill shows the command).
2. Verify with the real binary: `revision-open` for integrity (`revision_verified`),
   `plate-inspect` for content (element ids, placements, quantities).
3. Quote only captured JSON as evidence and state the head `sequence` you saw.
