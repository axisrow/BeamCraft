---
description: Open a BeamCraft store in the native viewport app
argument-hint: [store path]
---

Open the BeamCraft store in the native desktop viewport as described: $ARGUMENTS

Read and follow `${CLAUDE_PLUGIN_ROOT}/skills/beamcraft-view/SKILL.md` first - it carries the
exact invocations, output shapes and captured evidence. Then:

1. Build the app if it is not built (the skill shows the command; the CLI is built alongside,
   because the app finds it next to its own executable).
2. Launch the app in the background against the store exactly as the skill shows, with a log
   file.
3. Prove the launch with the real process: it is alive and the log is clean; quote what you
   captured. Refreshing the store's head inside the window is `R`.
4. The picture itself is accepted by the owner's eyes: report that the window is up and wait
   for that confirmation.
