// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! D01: admitted intent for one associative member view and one projected linear dimension.
//!
//! The contract holds semantic references, never renderer indices or cached successful values.
//! Geometry resolution, USD persistence and publication are separate operations. A storage DTO
//! must enter through [`Drawing::admit`] before it can authorize any of them.

pub mod drafting;
pub mod pdf;
pub mod print;
pub mod projection;
pub mod sheet;
pub mod svg;

pub use beamcraft_model::sheet::{SheetFrame, SheetSpec, TitleBlock};
pub use drafting::{
    DraftingGeometry, DraftingSpec, HatchPattern, LayerSpec, LineType, MAX_DRAFTING_ENTITIES,
    MAX_LAYERS, PaperSymbol,
};

use crate::query::feature::{BodyKind, FeatureId, FeatureKind};
use beamcraft_geom::tolerance::COORDINATE_CAP_MM;
use beamcraft_model::command::{CommandError, label_is_valid};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::{DimensionId, DraftingEntityId, DrawingId, DrawingViewId, ElementId};

/// Version of the first closed drawing intent contract (distinct from the model schema).
pub const CONTRACT_VERSION: &str = "beamcraft-drawing/1";
/// Largest paper-space coordinate or signed dimension offset in millimetres.
pub const MAX_PAPER_MM: f64 = 10_000.0;
/// Largest number of placed views beyond the primary view in one document.
pub const MAX_VIEWS: usize = 16;
/// Largest number of section views in one document.
pub const MAX_SECTIONS: usize = 16;
/// Largest number of detail views in one document.
pub const MAX_DETAILS: usize = 16;
/// Largest prim count of the closed one-page drawing intent, equal to the layer reader's page
/// ceiling: three fixed prims, one optional scope per non-empty collection (sheet, placed
/// views, sections, details, drawn objects), and every placed view, section, detail and drawn
/// object.
pub const MAX_INTENT_PAGE_ITEMS: usize = 64;
/// Largest scale denominator; the first slice admits scales from 1:1 through 1:10000.
pub const MAX_SCALE_DENOMINATOR: f64 = 10_000.0;
/// Default paper scale: one paper millimetre represents 100 model millimetres.
pub const DEFAULT_SCALE_DENOMINATOR: f64 = 100.0;

pub use beamcraft_model::drawing::{Freshness, LiveSource, SourceModelPin};

/// Fixed orthographic bases in a right-handed, Z-up model. Positive paper Y points up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plane {
    /// Model X goes right and model Z goes up.
    Front,
    /// Model X goes right and model Y goes up.
    Top,
    /// Model Y goes right and model Z goes up.
    Right,
}

impl Plane {
    /// Exact lowercase wire token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Front => "front",
            Self::Top => "top",
            Self::Right => "right",
        }
    }

    /// Parses a fixed basis; arbitrary camera matrices are outside this contract.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "front" => Some(Self::Front),
            "top" => Some(Self::Top),
            "right" => Some(Self::Right),
            _ => None,
        }
    }

    /// Index of the model axis this basis looks along: Front projects [x, z] and looks
    /// along Y, Top along Z, Right along X. A section's cut plane is perpendicular to it.
    pub const fn depth_axis(self) -> usize {
        match self {
            Self::Front => 1,
            Self::Top => 2,
            Self::Right => 0,
        }
    }
}

/// One view's authored settings; scale never changes a dimension's model-unit value.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewSpec {
    /// Stable identity retained through refresh.
    pub id: DrawingViewId,
    /// Explicit projection basis.
    pub plane: Plane,
    /// Paper/model scale denominator (1 through 10000, default supplied by the caller is 100).
    pub scale_denominator: f64,
    /// Paper-space view origin in millimetres, each component within 10000 in magnitude.
    pub paper_origin_mm: [f64; 2],
}

/// Which side of a section's cut plane stays visible: the half-space at greater or lesser
/// coordinates along the view basis's depth axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CutSide {
    /// Model depth-axis coordinates greater than the cut stay visible.
    Positive,
    /// Model depth-axis coordinates less than the cut stay visible.
    Negative,
}

