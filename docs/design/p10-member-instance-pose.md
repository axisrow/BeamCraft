# P10 member instance placement

Contributor-original BeamCraft code and documentation, MPL-2.0.

The follow-up to the P10 plate slice makes a straight member's body element-local too. Its
axis, roll, section and plane-cut records determine the mesh. Translation and quaternion
placement remain on the USD Xform and the scene instance. A placement rotation is applied after
the local axis/roll frame, once, for world bounds, features, picking and snaps. A cut point and
normal remain in the element's local frame and move with its placement.

`evaluate_member` validates the placement but does not rotate the body. The body cache excludes
both translation and rotation; cache hits still validate every supplied placement. Changing a
member's pose therefore preserves its mesh bytes and digest, quantity inputs, shape signature,
mass signature and stored quantity record. A pose edit rewrites the authored placement alone;
its footprint still invalidates world measurements through the existing derived group.

Committed member scene elements carry the full quaternion. The existing snapshot, bundle,
semantic-feature mapping and native instancing apply it to the local mesh. IFC now exports that
same local mesh with the full pose, replacing the former identity orientation used beside a
baked mesh. Independent copies keep the same body and their own placement. The member tool's
ghost explicitly rotates a temporary display copy; this does not change the committed body.

The local body representation changes the schema descriptor and pin. Older pre-alpha stores,
including stores with a baked member mesh, are refused without migration. This avoids silently
interpreting an old rotated mesh as local and applying the pose twice. No command argument,
engineering material, dependency or asset is added.

Existing placement expectations are updated to assert local-body invariance and to map world
features through the full pose. Existing world bounds, axis snaps and picking retain their
original geometric expectations. Their runtime acceptance remains a separate validation gate;
compiling a target does not establish that its tests pass.
