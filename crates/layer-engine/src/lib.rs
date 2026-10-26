// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Experimental bounded USD adapter. See the crate README for containment limits.

pub mod authored;
mod backend;
pub mod index;
mod preflight;

pub use backend::ForeignLayer;

use std::collections::BTreeSet;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Instant;

use beamcraft_model::{
    digest::{ContentDigest, Sha256},
    error::ErrorCode,
};

/// Exact package/feature and wrapper schema identity; no BeamCraft authored schema is implied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackendIdentity {
    /// Exact package version.
    pub version: &'static str,
    /// Complete crates.io archive SHA-256.
    pub archive: &'static str,
    /// Enabled optional features (none).
    pub features: &'static str,
    /// Closed static snapshot profile, separate from the U02 authored schema.
    pub schema: &'static str,
}

/// The only admitted backend identity for this adapter.
pub const BACKEND: BackendIdentity = BackendIdentity {
    version: "0.8.0",
    archive: "54f38d869c64c96492e1a482fb621eda620d120d91dbe93072112ee315a8923c",
    features: "",
    schema: "u01-static-read-v1",
};

/// Bounded safe diagnostic; never contains upstream error text or caller data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Diagnostic {
    /// F10 rule responsible for a domain validation finding, if applicable.
    pub rule_id: Option<&'static str>,
    /// Stable F04 code.
    pub code: ErrorCode,
    /// Fixed ASCII explanation.
    pub message: &'static str,
}

pub(crate) fn fail(code: ErrorCode, message: &'static str) -> Diagnostic {
    Diagnostic {
        code,
        message,
        rule_id: None,
    }
}
pub(crate) fn limit() -> Diagnostic {
    fail(ErrorCode::LimitExceeded, "stage budget exceeded")
}
pub(crate) fn corrupt() -> Diagnostic {
    fail(ErrorCode::IntegrityError, "invalid layer bytes or manifest")
}
pub(crate) fn unsupported() -> Diagnostic {
    fail(
        ErrorCode::UnsupportedProperty,
        "encoding or composition outside static read profile",
    )
}

impl Diagnostic {
    /// Attaches the precise F10 rule at the validation site.
    pub const fn with_rule(mut self, rule_id: &'static str) -> Self {
        self.rule_id = Some(rule_id);
        self
    }
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for Diagnostic {}

/// Explicit stage ceilings. A caller may tighten any of
/// them, never raise one above [`Budgets::default`].
#[derive(Debug, Clone, Copy)]
pub struct Budgets {
    /// Maximum bytes in one layer.
    pub layer_bytes: usize,
    /// Maximum sum of supplied asset bytes.
    pub total_bytes: usize,
    /// Maximum layers, including unused manifest entries.
    pub layers: usize,
    /// Maximum nesting and composition dependency depth.
    pub depth: usize,
    /// Maximum authored or composed prims.
    pub prims: usize,
    /// Maximum authored fields, structural entries, and snapshot properties.
    pub values: usize,
    /// Maximum scalar entries per array or structural vector.
    pub array_items: usize,
    /// Maximum estimated aggregate binary structural/value expansion, not total heap.
    pub decoded_bytes: usize,
    /// Maximum retained typed snapshot bytes (conservative accounting).
    pub snapshot_bytes: usize,
    /// Maximum prims per page.
    pub page_items: usize,
}
impl Budgets {
    /// The model profile: the ceilings of a BeamCraft project stage.
    ///
    /// Each value is sized from a limit that already binds the same data further along, so no
    /// stage can accept what a later one refuses, and none refuses what a later one accepts:
    ///
    /// * `layer_bytes` 32 MiB: one layer is one host blob (`host::storage::MAX_BLOB_BYTES`).
    /// * `total_bytes` 64 MiB: the host's content-verification bound for one revision
    ///   (`host::revision::MAX_CONTENT_VERIFY_BYTES`).
    /// * `prims` 262,144: four prims per element at the engine's 65,536-element view cap (element,
    ///   geometry, representation and a definition or relationship target).
    /// * `values` 4,194,304: 64 fields per element at the same cap.
    /// * `array_items` 1,572,864: one element mesh at the engine blob caps (524,288 triangles
    ///   are 1,572,864 indices; 262,144 vertices are 786,432 scalars).
    /// * `decoded_bytes` 256 MiB and `snapshot_bytes` 512 MiB: four and eight times the 64 MiB
    ///   stage, a conservative allowance for the binary expansion and the per-value accounting of
    ///   the typed snapshot (not measured at the cap).
    ///
    /// Layers, depth and page size are unchanged: a project stage has a fixed, shallow layer
    /// stack. Allocation is still charged against these ceilings before it happens.
    pub const MODEL: Self = Self {
        layer_bytes: 32 * 1024 * 1024,
        total_bytes: 64 * 1024 * 1024,
        layers: 8,
        depth: 16,
        prims: 262_144,
        values: 4_194_304,
        array_items: 1_572_864,
        decoded_bytes: 256 * 1024 * 1024,
        snapshot_bytes: 512 * 1024 * 1024,
        page_items: 64,
    };

