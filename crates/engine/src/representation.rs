// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim
//! V17 ordered filter appearances; one admitted dataset serves the complete request.
use crate::{
    class_palette::{DEFAULT_CLASS_PALETTE, class_colour},
    filter::{Dataset, Rule},
};
use beamcraft_model::{command::CommandError, error::ErrorCode, id::ElementId};
use std::collections::BTreeMap;
/// Display RGB and transparency, unrelated to engineering materials.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Appearance {
    /// Linear consumer-independent RGB channels in `[0,1]`.
    pub color: [f64; 3],
    /// Zero is opaque, one fully transparent.
    pub transparency: f64,
}
impl Appearance {
    /// Reject nonfinite/out-of-range values before any result is produced.
    pub fn validate(self) -> Result<(), CommandError> {
        if self
            .color
            .into_iter()
            .chain([self.transparency])
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(&v))
        {
            return Err(CommandError::new(
                ErrorCode::InvalidArgument,
                "appearance",
                "display channels must be finite in `[0,1]`",
            ));
        }
        Ok(())
    }
}
/// An ordered typed filter and complete appearance override.
#[derive(Debug, Clone)]
pub struct StyleRule {
    /// Shared V14 predicate.
    pub filter: Rule,
    /// Complete appearance; first matching rule wins.
    pub appearance: Appearance,
}
/// Evaluate bounded rules once, then display IDs. Overlay has final priority. It is never saved.
pub fn query(
    data: &Dataset,
    ids: &[ElementId],
    classes: &BTreeMap<ElementId, i32>,
    base: Appearance,
    by_class: bool,
    rules: &[StyleRule],
    overlay: &BTreeMap<ElementId, Appearance>,
) -> Result<BTreeMap<ElementId, Appearance>, CommandError> {
    if rules.len() > 64 || ids.len() > 65_536 || overlay.len() > 1024 {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "appearance",
            "appearance request exceeds bounds",
        ));
    }
    let all = Rule::All(Vec::new());
    let admitted = data.predicate(&all)?;
    if ids
        .iter()
        .chain(overlay.keys())
        .any(|id| !admitted.matches(*id))
    {
        return Err(CommandError::new(
            ErrorCode::NotFound,
            "appearance",
            "unknown display identity",
        ));
    }
    base.validate()?;
    for a in overlay.values() {
        a.validate()?;
    }
    let compiled = rules
        .iter()
        .map(|r| {
            r.appearance.validate()?;
            Ok((data.predicate(&r.filter)?, r.appearance))
        })
        .collect::<Result<Vec<_>, CommandError>>()?;
    Ok(ids
        .iter()
        .map(|id| {
            let mut a = base;
            if by_class
                && let Some(rgb) = classes
                    .get(id)
                    .and_then(|c| class_colour(*c, &DEFAULT_CLASS_PALETTE))
            {
                a.color = rgb.map(|v| f64::from(v) / 255.0);
            }
            if let Some((_, style)) = compiled.iter().find(|(p, _)| p.matches(*id)) {
                a = *style;
            }
            if let Some(style) = overlay.get(id) {
                a = *style;
            }
            (*id, a)
        })
        .collect())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ordered_rules_and_overlay_have_explicit_priority() {
        use crate::filter::Record;
        let id = ElementId::parse("019a7bf0-0000-7000-8000-000000ee0001").expect("id");
        let d = Dataset::new(
            BTreeMap::new(),
            vec![Record {
                id,
                kind: "plate".into(),
                name: None,
                class: Some(1),
                attributes: BTreeMap::new(),
            }],
        )
        .expect("data");
        let base = Appearance {
            color: [0.5; 3],
            transparency: 0.0,
        };
        let a = Appearance {
            color: [1.0, 0.0, 0.0],
            transparency: 0.4,
        };
        let b = Appearance {
            color: [0.0, 1.0, 0.0],
            transparency: 0.7,
        };
        let rules = vec![
            StyleRule {
                filter: Rule::All(vec![]),
                appearance: a,
            },
            StyleRule {
                filter: Rule::All(vec![]),
                appearance: b,
            },
        ];
        assert_eq!(
            query(
                &d,
                &[id],
                &BTreeMap::new(),
                base,
                false,
                &rules,
                &BTreeMap::new()
            )
            .expect("query")
            .get(&id),
            Some(&a)
        );
        assert_eq!(
            query(
                &d,
                &[id],
                &BTreeMap::new(),
                base,
                false,
                &rules,
                &BTreeMap::from([(id, b)])
            )
            .expect("query")
            .get(&id),
            Some(&b)
        );
        assert!(
            Appearance {
                color: [f64::NAN; 3],
                transparency: 0.0
            }
            .validate()
            .is_err()
        );
    }
}
