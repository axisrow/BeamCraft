// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! A09 numbering approval: turns the proposal of [`crate::numbering`] into marks that are stored
//! as authored data, and judges later whether the stored marks still match the model.
//!
//! The command is [`ApproveNumbering`]: the settings of a preview and the preview token the user
//! saw. [`approve`] evaluates the preview again against the facts of the current head and refuses
//! when the proposal is no longer the one the token names (`StaleWorkspace`), when two groups
//! claim one position (the series overlap) and when there is nothing to number. Otherwise every
//! member of every group gets the mark of its group. Equivalent parts therefore share one mark and
//! non-equivalent parts have different ones, and approving again after an edit that does not
//! change the proposal (a move, a rename when names are not compared) yields the same marks.
//!
//! Every mark is stored with a [`Basis`]: the settings and the preview token of the approval that
//! wrote it. [`assess`] recomputes the preview with the settings of the stored basis and compares
//! its token with the stored one, so the approval is stale exactly when the proposal would now
//! differ: a change of profile, material, dimensions, holes or outline, a new, copied or deleted
//! part, a regrouped assembly. A move, a camera, view or selection change never does, because
//! equivalence is position independent (A08) and none of them is a fact of the preview.
//!
//! Out of scope (A09 is the single-user form): renumber and clear commands, modified-part options
//! ("keep", "take new", "re-use old numbers"), welds, bolts, standard parts, family and control
//! numbers, and the multi-user coordinator. Re-approval always recomputes every number from the
//! rules of A08.

use crate::footprint::Footprint;
use crate::numbering::{
    AssemblyFact, Group, GroupKind, MAX_ITEMS, PartFact, Preview, Series, Settings, Status, preview,
};
use beamcraft_model::command::CommandError;
use beamcraft_model::digest::{ContentDigest, Sha256};
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::ElementId;
use std::collections::{BTreeMap, BTreeSet};

/// Longest approval record in bytes; the layer engine bounds the stored string the same way.
pub const MAX_BASIS_BYTES: usize = 512;
/// How many differing elements a [`Report`] names.
pub const MAX_REPORTED: usize = 32;
/// The version word that starts every approval record.
const BASIS_VERSION: &str = "BCA1";

/// Why an approval or an assessment could not be made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalErrorKind {
    /// A setting or a fact is outside the bounds of the preview, or a record is damaged.
    Invalid,
    /// The proposal is no longer the one the token names.
    Stale,
    /// Two groups would get one position.
    Overlap,
    /// There is no part and no assembly.
    Empty,
    /// The batch is larger than one revision takes.
    Limit,
}

/// Why an approval was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApprovalError {
    /// The class of the refusal.
    pub kind: ApprovalErrorKind,
    /// A fixed English explanation.
    pub message: &'static str,
}

impl ApprovalError {
    /// The F04 error code the adapters report for this refusal.
    pub const fn code(self) -> ErrorCode {
        match self.kind {
            ApprovalErrorKind::Invalid | ApprovalErrorKind::Overlap | ApprovalErrorKind::Empty => {
                ErrorCode::InvalidArgument
            }
            ApprovalErrorKind::Stale => ErrorCode::StaleWorkspace,
            ApprovalErrorKind::Limit => ErrorCode::LimitExceeded,
        }
    }
}

impl core::fmt::Display for ApprovalError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.message)
    }
}

impl core::error::Error for ApprovalError {}

const fn refuse(kind: ApprovalErrorKind, message: &'static str) -> ApprovalError {
    ApprovalError { kind, message }
}

fn float_key(v: f64) -> String {
    format!("{:016x}", (v + 0.0).to_bits())
}

/// The settings and the preview token of an approval: what the user approved. It is stored with
/// every mark as one line of printable ASCII.
#[derive(Debug, Clone, PartialEq)]
pub struct Basis {
    /// The settings of the preview.
    pub settings: Settings,
    /// The token of that preview.
    pub token: ContentDigest,
}

impl Basis {
    /// The canonical record: version, the four series, the tolerance as exact bits, whether names
    /// are compared and the token.
    pub fn to_text(&self) -> String {
        let s = &self.settings;
        format!(
            "{BASIS_VERSION} plate={}:{} beam={}:{} column={}:{} assembly={}:{} tol={} name={} token={}",
            s.plate.prefix,
            s.plate.start,
            s.beam.prefix,
            s.beam.start,
            s.column.prefix,
            s.column.start,
            s.assembly.prefix,
            s.assembly.start,
            float_key(s.tolerance_mm),
            u8::from(s.compare_name),
            self.token,
        )
    }