impl CutSide {
    /// Exact lowercase wire token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Positive => "positive",
            Self::Negative => "negative",
        }
    }

    /// Parses a fixed side token.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "positive" => Some(Self::Positive),
            "negative" => Some(Self::Negative),
            _ => None,
        }
    }
}

/// One section view: a cut plane perpendicular to the projection basis's depth axis, showing
/// the remaining half of the member plus the exact cut contour. The cut is a fixed
/// axis-aligned plane (`cut_mm` along the basis's depth axis); arbitrary cut planes are
/// outside this contract, exactly as arbitrary cameras are.
#[derive(Debug, Clone, PartialEq)]
pub struct SectionSpec {
    /// Stable identity retained through refresh.
    pub id: DrawingViewId,
    /// Explicit projection basis of the section view.
    pub plane: Plane,
    /// Model coordinate of the cut along the basis's depth axis, within the coordinate cap.
    pub cut_mm: f64,
    /// The half-space that stays visible.
    pub side: CutSide,
    /// Paper/model scale denominator (1 through 10000).
    pub scale_denominator: f64,
    /// Paper-space view origin in millimetres, each component within 10000 in magnitude.
    pub paper_origin_mm: [f64; 2],
}

/// One detail view: a circular region of one parent view (the primary view or a placed
/// view) redrawn at the detail's own, usually larger scale. The region is authored in the
/// parent's paper millimetres; the detail is a magnification of exactly that parent
/// content, never a fresh projection, so a detail of a section or of another detail is
/// outside this slice's contract.
#[derive(Debug, Clone, PartialEq)]
pub struct DetailSpec {
    /// Stable identity retained through refresh.
    pub id: DrawingViewId,
    /// The primary view's id or one placed view's id.
    pub parent: DrawingViewId,
    /// Region centre in the parent's paper millimetres, within 10000 per component.
    pub center_paper_mm: [f64; 2],
    /// Region radius in the parent's paper millimetres, greater than zero and within 10000.
    pub radius_mm: f64,
    /// Paper/model scale denominator (1 through 10000).
    pub scale_denominator: f64,
    /// Paper-space view origin in millimetres, each component within 10000 in magnitude.
    pub paper_origin_mm: [f64; 2],
}

/// One aligned projected linear dimension. Its endpoints must be vertices of the selected member.
#[derive(Debug, Clone, PartialEq)]
pub struct DimensionSpec {
    /// Stable dimension identity retained through refresh.
    pub id: DimensionId,
    /// Full semantic references in caller-chosen endpoint order.
    pub anchors: [FeatureId; 2],
    /// Signed perpendicular distance from the projected interval in paper millimetres.
    pub paper_offset_mm: f64,
    /// Decimal places for the millimetre label, from zero through three.
    pub decimal_places: u8,
}

/// Untrusted typed intent; use [`Drawing::admit`] before resolution or writing.
#[derive(Debug, Clone, PartialEq)]
pub struct DrawingSpec {
    /// Stable document identity.
    pub id: DrawingId,
    /// Label under the model's existing bounded label rule.
    pub name: String,
    /// Exact captured source identity; the adapter must verify the source manifest and contents.
    pub source: SourceModelPin,
    /// The sole selected straight member; copies have different identities and never relink.
    pub member: ElementId,
    /// The sole view.
    pub view: ViewSpec,
    /// Additional placed views of the same pinned source, each with its own basis, scale and
    /// paper origin. None is placed automatically: an admitted document starts with only the
    /// views its caller named, and freshness marks every placed view stale together with the
    /// document because they share one source pin.
    pub views: Vec<ViewSpec>,
    /// Section views of the same pinned source: cut planes perpendicular to their basis's
    /// depth axis, placed by hand like every other view.
    pub sections: Vec<SectionSpec>,
    /// Detail views magnifying one parent view's circular region at the detail's own scale.
    pub details: Vec<DetailSpec>,
    /// The sole dimension.
    pub dimension: DimensionSpec,
    /// Drawn paper-space objects owned by the document, never by the model.
    pub drafting: Vec<DraftingSpec>,
    /// The sheet this document is laid out on, when its caller chose one. Absent keeps the
    /// loose unbounded paper of the earlier contract; present, every export surface renders
    /// the same fixed sheet.
    pub sheet: Option<SheetSpec>,
    /// Document-owned layers that drawn objects join by reference; hiding a layer is paper
    /// visibility only and never touches the model.
    pub layers: Vec<LayerSpec>,
}

