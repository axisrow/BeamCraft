// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! V17 colour sets: saved representation sets and the temporary colouring overlay.
//!
//! A representation set is an ordered list of filter rules with a colour and a transparency
//! each. The rules are the V14 rule JSON, evaluated by the same `query-elements` command the
//! CLI answers with, so a coloured element is exactly an element the engine's own evaluator
//! matches. Sets live in `colour-sets.json` in the per-user configuration directory — the
//! personal display state of the user, next to the shortcuts — and never in the model store;
//! migrating them into the V12 view book's per-store sidecar is a later slice. The overlay is
//! the result of evaluating one set against the store head: a display-side map from element
//! id to colour. It is window state only; no part of it reaches the snapshot, the store or
//! any revision.

use crate::command;
use crate::stats::Stats;
use beamcraft_engine::ElementId;
use beamcraft_json::{Json, Limits};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::atomic::AtomicBool;

/// File name of the saved sets in the configuration directory.
pub const FILE_NAME: &str = "colour-sets.json";
/// Format version this module reads.
pub const VERSION: u64 = 1;
/// Largest colour-sets file, in bytes.
pub const MAX_BYTES: usize = 65_536;
/// Most sets in one file, the built-in preset included.
pub const MAX_SETS: usize = 16;
/// Most rules in one set.
pub const MAX_RULES: usize = 32;
/// Most matching elements collected per rule: the engine's own element cap, paged.
pub const MAX_ELEMENTS: usize = 65_536;
/// Name of the built-in class set, reserved so set names stay unique.
pub const CLASS_SET_NAME: &str = "Part classes";

/// One rule of a set: the V14 rule JSON with the display colour and transparency to apply to
/// everything it matches.
#[derive(Debug, Clone, PartialEq)]
pub struct RuleAppearance {
    /// The V14 rule, evaluated by `query-elements` on the store head.
    pub rule: Json,
    /// Display RGB bytes (the P15 palette is one source; any RGB is allowed).
    pub colour: [u8; 3],
    /// 0.0 opaque to 1.0 invisible, applied where the pipeline blends.
    pub transparency: f32,
}

/// The one reason a colour-sets file is refused; the wording stays the defaults-kept message's.
fn invalid<E>(_: E) -> &'static str {
    "the colour-sets file is not valid"
}

/// A saved, ordered list of rules with colours. The first matching rule wins.
#[derive(Debug, Clone, PartialEq)]
pub struct Set {
    /// Unique readable name.
    pub name: String,
    /// The ordered rules.
    pub rules: Vec<RuleAppearance>,
}

impl Set {
    /// The tint of one rule: colour bytes as display f32, transparency folded into alpha.
    pub fn tint(rule: &RuleAppearance) -> [f32; 4] {
        let transparency = rule.transparency.clamp(0.0, 1.0);
        [
            f32::from(rule.colour[0]) / 255.0,
            f32::from(rule.colour[1]) / 255.0,
            f32::from(rule.colour[2]) / 255.0,
            1.0 - transparency,
        ]
    }

    /// One-line description of a rule for the legend: the field, the operator and the value.
    pub fn describe(rule: &Json) -> String {
        let field = rule.get("field").and_then(Json::string).unwrap_or("?");
        let attribute = match rule.get("attribute") {
            Ok(name) => format!(" {}", name.string().unwrap_or("?")),
            Err(_) => String::new(),
        };
        let op = rule.get("op").and_then(Json::string).unwrap_or("?");
        let value = match rule.get("value") {
            Ok(Json::String(text)) => text.clone(),
            Ok(other) => match other {
                Json::Number(text) => text.clone(),
                Json::Bool(value) => value.to_string(),
                _ => "?".into(),
            },
            Err(_) => "?".into(),
        };
        format!("{field}{attribute} {op} {value}")
    }
}

/// The saved sets of one user: the built-in class preset first, then the file's sets.
#[derive(Debug, Clone, PartialEq)]
pub struct Sets(Vec<Set>);

/// The built-in class set: one rule per default palette entry (P15), classes 0 to 7 opaque.
/// It gives colour-by-class without authoring anything; a custom set replaces it in effect by
/// listing its own class rules first.
pub fn class_set() -> Set {
    Set {
        name: CLASS_SET_NAME.into(),
        rules: beamcraft_engine::class_palette::DEFAULT_CLASS_PALETTE
            .iter()
            .enumerate()
            .map(|(class, colour)| RuleAppearance {
                rule: Json::object([
                    ("field", Json::text("class")),
                    ("op", Json::text("eq")),
                    ("value", Json::uint(class as u64)),
                ]),
                colour: *colour,
                transparency: 0.0,
            })
            .collect(),
    }
}

