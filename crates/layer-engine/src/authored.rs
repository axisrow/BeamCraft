// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Codeless U02 schema and isolated layer authoring for a trusted command engine.
//! This is a worker-library boundary, not an F04 command validator or acceptance ledger.

mod derive;
mod edit;
mod guard;
mod reference;
mod validate;

pub use guard::WriteGrant;
pub use reference::{
    ForeignPrim, Reference, ReferenceView, SourceFormat, SourcePin, UnitsInterpretation, UpAxis,
    read_references,
};

use crate::*;
pub use beamcraft_model::command::MarkStatus;
pub use beamcraft_model::definition::SectionProperties;
pub use beamcraft_model::section::SectionShape;
use std::collections::BTreeMap;

/// Canonical UTF-8 schema descriptor. Its exact bytes commit the mapping and item profile.
/// No generated schema, plugin, fallback values or external schema resources are required.
pub const SCHEMA_DESCRIPTOR: &str = concat!(
    "BeamCraft/U02/codeless/1\nmodel=0.1.0;backend=openusd=0.8.0;features=none;save=usda-owned-v1\n",
    "root:metersPerUnit=0.001;upAxis=Z;customLayerData:bimSchemaDigest=sha256;",
    "bimProjectId=string(uuid7,optional,root layer only,written once with the first revision)\n",
    "element=/World/Steel/E_<uuid7-32>;type=Xform\n",
    "bim:id=string;bim:name=string;bim:elementKind=token;bim:geometryMode=token(parametric);",
    "bim:schemaVersion=string(0.1.0);bim:createdBy=string(uuid7);bim:revision=string(uuid7,optional)\n",
    "steel:plate:contourPointsMm=double3[];steel:plate:thicknessMm=double;",
    "steel:plate:holePointsMm=double3[](optional,z=0);steel:plate:holeVertexCounts=int[](optional,>=3 each);",
    "bim:axisPointsMm=double3[];steel:member:rollRad=double(optional)\n",
    "assembly=element;bim:elementKind=assembly;kind=assembly;no shape inputs, material or Geometry;placement=identity\n",
    "bim:partOf=rel(0..1 element target of kind assembly;source plate|beam|column|assembly;no self;acyclic)\n",
    "connection=element;bim:elementKind=connection;kind=component;a weld only (A06);no material or Geometry;placement=identity\n",
    "steel:weld:type=token(fillet);steel:weld:sizeMm=double(>0,<=1000);steel:weld:location=token(workshop|site);",
    "steel:weld:side=token(arrow|other|both)\n",
    "bim:connects=rel(2 distinct ordered element targets of kind plate|beam|column, main part first;source connection)\n",
    "level=element;bim:elementKind=level;kind=component;bim:elevationMm=double(finite,abs<=1e9);no material or Geometry;placement=identity\n",
    "bim:level=rel(0..1 element target of kind level;source plate|beam|column)\n",
    "mark=bim:mark=string(optional,1..64 bytes,no control characters);bim:markStatus=token(assigned|approved);",
    "authored together on one element of kind plate|beam|column|assembly|connection;unique per model (A08)\n",
    "xformOp:translate=double3;xformOp:orient=quatd;xformOpOrder=token[](translate,orient)\n",
    "kind:/World=assembly;/World/Steel=group;element=component;element/Geometry=subcomponent;",
    "displayName(element)=bim:name\n",
    "bim:profile=rel;bim:material=rel;definition=/World/Definitions/D_<sha256-64>\n",
    "definition:bim:contentDigest=string;bim:schemaVersion=string(0.1.0);bim:units=token(mm);",
    "bim:definitionKind=token;steel:profile:widthMm=double;steel:profile:heightMm=double;",
    "steel:profile:designation=string(optional);steel:profile:standard=string(optional);",
    "steel:profile:areaMm2=double(optional);steel:profile:massPerMeterKg=double(optional);",
    "steel:profile:surfacePerMeterM2=double(optional);",
    "section profile=bim:definitionKind=token(i_shape|channel|angle|rhs|chs|lipped_c|plain_c|lipped_z|omega);",
    "steel:profile:<parameter>Mm=double for each family parameter in order (h,b,tw,tf,r|h,b,t,r|d,t|h,b,c,t,r);",
    "custom profile=bim:definitionKind=token(custom);steel:profile:outerPointsMm=double3[](z=0);",
    "steel:profile:cavityPointsMm=double3[](optional,z=0);steel:profile:cavityVertexCounts=int[](optional,>=3 each);",
    "bim:densityKgM3=double(optional);bim:materialGrade=string(optional);",
    "bim:materialStandard=string(optional)\n",
    "definition displayName=prim metadata string(optional,any definition kind;part of the definition content)\n",
    "derived=<element>/Geometry:Mesh;points=point3f[];faceVertexCounts=int[];faceVertexIndices=int[];",
    "subdivisionScheme=token(none);orientation=token(rightHanded);",
    "extent=float3[](min,max of points);normals=normal3f[](uniform,one unit Newell normal per face);",
    "bim:shapeKey=string(sha256);",
    "quantities(optional,all or none)=bim:volumeMm3=double;bim:volumeStatus=token;bim:massKg=double(optional);",
    "bim:massStatus=token;bim:shapeSignature=string(sha256);bim:massSignature=string(sha256);",
    "bim:quantityAlgorithm=string\n",
    "reference=/World/Steel/E_<uuid7-32>;type=Xform;kind=group;bim:elementKind=reference;bim:geometryMode=reference;",
    "bim:reference:sourceDigest=string(sha256);bim:reference:sourceBytes=int;bim:reference:sourceFormat=token(usda|usdc);",
    "bim:reference:sourceVersion=int;bim:reference:sourceLabel=string;bim:reference:sourceDefaultPrim=string;",
    "bim:reference:declaredMetersPerUnit=double(optional);bim:reference:declaredUpAxis=token(Y|Z,optional)\n",
    "reference/Correction=Xform;kind=group;bim:reference:correctionMetersPerUnit=double;bim:reference:correctionUpAxis=token(Y|Z);",
    "bim:reference:correctionSourceDigest=string(sha256);xformOp:rotateX:bimUpAxis=double(degrees,Y up only);",
    "xformOp:scale:bimUnits=double3(metersPerUnit*1000);xformOpOrder=token[](rotateX,scale;empty without correction)\n",
    "reference/Correction/Source=def without type;references=@external/<sha256>.<usda|usdc>@</defaultPrim>;nothing else owned at or below\n",
    "changes=Create|Label|Contour|Thickness|Axis|Roll|Profile|Material|Placement|Mesh|AttachReference|ReferenceUnits|PartOf|Level|Mark|Delete;",
    "opinion=Absent|Block|Value|Remove;relationship=Targets|Empty|Remove\n",
    "snapshot=Prim(path,element_id,type_name,active,properties,relationships,sources);",
    "property=(name,type_name,optional-value);relationship=(name,ordered-targets)\n",
    "prepare=(context,op_id,actor,schema,resources,changes);resource=(digest,definition);",
    "element=(id,name,shape,placement,optional-material);placement=(translation_mm[3],orientation_xyzw[4]);",
    "mesh=(id,shape_key,points_mm,face_counts,indices);",
    "authored-item=(element_id,attribute,type_name,opinion);install=(context,token,op_id,Accepted);",
    "discard=(context,token,op_id,Undecided|Rejected);unknown=OUTCOME_UNKNOWN\n",
);

/// The one descriptor line that A08 added, and the `Mark` item it appended to the changes
/// enumeration; see [`pin_accepted`].
const NUMBERING_DESCRIPTOR_LINE: &str = "mark=bim:mark=string(optional,1..64 bytes,no control characters);bim:markStatus=token(assigned|approved);authored together on one element of kind plate|beam|column|assembly|connection;unique per model (A08)\n";
const NUMBERING_CHANGES_ITEM: &str = "|Level|Mark|Delete;";