    /// The first-slice profile of U01 (64 KiB per layer, 256 prims, 1024 array items): the
    /// tightened profile for small untrusted inputs that are not a project stage, such as the
    /// source layer of a read-only reference (R01).
    pub const FIRST_SLICE: Self = Self {
        layer_bytes: 65536,
        total_bytes: 262144,
        layers: 8,
        depth: 16,
        prims: 256,
        values: 4096,
        array_items: 1024,
        decoded_bytes: 4 * 1024 * 1024,
        snapshot_bytes: 1024 * 1024,
        page_items: 64,
    };
}
impl Default for Budgets {
    fn default() -> Self {
        Self::MODEL
    }
}
impl Budgets {
    fn validate(self) -> Result<(), Diagnostic> {
        let ceiling = Self::default();
        for (actual, max) in [
            (self.layer_bytes, ceiling.layer_bytes),
            (self.total_bytes, ceiling.total_bytes),
            (self.layers, ceiling.layers),
            (self.depth, ceiling.depth),
            (self.prims, ceiling.prims),
            (self.values, ceiling.values),
            (self.array_items, ceiling.array_items),
            (self.decoded_bytes, ceiling.decoded_bytes),
            (self.snapshot_bytes, ceiling.snapshot_bytes),
            (self.page_items, ceiling.page_items),
        ] {
            if actual == 0 || actual > max {
                return Err(limit());
            }
        }
        Ok(())
    }
}

/// Thread-safe request cancellation flag. No thread, process or control service is started.
#[derive(Debug, Default, Clone)]
pub struct Cancellation(Arc<AtomicBool>);
impl Cancellation {
    /// Requests cancellation; completion is cooperative, not preemptive.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
}
/// One monotonic deadline shared across the whole request.
pub struct Control {
    /// Explicit cancellation token.
    pub cancellation: Cancellation,
    /// Host-established absolute deadline, including preflight and result validation.
    pub deadline: Instant,
}
impl Control {
    pub(crate) fn check(&self) -> Result<(), Diagnostic> {
        let now = Instant::now();
        if self.deadline.saturating_duration_since(now) > std::time::Duration::from_secs(120) {
            return Err(fail(
                ErrorCode::InvalidArgument,
                "request deadline exceeds 120 seconds",
            ));
        }
        if self.cancellation.0.load(Ordering::Relaxed) || now >= self.deadline {
            Err(fail(
                ErrorCode::Cancelled,
                "request cancelled or deadline expired",
            ))
        } else {
            Ok(())
        }
    }
}

/// Host-authorized immutable asset bytes; `name` is a virtual relative identifier.
#[derive(Debug, Clone)]
pub struct Asset {
    /// Canonical slash-separated local identifier, never a filesystem path.
    pub name: String,
    /// Expected complete digest.
    pub digest: ContentDigest,
    /// Exact source bytes. The wrapper never edits them.
    pub bytes: Vec<u8>,
}

/// Full immutable view identity. UUIDs must be full lowercase RFC 9562 version 7 strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// Project UUIDv7.
    pub project: String,
    /// Workspace UUIDv7.
    pub workspace: String,
    /// Full base revision digest.
    pub revision: ContentDigest,
    /// Selected workspace generation (not assumed zero).
    pub generation: u64,
}

/// Closed composition/load profile for this slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// Static sublayers and explicit external references; no payloads, variants or instancing.
    StaticLocal,
}

