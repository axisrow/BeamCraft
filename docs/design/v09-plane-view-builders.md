# V09 plane and two-point cameras

Contributor-original Rust and synthetic fixtures, MPL-2.0.

`create-view --workspace-dir DIR --kind plane|two-points --name TEXT` constructs a personal
orthographic camera through `engine::plane_view::PlaneView` and saves it in W01. Plane mode
uses `--plane ORIGIN,XDIR,YDIR` (default world XY). Two-point mode takes `--start` and `--end`
and looks onto their vertical plane; the points may be entered in `--plane` with T04 numeric
expressions. A vertical-only or coincident horizontal line is degenerate and refused.

`--side positive|negative` defaults positive. Opposite-side views retain local Y/up and reverse
X/normal together, giving a proper camera frame and a matching work plane for tools. Depth and
width default to 10000 mm, aspect to 1. The camera is one depth away from the construction
plane; the depth describes a full band centered on that plane. V15 consumes the depth band for
non-destructive clipping. This slice stores it and does not claim renderer clipping acceptance.

The personal record preserves the admitted source frame or world points, side, depth, width
and aspect under the stable view UUID. Restore deterministically reconstructs the camera,
without changing its saved source model pin or either file. Width/aspect edits update the
construction; name/comment edits retain it. Explicit manual pose, projection, FOV or work-plane
edits detach construction so restoring cannot silently override the user's manual camera.
Unsupported construction versions or malformed records are errors, never a stale-camera fallback.

Real CLI tests cover both constructions, opposite side, persistence, property edits, multiple
independent views, unchanged model inspection and refusal without touching the personal file.
Engine tests cover frame orientation, opposite sides, degenerate lines and invalid depth.
Named grid-line/level/3D views and work areas are the separate V12 slice; UI windows remain out.