/// Descriptor of the A07.1 mapping revision (section profiles), before D1 and D2 added the optional
/// project record of the root layer, the `level` element and `bim:level`. Layers pinned to it
/// contain nothing the current mapping refuses (D1 and D2 only add optional content), so they stay
/// readable and editable.
const A07_1_DESCRIPTOR: &str = concat!(
    "BeamCraft/U02/codeless/1\nmodel=0.1.0;backend=openusd=0.8.0;features=none;save=usda-owned-v1\n",
    "root:metersPerUnit=0.001;upAxis=Z;customLayerData:bimSchemaDigest=sha256\n",
    "element=/World/Steel/E_<uuid7-32>;type=Xform\n",
    "bim:id=string;bim:name=string;bim:elementKind=token;bim:geometryMode=token(parametric);",
    "bim:schemaVersion=string(0.1.0);bim:createdBy=string(uuid7);bim:revision=string(uuid7,optional)\n",
    "steel:plate:contourPointsMm=double3[];steel:plate:thicknessMm=double;",
    "steel:plate:holePointsMm=double3[](optional,z=0);steel:plate:holeVertexCounts=int[](optional,>=3 each);",
    "bim:axisPointsMm=double3[];steel:member:rollRad=double(optional)\n",
    "assembly=element;bim:elementKind=assembly;kind=assembly;no shape inputs, material or Geometry;placement=identity\n",
    "bim:partOf=rel(0..1 element target of kind assembly;source plate|beam|column|assembly;no self;acyclic)\n",
    "connection=element;bim:elementKind=connection;kind=component;a weld only (A06);no material or Geometry;placement=identity\n",
    "steel:weld:type=token(fillet);steel:weld:sizeMm=double(>0,<=1000);steel:weld:location=token(workshop|site);",
    "steel:weld:side=token(arrow|other|both)\n",
    "bim:connects=rel(2 distinct ordered element targets of kind plate|beam|column, main part first;source connection)\n",
    "xformOp:translate=double3;xformOp:orient=quatd;xformOpOrder=token[](translate,orient)\n",
    "kind:/World=assembly;/World/Steel=group;element=component;element/Geometry=subcomponent;",
    "displayName(element)=bim:name\n",
    "bim:profile=rel;bim:material=rel;definition=/World/Definitions/D_<sha256-64>\n",
    "definition:bim:contentDigest=string;bim:schemaVersion=string(0.1.0);bim:units=token(mm);",
    "bim:definitionKind=token;steel:profile:widthMm=double;steel:profile:heightMm=double;",
    "steel:profile:designation=string(optional);steel:profile:standard=string(optional);",
    "steel:profile:areaMm2=double(optional);steel:profile:massPerMeterKg=double(optional);",
    "steel:profile:surfacePerMeterM2=double(optional);",
    "section profile=bim:definitionKind=token(i_shape|channel|angle|rhs|chs|lipped_c|plain_c|lipped_z|omega);",
    "steel:profile:<parameter>Mm=double for each family parameter in order (h,b,tw,tf,r|h,b,t,r|d,t|h,b,c,t,r);",
    "custom profile=bim:definitionKind=token(custom);steel:profile:outerPointsMm=double3[](z=0);",
    "steel:profile:cavityPointsMm=double3[](optional,z=0);steel:profile:cavityVertexCounts=int[](optional,>=3 each);",
    "bim:densityKgM3=double(optional);bim:materialGrade=string(optional);",
    "bim:materialStandard=string(optional)\n",
    "derived=<element>/Geometry:Mesh;points=point3f[];faceVertexCounts=int[];faceVertexIndices=int[];",
    "subdivisionScheme=token(none);orientation=token(rightHanded);",
    "extent=float3[](min,max of points);normals=normal3f[](uniform,one unit Newell normal per face);",
    "bim:shapeKey=string(sha256);",
    "quantities(optional,all or none)=bim:volumeMm3=double;bim:volumeStatus=token;bim:massKg=double(optional);",
    "bim:massStatus=token;bim:shapeSignature=string(sha256);bim:massSignature=string(sha256);",
    "bim:quantityAlgorithm=string\n",
    "reference=/World/Steel/E_<uuid7-32>;type=Xform;kind=group;bim:elementKind=reference;bim:geometryMode=reference;",
    "bim:reference:sourceDigest=string(sha256);bim:reference:sourceBytes=int;bim:reference:sourceFormat=token(usda|usdc);",
    "bim:reference:sourceVersion=int;bim:reference:sourceLabel=string;bim:reference:sourceDefaultPrim=string;",
    "bim:reference:declaredMetersPerUnit=double(optional);bim:reference:declaredUpAxis=token(Y|Z,optional)\n",
    "reference/Correction=Xform;kind=group;bim:reference:correctionMetersPerUnit=double;bim:reference:correctionUpAxis=token(Y|Z);",
    "bim:reference:correctionSourceDigest=string(sha256);xformOp:rotateX:bimUpAxis=double(degrees,Y up only);",
    "xformOp:scale:bimUnits=double3(metersPerUnit*1000);xformOpOrder=token[](rotateX,scale;empty without correction)\n",
    "reference/Correction/Source=def without type;references=@external/<sha256>.<usda|usdc>@</defaultPrim>;nothing else owned at or below\n",
    "changes=Create|Label|Contour|Thickness|Axis|Roll|Profile|Material|Placement|Mesh|AttachReference|ReferenceUnits|PartOf|Delete;",
    "opinion=Absent|Block|Value|Remove;relationship=Targets|Empty|Remove\n",
    "snapshot=Prim(path,element_id,type_name,active,properties,relationships,sources);",
    "property=(name,type_name,optional-value);relationship=(name,ordered-targets)\n",
    "prepare=(context,op_id,actor,schema,resources,changes);resource=(digest,definition);",
    "element=(id,name,shape,placement,optional-material);placement=(translation_mm[3],orientation_xyzw[4]);",
    "mesh=(id,shape_key,points_mm,face_counts,indices);",
    "authored-item=(element_id,attribute,type_name,opinion);install=(context,token,op_id,Accepted);",
    "discard=(context,token,op_id,Undecided|Rejected);unknown=OUTCOME_UNKNOWN\n",
);

/// Descriptor of the A06 mapping revision (welds), before the A07.1 section profiles. Layers
/// pinned to it contain nothing the current mapping refuses (A07.1 only adds definition kinds and
/// their fields), so they stay readable and editable.
const A06_DESCRIPTOR: &str = concat!(
    "BeamCraft/U02/codeless/1\nmodel=0.1.0;backend=openusd=0.8.0;features=none;save=usda-owned-v1\n",
    "root:metersPerUnit=0.001;upAxis=Z;customLayerData:bimSchemaDigest=sha256\n",
    "element=/World/Steel/E_<uuid7-32>;type=Xform\n",
    "bim:id=string;bim:name=string;bim:elementKind=token;bim:geometryMode=token(parametric);",
    "bim:schemaVersion=string(0.1.0);bim:createdBy=string(uuid7);bim:revision=string(uuid7,optional)\n",
    "steel:plate:contourPointsMm=double3[];steel:plate:thicknessMm=double;",
    "steel:plate:holePointsMm=double3[](optional,z=0);steel:plate:holeVertexCounts=int[](optional,>=3 each);",
    "bim:axisPointsMm=double3[];steel:member:rollRad=double(optional)\n",
    "assembly=element;bim:elementKind=assembly;kind=assembly;no shape inputs, material or Geometry;placement=identity\n",
    "bim:partOf=rel(0..1 element target of kind assembly;source plate|beam|column|assembly;no self;acyclic)\n",
    "connection=element;bim:elementKind=connection;kind=component;a weld only (A06);no material or Geometry;placement=identity\n",
    "steel:weld:type=token(fillet);steel:weld:sizeMm=double(>0,<=1000);steel:weld:location=token(workshop|site);",
    "steel:weld:side=token(arrow|other|both)\n",
    "bim:connects=rel(2 distinct ordered element targets of kind plate|beam|column, main part first;source connection)\n",
    "xformOp:translate=double3;xformOp:orient=quatd;xformOpOrder=token[](translate,orient)\n",
    "kind:/World=assembly;/World/Steel=group;element=component;element/Geometry=subcomponent;",
    "displayName(element)=bim:name\n",
    "bim:profile=rel;bim:material=rel;definition=/World/Definitions/D_<sha256-64>\n",
    "definition:bim:contentDigest=string;bim:schemaVersion=string(0.1.0);bim:units=token(mm);",
    "bim:definitionKind=token;steel:profile:widthMm=double;steel:profile:heightMm=double;",
    "steel:profile:designation=string(optional);steel:profile:standard=string(optional);",
    "steel:profile:areaMm2=double(optional);steel:profile:massPerMeterKg=double(optional);",
    "steel:profile:surfacePerMeterM2=double(optional);",
    "bim:densityKgM3=double(optional);bim:materialGrade=string(optional);",
    "bim:materialStandard=string(optional)\n",
    "derived=<element>/Geometry:Mesh;points=point3f[];faceVertexCounts=int[];faceVertexIndices=int[];",
    "subdivisionScheme=token(none);orientation=token(rightHanded);",
    "extent=float3[](min,max of points);normals=normal3f[](uniform,one unit Newell normal per face);",
    "bim:shapeKey=string(sha256);",
    "quantities(optional,all or none)=bim:volumeMm3=double;bim:volumeStatus=token;bim:massKg=double(optional);",
    "bim:massStatus=token;bim:shapeSignature=string(sha256);bim:massSignature=string(sha256);",
    "bim:quantityAlgorithm=string\n",
    "reference=/World/Steel/E_<uuid7-32>;type=Xform;kind=group;bim:elementKind=reference;bim:geometryMode=reference;",
    "bim:reference:sourceDigest=string(sha256);bim:reference:sourceBytes=int;bim:reference:sourceFormat=token(usda|usdc);",
    "bim:reference:sourceVersion=int;bim:reference:sourceLabel=string;bim:reference:sourceDefaultPrim=string;",
    "bim:reference:declaredMetersPerUnit=double(optional);bim:reference:declaredUpAxis=token(Y|Z,optional)\n",
    "reference/Correction=Xform;kind=group;bim:reference:correctionMetersPerUnit=double;bim:reference:correctionUpAxis=token(Y|Z);",
    "bim:reference:correctionSourceDigest=string(sha256);xformOp:rotateX:bimUpAxis=double(degrees,Y up only);",
    "xformOp:scale:bimUnits=double3(metersPerUnit*1000);xformOpOrder=token[](rotateX,scale;empty without correction)\n",
    "reference/Correction/Source=def without type;references=@external/<sha256>.<usda|usdc>@</defaultPrim>;nothing else owned at or below\n",
    "changes=Create|Label|Contour|Thickness|Axis|Roll|Profile|Material|Placement|Mesh|AttachReference|ReferenceUnits|PartOf|Delete;",
    "opinion=Absent|Block|Value|Remove;relationship=Targets|Empty|Remove\n",
    "snapshot=Prim(path,element_id,type_name,active,properties,relationships,sources);",
    "property=(name,type_name,optional-value);relationship=(name,ordered-targets)\n",
    "prepare=(context,op_id,actor,schema,resources,changes);resource=(digest,definition);",
    "element=(id,name,shape,placement,optional-material);placement=(translation_mm[3],orientation_xyzw[4]);",
    "mesh=(id,shape_key,points_mm,face_counts,indices);",
    "authored-item=(element_id,attribute,type_name,opinion);install=(context,token,op_id,Accepted);",
    "discard=(context,token,op_id,Undecided|Rejected);unknown=OUTCOME_UNKNOWN\n",
);

