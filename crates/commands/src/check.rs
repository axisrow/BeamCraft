// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! Q03 slices 2–3: collect and check one identified revision (`check-model`).
//!
//! The command is a read: it opens the verified head revision (the committed
//! head at the moment the check starts, named in the reply), re-evaluates its
//! parts with the same kernel path as every other reader, runs the supported
//! checks of `docs/contracts/q03-model-checks-draft-v0.1.md` — the geometric
//! `PART_CLASH` rule, the three completeness rules, and since slice 3 the
//! connectivity rules `PART_UNCONNECTED`, `WELD_WITHOUT_CONTACT` and
//! `CONTACT_WITHOUT_CONNECTION` over the recorded inputs — and reports the
//! findings with the whole-scope counts and coverage. Partial evaluation is
//! explicit: evaluated and unevaluated elements are collected separately and
//! every element of the scope lands in exactly one coverage bucket, so a
//! model that cannot be fully evaluated is never silently green. No store
//! carries joint intent or recorded requirements yet: a caller without them
//! passes the default inputs, the connectivity rules stay not applicable for
//! every element (contract section 5), and the coverage paragraph says so.
//! The corner-cut pair arrives with its slice; a check requested from the
//! supported set today never reports pass or fail for it.

use crate::plate::Inspection;
use beamcraft_model::error::ErrorCode;
use beamcraft_model::id::ElementId;
use session::Refusal;
use std::collections::BTreeMap;
use steel::engine::check;
use steel::engine::check::{CONTACT_TOLERANCE_MM, ClashState, ElementBody};
use steel::engine::quantity::is_superseded_algorithm;

type Run<T> = Result<T, Refusal>;

/// The supported rule tokens of this slice, in report order.
pub const RULES: [&str; 7] = [
    "PART_CLASH",
    "PART_UNCONNECTED",
    "WELD_WITHOUT_CONTACT",
    "CONTACT_WITHOUT_CONNECTION",
    "PART_WITHOUT_MATERIAL",
    "PART_DEGENERATE",
    "QUANTITY_STALE",
];

/// One finding of the report (`fail` or `unknown`; a pass is no finding).
#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    /// The rule token.
    pub rule: &'static str,
    /// `fail` or `unknown`.
    pub status: &'static str,
    /// The element ids and labels the finding is about.
    pub elements: Vec<(ElementId, String)>,
    /// What was found.
    pub message: String,
    /// The basis of the result: the geometric identity or the rule reference.
    pub basis: String,
    /// The measured value in millimetres, where the rule measures one.
    pub measured_mm: Option<f64>,
    /// The bound the measurement was compared against, where one applies.
    pub required_mm: Option<f64>,
    /// The tolerance the comparison used, where one applies.
    pub tolerance_mm: Option<f64>,
    /// The named bounds of a range comparison, where one measurement is
    /// judged against several recorded bounds (contract section 8.1).
    pub bounds: Vec<check::NamedBound>,
    /// The inputs whose absence kept the check from deciding.
    pub missing_inputs: Vec<&'static str>,
}

