// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Codeless U02 schema and isolated layer authoring for a trusted command engine.
//! This is a library boundary, not an F04 command validator or acceptance ledger.

mod derive;
mod edit;
pub mod grid;
mod guard;
pub mod rebase;
mod reference;
mod validate;

pub use guard::WriteGrant;
pub use reference::{
    ForeignPrim, Reference, ReferenceView, SourceFormat, SourcePin, UnitsInterpretation, UpAxis,
    read_references,
};

use crate::*;
pub use beamcraft_model::command::{CutRecord, KeepSide};
pub use beamcraft_model::definition::SectionProperties;
use beamcraft_model::prim::{DefinitionCategory, ElementCategory};
pub use beamcraft_model::section::SectionShape;
use std::collections::BTreeMap;

/// Canonical UTF-8 schema descriptor. Its exact bytes commit the mapping and item profile.
/// No generated schema, plugin, fallback values or external schema resources are required.
pub const SCHEMA_DESCRIPTOR: &str = concat!(
    "BeamCraft/U02/codeless/1\nmodel=0.1.0;backend=openusd=0.8.0;features=none;save=usda-owned-v1\n",
    "root:metersPerUnit=0.001;upAxis=Z;customLayerData:bimSchemaDigest=sha256;",
    "bimProjectId=string(uuid7,optional,root layer only,written once with the first revision)\n",
    "part=/World/Steel/Parts/PRT_<uuid7-32>;kinds=plate|beam|column;type=Xform\n",
    "assembly=/World/Steel/Assemblies/ASM_<uuid7-32>;connection=/World/Steel/Connections/CON_<uuid7-32>;",
    "level=/World/Levels/LVL_<uuid7-32>;reference=/World/References/REF_<uuid7-32>;type=Xform\n",
    "bim:id=string;bim:name=string;bim:elementKind=token;bim:geometryMode=token(parametric);",
    "bim:isDeactivated=bool(optional,default=false);bim:schemaVersion=string(0.1.0);bim:createdBy=string(uuid7);bim:revision=string(uuid7,optional)\n",
    "steel:plate:contourPointsMm=double3[];steel:plate:thicknessMm=double;",
    "steel:plate:holePointsMm=double3[](optional,z=0);steel:plate:holeVertexCounts=int[](optional,>=3 each);",
    "steel:cut:operationId=string[](optional,uuid7);steel:cut:ordinal=int[](optional,<=999);",
    "steel:cut:pointMm=double3[](optional);steel:cut:normal=double3[](optional,nonzero each);",
    "steel:cut:keep=token[](optional,positive|negative);all five or none,equal lengths,ascending ordinals\n",
    "bim:axisPointsMm=double3[];steel:member:rollRad=double(optional)\n",
    "assembly=element;bim:elementKind=assembly;kind=assembly;no shape inputs, material or Geometry;placement=identity\n",
    "bim:partOf=rel(0..1 element target of kind assembly;source plate|beam|column|assembly;no self;acyclic)\n",
    "bim:mainPart=rel(0..1 element target that is a direct part of the assembly;source assembly;one main per assembly)\n",
    "connection=element;bim:elementKind=connection;kind=component;a weld (A06) or bolt group (A05);no material;placement=identity;",
    "no authored Geometry except a weld with steel:weld:pathMm\n",
    "steel:connection:type=token(weld|bolt_group);steel:weld:type=token(fillet);steel:weld:sizeMm=double(>0,<=1000);steel:weld:location=token(workshop|site);",
    "steel:weld:side=token(arrow|other|both);",
    "steel:weld:pathMm=double3[](optional,2..=1024 world points);steel:weld:allAround=token(true|false,optional);",
    "steel:weld:contour=token(none|flush|concave|convex,optional)\n",
    "steel:bolt:diameterMm=double(>0,<=1000);steel:bolt:count=int(1..=512);steel:bolt:pitchMm=double(>0,<=1e6);",
    "steel:bolt:originMm=double3;steel:bolt:directionMm=double3(unit)\n",
    "bim:connects=rel(2 distinct ordered element targets of kind plate|beam|column, main part first;source connection)\n",
    "grid=/World/Grids/GRD_<uuid7-32>;bim:discipline=token(steel);bim:grid:u|v:mode=token(regular|drawn);expression=string;spanMm=double[];ids=string[];labels=string[];shapes=token[];pointsMm=double[];no authored Geometry or material\n",
    "level=element;bim:elementKind=level;kind=component;bim:elevationMm=double(finite,abs<=1e9);no material or Geometry;placement=identity\n",
    "bim:level=rel(0..1 element target of kind level;source plate|beam|column)\n",
    "numbering=approved mark of a plate|beam|column|assembly (A09);steel:numbering:series=string(<=16 of A-Z a-z 0-9 _ -);",
    "steel:numbering:number=int(0..=2147483647);steel:numbering:basis=string(printable ASCII,<=512);",
    "all three or none;never on a connection, level or reference\n",
    "xformOp:translate=double3;xformOp:orient=quatd;xformOpOrder=token[](translate,orient)\n",
    "kind:/World=assembly;/World/Steel=group;element category folders=group;element=component;element/Geometry=subcomponent;",
    "displayName(element)=bim:name\n",
    "bim:profile=rel;bim:material=rel;material=/World/Definitions/Materials/MAT_<sha256-64>;profile=/World/Definitions/Profiles/PRF_<sha256-64>\n",
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
    "reference=/World/References/REF_<uuid7-32>;type=Xform;kind=group;bim:elementKind=reference;bim:geometryMode=reference;",
    "bim:reference:sourceDigest=string(sha256);bim:reference:sourceBytes=int;bim:reference:sourceFormat=token(usda|usdc);",
    "bim:reference:sourceVersion=int;bim:reference:sourceLabel=string;bim:reference:sourceDefaultPrim=string;",
    "bim:reference:declaredMetersPerUnit=double(optional);bim:reference:declaredUpAxis=token(Y|Z,optional)\n",
    "reference/Correction=Xform;kind=group;bim:reference:correctionMetersPerUnit=double;bim:reference:correctionUpAxis=token(Y|Z);",
    "bim:reference:correctionSourceDigest=string(sha256);xformOp:rotateX:bimUpAxis=double(degrees,Y up only);",
    "xformOp:scale:bimUnits=double3(metersPerUnit*1000);xformOpOrder=token[](rotateX,scale;empty without correction)\n",
    "reference/Correction/Source=def without type;references=@external/<sha256>.<usda|usdc>@</defaultPrim>;nothing else owned at or below\n",
    "changes=Create|Label|Contour|Thickness|Axis|Roll|Profile|Material|Placement|Mesh|AttachReference|ReferenceUnits|PartOf|MainPart|Level|Mark|Delete;",
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

/// Only the U16 mapping is admitted. Pre-alpha stores are deliberately not migrated.
pub(crate) fn pin_accepted(pin: &str) -> Result<bool, Diagnostic> {
    Ok(pin == schema_identity()?.digest.to_string())
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

/// Names of the A09 numbering properties of a plate, beam, column or assembly. A reader outside
/// this crate (IFC and BOM export) reads exactly these three: the mark of an element is its
/// series prefix followed by its number, with a `-` between them for an assembly.
pub const MARK_SERIES: &str = "steel:numbering:series";
/// See [`MARK_SERIES`].
pub const MARK_NUMBER: &str = "steel:numbering:number";
/// The record of the approval that wrote the mark (settings and preview token), stored with every
/// mark so a later read can tell whether the model still matches what was approved.
pub const MARK_BASIS: &str = "steel:numbering:basis";
/// Longest series prefix in characters.
pub const MARK_MAX_SERIES: usize = 16;
/// Longest approval record in bytes.
pub const MARK_MAX_BASIS: usize = 512;

/// The approved mark of one element (A09): where it stands in its numbering series and the
/// record of the approval. The engine decides the values; this library stores and bounds them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mark {
    /// Series prefix: at most 16 of letters, digits, `_` and `-`. May be empty.
    pub series: String,
    /// Number in the series, at most `i32::MAX`.
    pub number: u32,
    /// Approval record: printable ASCII, at most 512 bytes.
    pub basis: String,
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
        /// The owned `plane_cut` operation records (A03, amendment A-2) in ascending ordinal
        /// order, stored as five parallel attributes. Empty means an uncut plate.
        cuts: Vec<CutRecord>,
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
        /// The owned `plane_cut` operation records (A03, amendment A-2) in ascending ordinal
        /// order. Empty means an uncut member.
        cuts: Vec<CutRecord>,
    },
    /// A01 assembly: a grouping target of `bim:partOf` with no shape inputs, no material and no
    /// derived body. Its placement must be the identity in this slice.
    Assembly,
    /// A06 weld: a `connection` element with explicitly authored weld properties and one
    /// `bim:connects` record towards two distinct parts. No material and no derived body; its
    /// placement must be the identity in this slice.
    Weld(Weld),
    /// A05 bolt group: a `connection` element with explicitly authored bolt properties and one
    /// `bim:connects` record towards two distinct parts. Like a weld it has no material, no
    /// derived body and the identity placement in this slice.
    Bolt(Bolt),
    /// T01 grid, with no authored engineering body.
    Grid(beamcraft_model::grid::Grid),
    /// D2 level (a storey): an element of kind `level` with a stored elevation, no material and no
    /// derived body. Plates, beams and columns are assigned to it by [`Change::Level`]. Its
    /// placement must be the identity: the elevation is a value, never a pose.
    Level {
        /// Elevation in millimetres, finite and at most 1,000,000,000 in magnitude.
        elevation_mm: f64,
    },
}