/// Descriptor of the A01 mapping revision (assemblies and `bim:partOf`), before the A06
/// `connection` kind, the weld properties and `bim:connects`. Layers pinned to it contain nothing
/// the current mapping refuses (A06 only adds an optional kind, its fields and a relationship), so
/// they stay readable and editable.
const A01_DESCRIPTOR: &str = concat!(
    "BeamCraft/U02/codeless/1\nmodel=0.1.0;backend=openusd=0.8.0;features=none;save=usda-owned-v1\n",
    "root:metersPerUnit=0.001;upAxis=Z;customLayerData:bimSchemaDigest=sha256\n",
    "element=/World/Steel/E_<uuid7-32>;type=Xform\n",
    "bim:id=string;bim:name=string;bim:elementKind=token;bim:geometryMode=token(parametric);",
    "bim:schemaVersion=string(0.1.0);bim:createdBy=string(uuid7);bim:revision=string(uuid7,optional)\n",
    "steel:plate:contourPointsMm=double3[];steel:plate:thicknessMm=double;",
    "steel:plate:holePointsMm=double3[](optional,z=0);steel:plate:holeVertexCounts=int[](optional,>=3 each);",
    "bim:axisPointsMm=double3[];steel:member:rollRad=double(optional)\n",
    "assembly=element;bim:elementKind=assembly;kind=assembly;no shape inputs, material or Geometry;placement=identity\n",
    "bim:partOf=rel(0..1 element target of kind assembly;source plate|beam|column|assembly;no self;acyclic)\n",
    "xformOp:translate=double3;xformOp:orient=quatd;xformOpOrder=token[](translate,orient)\n",
    "kind:/World=assembly;/World/Steel=group;element=component;element/Geometry=subcomponent;",
    "displayName(element)=bim:name\n",
    "bim:profile=rel;bim:material=rel;definition=/World/Definitions/D_<sha256-64>\n",
    "definition:bim:contentDigest=string;bim:schemaVersion=string(0.1.0);bim:units=token(mm);",
    "bim:definitionKind=token;steel:profile:widthMm=double;steel:profile:heightMm=double;",
    "steel:profile:designation=string(optional);steel:profile:standard=string(optional);",
    "steel:profile:areaMm2=double(optional);steel:profile:massPerMeterKg=double(optional);",
    "steel:profile:surfacePerMeterM2=double(optional);",
    "bim:densityKgM3=double(optional);bim:materialGrade=string(optional);",
    "bim:materialStandard=string(optional)\n",
    "derived=<element>/Geometry:Mesh;points=point3f[];faceVertexCounts=int[];faceVertexIndices=int[];",
    "subdivisionScheme=token(none);orientation=token(rightHanded);",
    "extent=float3[](min,max of points);normals=normal3f[](uniform,one unit Newell normal per face);",
    "bim:shapeKey=string(sha256);",
    "quantities(optional,all or none)=bim:volumeMm3=double;bim:volumeStatus=token;bim:massKg=double(optional);",
    "bim:massStatus=token;bim:shapeSignature=string(sha256);bim:massSignature=string(sha256);",
    "bim:quantityAlgorithm=string\n",
    "reference=/World/Steel/E_<uuid7-32>;type=Xform;kind=group;bim:elementKind=reference;bim:geometryMode=reference;",
    "bim:reference:sourceDigest=string(sha256);bim:reference:sourceBytes=int;bim:reference:sourceFormat=token(usda|usdc);",
    "bim:reference:sourceVersion=int;bim:reference:sourceLabel=string;bim:reference:sourceDefaultPrim=string;",
    "bim:reference:declaredMetersPerUnit=double(optional);bim:reference:declaredUpAxis=token(Y|Z,optional)\n",
    "reference/Correction=Xform;kind=group;bim:reference:correctionMetersPerUnit=double;bim:reference:correctionUpAxis=token(Y|Z);",
    "bim:reference:correctionSourceDigest=string(sha256);xformOp:rotateX:bimUpAxis=double(degrees,Y up only);",
    "xformOp:scale:bimUnits=double3(metersPerUnit*1000);xformOpOrder=token[](rotateX,scale;empty without correction)\n",
    "reference/Correction/Source=def without type;references=@external/<sha256>.<usda|usdc>@</defaultPrim>;nothing else owned at or below\n",
    "changes=Create|Label|Contour|Thickness|Axis|Roll|Profile|Material|Placement|Mesh|AttachReference|ReferenceUnits|PartOf|Delete;",
    "opinion=Absent|Block|Value|Remove;relationship=Targets|Empty|Remove\n",
    "snapshot=Prim(path,element_id,type_name,active,properties,relationships,sources);",
    "property=(name,type_name,optional-value);relationship=(name,ordered-targets)\n",
    "prepare=(context,op_id,actor,schema,resources,changes);resource=(digest,definition);",
    "element=(id,name,shape,placement,optional-material);placement=(translation_mm[3],orientation_xyzw[4]);",
    "mesh=(id,shape_key,points_mm,face_counts,indices);",
    "authored-item=(element_id,attribute,type_name,opinion);install=(context,token,op_id,Accepted);",
    "discard=(context,token,op_id,Undecided|Rejected);unknown=OUTCOME_UNKNOWN\n",
);

