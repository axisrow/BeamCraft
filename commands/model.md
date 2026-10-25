---
description: Create or edit BeamCraft steel members and plates through the beamcraft-cli
argument-hint: [what to create or edit, and where]
---

Model BeamCraft elements exactly as described: $ARGUMENTS

Read and follow `${CLAUDE_PLUGIN_ROOT}/skills/beamcraft-model/SKILL.md` first - it carries the
exact CLI invocations, output shapes, refusal codes and captured evidence. Then:

1. Build the `beamcraft` binary if it is not built (the skill shows the command).
2. Ensure a store exists - follow `${CLAUDE_PLUGIN_ROOT}/skills/beamcraft-store/SKILL.md` when
   the user has not named one.
3. Run the real binary for every step; quote only captured JSON as evidence.
4. Prove the result: run `plate-inspect` afterwards and report the element ids, the new revision
   `sequence` and the quantities from the real output.