/// Complete request. Host authorization is a prerequisite, never inferred from bytes.
#[derive(Clone)]
pub struct OpenRequest {
    /// Complete workspace identity.
    pub identity: Identity,
    /// Virtual root asset identifier.
    pub root: String,
    /// Complete authorized source set, including all dependencies.
    pub assets: Vec<Asset>,
    /// Host-selected owned layer identifiers; a digest match never implies ownership.
    pub owned_layers: BTreeSet<String>,
    /// Expected digest of [`Self::manifest_digest`].
    pub manifest: ContentDigest,
    /// Exact backend/features/snapshot schema pin.
    pub backend: BackendIdentity,
    /// Explicit static load profile.
    pub profile: Profile,
    /// Tightenable stage ceilings.
    pub budgets: Budgets,
}
impl OpenRequest {
    /// Computes the closed manifest commitment, including identity, ordered entries and ownership.
    /// The caller must compare against an independently verified manifest, not self-certify bytes.
    pub fn manifest_digest(&self) -> Result<ContentDigest, Diagnostic> {
        self.budgets.validate()?;
        if self.assets.len() > self.budgets.layers || self.owned_layers.len() > self.budgets.layers
        {
            return Err(limit());
        }
        let mut h = Sha256::new();
        let mut add = |s: &[u8]| -> Result<(), Diagnostic> {
            if s.len() > 1024 {
                return Err(limit());
            }
            h.update(&(s.len() as u64).to_le_bytes())
                .map_err(|_| limit())?;
            h.update(s).map_err(|_| limit())
        };
        add(b"BeamCraft/U01/manifest/v1")?;
        add(self.identity.project.as_bytes())?;
        add(self.identity.workspace.as_bytes())?;
        add(self.identity.revision.as_bytes())?;
        add(&self.identity.generation.to_le_bytes())?;
        add(self.root.as_bytes())?;
        add(self.backend.version.as_bytes())?;
        add(self.backend.archive.as_bytes())?;
        add(self.backend.features.as_bytes())?;
        add(self.backend.schema.as_bytes())?;
        add(b"StaticLocal")?;
        add(&(self.assets.len() as u64).to_le_bytes())?;
        for asset in &self.assets {
            add(asset.name.as_bytes())?;
            add(asset.digest.as_bytes())?;
            add(&(asset.bytes.len() as u64).to_le_bytes())?;
        }
        add(&(self.owned_layers.len() as u64).to_le_bytes())?;
        for name in &self.owned_layers {
            add(name.as_bytes())?;
        }
        Ok(h.finalize())
    }
}