/// Descriptor of the R01 mapping revision (reference wrappers), before the A01 assembly kind and
/// `bim:partOf`. Layers pinned to it contain nothing the current mapping refuses (A01 only adds an
/// optional kind and relationship), so they stay readable and editable.
const R01_DESCRIPTOR: &str = concat!(
    "BeamCraft/U02/codeless/1\nmodel=0.1.0;backend=openusd=0.8.0;features=none;save=usda-owned-v1\n",
    "root:metersPerUnit=0.001;upAxis=Z;customLayerData:bimSchemaDigest=sha256\n",
    "element=/World/Steel/E_<uuid7-32>;type=Xform\n",
    "bim:id=string;bim:name=string;bim:elementKind=token;bim:geometryMode=token(parametric);",
    "bim:schemaVersion=string(0.1.0);bim:createdBy=string(uuid7);bim:revision=string(uuid7,optional)\n",
    "steel:plate:contourPointsMm=double3[];steel:plate:thicknessMm=double;",
    "steel:plate:holePointsMm=double3[](optional,z=0);steel:plate:holeVertexCounts=int[](optional,>=3 each);",
    "bim:axisPointsMm=double3[];steel:member:rollRad=double(optional)\n",
    "xformOp:translate=double3;xformOp:orient=quatd;xformOpOrder=token[](translate,orient)\n",
    "kind:/World=assembly;/World/Steel=group;element=component;element/Geometry=subcomponent;",
    "displayName(element)=bim:name\n",
    "bim:profile=rel;bim:material=rel;definition=/World/Definitions/D_<sha256-64>\n",
    "definition:bim:contentDigest=string;bim:schemaVersion=string(0.1.0);bim:units=token(mm);",
    "bim:definitionKind=token;steel:profile:widthMm=double;steel:profile:heightMm=double;",
    "steel:profile:designation=string(optional);steel:profile:standard=string(optional);",
    "steel:profile:areaMm2=double(optional);steel:profile:massPerMeterKg=double(optional);",
    "steel:profile:surfacePerMeterM2=double(optional);",
    "bim:densityKgM3=double(optional);bim:materialGrade=string(optional);",
    "bim:materialStandard=string(optional)\n",
    "derived=<element>/Geometry:Mesh;points=point3f[];faceVertexCounts=int[];faceVertexIndices=int[];",
    "subdivisionScheme=token(none);orientation=token(rightHanded);",
    "extent=float3[](min,max of points);normals=normal3f[](uniform,one unit Newell normal per face);",
    "bim:shapeKey=string(sha256);",
    "quantities(optional,all or none)=bim:volumeMm3=double;bim:volumeStatus=token;bim:massKg=double(optional);",
    "bim:massStatus=token;bim:shapeSignature=string(sha256);bim:massSignature=string(sha256);",
    "bim:quantityAlgorithm=string\n",
    "reference=/World/Steel/E_<uuid7-32>;type=Xform;kind=group;bim:elementKind=reference;bim:geometryMode=reference;",
    "bim:reference:sourceDigest=string(sha256);bim:reference:sourceBytes=int;bim:reference:sourceFormat=token(usda|usdc);",
    "bim:reference:sourceVersion=int;bim:reference:sourceLabel=string;bim:reference:sourceDefaultPrim=string;",
    "bim:reference:declaredMetersPerUnit=double(optional);bim:reference:declaredUpAxis=token(Y|Z,optional)\n",
    "reference/Correction=Xform;kind=group;bim:reference:correctionMetersPerUnit=double;bim:reference:correctionUpAxis=token(Y|Z);",
    "bim:reference:correctionSourceDigest=string(sha256);xformOp:rotateX:bimUpAxis=double(degrees,Y up only);",
    "xformOp:scale:bimUnits=double3(metersPerUnit*1000);xformOpOrder=token[](rotateX,scale;empty without correction)\n",
    "reference/Correction/Source=def without type;references=@external/<sha256>.<usda|usdc>@</defaultPrim>;nothing else owned at or below\n",
    "changes=Create|Label|Contour|Thickness|Axis|Roll|Profile|Material|Placement|Mesh|AttachReference|ReferenceUnits;",
    "opinion=Absent|Block|Value|Remove;relationship=Targets|Empty|Remove\n",
    "snapshot=Prim(path,element_id,type_name,active,properties,relationships,sources);",
    "property=(name,type_name,optional-value);relationship=(name,ordered-targets)\n",
    "prepare=(context,op_id,actor,schema,resources,changes);resource=(digest,definition);",
    "element=(id,name,shape,placement,optional-material);placement=(translation_mm[3],orientation_xyzw[4]);",
    "mesh=(id,shape_key,points_mm,face_counts,indices);",
    "authored-item=(element_id,attribute,type_name,opinion);install=(context,token,op_id,Accepted);",
    "discard=(context,token,op_id,Undecided|Rejected);unknown=OUTCOME_UNKNOWN\n",
);

/// Descriptor of the mapping revision that preceded the R01 reference wrappers (the one with the
/// optional section properties of P02.2). Layers pinned to it contain nothing the current mapping
/// refuses (R01 only adds the `reference` element and its change items), so they stay readable
/// and editable; an edit rewrites the pin to the current digest.
const P02_2_DESCRIPTOR: &str = concat!(
    "BeamCraft/U02/codeless/1\nmodel=0.1.0;backend=openusd=0.8.0;features=none;save=usda-owned-v1\n",
    "root:metersPerUnit=0.001;upAxis=Z;customLayerData:bimSchemaDigest=sha256\n",
    "element=/World/Steel/E_<uuid7-32>;type=Xform\n",
    "bim:id=string;bim:name=string;bim:elementKind=token;bim:geometryMode=token(parametric);",
    "bim:schemaVersion=string(0.1.0);bim:createdBy=string(uuid7);bim:revision=string(uuid7,optional)\n",
    "steel:plate:contourPointsMm=double3[];steel:plate:thicknessMm=double;",
    "steel:plate:holePointsMm=double3[](optional,z=0);steel:plate:holeVertexCounts=int[](optional,>=3 each);",
    "bim:axisPointsMm=double3[];steel:member:rollRad=double(optional)\n",
    "xformOp:translate=double3;xformOp:orient=quatd;xformOpOrder=token[](translate,orient)\n",
    "kind:/World=assembly;/World/Steel=group;element=component;element/Geometry=subcomponent;",
    "displayName(element)=bim:name\n",
    "bim:profile=rel;bim:material=rel;definition=/World/Definitions/D_<sha256-64>\n",
    "definition:bim:contentDigest=string;bim:schemaVersion=string(0.1.0);bim:units=token(mm);",
    "bim:definitionKind=token;steel:profile:widthMm=double;steel:profile:heightMm=double;",
    "steel:profile:designation=string(optional);steel:profile:standard=string(optional);",
    "steel:profile:areaMm2=double(optional);steel:profile:massPerMeterKg=double(optional);",
    "steel:profile:surfacePerMeterM2=double(optional);",
    "bim:densityKgM3=double(optional);bim:materialGrade=string(optional);",
    "bim:materialStandard=string(optional)\n",
    "derived=<element>/Geometry:Mesh;points=point3f[];faceVertexCounts=int[];faceVertexIndices=int[];",
    "subdivisionScheme=token(none);orientation=token(rightHanded);",
    "extent=float3[](min,max of points);normals=normal3f[](uniform,one unit Newell normal per face);",
    "bim:shapeKey=string(sha256);",
    "quantities(optional,all or none)=bim:volumeMm3=double;bim:volumeStatus=token;bim:massKg=double(optional);",
    "bim:massStatus=token;bim:shapeSignature=string(sha256);bim:massSignature=string(sha256);",
    "bim:quantityAlgorithm=string\n",
    "changes=Create|Label|Contour|Thickness|Axis|Roll|Profile|Material|Placement|Mesh;",
    "opinion=Absent|Block|Value|Remove;relationship=Targets|Empty|Remove\n",
    "snapshot=Prim(path,element_id,type_name,active,properties,relationships,sources);",
    "property=(name,type_name,optional-value);relationship=(name,ordered-targets)\n",
    "prepare=(context,op_id,actor,schema,resources,changes);resource=(digest,definition);",
    "element=(id,name,shape,placement,optional-material);placement=(translation_mm[3],orientation_xyzw[4]);",
    "mesh=(id,shape_key,points_mm,face_counts,indices);",
    "authored-item=(element_id,attribute,type_name,opinion);install=(context,token,op_id,Accepted);",
    "discard=(context,token,op_id,Undecided|Rejected);unknown=OUTCOME_UNKNOWN\n",
);

/// Exact mapping identity, separate from U01's backend/static-reader pin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchemaIdentity {
    /// Frozen F04 version.
    pub model: &'static str,
    /// SHA-256 of [`SCHEMA_DESCRIPTOR`].
    pub digest: ContentDigest,
}

/// Computes the admitted mapping identity without consulting files or environment.
pub fn schema_identity() -> Result<SchemaIdentity, Diagnostic> {
    Ok(SchemaIdentity {
        model: "0.1.0",
        digest: Sha256::digest(SCHEMA_DESCRIPTOR.as_bytes()).map_err(|_| limit())?,
    })
}

/// Descriptor of the mapping revision that preceded the optional section properties of P02.2
/// (the one with the U09 standard metadata). Layers pinned to it contain nothing the current
/// mapping refuses (every added field is optional), so they stay readable and editable; an edit
/// rewrites the pin to the current digest. Layers written before U09 lack the kind, display name,
/// extent and normals and are not covered.
const PREVIOUS_DESCRIPTOR: &str = concat!(
    "BeamCraft/U02/codeless/1\nmodel=0.1.0;backend=openusd=0.8.0;features=none;save=usda-owned-v1\n",
    "root:metersPerUnit=0.001;upAxis=Z;customLayerData:bimSchemaDigest=sha256\n",
    "element=/World/Steel/E_<uuid7-32>;type=Xform\n",
    "bim:id=string;bim:name=string;bim:elementKind=token;bim:geometryMode=token(parametric);",
    "bim:schemaVersion=string(0.1.0);bim:createdBy=string(uuid7);bim:revision=string(uuid7,optional)\n",
    "steel:plate:contourPointsMm=double3[];steel:plate:thicknessMm=double;",
    "steel:plate:holePointsMm=double3[](optional,z=0);steel:plate:holeVertexCounts=int[](optional,>=3 each);",
    "bim:axisPointsMm=double3[];steel:member:rollRad=double(optional)\n",
    "xformOp:translate=double3;xformOp:orient=quatd;xformOpOrder=token[](translate,orient)\n",
    "kind:/World=assembly;/World/Steel=group;element=component;element/Geometry=subcomponent;",
    "displayName(element)=bim:name\n",
    "bim:profile=rel;bim:material=rel;definition=/World/Definitions/D_<sha256-64>\n",
    "definition:bim:contentDigest=string;bim:schemaVersion=string(0.1.0);bim:units=token(mm);",
    "bim:definitionKind=token;steel:profile:widthMm=double;steel:profile:heightMm=double;",
    "bim:densityKgM3=double(optional)\n",
    "derived=<element>/Geometry:Mesh;points=point3f[];faceVertexCounts=int[];faceVertexIndices=int[];",
    "subdivisionScheme=token(none);orientation=token(rightHanded);",
    "extent=float3[](min,max of points);normals=normal3f[](uniform,one unit Newell normal per face);",
    "bim:shapeKey=string(sha256);",
    "quantities(optional,all or none)=bim:volumeMm3=double;bim:volumeStatus=token;bim:massKg=double(optional);",
    "bim:massStatus=token;bim:shapeSignature=string(sha256);bim:massSignature=string(sha256);",
    "bim:quantityAlgorithm=string\n",
    "changes=Create|Label|Contour|Thickness|Axis|Roll|Profile|Material|Placement|Mesh;",
    "opinion=Absent|Block|Value|Remove;relationship=Targets|Empty|Remove\n",
    "snapshot=Prim(path,element_id,type_name,active,properties,relationships,sources);",
    "property=(name,type_name,optional-value);relationship=(name,ordered-targets)\n",
    "prepare=(context,op_id,actor,schema,resources,changes);resource=(digest,definition);",
    "element=(id,name,shape,placement,optional-material);placement=(translation_mm[3],orientation_xyzw[4]);",
    "mesh=(id,shape_key,points_mm,face_counts,indices);",
    "authored-item=(element_id,attribute,type_name,opinion);install=(context,token,op_id,Accepted);",
    "discard=(context,token,op_id,Undecided|Rejected);unknown=OUTCOME_UNKNOWN\n",
);

