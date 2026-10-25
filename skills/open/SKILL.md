---
name: open
description: Open a BeamCraft store in the native viewport app. Invoked only by the explicit /beamcraft:open: launching a desktop window is a side effect the model does not decide on its own.
disable-model-invocation: true
argument-hint: [store path]
---

# Open a BeamCraft store in the native viewport

Open the BeamCraft store named in the request. A store path is normally passed as the command
argument; if it is missing, ask for one before running anything.

1. Build the app if it is not built - the `view` skill, §3, shows the command; the CLI is
   built alongside, because the app finds it next to its own executable.
2. Launch the app in the background against the store exactly as §3 shows, with a log file.
3. Prove the launch with the real process: it is alive and the log is clean; quote what you
   captured. Refreshing the store's head inside the window is `R`.
4. The picture itself is accepted by the owner's eyes: report that the window is up and wait
   for that confirmation.