    /// Reads a record back. Anything that is not exactly the canonical text of a record is
    /// `None`; the values are checked again when the preview is evaluated.
    pub fn parse(text: &str) -> Option<Self> {
        if text.len() > MAX_BASIS_BYTES || !text.is_ascii() {
            return None;
        }
        let mut words = text.split(' ');
        if words.next()? != BASIS_VERSION {
            return None;
        }
        fn field<'a>(words: &mut core::str::Split<'a, char>, key: &str) -> Option<&'a str> {
            words.next()?.strip_prefix(key)?.strip_prefix('=')
        }
        fn series(value: &str) -> Option<Series> {
            let (prefix, start) = value.rsplit_once(':')?;
            Some(Series {
                prefix: prefix.to_owned(),
                start: start.parse().ok()?,
            })
        }
        let plate = series(field(&mut words, "plate")?)?;
        let beam = series(field(&mut words, "beam")?)?;
        let column = series(field(&mut words, "column")?)?;
        let assembly = series(field(&mut words, "assembly")?)?;
        let tolerance = field(&mut words, "tol")?;
        let compare_name = match field(&mut words, "name")? {
            "0" => false,
            "1" => true,
            _ => return None,
        };
        let token = field(&mut words, "token")?.parse::<ContentDigest>().ok()?;
        if words.next().is_some() || tolerance.len() != 16 {
            return None;
        }
        let basis = Self {
            settings: Settings {
                plate,
                beam,
                column,
                assembly,
                tolerance_mm: f64::from_bits(u64::from_str_radix(tolerance, 16).ok()?),
                compare_name,
            },
            token,
        };
        (basis.to_text() == text).then_some(basis)
    }
}

/// The mark of one element: its series prefix and its number in the series.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mark {
    /// The series prefix (may be empty).
    pub series: String,
    /// The number in the series.
    pub number: u64,
}

impl Mark {
    /// The mark as text: the prefix followed by the number, with a `-` between them for an
    /// assembly. This is the rule every reader of a stored mark (IFC and BOM export) applies.
    pub fn text(&self, kind: GroupKind) -> String {
        match kind {
            GroupKind::Assembly => format!("{}-{}", self.series, self.number),
            GroupKind::Part(_) => format!("{}{}", self.series, self.number),
        }
    }
}

/// A mark an approval gives to one element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    /// The element.
    pub id: ElementId,
    /// Whether it is a part or an assembly.
    pub kind: GroupKind,
    /// Its mark.
    pub mark: Mark,
}

/// The command `ApproveNumbering`: approve the numbering that the preview with these settings
/// proposed, provided it is still the proposal.
#[derive(Debug, Clone, PartialEq)]
pub struct ApproveNumbering {
    /// The settings of the preview the user saw.
    pub settings: Settings,
    /// The token of that preview.
    pub preview_token: ContentDigest,
}

impl ApproveNumbering {
    /// The content fingerprint of the command, the identity under which a retry of the same
    /// operation id is recognised: the canonical text of its settings and token.
    pub fn fingerprint(&self) -> Result<ContentDigest, CommandError> {
        let basis = Basis {
            settings: self.settings.clone(),
            token: self.preview_token,
        };
        Sha256::digest(format!("ApproveNumbering\n{}\n", basis.to_text()).as_bytes()).map_err(
            |_| {
                CommandError::new(
                    ErrorCode::LimitExceeded,
                    "settings",
                    "the command is too long to fingerprint",
                )
            },
        )
    }
}

/// The result of an approval: the marks to store and the record to store with them.
#[derive(Debug, Clone, PartialEq)]
pub struct Approval {
    /// A mark for every part and every assembly, in element-id order.
    pub assignments: Vec<Assignment>,
    /// The approval record every mark is stored with.
    pub basis: Basis,
    /// Groups approved.
    pub groups: usize,
}