/// The one descriptor line that A07.2 added; see [`pin_accepted`].
const A07_2_DISPLAY_NAME_LINE: &str = "definition displayName=prim metadata string(optional,any definition kind;part of the definition content)\n";

/// Whether a layer's saved `bimSchemaDigest` is the current mapping or an earlier one that the
/// current mapping reads without change.
pub(crate) fn pin_accepted(pin: &str) -> Result<bool, Diagnostic> {
    if pin == schema_identity()?.digest.to_string() {
        return Ok(true);
    }
    // A08 added one line (the element mark) and one changes item to the A07.2 descriptor and
    // changed nothing else, so the A07.2 mapping is the current descriptor without both.
    let a07_2 = SCHEMA_DESCRIPTOR
        .replacen(NUMBERING_DESCRIPTOR_LINE, "", 1)
        .replacen(NUMBERING_CHANGES_ITEM, "|Level|Delete;", 1);
    let digest = Sha256::digest(a07_2.as_bytes()).map_err(|_| limit())?;
    if pin == digest.to_string() {
        return Ok(true);
    }
    // A07.2 added one optional line (the definition display name) to the D1/D2 descriptor and
    // changed nothing else, so the D1/D2 mapping is the A07.2 descriptor without that line.
    let d1_d2 = a07_2.replacen(A07_2_DISPLAY_NAME_LINE, "", 1);
    let digest = Sha256::digest(d1_d2.as_bytes()).map_err(|_| limit())?;
    if pin == digest.to_string() {
        return Ok(true);
    }
    for descriptor in [
        A07_1_DESCRIPTOR,
        A06_DESCRIPTOR,
        A01_DESCRIPTOR,
        R01_DESCRIPTOR,
        P02_2_DESCRIPTOR,
        PREVIOUS_DESCRIPTOR,
    ] {
        let digest = Sha256::digest(descriptor.as_bytes()).map_err(|_| limit())?;
        if pin == digest.to_string() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Key of the D1 project record in the root layer's `customLayerData`.
pub const PROJECT_RECORD_KEY: &str = "bimProjectId";

/// The ProjectId recorded in the metadata of the root layer (D1), or `None` for a store that has
/// none. A record that is not a text holding a full UUIDv7 is `INVALID_ID`: it is never read as
/// "absent" and never repaired.
pub fn project_record(metadata: &BTreeMap<String, Value>) -> Result<Option<String>, Diagnostic> {
    let Some(Value::Dictionary(data)) = metadata.get("customLayerData") else {
        return Ok(None);
    };
    match data.get(PROJECT_RECORD_KEY) {
        None => Ok(None),
        Some(Value::Text(id)) if uuid7(id) => Ok(Some(id.clone())),
        Some(_) => Err(fail(ErrorCode::InvalidId, "invalid project record")),
    }
}

/// Explicit property opinion in the owned edit layer, distinct from composition results.
#[derive(Debug, Clone, PartialEq)]
pub enum Opinion<T> {
    /// Keep the property declaration but erase its default (reveals weaker defaults).
    Absent,
    /// Author a USD value block (suppresses weaker defaults).
    Block,
    /// Author this exact typed value.
    Value(T),
    /// Remove the property spec from the selected layer (reveals weaker opinions).
    Remove,
}

/// Explicit optional material relationship edit; omission of a change means no edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaterialOpinion {
    /// Set one immutable definition target, identified by its full content digest.
    Target(ContentDigest),
    /// Explicit empty target list, masking a weaker relationship.
    Empty,
    /// Remove the relationship spec from the edit layer.
    Remove,
}

/// Rigid F04 placement, with no normalization or implicit identity fallback.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    /// Translation in millimetres.
    pub translation_mm: [f64; 3],
    /// Quaternion in F04 x,y,z,w order; mapped to USD real-first storage.
    pub orientation_xyzw: [f64; 4],
}

/// Supported authored shapes; all engineering validation must already have run in the engine.
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    /// Element-local contour, preserved exactly; wrapper does not evaluate a solid.
    Plate {
        /// Ordered explicit contour, at most 1024 points and subject to tighter stage budgets.
        contour_mm: Vec<[f64; 3]>,
        /// Explicit extrusion thickness in millimetres.
        thickness_mm: f64,
        /// Through-hole polygons in plate-local XY (`z` is zero), each at least three points,
        /// stored as one flat point array plus the vertex counts. Empty means a solid plate.
        holes: Vec<Vec<[f64; 3]>>,
    },
    /// Straight member, with a required pinned rectangular profile.
    Member {
        /// Explicit beam or column kind.
        kind: MemberKind,
        /// Two element-local axis points.
        axis_mm: [[f64; 3]; 2],
        /// Immutable profile content digest.
        profile: ContentDigest,
        /// Missing roll has no authored default; engineering default is zero.
        roll_rad: Option<f64>,
    },
    /// A01 assembly: a grouping target of `bim:partOf` with no shape inputs, no material and no
    /// derived body. Its placement must be the identity in this slice.
    Assembly,
    /// A06 weld: a `connection` element with explicitly authored weld properties and one
    /// `bim:connects` record towards two distinct parts. No material and no derived body; its
    /// placement must be the identity in this slice.
    Weld(Weld),
    /// D2 level (a storey): an element of kind `level` with a stored elevation, no material and no
    /// derived body. Plates, beams and columns are assigned to it by [`Change::Level`]. Its
    /// placement must be the identity: the elevation is a value, never a pose.
    Level {
        /// Elevation in millimetres, finite and at most 1,000,000,000 in magnitude.
        elevation_mm: f64,
    },
}

/// The authored inputs of a weld (A06). Tokens are the exact stored values.
#[derive(Debug, Clone, PartialEq)]
pub struct Weld {
    /// Full UUIDv7 of the main part, then of the part welded to it. Distinct, each a plate, beam
    /// or column; the order is significant.
    pub connects: [String; 2],
    /// `fillet`, the only supported type.
    pub weld_type: String,
    /// Nominal size in millimetres, in `(0, 1000]`.
    pub size_mm: f64,
    /// `workshop` or `site`.
    pub location: String,
    /// `arrow`, `other` or `both`.
    pub side: String,
}

/// Closed initial member vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberKind {
    /// Straight beam.
    Beam,
    /// Straight column.
    Column,
}

/// Complete initial element, after host authorization and F04/F06 engineering validation.
#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    /// Full UUIDv7, never a caller-controlled USD path.
    pub id: String,
    /// Explicit user label.
    pub name: String,
    /// Authored parameters.
    pub shape: Shape,
    /// Explicit rigid placement.
    pub placement: Placement,
    /// Optional immutable material; absence means unknown.
    pub material: Option<ContentDigest>,
}

/// Contributor/engine-supplied immutable definitions, never a vendor catalogue.
#[derive(Debug, Clone, PartialEq)]
pub enum Definition {
    /// F06 section 2's only initial profile.
    RectangleProfile {
        /// Width in millimetres.
        width_mm: f64,
        /// Height in millimetres.
        height_mm: f64,
        /// Declared designation, standard and section numbers; absent values stay unknown.
        section: SectionProperties,
    },
    /// A07.1 section profile: a parametric family or an authored outline.
    SectionProfile {
        /// The section shape.
        shape: SectionShape,
        /// Declared designation, standard and section numbers; absent values stay unknown.
        section: SectionProperties,
    },
    /// Material with explicitly optional density.
    Material {
        /// Density in kg/m3; None remains unknown.
        density_kg_m3: Option<f64>,
        /// Declared grade, a label only.
        grade: Option<String>,
        /// Declared standard of the grade, a label only.
        standard: Option<String>,
        /// Readable name (prim `displayName` metadata), part of the definition content.
        display_name: Option<String>,
    },
}

