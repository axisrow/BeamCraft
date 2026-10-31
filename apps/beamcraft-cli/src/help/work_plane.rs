// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! V08 session frame construction and plane-local point input.

pub(super) const HELP: &str = concat!(
    "Work plane (V08, read-only): work-plane (--plane OX,OY,OZ,XX,XY,XZ,YX,YY,YZ | ",
    "--parallel xy|xz|yz [--offset MM] | --points OX,OY,OZ,PX,PY,PZ,QX,QY,QZ | ",
    "--vertical AX,AY,AZ,BX,BY,BZ | --store ROOT --origin X,Y,Z --direction DX,DY,DZ). ",
    "Returns an admitted --plane expression; store mode picks the nearest front face or returns no_face. ",
    "create-plate and create-member accept that --plane; member points may be X,Y (Z defaults to zero). ",
    "Input coordinates are local to the plane. A plate stores only the resolved rigid placement; ",
    "a member stores world endpoints. The session plane is never stored in the model. ",
    "A plane cannot be combined with explicit placement, grid placement or grid-height options.",
    " resolve-construction-point --reference X,Y,Z --toward X,Y,Z [--distance MM] [--ortho true|false] [--plane FRAME]: read-only 3D direction/length resolution; ortho chooses the dominant local X/Y/Z axis.",
);