impl Approval {
    /// The declared footprint of the approval: for every numbered element it reads the lifecycle
    /// and the label group and writes the label group (a mark is stored with the label's group,
    /// so it conflicts with a rename of the element and with another approval, and with nothing
    /// derived). Fails with `LIMIT_EXCEEDED` when the batch is beyond the footprint bound.
    pub fn footprint(&self) -> Result<Footprint, CommandError> {
        let mut footprint = Footprint::default();
        for assignment in &self.assignments {
            footprint = footprint.union(Footprint::for_update(
                assignment.id,
                true,
                false,
                false,
                false,
                None,
            )?)?;
        }
        Ok(footprint)
    }
}

fn series_of(settings: &Settings, kind: GroupKind) -> &Series {
    match kind {
        GroupKind::Part(crate::numbering::PartKind::Plate) => &settings.plate,
        GroupKind::Part(crate::numbering::PartKind::Beam) => &settings.beam,
        GroupKind::Part(crate::numbering::PartKind::Column) => &settings.column,
        GroupKind::Assembly => &settings.assembly,
    }
}

fn mark_of(settings: &Settings, group: &Group) -> Mark {
    Mark {
        series: series_of(settings, group.kind).prefix.clone(),
        number: group.number,
    }
}

/// The mark each numbered member gets. Overlapping groups give none.
fn assignments(proposal: &Preview, settings: &Settings) -> Vec<Assignment> {
    let mut all: BTreeMap<ElementId, Assignment> = BTreeMap::new();
    for group in proposal.groups.iter().filter(|g| g.status == Status::New) {
        let mark = mark_of(settings, group);
        for member in &group.members {
            all.insert(
                *member,
                Assignment {
                    id: *member,
                    kind: group.kind,
                    mark: mark.clone(),
                },
            );
        }
    }
    all.into_values().collect()
}

fn invalid_preview(e: crate::numbering::NumberingError) -> ApprovalError {
    refuse(ApprovalErrorKind::Invalid, e.0)
}

/// Approves the numbering of the given facts.
///
/// Fails with [`ApprovalErrorKind::Invalid`] for settings or facts the preview refuses,
/// [`ApprovalErrorKind::Empty`] when there is nothing to number,
/// [`ApprovalErrorKind::Stale`] when the proposal for these facts and settings is not the one the
/// token names, and [`ApprovalErrorKind::Overlap`] when two groups would get one position (an
/// approval never leaves a group unnumbered).
pub fn approve(
    parts: &[PartFact],
    assemblies: &[AssemblyFact],
    command: &ApproveNumbering,
) -> Result<Approval, ApprovalError> {
    let proposal = preview(parts, assemblies, &command.settings).map_err(invalid_preview)?;
    if proposal.groups.is_empty() {
        return Err(refuse(
            ApprovalErrorKind::Empty,
            "the model has no part and no assembly to number",
        ));
    }
    if proposal.token != command.preview_token {
        return Err(refuse(
            ApprovalErrorKind::Stale,
            "the model changed since the preview: the proposal is not the one that was approved",
        ));
    }
    if proposal
        .groups
        .iter()
        .any(|g| g.status == Status::Overlapping)
    {
        return Err(refuse(
            ApprovalErrorKind::Overlap,
            "two groups would get the same position: change a series start or prefix",
        ));
    }
    let basis = Basis {
        settings: command.settings.clone(),
        token: proposal.token,
    };
    if basis.to_text().len() > MAX_BASIS_BYTES {
        return Err(refuse(
            ApprovalErrorKind::Invalid,
            "the approval record is too long",
        ));
    }
    Ok(Approval {
        assignments: assignments(&proposal, &command.settings),
        basis,
        groups: proposal.groups.len(),
    })
}

/// A mark as read from the model, with the approval record stored beside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredMark {
    /// The element.
    pub id: ElementId,
    /// Its stored mark.
    pub mark: Mark,
    /// The approval record stored with it.
    pub basis: String,
}

/// Whether the stored numbering still matches the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Currency {
    /// No mark is stored.
    NotApproved,
    /// The numbering the model would get now is the approved one.
    Current,
    /// The model would get another numbering than the approved one, or the stored marks do not
    /// describe one approval.
    Stale,
}

impl Currency {
    /// The wire token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotApproved => "not_approved",
            Self::Current => "current",
            Self::Stale => "stale",
        }
    }
}