/// Pinned definition with verified canonical USD bytes, not a second persisted model.
#[derive(Debug, Clone, PartialEq)]
pub struct Resource {
    /// Full SHA-256 of the isolated definition layer returned by [`Self::bytes`].
    pub digest: ContentDigest,
    /// Complete logical definition.
    pub definition: Definition,
}
impl Resource {
    /// Validates and pins a definition to its canonical codeless USD layer bytes.
    pub fn new(definition: Definition) -> Result<Self, Diagnostic> {
        let bytes = edit::definition_bytes(&definition)?;
        Ok(Self {
            digest: Sha256::digest(&bytes).map_err(|_| limit())?,
            definition,
        })
    }
    /// Recomputes exact identity bytes; callers must not substitute a different definition.
    pub fn bytes(&self) -> Result<Vec<u8>, Diagnostic> {
        let bytes = edit::definition_bytes(&self.definition)?;
        if Sha256::digest(&bytes).map_err(|_| limit())? != self.digest {
            return Err(corrupt());
        }
        Ok(bytes)
    }
}

/// Quantity records stored beside a derived mesh (F04 section 5): each has a status, an optional
/// value and the signature of the inputs it was computed from, so a reader can tell a current
/// record from a stale one without trusting it. Computed by the engine; this library only stores,
/// bounds and reads them.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredQuantities {
    /// Volume in mm3.
    pub volume_mm3: f64,
    /// `current`, `stale` or `unknown`.
    pub volume_status: String,
    /// Mass in kg, present exactly for a `current` mass.
    pub mass_kg: Option<f64>,
    /// `current`, `stale` or `unknown`.
    pub mass_status: String,
    /// Signature of the shape inputs of the volume.
    pub shape_signature: ContentDigest,
    /// Signature of the mass inputs: the shape signature and the density or its absence.
    pub mass_signature: ContentDigest,
    /// Algorithm label of the volume record, at most 128 bytes.
    pub algorithm: String,
}

/// Already computed F06 mesh data; no geometry or quantity algorithm runs in this crate.
#[derive(Debug, Clone, PartialEq)]
pub struct Mesh {
    /// Owning element UUIDv7.
    pub id: String,
    /// Verified engine shape signature, not a certification by this library.
    pub shape_key: ContentDigest,
    /// Standard USD point3f buffer, element-local millimetres.
    pub points_mm: Vec<[f32; 3]>,
    /// Polygon sizes.
    pub face_counts: Vec<i32>,
    /// Flattened point indices.
    pub indices: Vec<i32>,
    /// Optional stored quantity records of this body. A rewrite replaces them as a whole.
    pub quantities: Option<StoredQuantities>,
}

/// Closed worker-facing batch items, produced by a trusted engine, never raw USD setters.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// Author a new complete element.
    Create(Element),
    /// Update only the user label.
    Label {
        /// Full target UUIDv7.
        id: String,
        /// Exact new label.
        name: String,
    },
    /// Replace the complete contour; engineering validity is a host prerequisite.
    Contour {
        /// Full target UUIDv7.
        id: String,
        /// Ordered contour.
        points_mm: Vec<[f64; 3]>,
    },
    /// Replace a required plate thickness.
    Thickness {
        /// Full target UUIDv7.
        id: String,
        /// New thickness.
        mm: f64,
    },
    /// Replace both member axis points.
    Axis {
        /// Full target UUIDv7.
        id: String,
        /// Element-local axis.
        points_mm: [[f64; 3]; 2],
    },
    /// Change the optional roll opinion in the selected authored layer.
    Roll {
        /// Full target UUIDv7.
        id: String,
        /// Exact authored opinion.
        opinion: Opinion<f64>,
    },
    /// Replace the required immutable member profile.
    Profile {
        /// Full target UUIDv7.
        id: String,
        /// Verified profile content digest.
        digest: ContentDigest,
    },
    /// Change the optional material relationship.
    Material {
        /// Full target UUIDv7.
        id: String,
        /// Exact authored relationship opinion.
        opinion: MaterialOpinion,
    },
    /// Replace rigid placement, preserving shape inputs.
    Placement {
        /// Full target UUIDv7.
        id: String,
        /// Explicit rigid pose.
        placement: Placement,
    },
    /// Transfer a complete validated engine result to the derived layer.
    Mesh(Mesh),
    /// Remove the whole element: its authored prim in the authored layer and its result prims in
    /// the derived layer. The caller proves with [`Author::check_delete`] that nothing links to
    /// it; the definitions it pinned are immutable and stay.
    Delete {
        /// Full target UUIDv7.
        id: String,
    },
    /// R01: author a new wrapper around a foreign source whose layer is already a non-owned entry
    /// of the manifest. The only foreign-facing opinion written is the reference arc.
    AttachReference(Reference),
    /// R01: write or clear the explicit units and up-axis interpretation on the owned units level
    /// of a wrapper. The foreign content is never touched.
    ReferenceUnits {
        /// Full target UUIDv7 of the wrapper.
        id: String,
        /// The interpretation to store, or `None` to clear it.
        interpretation: Option<UnitsInterpretation>,
    },
    /// Set or remove the one `bim:partOf` record of an element (A01). The target must be an owned
    /// assembly; kinds, cardinality, self-links and cycles are re-checked on the candidate.
    PartOf {
        /// Full source UUIDv7: the part.
        id: String,
        /// Full UUIDv7 of the assembly, or `None` for an explicit empty record (a detach).
        assembly: Option<String>,
    },
    /// Set or remove the one `bim:level` record of a plate, beam or column (D2). The target must
    /// be an active owned level; kind, cardinality and dangling targets are re-checked on the
    /// candidate.
    Level {
        /// Full source UUIDv7: the plate, beam or column.
        id: String,
        /// Full UUIDv7 of the level, or `None` for an explicit empty record (an unassignment).
        level: Option<String>,
    },
    /// Author the mark of an element, or only its approval state (A08). Assigning a value writes
    /// both attributes; approving, revoking or withdrawing an approval writes only the status and
    /// keeps the value. Bounds, the markable kinds, the status pairing and the per-model
    /// uniqueness are re-checked on the candidate.
    Mark {
        /// Full target UUIDv7.
        id: String,
        /// The exact mark value to author, or `None` to leave it unchanged.
        value: Option<String>,
        /// The exact approval state to author, or `None` to leave it unchanged.
        status: Option<MarkStatus>,
    },
}
impl Change {
    pub(super) fn key(&self) -> (&str, &'static str) {
        match self {
            Self::Create(e) => (&e.id, "lifecycle"),
            Self::Delete { id } => (id, "lifecycle"),
            Self::Label { id, .. } => (id, "bim:name"),
            Self::Contour { id, .. } => (id, "steel:plate:contourPointsMm"),
            Self::Thickness { id, .. } => (id, "steel:plate:thicknessMm"),
            Self::Axis { id, .. } => (id, "bim:axisPointsMm"),
            Self::Roll { id, .. } => (id, "steel:member:rollRad"),
            Self::Profile { id, .. } => (id, "bim:profile"),
            Self::Material { id, .. } => (id, "bim:material"),
            Self::Placement { id, .. } => (id, "placement"),
            Self::Mesh(m) => (&m.id, "derived"),
            Self::AttachReference(r) => (&r.id, "lifecycle"),
            Self::ReferenceUnits { id, .. } => (id, "reference:units"),
            Self::PartOf { id, .. } => (id, "bim:partOf"),
            Self::Level { id, .. } => (id, "bim:level"),
            Self::Mark { id, .. } => (id, "bim:mark"),
        }
    }
}

/// Semantic dependency groups. Property-name inequality is never sufficient for independence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    /// Existence and identity.
    Lifecycle,
    /// User label.
    Label,
    /// Complete authored shape inputs and profile.
    ShapeInputs,
    /// Rigid placement and world-measurement dependencies.
    Placement,
    /// Material relationship and definition.
    Material,
    /// Geometry, quantities and dependent signatures.
    Derived,
    /// Links to immutable definitions.
    Relationship,
    /// The element mark and its approval state (A08).
    Mark,
}
/// Engine-derived conservative footprint entry.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Footprint {
    /// Complete element UUIDv7 or immutable content digest.
    pub identity: String,
    /// Semantic dependency group.
    pub group: Group,
}

