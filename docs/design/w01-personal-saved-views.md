# W01 personal saved cameras

Contributor-original Rust and synthetic fixtures, MPL-2.0.

Saved cameras live in `views.usda` beside the personal workspace metadata, outside its model
manifest, journal and publication. A shared store without valid personal workspace metadata is
refused. `save-view`, `list-views`, `inspect-view`, `update-view`, `delete-view` and `restore-view`
use `--workspace-dir`; their API is `beamcraft_cli::saved_views`, backed by pure engine admission.

A native USD Camera carries a stable UUID, name, camera pose/projection/span/aspect/FOV, a work
plane, author, Unix-millisecond save time, comment and source revision/manifest. The camera looks
along local -Z with +Y up. CLI defaults are orthographic, position (0,0,10000) mm, identity
orientation, horizontal width 10000 mm, aspect 1, FOV pi/3 and the world XY work plane. Parameters
are explicit on inspection. Restore reports whether the current model differs from the source
manifest without modifying that model. The personal author label comes from the workspace.

The file has an independent version-1 format and exact byte-digest version. A writer obtains a
contained create-new lock, compares the observed digest and installs a complete synced temporary
file via the host's existing atomic-output boundary. `--expected-views DIGEST|none` also exposes
that concurrency check to callers. A busy or stale writer refuses; a crashed lock is not stolen
automatically. This is cooperative process concurrency within an authorized workspace directory,
not protection from an adversarial process replacing its filesystem concurrently. Directory
crash durability is not newly claimed. Damaged or unsupported documents are never overwritten.

At most 128 cameras and 256 auxiliary personal records fit a 4 MiB document. Camera labels are
bounded; comments and opaque records use bounded string chunks to respect the layer reader's
per-string ceiling. Auxiliary records support later view anchors, filters and representation
sets without adding a second persistence system. They are UTF-8 payloads retained unchanged;
the owning feature must validate their typed content before use. Foreign fields or composition
outside this format are refused rather than silently erased. Unit scale is mm and up is Z.

Tests exercise real-process persistence, full camera restore, CAS refusal, model inspection/token
invariance, publication exclusion, changed-model reporting, deletion, invalid camera input and
corrupt-file preservation. Engine tests cover duplicate admission and source-pin diagnostics.
No native view manager, anchored view construction or clipping is claimed by this slice.