/// One element's coverage bucket: every element of the scope lands in
/// exactly one.
#[derive(Debug, Clone, PartialEq)]
pub enum Bucket {
    /// The supported checks that apply to the element were decided for it.
    Checked,
    /// An established absence: the element is outside the supported checks
    /// (a weld or bolt of a later slice, a deactivated element).
    NotApplicable(&'static str),
    /// The element could not be checked.
    Unknown(&'static str),
}

/// The coverage row of one element.
#[derive(Debug, Clone, PartialEq)]
pub struct Coverage {
    /// The element.
    pub id: ElementId,
    /// The stable kind token of the element (`plate`, `beam`, `column`,
    /// `weld`, ...).
    pub kind: String,
    /// The bucket the element landed in.
    pub bucket: Bucket,
}

/// Per-rule counts over the whole checked scope, not only the returned page.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RuleCounts {
    /// Findings with status `fail`.
    pub fail: usize,
    /// Findings with status `unknown`.
    pub unknown: usize,
}

/// The whole check report for one identified revision.
#[derive(Debug, Clone)]
pub struct CheckReport {
    /// The revision the check identified (the committed head when it started).
    pub revision_id: String,
    /// The scope: the whole model, or one element's subtree.
    pub scope_mode: &'static str,
    /// The scope root when the scope is one element's subtree.
    pub scope_element: Option<ElementId>,
    /// The number of elements in the scope.
    pub scope_elements: usize,
    /// `green`, `red`, or `unknown`: red when a checked applicable rule
    /// fails, otherwise unknown when a requested applicable check could not
    /// complete, otherwise green.
    pub status: &'static str,
    /// The contact tolerance the checks used, reported with every result.
    pub tolerance_mm: f64,
    /// Per-rule counts over the whole scope.
    pub counts: BTreeMap<&'static str, RuleCounts>,
    /// Every element of the scope, in element-id order, in one bucket.
    pub coverage: Vec<Coverage>,
    /// The limitations of the supported set (what is not covered), stating
    /// the applicability of the connectivity rules for this run.
    pub limitations: String,
    /// The total number of findings, before pagination.
    pub findings_total: usize,
    /// One page of findings.
    pub findings: Vec<Finding>,
}

/// The limitations paragraph of the supported set, stating what the run
/// covered and the applicability of the connectivity rules: with no joint
/// intent they are not applicable for any element — an established absence
/// (contract section 5), which the coverage of every report names.
fn limitations_text(inputs: &check::RecordedInputs) -> String {
    let connectivity = match &inputs.intent {
        None => "no joint intent is recorded, so the connectivity rules \
                 (PART_UNCONNECTED, WELD_WITHOUT_CONTACT, CONTACT_WITHOUT_CONNECTION) are \
                 not applicable for any element"
            .to_owned(),
        Some(intent) => format!(
            "the connectivity rules ran under the declared joint intent with {} \
             declared connections; recorded gap requirements judge only the connections \
             they name and a declared weld connection without one stays unknown \
             (no_supported_rule)",
            intent.connections.len(),
        ),
    };
    format!(
        "the supported set covers the geometric clash rule, the completeness rules and \
         the connectivity rules under recorded joint intent; {connectivity}; corner-cut \
         rules are not supported yet and stay unknown, weld adequacy is not validated, \
         welds and bolt groups are not clash subjects yet, lofted plates and bent plates \
         outside the evaluated sets are not re-evaluated here, and the checks decide \
         contact and clearance, not structural adequacy"
    )
}

/// Checks one identified revision: the verified head of the store, or a
/// caller that already opened one passes the inspection it read. `inputs`
/// carries the recorded inputs of the run (contract sections 4-5): the joint
/// intent, the recorded gap requirements and the weld facts; a caller
/// without recorded inputs passes the default, and the connectivity rules
/// stay not applicable for every element.
///
/// `element` narrows the scope to that element's subtree (the element and the
/// parts of its assembly). `offset` and `limit` paginate the findings only;
/// the status, counts and coverage always cover the whole scope.
pub fn check_model(
    inspection: &Inspection,
    inputs: &check::RecordedInputs,
    element: Option<ElementId>,
    offset: usize,
    limit: usize,
) -> Run<CheckReport> {
    let revision_id = inspection.revision.revision_id.clone();
    let meta = &inspection.elements;
    // The scope: every element of the model, or the subtree of one element —
    // the element itself, or its assembly, and that assembly's parts.
    let scope: Vec<ElementId> = match element {
        None => meta.keys().copied().collect(),
        Some(target) => {
            if !meta.contains_key(&target) {
                return Err(Refusal::new(
                    ErrorCode::NotFound,
                    "the checked revision has no such element",
                ));
            }
            let root = meta.get(&target).and_then(|m| m.part_of).unwrap_or(target);
            meta.iter()
                .filter(|(id, m)| **id == root || m.part_of == Some(root))
                .map(|(id, _)| *id)
                .collect()
        }
    };
    let in_scope = |id: &ElementId| scope.binary_search(id).is_ok();
    let scope_elements = scope.len();

    // The evaluated parts in world coordinates, and the explicit remainder.
    let mut bodies: Vec<ElementBody> = Vec::new();
    let mut coverage = Vec::new();
    let mut findings: Vec<Finding> = Vec::new();
    let mut unknown_elements = 0usize;
    let mut covered: std::collections::BTreeSet<ElementId> = std::collections::BTreeSet::new();

    // Every finding is kept: the counts and the total below cover the whole
    // scope, and only the returned page is a slice of this list.
    let mut push = |finding: Finding| {
        findings.push(finding);
    };

    // The parts the checks run on, in element-id order, with their
    // completeness verdicts.
    let mut parts: Vec<(ElementId, String, &'static str)> = Vec::new();
    for record in &inspection.plates {
        parts.push((record.authored.id, record.authored.name.clone(), "plate"));
    }
    for record in &inspection.members {
        let kind = match record.authored.kind {
            steel::model::command::MemberKind::Beam => "beam",
            steel::model::command::MemberKind::Column => "column",
        };
        parts.push((record.authored.id, record.authored.name.clone(), kind));
    }
    parts.sort();
    parts.dedup();
    let part_ids: std::collections::BTreeSet<ElementId> = parts.iter().map(|p| p.0).collect();

    for (id, name, kind) in &parts {
        if !in_scope(id) {
            continue;
        }
        if meta.get(id).is_some_and(|m| m.deactivated) {
            covered.insert(*id);
            coverage.push(Coverage {
                id: *id,
                kind: (*kind).to_owned(),
                bucket: Bucket::NotApplicable("deactivated"),
            });
            continue;
        }
        // The world body; a body the kernel cannot place stays explicit.
        let body = match *kind {
            "plate" => inspection
                .plates
                .iter()
                .find(|r| r.authored.id == *id)
                .and_then(|r| ElementBody::of_plate(r).ok()),
            _ => inspection
                .members
                .iter()
                .find(|r| r.authored.id == *id)
                .and_then(|r| ElementBody::of_member(r).ok()),
        };
        let Some(body) = body else {
            covered.insert(*id);
            coverage.push(Coverage {
                id: *id,
                kind: (*kind).to_owned(),
                bucket: Bucket::Unknown("body_not_placeable"),
            });
            unknown_elements += 1;
            continue;
        };
        bodies.push(body);
        // PART_WITHOUT_MATERIAL: the part lacks a material, so its mass is
        // unknown (rule basis: the completeness rule of the contract 2.1).
        if !material_of(inspection, id) {
            push(Finding {
                rule: "PART_WITHOUT_MATERIAL",
                status: "fail",
                elements: vec![(*id, name.clone())],
                message: "the part has no material, so its mass is unknown".into(),
                basis: "completeness rule PART_WITHOUT_MATERIAL (contract 2.1)".into(),
                measured_mm: None,
                required_mm: None,
                tolerance_mm: None,
                bounds: Vec::new(),
                missing_inputs: Vec::new(),
            });
        }
        covered.insert(*id);
        coverage.push(Coverage {
            id: *id,
            kind: (*kind).to_owned(),
            bucket: Bucket::Checked,
        });
    }

    // PART_CLASH over the evaluated pairs, streamed: the pairs are visited
    // in place and pruned by the cached corner extents, never materialized
    // (a store at the engine's element cap would not fit them in memory).
    for index in 0..bodies.len() {
        let Some(a) = bodies.get(index) else {
            break;
        };
        for b in bodies.iter().skip(index + 1) {
            if !pair_bounds_overlap(a, b) {
                continue;
            }
            let outcome = check::clash_verdict(a, b, CONTACT_TOLERANCE_MM);
            let Ok(outcome) = outcome else {
                push(Finding {
                    rule: "PART_CLASH",
                    status: "unknown",
                    elements: vec![(a.id, a.name.clone()), (b.id, b.name.clone())],
                    message: "the contact decision could not be completed for this pair".into(),
                    basis: "geometric identity: solid interior overlap".into(),
                    measured_mm: None,
                    required_mm: None,
                    tolerance_mm: Some(CONTACT_TOLERANCE_MM),
                    bounds: Vec::new(),
                    missing_inputs: vec!["unresolved contact predicates"],
                });
                continue;
            };
            match outcome.state {
                ClashState::Clear => {}
                ClashState::Indeterminate => {
                    push(Finding {
                        rule: "PART_CLASH",
                        status: "unknown",
                        elements: vec![(a.id, a.name.clone()), (b.id, b.name.clone())],
                        message: format!(
                            "the parts overlap ({}) but the measured depth {} sits at the \
                         tolerance band edge",
                            outcome.contact,
                            outcome.measured_mm.map_or_else(
                                || "is unmeasured".to_owned(),
                                |m| format!("{m:.3} mm")
                            ),
                        ),
                        basis: "geometric identity: solid interior overlap".into(),
                        measured_mm: outcome.measured_mm,
                        required_mm: Some(CONTACT_TOLERANCE_MM),
                        tolerance_mm: Some(CONTACT_TOLERANCE_MM),
                        bounds: Vec::new(),
                        missing_inputs: vec!["indeterminate"],
                    });
                }
                ClashState::Penetrated => {
                    push(Finding {
                        rule: "PART_CLASH",
                        status: "fail",
                        elements: vec![(a.id, a.name.clone()), (b.id, b.name.clone())],
                        message: format!(
                            "the parts penetrate each other ({}) beyond the contact tolerance",
                            outcome.contact,
                        ),
                        basis: "geometric identity: solid interior overlap".into(),
                        measured_mm: outcome.measured_mm,
                        required_mm: Some(CONTACT_TOLERANCE_MM),
                        tolerance_mm: Some(CONTACT_TOLERANCE_MM),
                        bounds: Vec::new(),
                        missing_inputs: Vec::new(),
                    });
                }
            }
        }
    }

    // The connectivity rules over the recorded inputs (contract sections
    // 2.1 and 5). With no recorded joint intent they produce nothing — an
    // established absence the coverage paragraph names; with intent, each
    // declared connection is judged and every finding carries the element
    // labels of its parts.
    // The model's element vocabulary: a declared member absent from it is the
    // unresolved ghost, while one the model carries but the checked scope
    // omits is simply outside the scope (a subtree check of one joint member
    // never reports findings about its other members).
    let model_elements: Vec<ElementId> = inspection.elements.keys().copied().collect();
    for finding in check::connectivity_findings(
        inputs,
        &bodies,
        &scope,
        &model_elements,
        CONTACT_TOLERANCE_MM,
    ) {
        push(Finding {
            elements: finding
                .elements
                .iter()
                .map(|id| {
                    (
                        *id,
                        id_name(id, inspection).unwrap_or_else(|| id.to_string()),
                    )
                })
                .collect::<Vec<_>>(),
            rule: finding.rule,
            status: finding.status,
            message: finding.message,
            basis: finding.basis,
            measured_mm: finding.measured_mm,
            required_mm: finding.required_mm,
            tolerance_mm: finding.tolerance_mm,
            bounds: finding.bounds,
            missing_inputs: finding.missing_inputs,
        });
    }

    // PART_DEGENERATE over the established degenerate states, and the
    // explicit unknown bucket of every unevaluated part.
    for unsupported in &inspection.unsupported_plates {
        if !in_scope(&unsupported.authored.id) {
            continue;
        }
        let name = unsupported.authored.name.clone();
        let id = unsupported.authored.id;
        if unsupported.degenerate {
            push(Finding {
                rule: "PART_DEGENERATE",
                status: "fail",
                elements: vec![(id, name.clone())],
                message: format!("the part is degenerate: {}", unsupported.reason),
                basis: "completeness rule PART_DEGENERATE (contract 2.1)".into(),
                measured_mm: None,
                required_mm: None,
                tolerance_mm: None,
                bounds: Vec::new(),
                missing_inputs: Vec::new(),
            });
        }
        covered.insert(id);
        coverage.push(Coverage {
            id,
            kind: "plate".to_owned(),
            bucket: Bucket::Unknown("geometry_unsupported"),
        });
        unknown_elements += 1;
    }

    // QUANTITY_STALE over the existing assessments. A stale record is an
    // accepted snapshot for a grid-bound member (its stored values lag until
    // the release re-materializes them) and for a superseded algorithm the
    // engine recomputes; any other stale value is an integrity failure.
    let bound_members: std::collections::BTreeSet<ElementId> = inspection
        .members
        .iter()
        .filter(|r| r.authored.follow.is_some())
        .map(|r| r.authored.id)
        .collect();
    for (id, report) in &inspection.quantities {
        if !in_scope(id) {
            continue;
        }
        let stale_volume = report.assessment.volume.as_str() == "stale";
        let stale_mass = report.assessment.mass.as_str() == "stale";
        if !stale_volume && !stale_mass {
            continue;
        }
        let superseded = report
            .stored
            .as_ref()
            .is_some_and(|s| is_superseded_algorithm(&s.algorithm));
        let accepted = superseded || bound_members.contains(id);
        let sides = match (stale_volume, stale_mass) {
            (true, true) => "volume and mass",
            (true, false) => "volume",
            _ => "mass",
        };
        push(Finding {
            rule: "QUANTITY_STALE",
            status: if accepted { "unknown" } else { "fail" },
            elements: vec![(
                *id,
                id_name(id, inspection).unwrap_or_else(|| id.to_string()),
            )],
            message: format!(
                "the stored {sides} of the element is stale ({})",
                if accepted {
                    "an accepted snapshot the product recomputes or re-materializes"
                } else {
                    "an integrity failure: the stored record disagrees with the inputs"
                }
            ),
            basis: "existing quantity assessment (P04), reused (contract 2.1)".into(),
            measured_mm: None,
            required_mm: None,
            tolerance_mm: None,
            bounds: Vec::new(),
            missing_inputs: Vec::new(),
        });
    }

    // The coverage of every other scope element: established absences of the
    // supported set (welds, bolts, lofted plates, assemblies, ...) are named,
    // never silently skipped.
    for id in &scope {
        if part_ids.contains(id) || covered.contains(id) {
            continue;
        }
        let kind = meta
            .get(id)
            .map(|m| m.kind.clone())
            .unwrap_or_else(|| "unknown".to_owned());
        let bucket = if meta.get(id).is_some_and(|m| m.deactivated) {
            Bucket::NotApplicable("deactivated")
        } else {
            Bucket::NotApplicable("not_a_check_subject_yet")
        };
        covered.insert(*id);
        coverage.push(Coverage {
            id: *id,
            kind,
            bucket,
        });
    }
    coverage.sort_by_key(|row| row.id);

    // The rule counts over the whole scope, and the overall status.
    let mut counts: BTreeMap<&'static str, RuleCounts> =
        RULES.iter().map(|r| (*r, RuleCounts::default())).collect();
    for finding in &findings {
        if let Some(entry) = counts.get_mut(finding.rule) {
            if finding.status == "fail" {
                entry.fail += 1;
            } else {
                entry.unknown += 1;
            }
        }
    }
    let any_fail = findings.iter().any(|f| f.status == "fail");
    let any_unknown = findings.iter().any(|f| f.status == "unknown");
    let status = if any_fail {
        "red"
    } else if any_unknown || unknown_elements > 0 {
        "unknown"
    } else {
        "green"
    };
    let findings_total = findings.len();
    let page = findings
        .into_iter()
        .skip(offset)
        .take(limit)
        .collect::<Vec<_>>();
    Ok(CheckReport {
        revision_id,
        scope_mode: if element.is_some() {
            "element_subtree"
        } else {
            "model"
        },
        scope_element: element,
        scope_elements,
        status,
        tolerance_mm: CONTACT_TOLERANCE_MM,
        counts,
        coverage,
        limitations: limitations_text(inputs),
        findings_total,
        findings: page,
    })
}

fn material_of(inspection: &Inspection, id: &ElementId) -> bool {
    inspection
        .plates
        .iter()
        .any(|r| r.authored.id == *id && r.authored.material.is_some())
        || inspection
            .members
            .iter()
            .any(|r| r.authored.id == *id && r.authored.material.is_some())
}

/// The cheap world-bounds overlap of two bodies, before the exact decision.
fn pair_bounds_overlap(a: &ElementBody, b: &ElementBody) -> bool {
    a.bounds_overlap(b)
}

/// The label of an evaluated part, or nothing.
fn id_name(id: &ElementId, inspection: &Inspection) -> Option<String> {
    inspection
        .plates
        .iter()
        .find(|r| r.authored.id == *id)
        .map(|r| r.authored.name.clone())
        .or_else(|| {
            inspection
                .members
                .iter()
                .find(|r| r.authored.id == *id)
                .map(|r| r.authored.name.clone())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_supported_rules_are_the_slice_scope() {
        assert_eq!(RULES.len(), 7);
        assert_eq!(RULES[0], "PART_CLASH");
        assert_eq!(RULES[1], "PART_UNCONNECTED");
        assert_eq!(RULES[2], "WELD_WITHOUT_CONTACT");
        assert_eq!(RULES[3], "CONTACT_WITHOUT_CONNECTION");
    }
}