/// Complete isolated prepare request. The host retains logical command/canonical replay bytes.
pub struct Prepare {
    /// Still-live complete context.
    pub context: Context,
    /// Logical F04 UUIDv7.
    pub op_id: String,
    /// Trusted creation actor, never taken from model bytes.
    pub actor: String,
    /// Exact mapping pin.
    pub schema: SchemaIdentity,
    /// New immutable resources; their identities are recomputed before use.
    pub resources: Vec<Resource>,
    /// Closed changes, at most the smaller of 4096 and the stage value budget.
    pub changes: Vec<Change>,
}
/// Host-selected edit targets; ownership must also be proved from composition provenance.
#[derive(Debug, Clone)]
pub struct EditTargets {
    /// Existing owned USDA authored layer.
    pub authored: String,
    /// Existing, distinct owned USDA derived layer.
    pub derived: String,
}
/// Opaque candidate incarnation, scoped to one owner and one pinned live view.
#[derive(Debug, Clone)]
pub struct CandidateToken(Arc<()>);
/// Bounded prepare result; it proves neither acceptance nor publication.
#[derive(Debug, Clone)]
pub struct Prepared {
    /// Opaque candidate token.
    pub token: CandidateToken,
    /// Matching logical operation.
    pub op_id: String,
    /// Admitted schema pin.
    pub schema: SchemaIdentity,
    /// Exact owned layer bytes and digests (unchanged source layers are retained verbatim).
    pub layers: Vec<Asset>,
    /// The layers the host did not select as owned (foreign sources), byte for byte as they were
    /// supplied. They are never rewritten; a revision keeps them as external entries.
    pub retained: Vec<Asset>,
    /// Exact local authored opinions for changed supported fields, ordered by UUID/attribute.
    /// These describe the selected edit layer, not resolved weaker defaults.
    pub opinions: Vec<AuthoredItem>,
    /// Conservative semantic read set.
    pub reads: BTreeSet<Footprint>,
    /// Conservative semantic write set.
    pub writes: BTreeSet<Footprint>,
    /// Separate structural validator result; no external usdchecker claim.
    pub validation: Validation,
}
/// Closed supported-field authored snapshot item, produced by the wrapper, never a setter.
#[derive(Debug, Clone, PartialEq)]
pub struct AuthoredItem {
    /// Full matching element UUIDv7.
    pub element_id: String,
    /// Fixed schema field name from the supported mapping.
    pub attribute: &'static str,
    /// Fixed USD type name; relationships use `rel`.
    pub type_name: &'static str,
    /// Exact local opinion; missing property specs are reported as Remove.
    pub opinion: Opinion<Value>,
}
/// Trusted host decision gate. This library cannot authenticate a durable ledger claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Proven durable host acceptance, required for install.
    Accepted,
    /// Proven durable host rejection, permits discard.
    Rejected,
    /// Host has not started recording a decision, permits discard.
    Undecided,
    /// Recording may have occurred: preserve the candidate and query the host ledger.
    Unknown,
}
/// Fixed-rule structural validation receipt, bounded independently of model strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Validation {
    /// Exact rule profile and implementation version.
    pub profile: &'static str,
    /// Fixed rules successfully checked.
    pub rules: &'static [&'static str],
    /// Complete snapshot prim count checked.
    pub prims: usize,
}

struct Pending {
    token: CandidateToken,
    op_id: String,
    request: OpenRequest,
    candidate: backend::Candidate,
}
/// One stage owner, one isolated candidate; no shared mutable stage or transport implementation.
pub struct Author {
    reader: Reader,
    request: OpenRequest,
    targets: EditTargets,
    pending: Option<Pending>,
}
impl Author {
    /// Opens verified bytes and checks edit-target participation/ownership and schema compatibility.
    pub fn open(
        request: OpenRequest,
        targets: EditTargets,
        schema: SchemaIdentity,
        control: &Control,
    ) -> Result<(Self, Opened), Diagnostic> {
        validate::schema(schema)?;
        let mut reader = Reader::default();
        let mut opened = reader.open(request.clone(), control)?;
        opened.capabilities.mutations = true;
        edit::targets(&request, &targets)?;
        let prims = &reader.pinned(&opened.context)?.prims;
        validate::snapshot(prims, &opened.metadata)?;
        reference::layers(&request, prims)?;
        Ok((
            Self {
                reader,
                request,
                targets,
                pending: None,
            },
            opened,
        ))
    }
    /// Reads the pinned composed snapshot, including inert unknown attributes/relationships.
    pub fn read(
        &self,
        context: &Context,
        cursor: Option<&Cursor>,
        control: &Control,
    ) -> Result<Page, Diagnostic> {
        self.reader.read(context, cursor, control)
    }
    /// Validates privately and retains at most one candidate; failure leaves all live bytes intact.
    pub fn prepare(
        &mut self,
        mut input: Prepare,
        control: &Control,
    ) -> Result<Prepared, Diagnostic> {
        control.check()?;
        validate::schema(input.schema)?;
        let live = self.reader.pinned(&input.context)?;
        if self.pending.is_some() {
            return Err(fail(
                ErrorCode::DependencyConflict,
                "candidate already prepared",
            ));
        }
        if !uuid7(&input.op_id) || !uuid7(&input.actor) {
            return Err(fail(
                ErrorCode::InvalidId,
                "invalid operation or actor identity",
            ));
        }
        let b = self.request.budgets;
        if input.changes.is_empty() && input.resources.is_empty() {
            return Err(fail(ErrorCode::InvalidArgument, "empty authored batch"));
        }
        if input.changes.len() > b.values.min(4096) || input.resources.len() > b.prims {
            return Err(limit());
        }
        validate::input_bounds(&input, b)?;
        input.changes.sort_by(|a, b| a.key().cmp(&b.key()));
        if input
            .changes
            .windows(2)
            .any(|v| matches!(v, [a,b] if a.key() == b.key()))
        {
            return Err(fail(
                ErrorCode::MalformedCommand,
                "duplicate normalized change key",
            ));
        }
        let (reads, writes) = validate::changes(&input, &live.prims, b)?;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let (mut request, opinions) =
                edit::apply(&self.request, &self.targets, &input, &live.prims, control)?;
            request.identity.generation = request
                .identity
                .generation
                .checked_add(1)
                .ok_or_else(limit)?;
            request.manifest = request.manifest_digest()?;
            let candidate = backend::open(request.clone(), control)?;
            let validation = validate::snapshot(&candidate.prims, &candidate.metadata)?;
            reference::layers(&request, &candidate.prims)?;
            control.check()?;
            Ok::<_, Diagnostic>((request, candidate, opinions, validation))
        }))
        .map_err(|_| {
            fail(
                ErrorCode::BackendUnavailable,
                "backend unwound while preparing authored layers",
            )
        })??;
        let (request, candidate, opinions, validation) = result;
        let token = CandidateToken(Arc::new(()));
        let output = Prepared {
            token: token.clone(),
            op_id: input.op_id.clone(),
            schema: input.schema,
            layers: request
                .assets
                .iter()
                .filter(|a| request.owned_layers.contains(&a.name))
                .cloned()
                .collect(),
            retained: request
                .assets
                .iter()
                .filter(|a| !request.owned_layers.contains(&a.name))
                .cloned()
                .collect(),
            opinions,
            reads,
            writes,
            validation,
        };
        control.check()?;
        self.pending = Some(Pending {
            token,
            op_id: input.op_id,
            request,
            candidate,
        });
        Ok(output)
    }
    fn candidate(
        &self,
        context: &Context,
        token: &CandidateToken,
        op_id: &str,
    ) -> Result<(), Diagnostic> {
        self.reader.pinned(context)?;
        if !self
            .pending
            .as_ref()
            .is_some_and(|p| Arc::ptr_eq(&p.token.0, &token.0) && p.op_id == op_id)
        {
            return Err(fail(
                ErrorCode::StaleWorkspace,
                "candidate is no longer live",
            ));
        }
        Ok(())
    }
    /// Installs only after host acceptance. No ledger record or durable publication is created here.
    /// Cancellation after proven acceptance returns OUTCOME_UNKNOWN and retains the candidate.
    pub fn install(
        &mut self,
        context: &Context,
        token: &CandidateToken,
        op_id: &str,
        decision: Decision,
        control: &Control,
    ) -> Result<Opened, Diagnostic> {
        self.candidate(context, token, op_id)?;
        if decision != Decision::Accepted || control.check().is_err() {
            return Err(fail(
                ErrorCode::OutcomeUnknown,
                "installation requires proven host acceptance and recovery",
            ));
        }
        // No fallible upstream work remains after this acceptance/cancellation gate.
        // A cancellation racing after it cannot undo proven acceptance or fabricate rollback.
        let pending = self.pending.take().ok_or_else(corrupt)?;
        let mut opened = self.reader.install_checked(pending.candidate);
        opened.capabilities.mutations = true;
        self.request = pending.request;
        Ok(opened)
    }
    /// Discards before recording or after proven rejection; uncertainty/acceptance retains the candidate.
    pub fn discard(
        &mut self,
        context: &Context,
        token: &CandidateToken,
        op_id: &str,
        decision: Decision,
        control: &Control,
    ) -> Result<(), Diagnostic> {
        control.check()?;
        self.candidate(context, token, op_id)?;
        if !matches!(decision, Decision::Undecided | Decision::Rejected) {
            return Err(fail(
                ErrorCode::OutcomeUnknown,
                "candidate decision must be resolved by host",
            ));
        }
        self.pending = None;
        Ok(())
    }
    /// Complete immutable manifest for host-owned storage; this is never an atomic save operation.
    pub fn export(&self, context: &Context, control: &Control) -> Result<Vec<Asset>, Diagnostic> {
        control.check()?;
        self.reader.pinned(context)?;
        Ok(self.request.assets.clone())
    }
}