/// Closed, bounded typed values; unsupported values never become debug strings.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// Explicit USD block.
    Block,
    /// Boolean.
    Bool(bool),
    /// Signed integer (lossless).
    Int(i64),
    /// Unsigned integer (lossless).
    Uint(u64),
    /// Finite binary64; float32 values widen exactly.
    Number(f64),
    /// String or token, distinguished by the attribute type name.
    Text(String),
    /// Static vector/array, bounded and recursively validated.
    Array(Vec<Value>),
    /// Inert dictionary metadata, bounded by count and nesting depth.
    Dictionary(std::collections::BTreeMap<String, Value>),
}
/// Authored contributing site, retaining foreign reference source paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provenance {
    /// Authorized virtual source identifier.
    pub layer: String,
    /// Complete source digest.
    pub digest: ContentDigest,
    /// Source prim path, before reference remapping.
    pub path: String,
    /// Whether the host selected this layer as owned.
    pub owned: bool,
}
/// A static composed attribute.
#[derive(Debug, Clone, PartialEq)]
pub struct Property {
    /// Attribute name.
    pub name: String,
    /// Authored USD type name.
    pub type_name: String,
    /// Absent default is different from an explicit block.
    pub value: Option<Value>,
    /// Composed `interpolation` metadata, when authored (for example `uniform` on `normals`).
    pub interpolation: Option<String>,
}
/// One static composed prim; no engineering shape or write capability is implied.
#[derive(Debug, Clone, PartialEq)]
pub struct Prim {
    /// Complete composed path.
    pub path: String,
    /// Complete UUIDv7 if an element identity is authored.
    pub element_id: Option<String>,
    /// Composed USD type.
    pub type_name: Option<String>,
    /// Composed active state.
    pub active: bool,
    /// Composed model `kind` metadata, when authored.
    pub kind: Option<String>,
    /// Composed `displayName` metadata, when authored.
    pub display_name: Option<String>,
    /// Static attributes, ordered by full name.
    pub properties: Vec<Property>,
    /// Static composed relationships, ordered by full name; the UUID/link index is built
    /// separately by [`index`].
    pub relationships: Vec<Relationship>,
    /// All contributing prim sites (strongest first).
    pub sources: Vec<Provenance>,
}
/// A composed relationship with ordered targets. Unknown names remain inert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relationship {
    /// Full property name.
    pub name: String,
    /// Composed prim targets, without forwarding or implicit resolution.
    pub targets: Vec<String>,
}
/// Declared capabilities, including limitations that must not be inferred as passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// Exact backend identity.
    pub backend: BackendIdentity,
    /// Supported composition/load profile.
    pub profile: Profile,
    /// Whether parser execution has hard allocation/deadline containment (currently false).
    pub parser_contained: bool,
    /// Whether this owner exposes the separate isolated authoring API.
    pub mutations: bool,
    /// External references in USDA are supported; binary reference encodings are unavailable.
    pub usda_references: bool,
    /// Static composed relationships are delivered, including inert unknown names.
    pub relationships: bool,
}
/// Opaque in-process incarnation handle; it cannot be manufactured or serialized.
#[derive(Debug, Clone)]
pub struct Context {
    marker: Arc<()>,
    identity: Identity,
}
impl Context {
    /// Complete verified identity pinned by the stage.
    pub fn identity(&self) -> &Identity {
        &self.identity
    }
}
/// Opaque cursor valid only over its original live context and reader.
#[derive(Debug, Clone)]
pub struct Cursor {
    context: Context,
    offset: usize,
}
/// One bounded page over an immutable pinned result.
#[derive(Debug, Clone)]
pub struct Page {
    /// Matching stage incarnation.
    pub context: Context,
    /// Bounded prim items.
    pub items: Vec<Prim>,
    /// Continuation, or complete delivery when absent.
    pub next: Option<Cursor>,
    // Position of the first item in the frozen snapshot; lets the U03 builder detect gaps and replays.
    offset: usize,
}
/// Open result; metadata never comes from a partially validated stage.
#[derive(Debug, Clone)]
pub struct Opened {
    /// New live incarnation.
    pub context: Context,
    /// Explicit capabilities and limits of the adapter.
    pub capabilities: Capabilities,
    /// Exact tightenable stage budgets used for this view.
    pub budgets: Budgets,
    /// Stage length unit (metres per unit).
    pub meters_per_unit: f64,
    /// Stage up axis.
    pub up_axis: String,
    /// Authored root metadata, excluding structural child lists and composition arcs.
    pub metadata: std::collections::BTreeMap<String, Value>,
    /// Number of prims in the complete snapshot.
    pub prim_count: usize,
}