/// The judgement of [`assess`].
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    /// Current, stale or not approved.
    pub currency: Currency,
    /// Why it is stale.
    pub reason: Option<&'static str>,
    /// The approved settings and token, when one readable record is stored.
    pub basis: Option<Basis>,
    /// Parts and assemblies that have a stored mark.
    pub marked: usize,
    /// Parts and assemblies that have none.
    pub unmarked: usize,
    /// How many elements differ from the proposal under the approved settings.
    pub differing: usize,
    /// The first [`MAX_REPORTED`] of them, in id order.
    pub differing_ids: Vec<ElementId>,
}

/// Judges the stored marks against the facts of the model. Nothing is written.
///
/// Fails only when the facts or the stored list are outside the bounds ([`MAX_ITEMS`], an id
/// twice); a damaged or inconsistent record is a stale report, never an error.
pub fn assess(
    parts: &[PartFact],
    assemblies: &[AssemblyFact],
    stored: &[StoredMark],
) -> Result<Report, ApprovalError> {
    if stored.len() > MAX_ITEMS.saturating_mul(2) {
        return Err(refuse(ApprovalErrorKind::Invalid, "too many stored marks"));
    }
    let known: BTreeSet<ElementId> = parts
        .iter()
        .map(|p| p.id)
        .chain(assemblies.iter().map(|a| a.id))
        .collect();
    let by_id: BTreeMap<ElementId, &StoredMark> = stored.iter().map(|m| (m.id, m)).collect();
    if by_id.len() != stored.len() {
        return Err(refuse(
            ApprovalErrorKind::Invalid,
            "an element has two stored marks",
        ));
    }
    let marked = known.iter().filter(|id| by_id.contains_key(id)).count();
    let mut report = Report {
        currency: Currency::NotApproved,
        reason: None,
        basis: None,
        marked,
        unmarked: known.len().saturating_sub(marked),
        differing: 0,
        differing_ids: Vec::new(),
    };
    if stored.is_empty() {
        return Ok(report);
    }
    report.currency = Currency::Stale;
    let records: BTreeSet<&str> = stored.iter().map(|m| m.basis.as_str()).collect();
    let mut records = records.into_iter();
    let (Some(record), None) = (records.next(), records.next()) else {
        report.reason = Some("the stored marks come from different approvals");
        return Ok(report);
    };
    let Some(basis) = Basis::parse(record) else {
        report.reason = Some("the approval record of the marks is unreadable");
        return Ok(report);
    };
    report.basis = Some(basis.clone());
    let Ok(proposal) = preview(parts, assemblies, &basis.settings) else {
        report.reason = Some("the approved settings no longer evaluate");
        return Ok(report);
    };
    let expected: BTreeMap<ElementId, Assignment> = assignments(&proposal, &basis.settings)
        .into_iter()
        .map(|a| (a.id, a))
        .collect();
    let differing: Vec<ElementId> = known
        .iter()
        .chain(by_id.keys())
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|id| match (expected.get(id), by_id.get(id)) {
            (Some(want), Some(have)) => want.mark != have.mark,
            (None, None) => false,
            _ => true,
        })
        .collect();
    report.differing = differing.len();
    report.differing_ids = differing.into_iter().take(MAX_REPORTED).collect();
    if proposal.token != basis.token {
        report.reason = Some("the model would now get another numbering than the approved one");
    } else if report.differing > 0 {
        report.reason = Some("a stored mark differs from the approved numbering");
    } else {
        report.currency = Currency::Current;
    }
    Ok(report)
}

/// How the stored marks of a group's members compare with the position the preview proposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredStatus {
    /// No member has a stored mark.
    None,
    /// Every member has the stored mark that equals the proposed position.
    Same,
    /// Every member has one and the same stored mark, and it is not the proposed position.
    Different,
    /// The members have different stored marks or only some have one.
    Mixed,
}

impl StoredStatus {
    /// The wire token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Same => "same",
            Self::Different => "different",
            Self::Mixed => "mixed",
        }
    }
}