/// Separate offline structural validator using the real bounded reader, not external usdchecker.
/// F04/F06 engineering admission and platform/SDK conformance remain separate gates.
pub fn validate_layers(
    request: OpenRequest,
    schema: SchemaIdentity,
    control: &Control,
) -> Result<Validation, Diagnostic> {
    validate::schema(schema)?;
    let candidate = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let candidate = backend::open(request.clone(), control)?;
        reference::layers(&request, &candidate.prims)?;
        Ok::<_, Diagnostic>(candidate)
    }))
    .map_err(|_| {
        fail(
            ErrorCode::BackendUnavailable,
            "backend unwound during validation",
        )
    })??;
    validate::snapshot(&candidate.prims, &candidate.metadata)
}

pub(super) fn element_path(id: &str) -> Result<String, Diagnostic> {
    if !uuid7(id) {
        return Err(fail(ErrorCode::InvalidId, "invalid full element identity"));
    }
    Ok(format!("/World/Steel/E_{}", id.replace('-', "")))
}
pub(super) fn definition_path(digest: ContentDigest) -> String {
    format!("/World/Definitions/D_{digest}")
}
pub(super) fn invalid() -> Diagnostic {
    fail(ErrorCode::InvalidArgument, "invalid authored field value")
}

#[cfg(test)]
mod pin_tests {
    use super::*;

    #[test]
    fn the_a07_2_mapping_pin_is_accepted_and_is_not_the_current_one() {
        // Pinned value: the digest of the descriptor of master with A07.2, before A08.
        let a07_2 = "fdaf890401a7bf4d8d17eb84ad1901b461ff21f1e7cec1e93f9e71203b83865b";
        assert_eq!(
            Sha256::digest(
                SCHEMA_DESCRIPTOR
                    .replacen(NUMBERING_DESCRIPTOR_LINE, "", 1)
                    .replacen(NUMBERING_CHANGES_ITEM, "|Level|Delete;", 1)
                    .as_bytes()
            )
            .unwrap()
            .to_string(),
            a07_2
        );
        assert!(pin_accepted(a07_2).unwrap());
        assert_ne!(a07_2, schema_identity().unwrap().digest.to_string());
    }

    #[test]
    fn the_a07_1_mapping_pin_is_accepted_and_is_not_the_current_one() {
        // Pinned value: the digest of the descriptor of master with A07.1, before D1 and D2.
        let a07_1 = "0311df71b2f7a4bd7b19e87ca7daef1884e989bd099bca24ee165de8e6bc310c";
        assert_eq!(
            Sha256::digest(A07_1_DESCRIPTOR.as_bytes())
                .unwrap()
                .to_string(),
            a07_1
        );
        assert!(pin_accepted(a07_1).unwrap());
        assert_ne!(a07_1, schema_identity().unwrap().digest.to_string());
    }

    #[test]
    fn the_d1_d2_mapping_pin_is_accepted_and_is_the_current_one_without_the_display_name_line() {
        // Pinned value: the digest of the descriptor of master with D1 and D2 (the level and the
        // project record), before A07.2 added the optional definition display name.
        let d1_d2 = "b799ae17004c22a5c2eaf7512367f919aec4b82b9b4b223166310286e1414b5b";
        assert!(SCHEMA_DESCRIPTOR.contains(A07_2_DISPLAY_NAME_LINE));
        assert!(!A07_1_DESCRIPTOR.contains(A07_2_DISPLAY_NAME_LINE));
        assert_eq!(
            Sha256::digest(
                SCHEMA_DESCRIPTOR
                    .replacen(NUMBERING_DESCRIPTOR_LINE, "", 1)
                    .replacen(NUMBERING_CHANGES_ITEM, "|Level|Delete;", 1)
                    .replacen(A07_2_DISPLAY_NAME_LINE, "", 1)
                    .as_bytes()
            )
            .unwrap()
            .to_string(),
            d1_d2
        );
        assert!(pin_accepted(d1_d2).unwrap());
        assert_ne!(d1_d2, schema_identity().unwrap().digest.to_string());
    }

    #[test]
    fn every_previously_published_mapping_pin_is_still_accepted() {
        let current = schema_identity().unwrap().digest.to_string();
        let pins = [
            // A07.1, D1/D2 (pre-A07.2), A06, A01, R01, P02.2 and U09 descriptors.
            "0311df71b2f7a4bd7b19e87ca7daef1884e989bd099bca24ee165de8e6bc310c",
            "b799ae17004c22a5c2eaf7512367f919aec4b82b9b4b223166310286e1414b5b",
            "b8c4a0cbfcb62a1bc9662d74965d5cc6f3cb945870a461af3fe8a8bc89352978",
            "74580523c1bbf5ed0f440d6c054e705586970128feb0bb6b973951f69955cbcd",
            "960c23cae54987cab86028cde90e360eb44ed64f8b6847f3c4f98b02d630ef56",
            "70fc49a1d81314a6599284532bd2d45c18f77b4318266238cc6923b25c05f28e",
        ];
        for pin in pins {
            assert!(pin_accepted(pin).unwrap(), "{pin}");
            assert_ne!(pin, current);
        }
        let r01 = Sha256::digest(R01_DESCRIPTOR.as_bytes())
            .unwrap()
            .to_string();
        assert!(pin_accepted(&r01).unwrap());
        assert!(pin_accepted(&current).unwrap());
        assert!(!pin_accepted(&"0".repeat(64)).unwrap());
    }

    #[test]
    fn the_a06_mapping_pin_is_accepted_and_is_not_the_current_one() {
        // Pinned value: the digest of the descriptor of master with A06, before A07.1.
        let a06 = "b8c4a0cbfcb62a1bc9662d74965d5cc6f3cb945870a461af3fe8a8bc89352978";
        assert_eq!(
            Sha256::digest(A06_DESCRIPTOR.as_bytes())
                .unwrap()
                .to_string(),
            a06
        );
        assert!(pin_accepted(a06).unwrap());
        assert_ne!(a06, schema_identity().unwrap().digest.to_string());
    }

    #[test]
    fn the_a01_mapping_pin_is_accepted_and_is_not_the_current_one() {
        // Pinned value: the digest of the descriptor of master with A01 and C01, before A06.
        let a01 = "74580523c1bbf5ed0f440d6c054e705586970128feb0bb6b973951f69955cbcd";
        assert_eq!(
            Sha256::digest(A01_DESCRIPTOR.as_bytes())
                .unwrap()
                .to_string(),
            a01
        );
        assert!(pin_accepted(a01).unwrap());
        assert_ne!(a01, schema_identity().unwrap().digest.to_string());
    }

    #[test]
    fn the_p02_2_mapping_pin_is_accepted_and_is_not_the_current_one() {
        // Pinned value: the digest of the descriptor of master with P02.2, before R01.
        let p02_2 = "960c23cae54987cab86028cde90e360eb44ed64f8b6847f3c4f98b02d630ef56";
        assert_eq!(
            Sha256::digest(P02_2_DESCRIPTOR.as_bytes())
                .unwrap()
                .to_string(),
            p02_2
        );
        assert!(pin_accepted(p02_2).unwrap());
        assert_ne!(p02_2, schema_identity().unwrap().digest.to_string());
    }

    #[test]
    fn the_previous_mapping_pin_is_accepted_and_others_are_not() {
        // Pinned value: the digest of the descriptor of master with U09, before P02.2.
        let previous = "70fc49a1d81314a6599284532bd2d45c18f77b4318266238cc6923b25c05f28e";
        assert_eq!(
            Sha256::digest(PREVIOUS_DESCRIPTOR.as_bytes())
                .unwrap()
                .to_string(),
            previous
        );
        assert!(pin_accepted(previous).unwrap());
        // The R01 descriptor (the master mapping before A01) is accepted too; its digest is
        // checked against the R01 mapping identity recomputed from the descriptor text.
        let r01 = Sha256::digest(R01_DESCRIPTOR.as_bytes())
            .unwrap()
            .to_string();
        assert!(pin_accepted(&r01).unwrap());
        assert_ne!(r01, schema_identity().unwrap().digest.to_string());
        assert!(pin_accepted(&schema_identity().unwrap().digest.to_string()).unwrap());
        assert!(!pin_accepted(&"0".repeat(64)).unwrap());
        assert!(!pin_accepted("").unwrap());
    }
}