struct Installed {
    _stage: openusd::usd::Stage,
    context: Context,
    prims: Vec<Prim>,
    budgets: Budgets,
    /// Every supplied layer and whether the host selected it as owned.
    layers: std::collections::BTreeMap<String, bool>,
}
/// Single owner of one private stage. No upstream handle or setter is exposed.
#[derive(Default)]
pub struct Reader {
    live: Option<Installed>,
}
impl Reader {
    /// Builds privately, validates completely, then atomically replaces the live read view.
    /// An upstream synchronous call cannot be interrupted; check capabilities before admission.
    pub fn open(&mut self, request: OpenRequest, control: &Control) -> Result<Opened, Diagnostic> {
        self.prepare_read(|| backend::open(request, control), control)
    }
    fn prepare_read(
        &mut self,
        build: impl FnOnce() -> Result<backend::Candidate, Diagnostic>,
        control: &Control,
    ) -> Result<Opened, Diagnostic> {
        control.check()?;
        let candidate =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(build)).map_err(|_| {
                fail(
                    ErrorCode::BackendUnavailable,
                    "backend unwound while preparing read view",
                )
            })??;
        self.install(candidate, control)
    }
    fn install(
        &mut self,
        candidate: backend::Candidate,
        control: &Control,
    ) -> Result<Opened, Diagnostic> {
        control.check()?;
        Ok(self.install_checked(candidate))
    }
    // The caller has completed validation and its last cancellation/decision check.
    fn install_checked(&mut self, candidate: backend::Candidate) -> Opened {
        let context = Context {
            marker: Arc::new(()),
            identity: candidate.identity,
        };
        let result = Opened {
            context: context.clone(),
            capabilities: Capabilities {
                backend: BACKEND,
                profile: Profile::StaticLocal,
                parser_contained: false,
                mutations: false,
                usda_references: true,
                relationships: true,
            },
            budgets: candidate.budgets,
            meters_per_unit: candidate.meters_per_unit,
            up_axis: candidate.up_axis,
            metadata: candidate.metadata,
            prim_count: candidate.prims.len(),
        };
        self.live = Some(Installed {
            _stage: candidate.stage,
            context,
            prims: candidate.prims,
            budgets: candidate.budgets,
            layers: candidate.layers,
        });
        result
    }
    fn pinned(&self, context: &Context) -> Result<&Installed, Diagnostic> {
        self.live
            .as_ref()
            .filter(|s| {
                Arc::ptr_eq(&s.context.marker, &context.marker)
                    && s.context.identity == context.identity
            })
            .ok_or_else(|| fail(ErrorCode::StaleWorkspace, "stage context is no longer live"))
    }
    /// Reads a page from the already validated snapshot without recomposition or reparsing.
    pub fn read(
        &self,
        context: &Context,
        cursor: Option<&Cursor>,
        control: &Control,
    ) -> Result<Page, Diagnostic> {
        control.check()?;
        let stage = self.pinned(context)?;
        let offset = if let Some(c) = cursor {
            self.pinned(&c.context)?;
            c.offset
        } else {
            0
        };
        let end = offset
            .checked_add(stage.budgets.page_items)
            .ok_or_else(limit)?
            .min(stage.prims.len());
        let items = stage.prims.get(offset..end).ok_or_else(corrupt)?.to_vec();
        control.check()?;
        Ok(Page {
            context: context.clone(),
            items,
            next: (end < stage.prims.len()).then(|| Cursor {
                context: context.clone(),
                offset: end,
            }),
            offset,
        })
    }
    /// Releases only the matching live read view, invalidating all its contexts and cursors.
    pub fn close(&mut self, context: &Context, control: &Control) -> Result<(), Diagnostic> {
        control.check()?;
        self.pinned(context)?;
        self.live = None;
        Ok(())
    }
    /// Read-slice write refusal. Even an owned prim returns unsupported; this grants no authority.
    pub fn refuse_write(&self, context: &Context, path: &str) -> Result<(), Diagnostic> {
        let stage = self.pinned(context)?;
        let prim = stage
            .prims
            .iter()
            .find(|p| p.path == path)
            .ok_or_else(|| fail(ErrorCode::NotFound, "prim not present in pinned view"))?;
        if prim.sources.is_empty() || prim.sources.iter().any(|s| !s.owned) {
            return Err(fail(
                ErrorCode::ForeignReadOnly,
                "foreign prims are read-only",
            ));
        }
        Err(fail(
            ErrorCode::UnsupportedCommand,
            "mutations are unavailable in read profile",
        ))
    }
}

impl Reader {
    /// Read-slice refusal of a write to a whole layer. A layer the host did not select as owned
    /// (a foreign source) is `FOREIGN_READ_ONLY`; an unknown name is `NOT_FOUND`; even an owned
    /// layer is `UNSUPPORTED_COMMAND` here, because this slice has no mutation path. Like
    /// [`Self::refuse_write`] it grants nothing.
    pub fn refuse_layer_write(&self, context: &Context, layer: &str) -> Result<(), Diagnostic> {
        let stage = self.pinned(context)?;
        match stage.layers.get(layer) {
            None => Err(fail(
                ErrorCode::NotFound,
                "layer not present in pinned view",
            )),
            Some(false) => Err(fail(
                ErrorCode::ForeignReadOnly,
                "foreign layers are read-only",
            )),
            Some(true) => Err(fail(
                ErrorCode::UnsupportedCommand,
                "mutations are unavailable in read profile",
            )),
        }
    }
}