/// Validated immutable intent. Admission grants no claim that the referenced geometry exists.
#[derive(Debug, Clone, PartialEq)]
pub struct Drawing(DrawingSpec);

/// The five paper-space collections of one document, always validated together: bounded
/// counts, one closed intent page, per-entity bounds and one shared identity space.
struct PaperCollections<'a> {
    views: &'a [ViewSpec],
    sections: &'a [SectionSpec],
    details: &'a [DetailSpec],
    drafting: &'a [DraftingSpec],
    layers: &'a [LayerSpec],
}

fn invalid(field: &'static str, message: &'static str) -> CommandError {
    CommandError::new(ErrorCode::InvalidArgument, field, message)
}

fn finite_range(value: f64, min: f64, max: f64, field: &'static str) -> Result<(), CommandError> {
    if !value.is_finite() {
        return Err(CommandError::new(
            ErrorCode::NonFiniteNumber,
            field,
            "number must be finite",
        ));
    }
    if value < min || value > max {
        return Err(invalid(
            field,
            "number is outside the drawing contract's range",
        ));
    }
    Ok(())
}

/// Validates one view's paper settings; the primary and every placed/section/detail view
/// obey the same bounds.
fn validate_paper_settings(
    scale_denominator: f64,
    paper_origin_mm: [f64; 2],
) -> Result<(), CommandError> {
    finite_range(
        scale_denominator,
        1.0,
        MAX_SCALE_DENOMINATOR,
        "scale_denominator",
    )?;
    for value in paper_origin_mm {
        finite_range(value, -MAX_PAPER_MM, MAX_PAPER_MM, "paper_origin_mm")?;
    }
    Ok(())
}

/// Validates one section's cut plane and paper settings.
fn validate_section(section: &SectionSpec) -> Result<(), CommandError> {
    if !section.cut_mm.is_finite() {
        return Err(CommandError::new(
            ErrorCode::NonFiniteNumber,
            "cut_mm",
            "section cut must be finite",
        ));
    }
    if section.cut_mm.abs() > COORDINATE_CAP_MM {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "cut_mm",
            "section cut exceeds the model coordinate cap",
        ));
    }
    validate_paper_settings(section.scale_denominator, section.paper_origin_mm)
}

/// Validates one detail's circular region and paper settings.
fn validate_detail(detail: &DetailSpec) -> Result<(), CommandError> {
    for value in detail.center_paper_mm {
        finite_range(value, -MAX_PAPER_MM, MAX_PAPER_MM, "center_paper_mm")?;
    }
    if !detail.radius_mm.is_finite() {
        return Err(CommandError::new(
            ErrorCode::NonFiniteNumber,
            "radius_mm",
            "detail radius must be finite",
        ));
    }
    if detail.radius_mm <= 0.0 || detail.radius_mm > MAX_PAPER_MM {
        return Err(invalid(
            "radius_mm",
            "detail radius must be greater than zero and within the paper coordinate cap",
        ));
    }
    validate_paper_settings(detail.scale_denominator, detail.paper_origin_mm)
}

