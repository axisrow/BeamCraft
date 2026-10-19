// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! The fixed entity templates, in the emission order of E01 section 11.2.
//!
//! There is no generic IFC object graph, no schema table and no reflection: every entity is one
//! template here. Instance numbers come from one counter in emission order, so the same model
//! gives the same bytes. Nothing is read from a clock, the environment or a path.

use crate::guid::{self, GlobalId};
use crate::model::{
    Declared, ElementData, Entity, ExportModel, LevelData, Limits, MaterialData, Mesh, Pose,
};
use crate::step::{self, BoundedSink, SinkError};
use beamcraft_model::digest::ContentDigest;
use beamcraft_model::id::Uuid7;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;

/// The exporter version written into the header (`IFCX-DET-005`).
pub const EXPORTER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Characters of an `IfcLabel` attribute (IFC4 `IfcLabel`).
const LABEL_CHARS: usize = 255;

/// Description of every declared property (`IFCX-DECL-002`).
const DECLARED_NOTE: &str = "Declared in the profile definition; not computed by BeamCraft.";

/// Why an export was refused.
#[derive(Debug)]
pub enum WriteError {
    /// An input value the writer cannot represent (`INVALID_ARGUMENT`).
    Invalid(&'static str),
    /// A bound of section 14 was exceeded (`LIMIT_EXCEEDED`).
    Limit(&'static str),
    /// The input contradicts itself (`INTEGRITY_ERROR`).
    Integrity(&'static str),
    /// The output sink failed or reached its cap.
    Sink(SinkError),
}

impl From<SinkError> for WriteError {
    fn from(error: SinkError) -> Self {
        Self::Sink(error)
    }
}

/// What a truncated text belonged to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Subject {
    /// The project name.
    Project,
    /// A level.
    Level(Uuid7),
    /// An element.
    Element(Uuid7),
    /// A material.
    Material(ContentDigest),
}

/// A text written shorter than it is (`IFC_NAME_TRUNCATED`, `IFCX-PSET-004`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Truncation {
    /// Whose text.
    pub subject: Subject,
    /// The fixed schema field token.
    pub field: &'static str,
}

/// The result of a successful write.
#[derive(Debug)]
pub struct WriteOutcome {
    /// Bytes written.
    pub bytes: u64,
    /// SHA-256 of the bytes.
    pub sha256: ContentDigest,
    /// Number of STEP instances.
    pub instances: u64,
    /// Truncated texts, in emission order.
    pub truncations: Vec<Truncation>,
}

/// The GlobalId of an element or level (`IFCX-GUID-001`).
pub fn global_id(id: &Uuid7) -> GlobalId {
    guid::encode(id)
}

/// The GlobalId of the `IfcBuildingStorey` of a level (`IFCX-GUID-004`, role `storey`): a
/// function of the stored ProjectId and the LevelId, not the encoding of the LevelId itself.
pub fn storey_global_id(project: &Uuid7, level: &Uuid7) -> GlobalId {
    guid::synthetic("storey", &[project.as_bytes(), level.as_bytes()])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Ref(u64);

impl Ref {
    fn s(self) -> String {
        format!("#{}", self.0)
    }
}

fn refs(items: &[Ref]) -> String {
    let parts: Vec<String> = items.iter().map(|r| r.s()).collect();
    format!("({})", parts.join(","))
}

fn real(value: f64) -> Result<String, WriteError> {
    step::real64(value).ok_or(WriteError::Invalid("a number is not finite"))
}

fn real3(value: [f64; 3]) -> Result<String, WriteError> {
    Ok(format!(
        "({},{},{})",
        real(value[0])?,
        real(value[1])?,
        real(value[2])?
    ))
}

fn opt_string(value: Option<&str>) -> String {
    value.map_or_else(|| "$".into(), step::string)
}

struct Writer<W: Write> {
    sink: BoundedSink<W>,
    next: u64,
    used: BTreeSet<GlobalId>,
    truncations: Vec<Truncation>,
    project: [u8; 16],
}

impl<W: Write> Writer<W> {
    fn put(&mut self, text: &str) -> Result<(), WriteError> {
        Ok(self.sink.put(text)?)
    }

    /// Writes `#n=BODY;` with the next instance number.
    fn line(&mut self, body: &str) -> Result<Ref, WriteError> {
        let number = self
            .next
            .checked_add(1)
            .ok_or(WriteError::Limit("instance counter overflow"))?;
        self.next = number;
        self.put(&format!("#{number}={body};\n"))?;
        Ok(Ref(number))
    }

    /// A GlobalId literal; a repeated one refuses the export (`IFCX-GUID-006`).
    fn gid(&mut self, id: GlobalId) -> Result<String, WriteError> {
        if !self.used.insert(id) {
            return Err(WriteError::Integrity("a GlobalId occurs twice in one file"));
        }
        Ok(format!("'{}'", id.as_str()))
    }

    fn synthetic(&mut self, role: &str, keys: &[&[u8]]) -> Result<String, WriteError> {
        self.gid(guid::synthetic(role, keys))
    }

    /// An `IfcLabel` attribute: at most 255 code points; a longer text is cut and recorded
    /// (`IFCX-PSET-004`). Returns the literal and whether it was cut.
    fn label(&mut self, text: &str, subject: Subject, field: &'static str) -> (String, bool) {
        if text.chars().count() > LABEL_CHARS {
            let cut: String = text.chars().take(LABEL_CHARS).collect();
            self.truncations.push(Truncation { subject, field });
            (step::string(&cut), true)
        } else {
            (step::string(text), false)
        }
    }

    fn point(&mut self, p: [f64; 3]) -> Result<Ref, WriteError> {
        self.line(&format!("IFCCARTESIANPOINT({})", real3(p)?))
    }

    fn direction(&mut self, d: [f64; 3]) -> Result<Ref, WriteError> {
        self.line(&format!("IFCDIRECTION({})", real3(d)?))
    }

    /// An `IfcLocalPlacement` over an axis placement of `pose`, relative to `rel`.
    fn placement(&mut self, rel: Option<Ref>, pose: &Pose) -> Result<Ref, WriteError> {
        let location = self.point(pose.translation_mm)?;
        let axis = self.direction(pose.axis)?;
        let reference = self.direction(pose.ref_direction)?;
        let axes = self.line(&format!(
            "IFCAXIS2PLACEMENT3D({},{},{})",
            location.s(),
            axis.s(),
            reference.s()
        ))?;
        self.local_placement(rel, axes)
    }

    fn local_placement(&mut self, rel: Option<Ref>, axes: Ref) -> Result<Ref, WriteError> {
        self.line(&format!(
            "IFCLOCALPLACEMENT({},{})",
            rel.map_or_else(|| "$".into(), Ref::s),
            axes.s()
        ))
    }

    /// A placement with default axes at `at`.
    fn plain_placement(&mut self, rel: Option<Ref>, at: [f64; 3]) -> Result<Ref, WriteError> {
        let location = self.point(at)?;
        let axes = self.line(&format!("IFCAXIS2PLACEMENT3D({},$,$)", location.s()))?;
        self.local_placement(rel, axes)
    }

    fn single(
        &mut self,
        name: &str,
        value: &str,
        description: Option<&str>,
    ) -> Result<Ref, WriteError> {
        self.line(&format!(
            "IFCPROPERTYSINGLEVALUE({},{},{},$)",
            step::string(name),
            opt_string(description),
            value
        ))
    }
}

fn ident(text: &str) -> String {
    format!("IFCIDENTIFIER({})", step::string(text))
}

fn text(value: &str) -> String {
    format!("IFCTEXT({})", step::string(value))
}

fn label_value(value: &str) -> String {
    format!("IFCLABEL({})", step::string(value))
}

fn check_order<T, K: Ord>(items: &[T], key: impl Fn(&T) -> K) -> bool {
    items.windows(2).all(|w| match w {
        [a, b] => key(a) < key(b),
        _ => true,
    })
}

fn validate(model: &ExportModel, limits: &Limits) -> Result<(), WriteError> {
    if model.elements.len() > limits.max_elements {
        return Err(WriteError::Limit("too many elements"));
    }
    if model.levels.len() > limits.max_levels {
        return Err(WriteError::Limit("too many levels"));
    }
    if !check_order(&model.elements, |e| e.id)
        || !check_order(&model.levels, |l| l.id)
        || !check_order(&model.materials, |m| *m.digest.as_bytes())
    {
        return Err(WriteError::Integrity(
            "elements, levels or materials are not in strictly ascending order",
        ));
    }
    let levels: BTreeSet<Uuid7> = model.levels.iter().map(|l| l.id).collect();
    let materials: BTreeSet<ContentDigest> = model.materials.iter().map(|m| m.digest).collect();
    for e in &model.elements {
        if e.level.is_some_and(|l| !levels.contains(&l)) {
            return Err(WriteError::Integrity("an element names an unknown level"));
        }
        if e.material.is_some_and(|m| !materials.contains(&m)) {
            return Err(WriteError::Integrity(
                "an element names an unknown material",
            ));
        }
        if let Some(mesh) = &e.mesh {
            if e.pose.is_none() {
                return Err(WriteError::Integrity("a mesh without a placement"));
            }
            if mesh.points.len() > limits.max_vertices
                || mesh.triangles.len() > limits.max_triangles
            {
                return Err(WriteError::Limit("a mesh is larger than the bound"));
            }
            let count = mesh.points.len() as u64;
            let in_range = mesh
                .triangles
                .iter()
                .all(|t| t.iter().all(|i| u64::from(*i) < count));
            let finite = mesh.points.iter().all(|p| p.iter().all(|v| v.is_finite()));
            if mesh.points.is_empty() || mesh.triangles.is_empty() || !in_range || !finite {
                return Err(WriteError::Integrity("a mesh is empty or inconsistent"));
            }
        }
    }
    Ok(())
}

/// Writes the model as one IFC4 STEP file to `out` and returns the writer with the outcome.
///
/// Validates the whole model and the bounds before the first byte, then streams the fixed
/// sequence of 11.2 through a sink that refuses to pass `limits.max_bytes`.
pub fn write_ifc<W: Write>(
    model: &ExportModel,
    limits: &Limits,
    out: W,
) -> Result<(W, WriteOutcome), WriteError> {
    validate(model, limits)?;
    let mut w = Writer {
        sink: BoundedSink::new(out, limits.max_bytes),
        next: 0,
        used: BTreeSet::new(),
        truncations: Vec::new(),
        project: *model.project_id.as_bytes(),
    };
    header(&mut w, model)?;
    let (ctx, units) = units_and_context(&mut w)?;
    let spatial = spatial_structure(&mut w, model, ctx, units)?;
    let materials = materials(&mut w, model)?;
    let mut contained: BTreeMap<Option<Uuid7>, Vec<Ref>> = BTreeMap::new();
    let mut associated: BTreeMap<ContentDigest, Vec<Ref>> = BTreeMap::new();
    for element in &model.elements {
        let product = emit_element(&mut w, element, &spatial, ctx.body)?;
        contained.entry(element.level).or_default().push(product);
        if let Some(material) = element.material {
            associated.entry(material).or_default().push(product);
        }
    }
    containment(&mut w, model, &spatial, &contained)?;
    for material in &model.materials {
        if let (Some(members), Some(&at)) = (
            associated.get(&material.digest),
            materials.get(&material.digest),
        ) {
            let gid = w.synthetic(
                "rel-material",
                &[&w.project.clone(), material.digest.as_bytes()],
            )?;
            w.line(&format!(
                "IFCRELASSOCIATESMATERIAL({gid},$,$,$,{},{})",
                refs(members),
                at.s()
            ))?;
        }
    }
    w.put("ENDSEC;\nEND-ISO-10303-21;\n")?;
    let instances = w.next;
    let truncations = w.truncations;
    let (inner, bytes, sha256) = w.sink.finish()?;
    Ok((
        inner,
        WriteOutcome {
            bytes,
            sha256,
            instances,
            truncations,
        },
    ))
}

fn header<W: Write>(w: &mut Writer<W>, model: &ExportModel) -> Result<(), WriteError> {
    let stamp = step::utc_timestamp(model.revision_unix_ms);
    w.put("ISO-10303-21;\nHEADER;\n")?;
    w.put("FILE_DESCRIPTION(('BeamCraft IFC export profile 0.1.0'),'2;1');\n")?;
    w.put(&format!(
        "FILE_NAME('beamcraft-export.ifc','{stamp}',(''),(''),'BeamCraft IFC writer {EXPORTER_VERSION}','BeamCraft {EXPORTER_VERSION}','');\n"
    ))?;
    w.put("FILE_SCHEMA(('IFC4'));\nENDSEC;\nDATA;\n")
}

#[derive(Clone, Copy)]
struct Contexts {
    body: Ref,
    parent: Ref,
}

fn units_and_context<W: Write>(w: &mut Writer<W>) -> Result<(Contexts, Ref), WriteError> {
    let length = w.line("IFCSIUNIT(*,.LENGTHUNIT.,.MILLI.,.METRE.)")?;
    let angle = w.line("IFCSIUNIT(*,.PLANEANGLEUNIT.,$,.RADIAN.)")?;
    let area = w.line("IFCSIUNIT(*,.AREAUNIT.,$,.SQUARE_METRE.)")?;
    let volume = w.line("IFCSIUNIT(*,.VOLUMEUNIT.,$,.CUBIC_METRE.)")?;
    let mass = w.line("IFCSIUNIT(*,.MASSUNIT.,.KILO.,.GRAM.)")?;
    let metre = w.line("IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.)")?;
    let kilogram = w.line(&format!("IFCDERIVEDUNITELEMENT({},1)", mass.s()))?;
    let per_cube = w.line(&format!("IFCDERIVEDUNITELEMENT({},-3)", metre.s()))?;
    let density = w.line(&format!(
        "IFCDERIVEDUNIT({},.MASSDENSITYUNIT.,$)",
        refs(&[kilogram, per_cube])
    ))?;
    let assignment = w.line(&format!(
        "IFCUNITASSIGNMENT({})",
        refs(&[length, angle, area, volume, mass, density])
    ))?;
    let origin = w.point([0.0; 3])?;
    let world = w.line(&format!("IFCAXIS2PLACEMENT3D({},$,$)", origin.s()))?;
    let parent = w.line(&format!(
        "IFCGEOMETRICREPRESENTATIONCONTEXT($,'Model',3,0.001,{},$)",
        world.s()
    ))?;
    let body = w.line(&format!(
        "IFCGEOMETRICREPRESENTATIONSUBCONTEXT('Body','Model',*,*,*,*,{},$,.MODEL_VIEW.,$)",
        parent.s()
    ))?;
    Ok((Contexts { body, parent }, assignment))
}

struct Spatial {
    building_placement: Ref,
    building: Ref,
    storeys: BTreeMap<Uuid7, Ref>,
}

fn spatial_structure<W: Write>(
    w: &mut Writer<W>,
    model: &ExportModel,
    ctx: Contexts,
    units: Ref,
) -> Result<Spatial, WriteError> {
    let project = w.project;
    let gid = w.synthetic("project", &[&project])?;
    let (name, name_cut) = match &model.project_name {
        Some(n) => {
            let (literal, cut) = w.label(n, Subject::Project, "project.name");
            (literal, cut)
        }
        None => ("$".into(), false),
    };
    let description = match (&model.project_name, name_cut) {
        (Some(n), true) => step::string(n),
        _ => "$".into(),
    };
    let project_ref = w.line(&format!(
        "IFCPROJECT({gid},$,{name},{description},$,$,$,{},{})",
        refs(&[ctx.parent]),
        units.s()
    ))?;
    let site_placement = w.plain_placement(None, [0.0; 3])?;
    let gid = w.synthetic("site", &[&project])?;
    let site = w.line(&format!(
        "IFCSITE({gid},$,'Site',$,$,{},$,$,.ELEMENT.,$,$,$,$,$)",
        site_placement.s()
    ))?;
    let building_placement = w.plain_placement(Some(site_placement), [0.0; 3])?;
    let gid = w.synthetic("building", &[&project])?;
    let building = w.line(&format!(
        "IFCBUILDING({gid},$,'Building',$,$,{},$,$,.ELEMENT.,$,$,$)",
        building_placement.s()
    ))?;
    let mut storeys = BTreeMap::new();
    let mut order = Vec::new();
    for level in &model.levels {
        let at = emit_storey(w, level, building_placement)?;
        storeys.insert(level.id, at);
        order.push(at);
    }
    let gid = w.synthetic("rel-project-site", &[&project])?;
    w.line(&format!(
        "IFCRELAGGREGATES({gid},$,$,$,{},{})",
        project_ref.s(),
        refs(&[site])
    ))?;
    let gid = w.synthetic("rel-site-building", &[&project])?;
    w.line(&format!(
        "IFCRELAGGREGATES({gid},$,$,$,{},{})",
        site.s(),
        refs(&[building])
    ))?;
    if !order.is_empty() {
        let gid = w.synthetic("rel-building-storeys", &[&project])?;
        w.line(&format!(
            "IFCRELAGGREGATES({gid},$,$,$,{},{})",
            building.s(),
            refs(&order)
        ))?;
    }
    Ok(Spatial {
        building_placement,
        building,
        storeys,
    })
}

fn emit_storey<W: Write>(
    w: &mut Writer<W>,
    level: &LevelData,
    building_placement: Ref,
) -> Result<Ref, WriteError> {
    let placement = w.plain_placement(Some(building_placement), [0.0, 0.0, level.elevation_mm])?;
    let project = w.project;
    let gid = w.synthetic("storey", &[&project, level.id.as_bytes()])?;
    let (name, cut) = w.label(&level.name, Subject::Level(level.id), "level.name");
    let description = if cut {
        step::string(&level.name)
    } else {
        "$".into()
    };
    w.line(&format!(
        "IFCBUILDINGSTOREY({gid},$,{name},{description},$,{},$,$,.ELEMENT.,{})",
        placement.s(),
        real(level.elevation_mm)?
    ))
}

fn materials<W: Write>(
    w: &mut Writer<W>,
    model: &ExportModel,
) -> Result<BTreeMap<ContentDigest, Ref>, WriteError> {
    let used: BTreeSet<ContentDigest> = model.elements.iter().filter_map(|e| e.material).collect();
    let mut out = BTreeMap::new();
    for material in model.materials.iter().filter(|m| used.contains(&m.digest)) {
        out.insert(material.digest, emit_material(w, material)?);
    }
    Ok(out)
}

fn emit_material<W: Write>(w: &mut Writer<W>, material: &MaterialData) -> Result<Ref, WriteError> {
    let grade = material.grade.as_deref().unwrap_or("Unspecified");
    let (name, cut) = w.label(grade, Subject::Material(material.digest), "material.name");
    let description = opt_string(material.standard.as_deref());
    let at = w.line(&format!("IFCMATERIAL({name},{description},$)"))?;
    let digest = w.single("MaterialDigest", &ident(&material.digest.to_string()), None)?;
    let mut props = vec![digest];
    if cut {
        props.push(w.single("FullName", &text(grade), None)?);
    }
    w.line(&format!(
        "IFCMATERIALPROPERTIES('BeamCraft_Material',$,{},{})",
        refs(&props),
        at.s()
    ))?;
    if let Some(density) = material.density_kg_m3 {
        let value = format!("IFCMASSDENSITYMEASURE({})", real(density)?);
        let prop = w.single("MassDensity", &value, None)?;
        w.line(&format!(
            "IFCMATERIALPROPERTIES('Pset_MaterialCommon',$,{},{})",
            refs(&[prop]),
            at.s()
        ))?;
    }
    Ok(at)
}

fn mesh_items<W: Write>(w: &mut Writer<W>, mesh: &Mesh, body: Ref) -> Result<Ref, WriteError> {
    let mut points = String::from("(");
    for (i, p) in mesh.points.iter().enumerate() {
        if i > 0 {
            points.push(',');
        }
        let coordinate =
            |v: f32| step::real32(v).ok_or(WriteError::Invalid("a coordinate is not finite"));
        points.push_str(&format!(
            "({},{},{})",
            coordinate(p[0])?,
            coordinate(p[1])?,
            coordinate(p[2])?
        ));
    }
    points.push(')');
    let list = w.line(&format!("IFCCARTESIANPOINTLIST3D({points})"))?;
    let mut triangles = String::from("(");
    for (i, t) in mesh.triangles.iter().enumerate() {
        if i > 0 {
            triangles.push(',');
        }
        // IFC indices are one-based; the derived layer's are zero-based.
        let one = |v: u32| u64::from(v) + 1;
        triangles.push_str(&format!("({},{},{})", one(t[0]), one(t[1]), one(t[2])));
    }
    triangles.push(')');
    let closed = if mesh.closed { ".T." } else { "$" };
    let set = w.line(&format!(
        "IFCTRIANGULATEDFACESET({},$,{closed},{triangles},$)",
        list.s()
    ))?;
    let representation = w.line(&format!(
        "IFCSHAPEREPRESENTATION({},'Body','Tessellation',{})",
        body.s(),
        refs(&[set])
    ))?;
    w.line(&format!(
        "IFCPRODUCTDEFINITIONSHAPE($,$,{})",
        refs(&[representation])
    ))
}

fn emit_element<W: Write>(
    w: &mut Writer<W>,
    e: &ElementData,
    spatial: &Spatial,
    body: Ref,
) -> Result<Ref, WriteError> {
    let placement = match &e.pose {
        Some(pose) => Some(w.placement(Some(spatial.building_placement), pose)?),
        None => None,
    };
    let shape = match &e.mesh {
        Some(mesh) => Some(mesh_items(w, mesh, body)?),
        None => None,
    };
    let gid = w.gid(guid::encode(&e.id))?;
    let (name, name_cut) = w.label(&e.name, Subject::Element(e.id), "element.name");
    let object_type = if e.entity == Entity::Proxy {
        w.label(&e.kind_token, Subject::Element(e.id), "element.kind")
            .0
    } else {
        "$".into()
    };
    let product = w.line(&format!(
        "{}({gid},$,{name},$,{object_type},{},{},$,{})",
        e.entity.step_name(),
        placement.map_or_else(|| "$".into(), Ref::s),
        shape.map_or_else(|| "$".into(), Ref::s),
        e.entity.predefined()
    ))?;
    element_properties(w, e, product, name_cut)?;
    if let Some(declared) = e.declared.as_ref().filter(|d| !d.is_empty()) {
        declared_properties(w, e, product, declared)?;
    }
    quantity_set(w, e, product)?;
    Ok(product)
}

fn element_properties<W: Write>(
    w: &mut Writer<W>,
    e: &ElementData,
    product: Ref,
    name_cut: bool,
) -> Result<(), WriteError> {
    let mut props = vec![
        w.single("ElementId", &ident(&e.id.to_string()), None)?,
        w.single("ElementKind", &text(&e.kind_token), None)?,
        w.single("SchemaVersion", &text(&e.schema_version), None)?,
        w.single("GeometryMode", &text(&e.geometry_mode), None)?,
    ];
    if let Some(profile) = e.profile {
        props.push(w.single("ProfileDigest", &ident(&profile.to_string()), None)?);
    }
    if let Some(material) = e.material {
        props.push(w.single("MaterialDigest", &ident(&material.to_string()), None)?);
    }
    if let Some(count) = e.hole_count {
        props.push(w.single("HoleCount", &format!("IFCINTEGER({count})"), None)?);
    }
    if let Some(q) = &e.quantities {
        props.push(w.single("VolumeStatus", &label_value(&q.volume_status), None)?);
        props.push(w.single("MassStatus", &label_value(&q.mass_status), None)?);
        if let Some(sig) = q.shape_signature {
            props.push(w.single("ShapeSignature", &ident(&sig.to_string()), None)?);
        }
        if let Some(sig) = q.mass_signature {
            props.push(w.single("MassSignature", &ident(&sig.to_string()), None)?);
        }
        if let Some(algorithm) = &q.algorithm {
            props.push(w.single("QuantityAlgorithm", &text(algorithm), None)?);
        }
        if e.entity == Entity::Plate {
            for (name, value) in [
                ("NetWeightKg", q.net_weight_kg),
                ("GrossWeightKg", q.gross_weight_kg),
            ] {
                if let Some(value) = value {
                    let v = format!("IFCMASSMEASURE({})", real(value)?);
                    props.push(w.single(name, &v, None)?);
                }
            }
        }
    }
    if let Some(assembly) = e.assembly {
        props.push(w.single("AssemblyId", &ident(&assembly.to_string()), None)?);
    }
    if name_cut {
        props.push(w.single("Label", &text(&e.name), None)?);
    }
    define_by_properties(w, e, product, "BeamCraft_Element", "element", &props)
}

fn declared_properties<W: Write>(
    w: &mut Writer<W>,
    e: &ElementData,
    product: Ref,
    d: &Declared,
) -> Result<(), WriteError> {
    let mut props = Vec::new();
    if let Some(v) = &d.designation {
        props.push(w.single("DeclaredDesignation", &text(v), Some(DECLARED_NOTE))?);
    }
    if let Some(v) = &d.standard {
        props.push(w.single("DeclaredStandard", &text(v), Some(DECLARED_NOTE))?);
    }
    for (name, value) in [
        ("DeclaredSectionAreaMm2", d.area_mm2),
        ("DeclaredMassPerMeterKg", d.mass_per_meter_kg),
        ("DeclaredSurfacePerMeterM2", d.surface_per_meter_m2),
    ] {
        if let Some(value) = value {
            let v = format!("IFCREAL({})", real(value)?);
            props.push(w.single(name, &v, Some(DECLARED_NOTE))?);
        }
    }
    define_by_properties(w, e, product, "BeamCraft_Declared", "declared", &props)
}

fn define_by_properties<W: Write>(
    w: &mut Writer<W>,
    e: &ElementData,
    product: Ref,
    set_name: &str,
    role: &str,
    props: &[Ref],
) -> Result<(), WriteError> {
    let gid = w.synthetic(&format!("pset-{role}"), &[e.id.as_bytes()])?;
    let set = w.line(&format!(
        "IFCPROPERTYSET({gid},$,{},$,{})",
        step::string(set_name),
        refs(props)
    ))?;
    let gid = w.synthetic(&format!("rel-pset-{role}"), &[e.id.as_bytes()])?;
    w.line(&format!(
        "IFCRELDEFINESBYPROPERTIES({gid},$,$,$,{},{})",
        refs(&[product]),
        set.s()
    ))?;
    Ok(())
}

fn quantity_set<W: Write>(
    w: &mut Writer<W>,
    e: &ElementData,
    product: Ref,
) -> Result<(), WriteError> {
    let (Some(set_name), Some(q)) = (e.entity.quantity_set(), e.quantities.as_ref()) else {
        return Ok(());
    };
    let mut items = Vec::new();
    let volume =
        |w: &mut Writer<W>, name: &str, value: Option<f64>| -> Result<Option<Ref>, WriteError> {
            value
                .map(|v| w.line(&format!("IFCQUANTITYVOLUME('{name}',$,$,{},$)", real(v)?)))
                .transpose()
        };
    let weight =
        |w: &mut Writer<W>, name: &str, value: Option<f64>| -> Result<Option<Ref>, WriteError> {
            value
                .map(|v| w.line(&format!("IFCQUANTITYWEIGHT('{name}',$,$,{},$)", real(v)?)))
                .transpose()
        };
    items.extend(volume(w, "GrossVolume", q.gross_volume_m3)?);
    items.extend(volume(w, "NetVolume", q.net_volume_m3)?);
    if e.entity != Entity::Plate {
        items.extend(weight(w, "GrossWeight", q.gross_weight_kg)?);
        items.extend(weight(w, "NetWeight", q.net_weight_kg)?);
    }
    if items.is_empty() {
        return Ok(());
    }
    let gid = w.synthetic("qto-element", &[e.id.as_bytes()])?;
    let set = w.line(&format!(
        "IFCELEMENTQUANTITY({gid},$,{},$,$,{})",
        step::string(set_name),
        refs(&items)
    ))?;
    let gid = w.synthetic("rel-qto-element", &[e.id.as_bytes()])?;
    w.line(&format!(
        "IFCRELDEFINESBYPROPERTIES({gid},$,$,$,{},{})",
        refs(&[product]),
        set.s()
    ))?;
    Ok(())
}

fn containment<W: Write>(
    w: &mut Writer<W>,
    model: &ExportModel,
    spatial: &Spatial,
    contained: &BTreeMap<Option<Uuid7>, Vec<Ref>>,
) -> Result<(), WriteError> {
    let project = w.project;
    for level in &model.levels {
        if let (Some(members), Some(&storey)) = (
            contained.get(&Some(level.id)),
            spatial.storeys.get(&level.id),
        ) {
            let gid = w.synthetic("rel-contained-storey", &[&project, level.id.as_bytes()])?;
            w.line(&format!(
                "IFCRELCONTAINEDINSPATIALSTRUCTURE({gid},$,$,$,{},{})",
                refs(members),
                storey.s()
            ))?;
        }
    }
    if let Some(members) = contained.get(&None) {
        let gid = w.synthetic("rel-contained-building", &[&project])?;
        w.line(&format!(
            "IFCRELCONTAINEDINSPATIALSTRUCTURE({gid},$,$,$,{},{})",
            refs(members),
            spatial.building.s()
        ))?;
    }
    Ok(())
}