/// Compares the stored marks of a group's members with the group's proposed position.
pub fn compare_group(group: &Group, stored: &BTreeMap<ElementId, Mark>) -> StoredStatus {
    let marks: Vec<Option<&Mark>> = group.members.iter().map(|m| stored.get(m)).collect();
    if marks.iter().all(Option::is_none) {
        return StoredStatus::None;
    }
    let first = marks.first().copied().flatten();
    if first.is_none() || marks.iter().any(|m| *m != first) {
        return StoredStatus::Mixed;
    }
    match first {
        Some(mark) if mark.text(group.kind) == group.position => StoredStatus::Same,
        _ => StoredStatus::Different,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::numbering::PartKind;

    fn id(n: u16) -> ElementId {
        ElementId::parse(&format!("019a7bf0-0000-7000-8000-{n:012x}")).unwrap()
    }

    fn digest(seed: &str) -> ContentDigest {
        Sha256::digest(seed.as_bytes()).unwrap()
    }

    fn plate(n: u16, thickness: f64) -> PartFact {
        PartFact {
            id: id(n),
            kind: PartKind::Plate,
            name: format!("P{n}"),
            profile: None,
            material: Some(digest("s355")),
            dimensions: vec![400.0, 300.0, thickness],
            angles_rad: Vec::new(),
            holes: Vec::new(),
            outline: Vec::new(),
            geometry: None,
            assembly: None,
        }
    }

    fn command(parts: &[PartFact], assemblies: &[AssemblyFact]) -> ApproveNumbering {
        let settings = Settings::default();
        let token = preview(parts, assemblies, &settings).unwrap().token;
        ApproveNumbering {
            settings,
            preview_token: token,
        }
    }

    fn stored(approval: &Approval) -> Vec<StoredMark> {
        approval
            .assignments
            .iter()
            .map(|a| StoredMark {
                id: a.id,
                mark: a.mark.clone(),
                basis: approval.basis.to_text(),
            })
            .collect()
    }

    #[test]
    fn equivalent_parts_share_a_mark_and_the_others_differ() {
        let parts = [plate(1, 20.0), plate(2, 20.0), plate(3, 25.0)];
        let approval = approve(&parts, &[], &command(&parts, &[])).unwrap();
        let marks: Vec<String> = approval
            .assignments
            .iter()
            .map(|a| a.mark.text(a.kind))
            .collect();
        assert_eq!(marks, ["PL1", "PL1", "PL2"]);
        assert_eq!(approval.groups, 2);
    }

    #[test]
    fn an_assembly_mark_has_a_dash() {
        let parts = [PartFact {
            assembly: Some(id(100)),
            ..plate(1, 20.0)
        }];
        let assemblies = [AssemblyFact {
            id: id(100),
            name: "Frame".into(),
            parent: None,
        }];
        let approval = approve(&parts, &assemblies, &command(&parts, &assemblies)).unwrap();
        let texts: Vec<String> = approval
            .assignments
            .iter()
            .map(|a| a.mark.text(a.kind))
            .collect();
        assert_eq!(texts, ["PL1", "A-1"]);
    }

    #[test]
    fn a_stale_token_is_refused() {
        let parts = [plate(1, 20.0), plate(2, 20.0)];
        let ask = command(&parts, &[]);
        let changed = [plate(1, 20.0), plate(2, 22.0)];
        let refused = approve(&changed, &[], &ask).unwrap_err();
        assert_eq!(refused.kind, ApprovalErrorKind::Stale);
        assert_eq!(refused.code(), ErrorCode::StaleWorkspace);
        let other_settings = ApproveNumbering {
            settings: Settings {
                compare_name: true,
                ..Settings::default()
            },
            ..ask
        };
        assert_eq!(
            approve(&parts, &[], &other_settings).unwrap_err().kind,
            ApprovalErrorKind::Stale
        );
    }

    #[test]
    fn overlapping_series_are_refused_and_an_empty_model_has_nothing_to_number() {
        let parts = [plate(1, 20.0)];
        let beam = PartFact {
            kind: PartKind::Beam,
            profile: Some(digest("p")),
            dimensions: vec![1000.0],
            angles_rad: vec![0.0],
            ..plate(2, 20.0)
        };
        let both = [parts[0].clone(), beam];
        let settings = Settings {
            beam: Series {
                prefix: "PL".into(),
                start: 1,
            },
            ..Settings::default()
        };
        let token = preview(&both, &[], &settings).unwrap().token;
        let overlap = ApproveNumbering {
            settings,
            preview_token: token,
        };
        assert_eq!(
            approve(&both, &[], &overlap).unwrap_err().kind,
            ApprovalErrorKind::Overlap
        );
        let nothing = command(&[], &[]);
        assert_eq!(
            approve(&[], &[], &nothing).unwrap_err().kind,
            ApprovalErrorKind::Empty
        );
        let bad = ApproveNumbering {
            settings: Settings {
                tolerance_mm: f64::NAN,
                ..Settings::default()
            },
            preview_token: token,
        };
        assert_eq!(
            approve(&both, &[], &bad).unwrap_err().kind,
            ApprovalErrorKind::Invalid
        );
    }

    #[test]
    fn approving_again_gives_the_same_marks_and_a_copy_renumbers_nothing() {
        let parts = [plate(1, 20.0), plate(2, 25.0)];
        let first = approve(&parts, &[], &command(&parts, &[])).unwrap();
        let again = approve(&parts, &[], &command(&parts, &[])).unwrap();
        assert_eq!(first, again);
        let with_copy = [plate(1, 20.0), plate(2, 25.0), plate(3, 20.0)];
        let grown = approve(&with_copy, &[], &command(&with_copy, &[])).unwrap();
        let marks: Vec<String> = grown
            .assignments
            .iter()
            .map(|a| a.mark.text(a.kind))
            .collect();
        assert_eq!(marks, ["PL1", "PL2", "PL1"]);
    }

    #[test]
    fn the_basis_round_trips_and_only_its_canonical_text_is_read() {
        let basis = Basis {
            settings: Settings {
                plate: Series {
                    prefix: String::new(),
                    start: 1000,
                },
                assembly: Series {
                    prefix: "AS-M".into(),
                    start: 5,
                },
                tolerance_mm: 0.25,
                compare_name: true,
                ..Settings::default()
            },
            token: digest("t"),
        };
        let text = basis.to_text();
        assert!(text.len() < MAX_BASIS_BYTES && text.is_ascii());
        assert_eq!(Basis::parse(&text), Some(basis.clone()));
        for broken in [
            "",
            "BCA1",
            &text.replace("BCA1", "BCA2"),
            &format!("{text} extra"),
            &text.replace("name=1", "name=2"),
            &text.replace("beam=B:1", "beam=B1"),
            &text.replace("tol=", "tol=0"),
            &text.replace(" name=1", " name=1 "),
            &"x".repeat(MAX_BASIS_BYTES + 1),
        ] {
            assert_eq!(Basis::parse(broken), None, "{broken}");
        }
    }

    #[test]
    fn a_fabrication_change_makes_the_approval_stale_and_a_rename_or_move_does_not() {
        let parts = [plate(1, 20.0), plate(2, 20.0)];
        let approval = approve(&parts, &[], &command(&parts, &[])).unwrap();
        let marks = stored(&approval);
        let now = assess(&parts, &[], &marks).unwrap();
        assert_eq!(now.currency, Currency::Current);
        assert_eq!((now.marked, now.unmarked, now.differing), (2, 0, 0));
        // A move is no fact of the preview, and a rename is one only when names are compared.
        let renamed = [
            PartFact {
                name: "other".into(),
                ..plate(1, 20.0)
            },
            plate(2, 20.0),
        ];
        assert_eq!(
            assess(&renamed, &[], &marks).unwrap().currency,
            Currency::Current
        );
        // A thickness, a material, a hole, a length and a deletion each make it stale.
        let thicker = [plate(1, 20.0), plate(2, 21.0)];
        let other_material = [
            plate(1, 20.0),
            PartFact {
                material: Some(digest("s275")),
                ..plate(2, 20.0)
            },
        ];
        let holed = [
            plate(1, 20.0),
            PartFact {
                holes: vec![vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]],
                ..plate(2, 20.0)
            },
        ];
        let longer = [
            plate(1, 20.0),
            PartFact {
                dimensions: vec![400.0, 310.0, 20.0],
                ..plate(2, 20.0)
            },
        ];
        for changed in [
            &thicker[..],
            &other_material[..],
            &holed[..],
            &longer[..],
            &parts[..1],
        ] {
            let report = assess(changed, &[], &marks).unwrap();
            assert_eq!(report.currency, Currency::Stale);
            assert!(report.reason.is_some());
        }
        // The part that changed is named.
        let report = assess(&thicker, &[], &marks).unwrap();
        assert_eq!(report.differing_ids, [id(2)]);
        // A new part has no mark: stale, one unmarked.
        let added = [plate(1, 20.0), plate(2, 20.0), plate(3, 20.0)];
        let report = assess(&added, &[], &marks).unwrap();
        assert_eq!((report.currency, report.unmarked), (Currency::Stale, 1));
    }

    #[test]
    fn no_marks_is_not_approved_and_damaged_marks_are_stale_not_errors() {
        let parts = [plate(1, 20.0), plate(2, 25.0)];
        let none = assess(&parts, &[], &[]).unwrap();
        assert_eq!((none.currency, none.unmarked), (Currency::NotApproved, 2));
        let approval = approve(&parts, &[], &command(&parts, &[])).unwrap();
        let marks = stored(&approval);
        let mut garbled = marks.clone();
        for m in &mut garbled {
            m.basis = "not a record".into();
        }
        let report = assess(&parts, &[], &garbled).unwrap();
        assert_eq!(report.currency, Currency::Stale);
        assert_eq!(report.basis, None);
        assert_eq!(
            report.reason,
            Some("the approval record of the marks is unreadable")
        );
        let mut mixed = marks.clone();
        mixed[1].basis = Basis {
            settings: Settings::default(),
            token: digest("other"),
        }
        .to_text();
        assert_eq!(
            assess(&parts, &[], &mixed).unwrap().reason,
            Some("the stored marks come from different approvals")
        );
        let mut tampered = marks.clone();
        tampered[0].mark.number = 9;
        let report = assess(&parts, &[], &tampered).unwrap();
        assert_eq!(report.currency, Currency::Stale);
        assert_eq!(report.differing_ids, [id(1)]);
        let mut twice = marks;
        twice.push(twice[0].clone());
        assert_eq!(
            assess(&parts, &[], &twice).unwrap_err().kind,
            ApprovalErrorKind::Invalid
        );
    }

    #[test]
    fn a_group_compares_with_the_stored_marks_of_its_members() {
        let parts = [plate(1, 20.0), plate(2, 20.0)];
        let proposal = preview(&parts, &[], &Settings::default()).unwrap();
        let group = &proposal.groups[0];
        let mark = |n: u64| Mark {
            series: "PL".into(),
            number: n,
        };
        let map = |pairs: &[(u16, u64)]| -> BTreeMap<ElementId, Mark> {
            pairs.iter().map(|(i, n)| (id(*i), mark(*n))).collect()
        };
        assert_eq!(compare_group(group, &map(&[])), StoredStatus::None);
        assert_eq!(
            compare_group(group, &map(&[(1, 1), (2, 1)])),
            StoredStatus::Same
        );
        assert_eq!(
            compare_group(group, &map(&[(1, 7), (2, 7)])),
            StoredStatus::Different
        );
        assert_eq!(compare_group(group, &map(&[(1, 1)])), StoredStatus::Mixed);
        assert_eq!(
            compare_group(group, &map(&[(1, 1), (2, 2)])),
            StoredStatus::Mixed
        );
    }

    #[test]
    fn a_batch_beyond_the_footprint_bound_is_refused() {
        let batch = |n: u16| Approval {
            assignments: (1..=n)
                .map(|i| Assignment {
                    id: id(i),
                    kind: GroupKind::Part(PartKind::Plate),
                    mark: Mark {
                        series: "PL".into(),
                        number: u64::from(i),
                    },
                })
                .collect(),
            basis: Basis {
                settings: Settings::default(),
                token: digest("t"),
            },
            groups: usize::from(n),
        };
        // Three entries per element (lifecycle and label read, label written) in 4096.
        assert!(batch(1365).footprint().is_ok());
        assert_eq!(
            batch(1366).footprint().unwrap_err().code,
            ErrorCode::LimitExceeded
        );
    }

    #[test]
    fn the_footprint_reads_and_writes_the_label_group_of_every_numbered_element() {
        let parts = [plate(1, 20.0), plate(2, 25.0)];
        let approval = approve(&parts, &[], &command(&parts, &[])).unwrap();
        let footprint = approval.footprint().unwrap();
        assert_eq!(footprint.writes().len(), 2);
        assert!(
            footprint
                .writes()
                .iter()
                .all(|e| e.group == crate::footprint::Group::Label)
        );
        assert_ne!(
            command(&parts, &[]).fingerprint().unwrap(),
            command(&parts[..1], &[]).fingerprint().unwrap()
        );
    }
}