impl Drawing {
    /// Admits one bounded document and its qualified semantic references without any geometry work.
    /// Duplicate drawing/view/dimension UUIDs, invalid labels, foreign/non-vertex references,
    /// nonfinite numbers and out-of-range paper settings refuse with no partial result.
    pub fn admit(spec: DrawingSpec) -> Result<Self, CommandError> {
        let ids = [
            spec.id.uuid(),
            spec.view.id.uuid(),
            spec.dimension.id.uuid(),
        ];
        if ids
            .iter()
            .enumerate()
            .any(|(i, id)| ids.iter().skip(i + 1).any(|other| id == other))
        {
            return Err(CommandError::new(
                ErrorCode::DuplicateId,
                "drawing_ids",
                "document object UUIDs must be distinct",
            ));
        }
        if !label_is_valid(&spec.name) {
            return Err(invalid(
                "name",
                "drawing name must satisfy the bounded label rule",
            ));
        }
        validate_paper_settings(spec.view.scale_denominator, spec.view.paper_origin_mm)?;
        finite_range(
            spec.dimension.paper_offset_mm,
            -MAX_PAPER_MM,
            MAX_PAPER_MM,
            "paper_offset_mm",
        )?;
        if spec.dimension.decimal_places > 3 {
            return Err(invalid(
                "decimal_places",
                "use zero through three decimal places",
            ));
        }
        for anchor in &spec.dimension.anchors {
            if anchor.element() != spec.member
                || anchor.body() != BodyKind::Member
                || anchor.kind() != FeatureKind::Vertex
            {
                return Err(invalid(
                    "anchors",
                    "dimension anchors must be qualified vertices of the selected member",
                ));
            }
        }
        // Equal semantic endpoints are invalid intent. Distinct endpoints that collapse only in
        // the chosen projection remain valid intent and resolve as explicitly unresolved later.
        let [first, second] = &spec.dimension.anchors;
        if first == second {
            return Err(invalid(
                "anchors",
                "dimension endpoints must be distinct semantic references",
            ));
        }
        if let Some(sheet) = &spec.sheet {
            sheet.admit()?;
        }
        Self::validate_paper_objects(
            &spec.id,
            &spec.view.id,
            &spec.dimension.id,
            spec.sheet.as_ref(),
            PaperCollections {
                views: &spec.views,
                sections: &spec.sections,
                details: &spec.details,
                drafting: &spec.drafting,
                layers: &spec.layers,
            },
        )?;
        Ok(Self(spec))
    }