impl Default for Sets {
    fn default() -> Self {
        Self(vec![class_set()])
    }
}

impl Sets {
    /// The sets in display order.
    pub fn list(&self) -> &[Set] {
        &self.0
    }

    /// The set of `name`, if saved.
    pub fn find(&self, name: &str) -> Option<&Set> {
        self.0.iter().find(|set| set.name == name)
    }

    /// The sets of `config_dir` (`None` is the user's default directory): the class preset
    /// always; a missing file is a normal first run, a damaged one keeps the preset and is
    /// reported, never overwritten.
    pub fn load(config_dir: Option<&Path>) -> (Self, Option<String>) {
        let Some(dir) =
            config_dir.map_or_else(crate::shortcuts::default_dir, |d| Some(d.to_path_buf()))
        else {
            return (Self::default(), None);
        };
        let bytes = match std::fs::read(dir.join(FILE_NAME)) {
            Ok(bytes) if bytes.len() <= MAX_BYTES => bytes,
            Ok(_) => {
                return (
                    Self::default(),
                    Some("the colour-sets file is larger than 64 KiB; the preset is kept".into()),
                );
            }
            // A missing file is a normal first run, not a failure.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Self::default(), None),
            Err(e) => {
                return (
                    Self::default(),
                    Some(format!(
                        "the colour-sets file could not be read ({e}); the preset is kept"
                    )),
                );
            }
        };
        match Self::decode(&bytes) {
            Ok(sets) => (sets, None),
            Err(reason) => (
                Self::default(),
                Some(format!("{reason}; the preset is kept")),
            ),
        }
    }