/// Reads what a foreign layer declares (format, default prim, units, up axis) after the same
/// byte, field and budget admission as every stage asset, without composing or resolving anything.
/// This is how a foreign source is examined before it is attached; see [`ForeignLayer`] for the
/// refusals. It is parser work in this process and carries the same containment limits as
/// [`Reader::open`].
pub fn inspect_foreign_layer(
    bytes: &[u8],
    budgets: Budgets,
    control: &Control,
) -> Result<ForeignLayer, Diagnostic> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        backend::inspect_foreign(bytes, budgets, control)
    }))
    .map_err(|_| {
        fail(
            ErrorCode::BackendUnavailable,
            "backend unwound while inspecting a foreign layer",
        )
    })?
}

/// The single label character rule (F04 section 3, `DV-BASE-003`): U+0000-001F and U+007F.
/// Shared by authoring and the index so a label that authors can always be indexed.
pub(crate) fn label_char_forbidden(c: char) -> bool {
    c <= '\u{1f}' || c == '\u{7f}'
}

pub(crate) fn uuid7(s: &str) -> bool {
    s.len() == 36
        && s.bytes().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            14 => b == b'7',
            19 => matches!(b, b'8'..=b'9' | b'a'..=b'b'),
            _ => b.is_ascii_digit() || (b'a'..=b'f').contains(&b),
        })
}

/// Reads the F10 deactivation attribute; absence alone defaults to false (DV-BASE-004).
pub fn is_deactivated(prim: &Prim) -> Result<bool, Diagnostic> {
    match prim
        .properties
        .iter()
        .find(|p| p.name == "bim:isDeactivated")
    {
        None => Ok(false),
        Some(p) => match (p.type_name.as_str(), p.value.as_ref()) {
            ("bool", Some(Value::Bool(value))) => Ok(*value),
            _ => Err(fail(
                ErrorCode::InvalidArgument,
                "bim:isDeactivated must be a boolean",
            )
            .with_rule("DV-BASE-004")),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[allow(clippy::unwrap_used)] // Test fixture construction only.
    fn input() -> OpenRequest {
        let bytes =
            b"#usda 1.0\n(metersPerUnit=0.001\nupAxis=\"Z\")\ndef Xform \"A\" {}\n".to_vec();
        let digest = Sha256::digest(&bytes).unwrap();
        let mut request = OpenRequest {
            identity: Identity {
                project: "0199e7a0-0000-7000-8000-000000000001".into(),
                workspace: "0199e7a0-0000-7000-8000-000000000002".into(),
                revision: digest,
                generation: 91,
            },
            root: "root.usda".into(),
            assets: vec![Asset {
                name: "root.usda".into(),
                digest,
                bytes,
            }],
            owned_layers: ["root.usda".into()].into(),
            manifest: digest,
            backend: BACKEND,
            profile: Profile::StaticLocal,
            budgets: Budgets::default(),
        };
        request.manifest = request.manifest_digest().unwrap();
        request
    }
    fn control() -> Control {
        Control {
            cancellation: Cancellation::default(),
            deadline: Instant::now() + Duration::from_secs(5),
        }
    }

    #[test]
    fn cancellation_after_real_backend_prepare_cannot_install_or_imply_rollback() {
        let mut reader = Reader::default();
        let original = reader.open(input(), &control()).unwrap();
        let control = control();
        let candidate = backend::open(input(), &control).unwrap();
        control.cancellation.cancel();
        assert_eq!(
            reader.install(candidate, &control).unwrap_err().code,
            ErrorCode::Cancelled
        );
        assert_eq!(
            reader
                .read(&original.context, None, &super::tests::control())
                .unwrap()
                .items
                .len(),
            1
        );
        assert_eq!(original.context.identity().generation, 91);
    }

    #[test]
    fn backend_unwind_has_safe_error_and_keeps_live_context() {
        let mut reader = Reader::default();
        let original = reader.open(input(), &control()).unwrap();
        let error = reader
            .prepare_read(|| panic!("synthetic private backend failure"), &control())
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::BackendUnavailable);
        assert!(!error.message.contains("synthetic"));
        assert_eq!(
            reader
                .read(&original.context, None, &control())
                .unwrap()
                .items
                .len(),
            1
        );
        assert_eq!(original.context.identity().generation, 91);
    }
}
