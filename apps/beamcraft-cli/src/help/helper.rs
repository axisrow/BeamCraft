// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
pub(super) const COMMANDS: &[&str] = &["create-helper", "helpers", "snap-helper"];
pub(super) const HELP: &str = "\
Construction objects (T02):
  create-helper --store ROOT --name TEXT --kind point|line|circle|plane --parameters NUMBERS
    point: x,y,z; line: ax,ay,az,bx,by,bz; circle: cx,cy,cz,nx,ny,nz,radius;
    plane: ox,oy,oz,nx,ny,nz. All values in world millimetres. Normals must be nonzero.
    Optional --new-id UUID and the common command context options.
  helpers --store ROOT
  snap-helper --store ROOT --point X,Y,Z --tolerance MM
  snap --store ROOT --point X,Y,Z --radius MM --kinds helper
  delete-element --store ROOT --id HELPER_UUID
Helpers have no engineering body, quantities, BOM rows or numbering marks.
";