    /// Validates the document's paper objects together: bounded counts, one shared distinct
    /// identity space across every collection, per-entity/per-view bounds, every detail's
    /// parent present, and every drawn object on a layer the same document defines. Drawn
    /// objects own no model reference of any kind.
    fn validate_paper_objects(
        drawing: &DrawingId,
        view: &DrawingViewId,
        dimension: &DimensionId,
        sheet: Option<&SheetSpec>,
        paper: PaperCollections<'_>,
    ) -> Result<(), CommandError> {
        let PaperCollections {
            views,
            sections,
            details,
            drafting,
            layers,
        } = paper;
        for (count, cap, field) in [
            (views.len(), MAX_VIEWS, "views"),
            (sections.len(), MAX_SECTIONS, "sections"),
            (details.len(), MAX_DETAILS, "details"),
            (drafting.len(), MAX_DRAFTING_ENTITIES, "drafting"),
            (layers.len(), MAX_LAYERS, "layers"),
        ] {
            if count > cap {
                return Err(CommandError::new(
                    ErrorCode::LimitExceeded,
                    field,
                    "too many paper objects of this kind in one document",
                ));
            }
        }
        // The whole intent must stay within one closed reader page: the fixed document prims,
        // one optional scope per non-empty collection and the objects themselves.
        let page_items = 3
            + usize::from(sheet.is_some())
            + usize::from(!views.is_empty())
            + usize::from(!sections.is_empty())
            + usize::from(!details.is_empty())
            + usize::from(!drafting.is_empty())
            + usize::from(!layers.is_empty())
            + views.len()
            + sections.len()
            + details.len()
            + drafting.len()
            + layers.len();
        if page_items > MAX_INTENT_PAGE_ITEMS {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "paper",
                "placed views, sections, details, drawn objects and layers together exceed the one-page intent",
            ));
        }
        for layer in layers {
            if !label_is_valid(&layer.name) {
                return Err(invalid(
                    "layers",
                    "layer name must satisfy the bounded label rule",
                ));
            }
        }
        for entity in drafting {
            drafting::validate_geometry(&entity.geometry)?;
            if !layers
                .iter()
                .any(|layer| layer.id.uuid() == entity.layer.uuid())
            {
                return Err(invalid(
                    "drafting",
                    "drawn object names a layer the document does not define",
                ));
            }
        }
        for placed in views {
            validate_paper_settings(placed.scale_denominator, placed.paper_origin_mm)?;
        }
        for section in sections {
            validate_section(section)?;
        }
        for detail in details {
            validate_detail(detail)?;
            // A detail magnifies the primary view or a placed view; sections and other
            // details are outside this slice's detail contract.
            if detail.parent != *view && !views.iter().any(|placed| placed.id == detail.parent) {
                return Err(invalid(
                    "parent",
                    "detail parent must be the primary view or a placed view",
                ));
            }
        }
        // One shared identity space: every paper object's UUID is distinct from the three
        // document objects and from every other paper object, whichever collections hold them.
        let reserved = [drawing.uuid(), view.uuid(), dimension.uuid()];
        let all = reserved
            .into_iter()
            .chain(views.iter().map(|placed| placed.id.uuid()))
            .chain(sections.iter().map(|section| section.id.uuid()))
            .chain(details.iter().map(|detail| detail.id.uuid()))
            .chain(drafting.iter().map(|entity| entity.id.uuid()))
            .chain(layers.iter().map(|layer| layer.id.uuid()))
            .collect::<Vec<_>>();
        for (index, id) in all.iter().enumerate() {
            if all.iter().skip(index + 1).any(|other| other == id) {
                return Err(CommandError::new(
                    ErrorCode::DuplicateId,
                    "paper",
                    "paper object UUIDs must be pairwise distinct and distinct from the document object UUIDs",
                ));
            }
        }
        Ok(())
    }

    /// Borrows the validated intent; callers cannot mutate it around admission.
    pub fn spec(&self) -> &DrawingSpec {
        &self.0
    }

    /// Borrows the sheet this document is laid out on, when one was chosen.
    pub fn sheet(&self) -> Option<&SheetSpec> {
        self.0.sheet.as_ref()
    }

    /// Lays this document on a sheet, retaining every other document object, its identity
    /// and its source pin. This is a whole-document intent operation: the engineering model
    /// is neither read nor touched.
    pub fn with_sheet(mut self, sheet: SheetSpec) -> Result<Self, CommandError> {
        sheet.admit()?;
        Self::validate_paper_objects(
            &self.0.id,
            &self.0.view.id,
            &self.0.dimension.id,
            Some(&sheet),
            PaperCollections {
                views: &self.0.views,
                sections: &self.0.sections,
                details: &self.0.details,
                drafting: &self.0.drafting,
                layers: &self.0.layers,
            },
        )?;
        self.0.sheet = Some(sheet);
        Ok(self)
    }

    /// Removes the sheet, returning the document to loose unbounded paper. This is a
    /// whole-document intent operation: the engineering model is neither read nor touched.
    pub fn without_sheet(mut self) -> Self {
        self.0.sheet = None;
        Self(self.0)
    }

    /// Borrows the drawn objects of this document.
    pub fn drafting(&self) -> &[DraftingSpec] {
        &self.0.drafting
    }

    /// Borrows the document's layers.
    pub fn layers(&self) -> &[LayerSpec] {
        &self.0.layers
    }

    /// Iterates the drawn objects whose layer is visible. Visibility is a paper-only filter:
    /// hidden objects stay in the document unchanged and the model is never consulted.
    pub fn visible_drafting(&self) -> impl Iterator<Item = &DraftingSpec> {
        let visible = move |layer: &DraftingEntityId| {
            self.0
                .layers
                .iter()
                .any(|spec| spec.id.uuid() == layer.uuid() && spec.visible)
        };
        self.0
            .drafting
            .iter()
            .filter(move |entity| visible(&entity.layer))
    }

    /// Borrows the placed views beyond the primary view, in caller order.
    pub fn views(&self) -> &[ViewSpec] {
        &self.0.views
    }

    /// Borrows the section views, in caller order.
    pub fn sections(&self) -> &[SectionSpec] {
        &self.0.sections
    }

    /// Borrows the detail views, in caller order.
    pub fn details(&self) -> &[DetailSpec] {
        &self.0.details
    }

    /// Replaces the layers and drawn objects with a newly admitted set, retaining every other
    /// document object, its identity and its source pin. One atomic intent operation: the new
    /// objects must name layers the same replacement defines, so no intermediate state can hold
    /// a dangling layer reference. The engineering model is neither read nor touched.
    pub fn with_drafting(
        mut self,
        layers: Vec<LayerSpec>,
        entities: Vec<DraftingSpec>,
    ) -> Result<Self, CommandError> {
        Self::validate_paper_objects(
            &self.0.id,
            &self.0.view.id,
            &self.0.dimension.id,
            self.0.sheet.as_ref(),
            PaperCollections {
                views: &self.0.views,
                sections: &self.0.sections,
                details: &self.0.details,
                drafting: &entities,
                layers: &layers,
            },
        )?;
        self.0.layers = layers;
        self.0.drafting = entities;
        Ok(self)
    }

    /// Replaces the placed views with a newly admitted set, retaining every other document
    /// object, its identity and its source pin. This is a whole-document intent operation: the
    /// engineering model is neither read nor touched.
    pub fn with_views(mut self, views: Vec<ViewSpec>) -> Result<Self, CommandError> {
        Self::validate_paper_objects(
            &self.0.id,
            &self.0.view.id,
            &self.0.dimension.id,
            self.0.sheet.as_ref(),
            PaperCollections {
                views: &views,
                sections: &self.0.sections,
                details: &self.0.details,
                drafting: &self.0.drafting,
                layers: &self.0.layers,
            },
        )?;
        self.0.views = views;
        Ok(self)
    }

    /// Replaces the section views with a newly admitted set, retaining every other document
    /// object, its identity and its source pin. This is a whole-document intent operation: the
    /// engineering model is neither read nor touched.
    pub fn with_sections(mut self, sections: Vec<SectionSpec>) -> Result<Self, CommandError> {
        Self::validate_paper_objects(
            &self.0.id,
            &self.0.view.id,
            &self.0.dimension.id,
            self.0.sheet.as_ref(),
            PaperCollections {
                views: &self.0.views,
                sections: &sections,
                details: &self.0.details,
                drafting: &self.0.drafting,
                layers: &self.0.layers,
            },
        )?;
        self.0.sections = sections;
        Ok(self)
    }

    /// Replaces the detail views with a newly admitted set, retaining every other document
    /// object, its identity and its source pin. This is a whole-document intent operation: the
    /// engineering model is neither read nor touched.
    pub fn with_details(mut self, details: Vec<DetailSpec>) -> Result<Self, CommandError> {
        Self::validate_paper_objects(
            &self.0.id,
            &self.0.view.id,
            &self.0.dimension.id,
            self.0.sheet.as_ref(),
            PaperCollections {
                views: &self.0.views,
                sections: &self.0.sections,
                details: &details,
                drafting: &self.0.drafting,
                layers: &self.0.layers,
            },
        )?;
        self.0.details = details;
        Ok(self)
    }

    /// Explicit refresh replaces only the captured source, retaining object IDs and references.
    /// The adapter is responsible for verifying/capturing this pin before persistence.
    pub fn refreshed(&self, source: SourceModelPin) -> Self {
        let mut spec = self.0.clone();
        spec.source = source;
        Self(spec)
    }
}
