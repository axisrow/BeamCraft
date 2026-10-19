# Visual bug report: beamcraft-ui, 2026-10-19

Every item below was reproduced in the running `beamcraft-ui` window and has a screenshot in
`screenshots/` (also bundled in `visual-bug-screenshots.zip`). A problem that could not be seen on
screen is not listed. The captions drawn on the screenshots are in Russian.

## Setup

- Commit `11a2139` (`master`), release build of `beamcraft` and `beamcraft-ui`.
- Linux container, Xvfb, Mesa llvmpipe (software Vulkan), 4x MSAA, window 1600 x 1000 unless
  noted. Driven with the `run-beamcraft-ui` skill (branch `claude/app-build-screenshots-hzuyd2`,
  PR #17).
- Sample stores were authored with the CLI (`define-material`, `define-profile`, `create-member`,
  `create-plate`, `reference-attach`, `copy-element`, `update-element`, `delete-element`).

## Serious

### 1. Wide window stretches the picture (`01`, `01b`)

A 3 x 3 m plate with a 2 m round hole, seen from above. In a 1600 px window the hole is
409 x 408 px (a circle). In a 5400 px window it is 516 x 408 px: +26 % horizontally, and the
square plate becomes a rectangle. With a simulated Retina scale factor of 2
(`WINIT_X11_SCALE_FACTOR=2`, window 2560 x 1300 pt) the stretch is +14 %.

Cause: the offscreen target is clamped to 4096 px (`apps/beamcraft-ui/src/app.rs`, central panel,
`clamp(1, 4096)`), while the image is drawn over the whole panel. The aspect used by the camera
comes from the clamped target (`app.rs`, `fn aspect`). It starts whenever the viewport is wider
than 4096 physical pixels, which a maximized window on a 5K display reaches.

### 2. Elements over 1 m3 cannot be created (`02`)

`create-plate` 10 x 10 m x 12 mm and `create-member` 300 x 300 mm, 12 m long fail with
`LIMIT_EXCEEDED: stage budget exceeded`. The same plate at 10 mm (exactly 1.000 m3) is created;
10.01 mm is refused. A 400 x 400 mm column is accepted at 6 m and refused at 7 m. The UI shows the
error in red and nothing is published, so a long column or a large plate cannot be entered at all.

Cause: the layer engine rejects any stored number above 1e9
(`crates/layer-engine/src/backend.rs`, `fn number`), and the element volume is stored in mm3
(`bim:volumeMm3`).

### 3. Hidden parts show through on large models (`03`)

A 100 m bridge model (girders under a 10 mm deck plate), default view after `F`: the deck edge is
jagged and the girders below show through as dark teeth. Zoomed in, the same deck is clean.
Rendering the same model with the near plane set to 1 % of the camera distance (a local
experiment, not committed) removes the artifact.

Cause: the near plane is fixed at 1 mm (`apps/beamcraft-ui/src/camera.rs`, `view`) with a
`Depth32Float` buffer and a far plane of `5 * distance + 1e7`, which leaves too little depth
resolution at long distances.

## Medium

### 4. The camera resets after every Commit and after R (`04`)

Zoom in on a column base, preview and commit a 200 x 200 x 10 mm plate (the model bounds do not
change): the view jumps back to the whole model. `R` does the same. Cause: `poll_reload` calls
`camera.fit` on every refresh (`app.rs`).

### 5. A small volume is shown as 0.000 m3 (`05`)

The 200 x 200 x 10 mm plate (0.0004 m3) reports "volume 0.000 m3" after Commit
(`app.rs`, `format!(", volume {:.3} m3", ...)`). Most plates and stiffeners are this small.

### 6. An attached reference model is not shown (`06`)

`reference-attach` of a 2 x 2 x 2 m USD box answers `reference_attached`, the revision generation
goes from 21 to 22, but the window keeps showing 18 elements with the same bounds and no box. The
R01 design note says a reference has a full pose and no kernel body, so the translation-only
view contract does not cover it; the viewport has no path for it yet.

## Minor (interface)

### 7. Labels sit under their drop-downs (`07`)

On the Member tab the label "Profile" is drawn below the profile list and above the material list,
so it reads as the label of the material list (`app.rs`, `choice_combo`).

### 8. The left panel changes width (`08`)

The 3D view starts at x = 225 on the Plate tab, 264 on the Member tab and 307 after choosing a
profile with a long name, so the model shifts sideways while you work (`app.rs`, the left panel has
no fixed width).

### 9. A new store shows a red error (`09`)

Opening a store with no revision shows "beamcraft CLI: view-export rejected: NOT_FOUND" in red,
although nothing is wrong. The definitions list treats the same `NOT_FOUND` as normal
(`tools.rs`, `apply_definitions`).

### 10. Name limit counts characters, the engine counts bytes (`10`)

A name of 150 Russian letters (282 bytes) passes the field limit and Preview, then Commit fails with
`INVALID_ARGUMENT: label must be 1 to 256 bytes without control characters`
(`tools.rs`, `push_name` uses `chars().count()`; `crates/model/src/limits.rs`,
`MAX_LABEL_BYTES`).

### 11. Decimal comma gives a confusing message (`11`)

Thickness `12,5` shows `Thickness: "12,5" is not a finite number`. The value is finite; the
comma is the problem (`tools.rs`, `fn number`).

## Checked and fine

Beams and columns in every direction (+X, -X, +Y, -Y, up, down, diagonal) and with a 90 degree
roll; plates with round holes; copy, move, thickness change and delete through the CLI followed by
`R`; Cyrillic names (drawn correctly); the first plate in an empty store; zero and negative
thickness (clear message).