    /// Reads sets bytes: the class preset plus every complete version-1 set.
    pub fn decode(bytes: &[u8]) -> Result<Self, &'static str> {
        let json = beamcraft_json::decode(bytes, &limits())
            .map_err(|_| "the colour-sets file is not valid")?;
        json.closed(&["version", "sets"])
            .map_err(|_| "the colour-sets file is not valid")?;
        let version = json
            .get("version")
            .and_then(Json::unsigned)
            .map_err(|_| "the colour-sets file is not valid")?;
        if version != VERSION {
            return Err("the colour-sets file has another version");
        }
        let mut sets = vec![class_set()];
        for entry in json.get("sets").and_then(Json::array).map_err(invalid)? {
            if sets.len() >= MAX_SETS {
                return Err("the colour-sets file holds too many sets");
            }
            entry.closed(&["name", "rules"]).map_err(invalid)?;
            let name = entry
                .get("name")
                .and_then(Json::string)
                .map_err(invalid)?
                .to_owned();
            if name.is_empty() || name.len() > 64 {
                return Err("a set name is empty or longer than 64 bytes");
            }
            if sets.iter().any(|set| set.name == name) {
                return Err("two sets share one name");
            }
            let rules = entry.get("rules").and_then(Json::array).map_err(invalid)?;
            if rules.is_empty() || rules.len() > MAX_RULES {
                return Err("a set holds no rules or too many");
            }
            let mut appearances = Vec::new();
            for rule in rules {
                rule.closed(&["rule", "colour", "transparency"])
                    .map_err(invalid)?;
                let colour = rule
                    .get("colour")
                    .and_then(Json::array)
                    .map_err(invalid)?
                    .iter()
                    .map(|c| {
                        c.unsigned().map_err(invalid).and_then(|v| {
                            u8::try_from(v).map_err(|_| "a colour byte is outside 0 to 255")
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let colour =
                    <[u8; 3]>::try_from(colour).map_err(|_| "a colour is not three bytes")?;
                let transparency = rule
                    .get("transparency")
                    .and_then(Json::float)
                    .map_err(|_| "a transparency is not a number")?;
                if !(0.0..=1.0).contains(&transparency) {
                    return Err("a transparency is outside 0.0 to 1.0");
                }
                appearances.push(RuleAppearance {
                    rule: rule.get("rule").map_err(invalid)?.clone(),
                    colour,
                    transparency: transparency as f32,
                });
            }
            sets.push(Set {
                name,
                rules: appearances,
            });
        }
        Ok(Self(sets))
    }
}

fn limits() -> Limits {
    Limits {
        bytes: MAX_BYTES,
        // A set adds three levels over a V14 rule's depth eight.
        depth: 12,
        tokens: 4_096,
        string_bytes: 256,
        scratch_bytes: 4_096 * 256,
    }
}

/// The display colour of everything a set matched: element id to tint, first rule wins.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Overlay(BTreeMap<ElementId, [f32; 4]>);

/// The neutral tint: no overlay at all, the pipeline's own colours.
pub const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

impl Overlay {
    /// Builds the overlay of rules in order: an element takes the first rule that matched it.
    pub fn from_rules(matches: Vec<(BTreeSet<ElementId>, [f32; 4])>) -> Self {
        let mut map = BTreeMap::new();
        for (ids, tint) in matches {
            for id in ids {
                map.entry(id).or_insert(tint);
            }
        }
        Self(map)
    }

    /// The tint of `id`: the matching rule's colour, or [`WHITE`].
    pub fn tint_of(&self, id: ElementId) -> [f32; 4] {
        self.0.get(&id).copied().unwrap_or(WHITE)
    }

    /// How many elements carry a colour.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether nothing is coloured.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Evaluates one set against the store head: one paged `query-elements` per rule, each rule
/// JSON handed to the CLI inside a temporary directory of its own evaluation, removed again
/// afterwards — a cancelled run and a new one started in the same process never share a rule
/// file. Bounded by [`MAX_RULES`] rules and [`MAX_ELEMENTS`] ids per rule; a refused rule or
/// a cancellation fails the whole set.
pub fn evaluate(
    cli: &Path,
    store: &Path,
    set: &Set,
    cancel: &AtomicBool,
    stats: &Stats,
) -> Result<Overlay, String> {
    let evaluation = EVALUATIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("beamcraft-v17-{}-{evaluation}", std::process::id()));
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("the rule directory could not be created ({e})"))?;
    let mut matches = Vec::new();
    for (index, rule) in set.rules.iter().enumerate() {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            let _ = std::fs::remove_dir_all(&dir);
            return Err("the evaluation was cancelled".into());
        }
        let path = dir.join(format!("rule-{index}.json"));
        let written = std::fs::write(
            &path,
            rule.rule
                .encode(&limits())
                .map_err(|e| format!("the rule of `{}` does not encode ({e:?})", set.name))?,
        );
        if let Err(e) = written {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(format!("the rule file could not be written ({e})"));
        }
        let ids = match query_all(cli, store, &path, cancel, stats) {
            Ok(ids) => ids,
            Err(message) => {
                let _ = std::fs::remove_dir_all(&dir);
                return Err(message);
            }
        };
        matches.push((ids.into_iter().collect(), Set::tint(rule)));
    }
    let _ = std::fs::remove_dir_all(&dir);
    Ok(Overlay::from_rules(matches))
}

/// Counting the evaluations of this process, so two jobs never share one rule directory.
static EVALUATIONS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Every matching id of one rule, paged at the CLI's page cap.
fn query_all(
    cli: &Path,
    store: &Path,
    rule: &Path,
    cancel: &AtomicBool,
    stats: &Stats,
) -> Result<Vec<ElementId>, String> {
    let mut ids = Vec::new();
    let mut after: Option<String> = None;
    let pages = MAX_ELEMENTS / 1024;
    for _ in 0..pages {
        let mut args = vec![
            "query-elements".into(),
            "--store".into(),
            store.to_string_lossy().into_owned(),
            "--rule".into(),
            rule.to_string_lossy().into_owned(),
            "--limit".into(),
            "1024".into(),
        ];
        if let Some(cursor) = &after {
            args.extend(["--after".into(), cursor.clone()]);
        }
        let output = command::run_cancellable(
            cli,
            &args,
            command::DEADLINE,
            command::MAX_OUTPUT,
            cancel,
            stats,
        )
        .map_err(|e| format!("query-elements failed: {}", e.message))?;
        let (page, next) = parse_reply(output.code, &output.stdout)?;
        ids.extend(page);
        match next {
            Some(cursor) => after = Some(cursor),
            None => return Ok(ids),
        }
    }
    Err("more than 65 536 elements match one rule; the set is not applied".into())
}

/// One `element_query` reply: its page of ids and the `next` cursor, if more follow.
fn parse_reply(
    code: Option<i32>,
    stdout: &[u8],
) -> Result<(Vec<ElementId>, Option<String>), String> {
    if code != Some(0) {
        // A rejection is one JSON object with a stable code; show it, the rule is the cause.
        let reason = beamcraft_json::decode(stdout, &Limits::DEFAULT)
            .ok()
            .and_then(|j| {
                j.get("code")
                    .ok()
                    .and_then(|c| c.string().ok().map(str::to_owned))
            })
            .unwrap_or_else(|| "unusable reply".into());
        return Err(format!("query-elements refused the rule: {reason}"));
    }
    let json = beamcraft_json::decode(stdout, &Limits::DEFAULT)
        .map_err(|_| "query-elements returned an unreadable reply".to_string())?;
    if json.get("outcome").and_then(Json::string) != Ok("element_query") {
        return Err("query-elements returned another outcome".into());
    }
    let page = json
        .get("elements")
        .and_then(Json::array)
        .map_err(|_| "query-elements returned no element page".to_string())?
        .iter()
        .map(|id| {
            id.string()
                .ok()
                .and_then(|s| ElementId::parse(s).ok())
                .ok_or_else(|| "query-elements returned an unreadable id".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let next = match json.get("next") {
        Ok(Json::Null) | Err(_) => None,
        Ok(cursor) => Some(
            cursor
                .string()
                .map_err(|_| "query-elements returned an unreadable cursor".to_string())?
                .to_owned(),
        ),
    };
    Ok((page, next))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
    use super::*;

    fn id(n: u64) -> ElementId {
        ElementId::parse(&format!("019a7bf0-0000-7000-8000-{n:012x}")).unwrap()
    }

    fn rule_json(field: &str, value: u64) -> Json {
        Json::object([
            ("field", Json::text(field)),
            ("op", Json::text("eq")),
            ("value", Json::uint(value)),
        ])
    }

    fn file_bytes(sets: &Set) -> Vec<u8> {
        Json::object([
            ("version", Json::uint(VERSION)),
            (
                "sets",
                Json::Array(vec![Json::object([
                    ("name", Json::text(sets.name.clone())),
                    (
                        "rules",
                        Json::Array(
                            sets.rules
                                .iter()
                                .map(|r| {
                                    Json::object([
                                        ("rule", r.rule.clone()),
                                        (
                                            "colour",
                                            Json::Array(
                                                r.colour
                                                    .iter()
                                                    .map(|c| Json::uint(u64::from(*c)))
                                                    .collect(),
                                            ),
                                        ),
                                        ("transparency", Json::Number(r.transparency.to_string())),
                                    ])
                                })
                                .collect(),
                        ),
                    ),
                ])]),
            ),
        ])
        .encode(&limits())
        .unwrap()
    }

    #[test]
    fn the_preset_has_one_rule_per_palette_class_and_loads_without_a_file() {
        let preset = class_set();
        assert_eq!(preset.name, CLASS_SET_NAME);
        assert_eq!(preset.rules.len(), 8);
        for (class, rule) in preset.rules.iter().enumerate() {
            assert_eq!(
                rule.colour,
                beamcraft_engine::class_palette::DEFAULT_CLASS_PALETTE[class]
            );
            assert_eq!(rule.transparency, 0.0);
            assert_eq!(
                rule.rule.get("value").and_then(Json::unsigned).unwrap(),
                class as u64
            );
        }
        let (sets, note) = Sets::load(None);
        assert_eq!(sets, Sets::default());
        assert!(note.is_none(), "a missing file is a normal first run");
    }

    #[test]
    fn sets_round_trip_through_the_file_shape_and_names_stay_unique() {
        let set = Set {
            name: "Bold plates".into(),
            rules: vec![
                RuleAppearance {
                    rule: rule_json("class", 2),
                    colour: [225, 157, 65],
                    transparency: 0.25,
                },
                RuleAppearance {
                    rule: Json::object([("any", Json::Array(vec![rule_json("class", 3)]))]),
                    colour: [112, 139, 220],
                    transparency: 0.5,
                },
            ],
        };
        let bytes = file_bytes(&set);
        let sets = Sets::decode(&bytes).unwrap();
        assert_eq!(sets.list().len(), 2, "preset plus the file's one set");
        assert_eq!(sets.find("Bold plates"), Some(&set));
        assert_eq!(sets.find(CLASS_SET_NAME), Some(&class_set()));
        // The preset's own name is reserved, and two file sets cannot share one name.
        let clash = Set {
            name: CLASS_SET_NAME.into(),
            rules: vec![RuleAppearance {
                rule: rule_json("class", 1),
                colour: [1, 2, 3],
                transparency: 0.0,
            }],
        };
        assert!(Sets::decode(&file_bytes(&clash)).is_err());
    }

    #[test]
    fn damaged_bounds_and_other_versions_are_refused_with_the_preset_kept() {
        for bytes in [
            &b"{"[..],
            b"{}",
            br#"{"version":2,"sets":[]}"#,
            br#"{"version":1,"sets":[{"name":"","rules":[{"rule":{},"colour":[1,2,3],"transparency":0.0}]}]}"#,
            br#"{"version":1,"sets":[{"name":"A","rules":[]}]}"#,
            br#"{"version":1,"sets":[{"name":"A","rules":[{"rule":{},"colour":[1,2],"transparency":0.0}]}]}"#,
            br#"{"version":1,"sets":[{"name":"A","rules":[{"rule":{},"colour":[1,2,3],"transparency":1.5}]}]}"#,
            br#"{"version":1,"sets":[{"name":"A","rules":[{"rule":{},"colour":[1,2,3],"transparency":0.0,"extra":1}]}]}"#,
        ] {
            assert!(Sets::decode(bytes).is_err(), "{:?}", String::from_utf8_lossy(bytes));
        }
        // A damaged file loads the preset and reports, like the shortcuts file.
        let dir = std::env::temp_dir().join(format!(
            "beamcraft-views-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(FILE_NAME), b"{").unwrap();
        let (sets, note) = Sets::load(Some(&dir));
        assert_eq!(sets, Sets::default());
        assert!(note.is_some());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn tints_fold_transparency_and_the_first_matching_rule_wins() {
        let rule = RuleAppearance {
            rule: rule_json("class", 1),
            colour: [225, 157, 65],
            transparency: 0.5,
        };
        assert_eq!(
            Set::tint(&rule),
            [225.0 / 255.0, 157.0 / 255.0, 65.0 / 255.0, 0.5]
        );
        let overlay = Overlay::from_rules(vec![
            ([id(1), id(2)].into_iter().collect(), [1.0, 0.0, 0.0, 1.0]),
            ([id(2), id(3)].into_iter().collect(), [0.0, 1.0, 0.0, 1.0]),
        ]);
        assert_eq!(overlay.len(), 3);
        assert_eq!(overlay.tint_of(id(1)), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(
            overlay.tint_of(id(2)),
            [1.0, 0.0, 0.0, 1.0],
            "first rule wins"
        );
        assert_eq!(overlay.tint_of(id(3)), [0.0, 1.0, 0.0, 1.0]);
        assert_eq!(overlay.tint_of(id(9)), WHITE, "unmatched stays neutral");
        assert!(!overlay.is_empty());
        assert!(Overlay::default().is_empty());
    }

    #[test]
    fn replies_parse_pages_and_rejections_carry_the_code() {
        let reply = Json::object([
            ("outcome", Json::text("element_query")),
            (
                "elements",
                Json::Array(vec![
                    Json::text(id(7).to_string()),
                    Json::text(id(9).to_string()),
                ]),
            ),
            ("next", Json::text(id(9).to_string())),
            ("model_manifest", Json::Null),
        ])
        .encode(&Limits::DEFAULT)
        .unwrap();
        let (page, next) = parse_reply(Some(0), &reply).unwrap();
        assert_eq!(page, vec![id(7), id(9)]);
        assert_eq!(next, Some(id(9).to_string()));
        let end = Json::object([
            ("outcome", Json::text("element_query")),
            ("elements", Json::Array(Vec::new())),
            ("next", Json::Null),
            ("model_manifest", Json::Null),
        ])
        .encode(&Limits::DEFAULT)
        .unwrap();
        let (page, next) = parse_reply(Some(0), &end).unwrap();
        assert!(page.is_empty() && next.is_none());
        let refused = br#"{"outcome":"rejected","code":"NOT_FOUND","message":"no revision"}"#;
        let error = parse_reply(Some(3), refused).unwrap_err();
        assert!(error.contains("NOT_FOUND"), "{error}");
        assert!(parse_reply(Some(0), b"not json").is_err());
    }

    #[test]
    fn rules_describe_their_field_operator_and_value() {
        assert_eq!(Set::describe(&rule_json("class", 3)), "class eq 3");
        assert_eq!(
            Set::describe(&Json::object([
                ("field", Json::text("attribute")),
                ("attribute", Json::text("Mark")),
                ("op", Json::text("starts-with")),
                ("value", Json::text("B")),
            ])),
            "attribute Mark starts-with B"
        );
    }

    #[test]
    fn evaluations_use_their_own_rule_directories_and_leave_nothing_behind() {
        // Without a CLI every rule fails, which is exactly the cancelled-or-refused path: the
        // directory of each evaluation is removed again, and a second evaluation collides with
        // nothing because its directory is counted per evaluation.
        let missing = std::env::temp_dir().join("beamcraft-v17-no-cli");
        let stats = crate::stats::Stats::default();
        let never = std::sync::atomic::AtomicBool::new(false);
        let stop = std::sync::atomic::AtomicBool::new(true);
        for cancel in [&never, &never, &stop] {
            assert!(evaluate(&missing, &missing, &class_set(), cancel, &stats).is_err());
        }
        let prefix = format!("beamcraft-v17-{}-", std::process::id());
        let leftovers: Vec<_> = std::fs::read_dir(std::env::temp_dir())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }
}
