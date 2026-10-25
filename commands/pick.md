---
description: Pick, snap and resolve features at coordinates in a BeamCraft store
argument-hint: [coordinate or element to resolve]
---

Resolve the coordinate or element as described: $ARGUMENTS

Read and follow `${CLAUDE_PLUGIN_ROOT}/skills/beamcraft-pick/SKILL.md` first - it carries the
exact CLI invocations, output shapes and captured evidence. Then:

1. Build the `beamcraft` binary if it is not built (the skill shows the command).
2. Use `feature-list`/`feature-lookup` for semantic feature addresses, `pick` for a ray or a
   point, `snap` for the nearest vertex, axis end or axis point (pass `--radius`).
3. Quote only captured JSON as evidence. An empty hit list is a normal result; feature ids exist
   for canonical box bodies only.