impl Shape {
    /// The owned `plane_cut` records of a plate, beam or column (A03); other kinds have none.
    pub(crate) fn cuts(&self) -> &[CutRecord] {
        match self {
            Self::Plate { cuts, .. } | Self::Member { cuts, .. } => cuts,
            Self::Assembly | Self::Weld(_) | Self::Bolt(_) | Self::Grid(_) | Self::Level { .. } => {
                &[]
            }
        }
    }
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
    /// Optional geometry (A15): the path the weld runs along and its symbol data. `None` authors
    /// an A06 weld without a body; `Some` also derives a solid under the connection.
    pub geometry: Option<WeldPath>,
}

/// The authored geometry of one weld (A15). The path is an ordered world polyline in
/// millimetres; the symbol tokens are the exact stored values.
#[derive(Debug, Clone, PartialEq)]
pub struct WeldPath {
    /// Ordered world points of the path, 2 to 1024, each within the coordinate cap.
    pub points_mm: Vec<[f64; 3]>,
    /// All-around symbol. Symbol data only.
    pub all_around: bool,
    /// `none`, `flush`, `concave` or `convex`.
    pub contour: String,
}

/// The authored inputs of a bolt group (A05). The group is `count` bolts in a straight world line
/// `origin + direction * (pitch * i)`; the direction is a stored unit vector.
#[derive(Debug, Clone, PartialEq)]
pub struct Bolt {
    /// Full UUIDv7 of the main part, then of the part bolted to it. Distinct, each a plate, beam
    /// or column; the order is significant.
    pub connects: [String; 2],
    /// Nominal bolt diameter in millimetres, in `(0, 1000]`.
    pub diameter_mm: f64,
    /// Bolts in the group, 1 to 512.
    pub count: usize,
    /// Centre-to-centre distance along the line in millimetres, in `(0, 1000000]`.
    pub pitch_mm: f64,
    /// World position of the first bolt in millimetres, each component within the coordinate cap.
    pub origin_mm: [f64; 3],
    /// Unit world direction of the line: finite, squared norm within 1e-6 of one.
    pub direction_mm: [f64; 3],
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

/// Closed batch items, produced by a trusted engine, never raw USD setters.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// Author a new complete element.
    Create(Element),
    /// Replace validated grid-family attributes without changing its frame or identity.
    Grid {
        /// Existing GridId.
        id: String,
        /// Complete replacement record from the grid engine.
        grid: beamcraft_model::grid::Grid,
    },
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
    /// Replace the owned plane-cut records of a plate, beam or column (A03, amendment A-2) as a
    /// whole: five parallel attributes, or their removal for an empty list. Engineering validity
    /// is a host prerequisite; the candidate re-validates the stored form.
    Cuts {
        /// Full target UUIDv7.
        id: String,
        /// The records in ascending ordinal order.
        cuts: Vec<CutRecord>,
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
    /// Name the one main part of an assembly (A16.1). The record lives on the assembly; the
    /// target must be a direct part of it, re-checked on the candidate. There is no clear: the
    /// record is replaced by naming another part, and detaching the main part is refused.
    MainPart {
        /// Full source UUIDv7: the assembly.
        id: String,
        /// Full UUIDv7 of the main part.
        part: String,
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
    /// Write the approved mark of a plate, beam, column or assembly (A09): three optional
    /// properties of the element in the authored layer, replaced as a whole. Nothing derived
    /// depends on a mark, so no result is invalidated.
    Mark {
        /// Full target UUIDv7.
        id: String,
        /// The mark to store.
        mark: Mark,
    },
}
impl Change {
    pub(super) fn key(&self) -> (&str, &'static str) {
        match self {
            Self::Create(e) => (&e.id, "lifecycle"),
            Self::Delete { id } => (id, "lifecycle"),
            Self::Label { id, .. } => (id, "bim:name"),
            Self::Grid { id, .. } => (id, "bim:grid"),
            Self::Contour { id, .. } => (id, "steel:plate:contourPointsMm"),
            Self::Thickness { id, .. } => (id, "steel:plate:thicknessMm"),
            Self::Cuts { id, .. } => (id, "steel:cut"),
            Self::Axis { id, .. } => (id, "bim:axisPointsMm"),
            Self::Roll { id, .. } => (id, "steel:member:rollRad"),
            Self::Profile { id, .. } => (id, "bim:profile"),
            Self::Material { id, .. } => (id, "bim:material"),
            Self::Placement { id, .. } => (id, "placement"),
            Self::Mesh(m) => (&m.id, "derived"),
            Self::AttachReference(r) => (&r.id, "lifecycle"),
            Self::ReferenceUnits { id, .. } => (id, "reference:units"),
            Self::PartOf { id, .. } => (id, "bim:partOf"),
            Self::MainPart { id, .. } => (id, "bim:mainPart"),
            Self::Level { id, .. } => (id, "bim:level"),
            Self::Mark { id, .. } => (id, "steel:numbering"),
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
    validated(&request, schema, control).map(|(_, validation)| validation)
}

/// The rules of [`validate_layers`], in one place, with the candidate that passed them:
/// [`Reader::open_validated`] installs that candidate, so a rule added here applies to both.
fn validated(
    request: &OpenRequest,
    schema: SchemaIdentity,
    control: &Control,
) -> Result<(backend::Candidate, Validation), Diagnostic> {
    validate::schema(schema)?;
    let candidate = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let candidate = backend::open(request.clone(), control)?;
        edit::schema_pins(request)?;
        reference::layers(request, &candidate.prims)?;
        Ok::<_, Diagnostic>(candidate)
    }))
    .map_err(|_| {
        fail(
            ErrorCode::BackendUnavailable,
            "backend unwound during validation",
        )
    })??;
    let validation = validate::snapshot(&candidate.prims, &candidate.metadata)?;
    Ok((candidate, validation))
}

impl Reader {
    /// [`validate_layers`] and then [`Reader::open`] of the same request, in one pass: the
    /// structural rules run on the very candidate that is then installed, so the stage is parsed
    /// and composed once instead of twice. The checks, their order, codes and messages are those
    /// of the two calls in sequence, and nothing is installed unless every rule passed.
    pub fn open_validated(
        &mut self,
        request: OpenRequest,
        schema: SchemaIdentity,
        control: &Control,
    ) -> Result<(Opened, Validation), Diagnostic> {
        let (candidate, validation) = validated(&request, schema, control)?;
        let opened = self.install(candidate, control)?;
        Ok((opened, validation))
    }
}

pub(super) fn element_path(id: &str, category: ElementCategory) -> Result<String, Diagnostic> {
    beamcraft_model::id::ElementId::parse(id)
        .map(|id| id.prim_path(category))
        .map_err(|_| fail(ErrorCode::InvalidId, "invalid full element identity"))
}
fn shape_category(shape: &Shape) -> ElementCategory {
    match shape {
        Shape::Assembly => ElementCategory::Assembly,
        Shape::Weld(_) | Shape::Bolt(_) => ElementCategory::Connection,
        Shape::Level { .. } => ElementCategory::Level,
        Shape::Grid(_) => ElementCategory::Grid,
        Shape::Plate { .. } | Shape::Member { .. } => ElementCategory::Part,
    }
}
fn change_path(id: &str, input: &Prepare, prims: &[Prim]) -> Result<String, Diagnostic> {
    if let Some(p) = prims.iter().find(|p| p.element_id.as_deref() == Some(id)) {
        return Ok(p.path.clone());
    }
    for change in &input.changes {
        match change {
            Change::Create(e) if e.id == id => return element_path(id, shape_category(&e.shape)),
            Change::AttachReference(r) if r.id == id => {
                return element_path(id, ElementCategory::Reference);
            }
            _ => (),
        }
    }
    Err(fail(ErrorCode::NotFound, "element not found"))
}
/// Resolve a relationship by identity before checking its semantic kind. A missing identity
/// retains its expected-category path so the candidate reports a dangling relationship.
fn target_path(
    id: &str,
    category: ElementCategory,
    input: &Prepare,
    prims: &[Prim],
) -> Result<String, Diagnostic> {
    change_path(id, input, prims).or_else(|error| {
        if error.code == ErrorCode::NotFound {
            element_path(id, category)
        } else {
            Err(error)
        }
    })
}
fn definition_category(definition: &Definition) -> DefinitionCategory {
    match definition {
        Definition::Material { .. } => DefinitionCategory::Material,
        Definition::RectangleProfile { .. } | Definition::SectionProfile { .. } => {
            DefinitionCategory::Profile
        }
    }
}
pub(super) fn definition_path(digest: ContentDigest, category: DefinitionCategory) -> String {
    category.path(digest)
}
pub(super) fn invalid() -> Diagnostic {
    fail(ErrorCode::InvalidArgument, "invalid authored field value")
}

#[cfg(test)]
mod pin_tests {
    use super::*;
    #[test]
    fn only_the_current_mapping_is_accepted() {
        assert!(pin_accepted(&schema_identity().unwrap().digest.to_string()).unwrap());
        let before_f10 = SCHEMA_DESCRIPTOR
            .replace("bim:isDeactivated=bool(optional,default=false);", "")
            .replace("steel:connection:type=token(weld|bolt_group);", "");
        let old_pin = Sha256::digest(before_f10.as_bytes()).unwrap();
        assert!(!pin_accepted(&old_pin.to_string()).unwrap());
        for pin in [
            "f351bf3e2d6f52dc65605dd48c44dae4fbee940b3c69ffcf8e558792b2a7ce37",
            "0311df71b2f7a4bd7b19e87ca7daef1884e989bd099bca24ee165de8e6bc310c",
            "085a4f27db4ba0e60eea1a88630df1ab9984930a8e4e4a9bd4f3eb31f812d012",
            "70fc49a1d81314a6599284532bd2d45c18f77b4318266238cc6923b25c05f28e",
            "74580523c1bbf5ed0f440d6c054e705586970128feb0bb6b973951f69955cbcd",
            "960c23cae54987cab86028cde90e360eb44ed64f8b6847f3c4f98b02d630ef56",
            "b799ae17004c22a5c2eaf7512367f919aec4b82b9b4b223166310286e1414b5b",
            "b8c4a0cbfcb62a1bc9662d74965d5cc6f3cb945870a461af3fe8a8bc89352978",
            "cf6afbdfecf652ea85f9b89319a4cda389b64ce3bd71ca63e268ea4c1007ec86",
            "fdaf890401a7bf4d8d17eb84ad1901b461ff21f1e7cec1e93f9e71203b83865b",
            "",
            "unknown",
        ] {
            assert!(!pin_accepted(pin).unwrap(), "{pin}");
        }
    }
}
