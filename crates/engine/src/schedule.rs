// SPDX-License-Identifier: MPL-2.0
// SPDX-FileCopyrightText: 2026 Apibim

//! I05 schedules by attributes: a template and neutral rows in, the sorted element table and
//! the group table out, with CSV and HTML text and the filter evaluator. Like [`crate::bom`],
//! the module owns the report rules and knows nothing about USD, stores or definitions, per
//! [the design note](../../../docs/design/i05-schedules.md). A missing, blocked or stale
//! value is an empty cell, never a default, and is counted next to its total.

use crate::element::ElementId;
use beamcraft_model::command::CommandError;
use beamcraft_model::error::ErrorCode;
use std::cmp::Ordering;
use std::collections::BTreeMap;

/// Maximum columns in one template.
pub const MAX_COLUMNS: usize = 64;

/// The value type of one column, independent of how the attribute layer spells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellKind {
    /// UTF-8 text; a choice value arrives already validated as text.
    Text,
    /// Finite number.
    Number,
    /// Exact integer.
    Integer,
    /// Boolean.
    Boolean,
}

/// One schedule cell's value; the attribute layer's scalars map onto these.
#[derive(Debug, Clone, PartialEq)]
pub enum CellValue {
    /// UTF-8 text.
    Text(String),
    /// Finite number.
    Number(f64),
    /// Exact integer.
    Integer(i64),
    /// Boolean.
    Boolean(bool),
}

/// A standard report field token; the same tokens as the E03 CSV headers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StandardField {
    /// The approved numbering mark, else the element id.
    Mark,
    /// The full element id.
    Id,
    /// Plate, beam or column.
    Kind,
    /// The user label as authored.
    Name,
    /// Profile label: the designation, else a caller-chosen readable label.
    Profile,
    /// Material label; empty when the element pins no material.
    Material,
    /// Axis length of a member; `None` for a plate.
    LengthMm,
    /// Area in mm2 derived from the current volume.
    AreaMm2,
    /// Current volume in mm3.
    VolumeMm3,
    /// Current volume in m3.
    VolumeM3,
    /// Current mass in kg; `None` when unknown.
    MassKg,
}

impl StandardField {
    /// Stable template token.
    pub const fn token(self) -> &'static str {
        match self {
            Self::Mark => "mark",
            Self::Id => "element_id",
            Self::Kind => "kind",
            Self::Name => "name",
            Self::Profile => "profile",
            Self::Material => "material",
            Self::LengthMm => "length_mm",
            Self::AreaMm2 => "area_mm2",
            Self::VolumeMm3 => "volume_mm3",
            Self::VolumeM3 => "volume_m3",
            Self::MassKg => "mass_kg",
        }
    }

    /// Parse an exact template token.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "mark" => Self::Mark,
            "element_id" => Self::Id,
            "kind" => Self::Kind,
            "name" => Self::Name,
            "profile" => Self::Profile,
            "material" => Self::Material,
            "length_mm" => Self::LengthMm,
            "area_mm2" => Self::AreaMm2,
            "volume_mm3" => Self::VolumeMm3,
            "volume_m3" => Self::VolumeM3,
            "mass_kg" => Self::MassKg,
            _ => return None,
        })
    }

    /// Whether the field holds numbers, so a column over it may total.
    pub const fn numeric(self) -> bool {
        matches!(
            self,
            Self::LengthMm | Self::AreaMm2 | Self::VolumeMm3 | Self::VolumeM3 | Self::MassKg
        )
    }
}

/// What one column reads.
#[derive(Debug, Clone, PartialEq)]
pub enum Field {
    /// A standard report field.
    Standard(StandardField),
    /// A known user attribute column and its value type.
    Attribute(CellKind),
    /// A definition the registry does not know; the column renders empty and stays visible.
    Unknown,
}

impl Field {
    /// The value type cells of this column may carry; `None` for an unknown column.
    pub fn cell_kind(&self) -> Option<CellKind> {
        match self {
            Self::Standard(f) if f.numeric() => Some(CellKind::Number),
            Self::Standard(_) => Some(CellKind::Text),
            Self::Attribute(kind) => Some(*kind),
            Self::Unknown => None,
        }
    }
}

/// One schedule column: its header label, what it reads and whether it totals.
#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    /// Header label in the rendered tables.
    pub label: String,
    /// What the column reads.
    pub field: Field,
    /// Sum this column in the group table; unknown cells are counted, never zeroed.
    pub total: bool,
}

/// An admitted schedule: the columns, their grouping and their sort keys.
#[derive(Debug, Clone, PartialEq)]
pub struct Template {
    /// One to [`MAX_COLUMNS`] columns.
    pub columns: Vec<Column>,
    /// Indices into `columns`; equal rendered values group together, in first-seen order.
    pub group_by: Vec<usize>,
    /// Indices into `columns`; empty sorts by the element id, always the final tiebreaker.
    pub sort_by: Vec<usize>,
}

/// One element's cells, aligned to the template columns.
#[derive(Debug, Clone, PartialEq)]
pub struct InputRow {
    /// Full element identity; always the final sort tiebreaker.
    pub id: ElementId,
    /// One cell per template column; `None` is missing, blocked or stale.
    pub cells: Vec<Option<CellValue>>,
}

/// One group total: the sum of a totalled column's known cells and the count of its
/// unknown ones, the generalization of the BOM's `mass_unknown`.
#[derive(Debug, Clone, PartialEq)]
pub struct Total {
    /// Empty when no known value contributed.
    pub sum: Option<CellValue>,
    /// Cells of this column in the group with no value.
    pub unknown: u64,
}

/// One group of the summary table.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    /// Rendered group-key cells, one per `group_by` column.
    pub cells: Vec<String>,
    /// Elements in the group.
    pub count: u64,
    /// One entry per totalled column, in template order.
    pub totals: Vec<Total>,
}

/// The two tables of one schedule; the column labels live in the template.
#[derive(Debug, Clone, PartialEq)]
pub struct Schedule {
    /// Sorted element rows of rendered cells.
    pub rows: Vec<Vec<String>>,
    /// Groups in first-seen order of the sorted rows.
    pub groups: Vec<Group>,
}

fn invalid(message: &'static str) -> CommandError {
    CommandError::new(ErrorCode::InvalidArgument, "schedule", message)
}

/// Whether one cell may sit in this column, checked against the column's value type.
fn admits(field: &Field, cell: &CellValue) -> bool {
    let Some(kind) = field.cell_kind() else {
        return false;
    };
    matches!(
        (kind, cell),
        (CellKind::Text, CellValue::Text(_))
            | (CellKind::Number, CellValue::Number(_))
            | (CellKind::Integer, CellValue::Integer(_))
            | (CellKind::Boolean, CellValue::Boolean(_))
    )
}

/// The total order of one column's cells: missing first, then by value.
fn cell_cmp(a: Option<&CellValue>, b: Option<&CellValue>) -> Ordering {
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(x), Some(y)) => match (x, y) {
            (CellValue::Text(u), CellValue::Text(v)) => u.cmp(v),
            (CellValue::Number(u), CellValue::Number(v)) => u.total_cmp(v),
            (CellValue::Integer(u), CellValue::Integer(v)) => u.cmp(v),
            (CellValue::Boolean(u), CellValue::Boolean(v)) => u.cmp(v),
            _ => rank(x).cmp(&rank(y)),
        },
    }
}

/// The stable variant order for cells of mixed types.
fn rank(value: &CellValue) -> u8 {
    match value {
        CellValue::Text(_) => 0,
        CellValue::Number(_) => 1,
        CellValue::Integer(_) => 2,
        CellValue::Boolean(_) => 3,
    }
}

/// The rendered text of one cell; missing, blocked and stale stay empty, never zero.
fn render(cell: Option<&CellValue>) -> String {
    match cell {
        None => String::new(),
        Some(CellValue::Text(s)) => s.clone(),
        Some(CellValue::Number(v)) => crate::bom::num(*v),
        Some(CellValue::Integer(v)) => v.to_string(),
        Some(CellValue::Boolean(v)) => v.to_string(),
    }
}

/// A closed comparison operator of a schedule filter rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operator {
    /// Equal.
    Eq,
    /// Not equal; a missing value still does not match.
    Ne,
    /// Less than.
    Lt,
    /// Less or equal.
    Le,
    /// Greater than.
    Gt,
    /// Greater or equal.
    Ge,
    /// Case-sensitive substring.
    Contains,
    /// Case-sensitive prefix.
    StartsWith,
}

impl Operator {
    /// Parse an exact JSON token.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "eq" => Self::Eq,
            "ne" => Self::Ne,
            "lt" => Self::Lt,
            "le" => Self::Le,
            "gt" => Self::Gt,
            "ge" => Self::Ge,
            "contains" => Self::Contains,
            "starts-with" => Self::StartsWith,
            _ => return None,
        })
    }

    /// The operators one value type admits.
    fn admits_kind(self, kind: CellKind) -> bool {
        match kind {
            CellKind::Text => matches!(
                self,
                Self::Eq | Self::Ne | Self::Contains | Self::StartsWith
            ),
            CellKind::Number | CellKind::Integer => {
                matches!(
                    self,
                    Self::Eq | Self::Ne | Self::Lt | Self::Le | Self::Gt | Self::Ge
                )
            }
            CellKind::Boolean => matches!(self, Self::Eq | Self::Ne),
        }
    }
}

/// One filter rule over the template's own columns; admitted before rows are evaluated, and
/// a missing, blocked or stale cell never matches, `ne` included.
#[derive(Debug, Clone, PartialEq)]
pub enum Rule {
    /// Logical AND; an empty group is true.
    All(Vec<Rule>),
    /// Logical OR; an empty group is false.
    Any(Vec<Rule>),
    /// One comparison against a column's cells, named by the column label.
    Compare {
        /// The column label the rule compares.
        key: String,
        /// The operator.
        op: Operator,
        /// The literal, in the column's value type.
        literal: CellValue,
    },
}

impl Rule {
    /// Admits the rule against one template: known columns, compatible literals, bounded
    /// depth and size. A rule that admits never errors during matching.
    pub fn admit(&self, template: &Template) -> Result<(), CommandError> {
        self.admit_at(template, 0, &mut 0)
    }

    fn admit_at(
        &self,
        template: &Template,
        depth: usize,
        nodes: &mut usize,
    ) -> Result<(), CommandError> {
        *nodes = nodes.saturating_add(1);
        if depth >= 8 || *nodes > 256 {
            return Err(CommandError::new(
                ErrorCode::LimitExceeded,
                "schedule",
                "a filter exceeds 256 nodes or depth eight",
            ));
        }
        match self {
            Self::All(rules) | Self::Any(rules) => {
                for rule in rules {
                    rule.admit_at(template, depth + 1, nodes)?;
                }
                Ok(())
            }
            Self::Compare { key, op, literal } => {
                let column = template
                    .columns
                    .iter()
                    .find(|column| &column.label == key)
                    .ok_or_else(|| invalid("a filter names known columns"))?;
                let Some(kind) = column.field.cell_kind() else {
                    return Err(invalid("a filter compares a column with values"));
                };
                let fits = matches!(
                    (kind, literal),
                    (CellKind::Text, CellValue::Text(_))
                        | (CellKind::Number, CellValue::Number(_))
                        | (CellKind::Integer, CellValue::Integer(_))
                        | (CellKind::Boolean, CellValue::Boolean(_))
                );
                if !fits || !op.admits_kind(kind) {
                    return Err(invalid("a filter literal does not match its column"));
                }
                Ok(())
            }
        }
    }

    /// Whether one row's cells satisfy the rule. Admitted rules never error here; the
    /// result is `Err` only for a rule that skipped admission.
    pub fn matches(&self, template: &Template, row: &InputRow) -> Result<bool, CommandError> {
        match self {
            Self::All(rules) => {
                for rule in rules {
                    if !rule.matches(template, row)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Self::Any(rules) => {
                for rule in rules {
                    if rule.matches(template, row)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            Self::Compare { key, op, literal } => {
                let index = template
                    .columns
                    .iter()
                    .position(|column| &column.label == key)
                    .ok_or_else(|| invalid("a filter names known columns"))?;
                let Some(cell) = row.cells.get(index).and_then(|cell| cell.as_ref()) else {
                    // A missing, blocked or stale value never matches, `ne` included.
                    return Ok(false);
                };
                Ok(compare(*op, cell, literal))
            }
        }
    }
}

/// One admitted comparison; same-variant cells compare in their natural order.
fn compare(op: Operator, cell: &CellValue, literal: &CellValue) -> bool {
    let ord = match (cell, literal) {
        (CellValue::Text(u), CellValue::Text(v)) => u.as_str().cmp(v.as_str()),
        (CellValue::Number(u), CellValue::Number(v)) => u.total_cmp(v),
        (CellValue::Integer(u), CellValue::Integer(v)) => u.cmp(v),
        (CellValue::Boolean(u), CellValue::Boolean(v)) => u.cmp(v),
        _ => return false,
    };
    match op {
        Operator::Eq => ord == Ordering::Equal,
        Operator::Ne => ord != Ordering::Equal,
        Operator::Lt => ord == Ordering::Less,
        Operator::Le => ord != Ordering::Greater,
        Operator::Gt => ord == Ordering::Greater,
        Operator::Ge => ord != Ordering::Less,
        // A substring or prefix over text; the order established above is unused here.
        Operator::Contains => match (cell, literal) {
            (CellValue::Text(u), CellValue::Text(v)) => u.contains(v.as_str()),
            _ => false,
        },
        Operator::StartsWith => match (cell, literal) {
            (CellValue::Text(u), CellValue::Text(v)) => u.starts_with(v.as_str()),
            _ => false,
        },
    }
}

/// Builds the tables; `LIMIT_EXCEEDED` beyond `max_rows` rows, before anything is rendered.
pub fn build(
    template: &Template,
    rows: &[InputRow],
    max_rows: usize,
) -> Result<Schedule, CommandError> {
    if template.columns.is_empty() || template.columns.len() > MAX_COLUMNS {
        return Err(invalid("a schedule needs one to 64 columns"));
    }
    if rows.len() > max_rows {
        return Err(CommandError::new(
            ErrorCode::LimitExceeded,
            "element_count",
            "the model has more elements than a schedule may list",
        ));
    }
    for column in &template.columns {
        if column.label.is_empty() || column.label.len() > 256 {
            return Err(invalid("column labels are one to 256 bytes"));
        }
        // Only numeric columns may total; an unknown column has no values to sum.
        let totalable = matches!(column.field, Field::Standard(f) if f.numeric())
            || matches!(
                column.field,
                Field::Attribute(CellKind::Number | CellKind::Integer)
            );
        if column.total && !totalable {
            return Err(invalid("only numeric columns may total"));
        }
    }
    for key in [&template.group_by, &template.sort_by] {
        for (seen, index) in key.iter().enumerate() {
            if *index >= template.columns.len() {
                return Err(invalid("group_by and sort_by name known columns"));
            }
            if key.iter().take(seen).any(|earlier| earlier == index) {
                return Err(invalid("group_by and sort_by repeat no column"));
            }
        }
    }
    let totalled: Vec<usize> = template
        .columns
        .iter()
        .enumerate()
        .filter(|(_, column)| column.total)
        .map(|(i, _)| i)
        .collect();
    for row in rows {
        if row.cells.len() != template.columns.len() {
            return Err(invalid("row cells do not match the template"));
        }
        for (cell, column) in row.cells.iter().zip(&template.columns) {
            if let Some(value) = cell
                && (column.field.cell_kind().is_none() || !admits(&column.field, value))
            {
                return Err(invalid("a cell does not match its column type"));
            }
        }
    }
    // Stable sort by the template keys, then the id, so the order is total regardless of the
    // caller's row order; admission bounded every key to each row's cells.
    let mut ordered: Vec<&InputRow> = rows.iter().collect();
    ordered.sort_by(|a, b| {
        for index in &template.sort_by {
            let ord = cell_cmp(
                a.cells.get(*index).and_then(|cell| cell.as_ref()),
                b.cells.get(*index).and_then(|cell| cell.as_ref()),
            );
            if ord != Ordering::Equal {
                return ord;
            }
        }
        a.id.cmp(&b.id)
    });
    let rendered: Vec<Vec<String>> = ordered
        .iter()
        .map(|row| row.cells.iter().map(|c| render(c.as_ref())).collect())
        .collect();
    // Group keys are the rendered cells, so equal labels group together; the index map keeps
    // the first-seen order of the sorted rows.
    let mut order = BTreeMap::new();
    let mut groups: Vec<Group> = Vec::new();
    for (row, cells) in ordered.iter().zip(&rendered) {
        // Admission bounded every group_by key to each row's length, so no cell is skipped.
        let key: Vec<String> = template
            .group_by
            .iter()
            .filter_map(|index| cells.get(*index).cloned())
            .collect();
        let position = match order.get(&key) {
            Some(&p) => p,
            None => {
                let p = groups.len();
                order.insert(key.clone(), p);
                groups.push(Group {
                    cells: key,
                    count: 0,
                    totals: totalled
                        .iter()
                        .map(|_| Total {
                            sum: None,
                            unknown: 0,
                        })
                        .collect(),
                });
                p
            }
        };
        // Positions come from the same map that appended the groups, so the slot exists.
        if let Some(group) = groups.get_mut(position) {
            group.count += 1;
            for (slot, index) in group.totals.iter_mut().zip(&totalled) {
                match (
                    row.cells.get(*index).and_then(|cell| cell.as_ref()),
                    &mut slot.sum,
                ) {
                    (None, _) => slot.unknown += 1,
                    (Some(CellValue::Number(v)), sum) => {
                        let next = sum.as_ref().map_or(0.0, |s| match s {
                            CellValue::Number(n) => *n,
                            _ => 0.0,
                        });
                        *sum = Some(CellValue::Number(next + v));
                    }
                    (Some(CellValue::Integer(v)), sum) => {
                        let next = sum.as_ref().map_or(0i64, |s| match s {
                            CellValue::Integer(n) => *n,
                            _ => 0,
                        });
                        match next.checked_add(*v) {
                            Some(total) => *sum = Some(CellValue::Integer(total)),
                            None => {
                                return Err(CommandError::new(
                                    ErrorCode::LimitExceeded,
                                    "schedule",
                                    "an integer total exceeds int64",
                                ));
                            }
                        }
                    }
                    (Some(_), _) => {}
                }
            }
        }
    }
    Ok(Schedule {
        rows: rendered,
        groups,
    })
}

/// The header labels of one index-selected column set, in template order; admission bounded
/// every key to the template, so every label resolves.
fn selected(template: &Template, indices: &[usize]) -> Vec<String> {
    indices
        .iter()
        .filter_map(|index| template.columns.get(*index).map(|c| c.label.clone()))
        .collect()
}

/// One CSV line of labels, quoted exactly like a body cell: a label is caller input (an
/// unknown-attribute selector is arbitrary template text), so it carries no trust.
fn header(labels: &[String]) -> String {
    let mut out = String::new();
    for (i, label) in labels.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        crate::bom::field(label, &mut out);
    }
    out
}

/// The element table as CSV.
pub fn elements_csv(template: &Template, schedule: &Schedule) -> String {
    let labels: Vec<String> = template.columns.iter().map(|c| c.label.clone()).collect();
    crate::bom::csv(
        &header(&labels),
        schedule.rows.iter().map(|row| row.to_vec()),
    )
}

/// The header labels and the rows of the group table, shared by the CSV and HTML writers.
fn group_rows(template: &Template, schedule: &Schedule) -> (Vec<String>, Vec<Vec<String>>) {
    let mut labels = selected(template, &template.group_by);
    labels.push("count".to_owned());
    let totalled: Vec<usize> = template
        .columns
        .iter()
        .enumerate()
        .filter(|(_, column)| column.total)
        .map(|(i, _)| i)
        .collect();
    for label in selected(template, &totalled) {
        labels.push(label.clone());
        labels.push(format!("{label}_unknown"));
    }
    let rows = schedule
        .groups
        .iter()
        .map(|g| {
            let mut cells: Vec<String> = g.cells.clone();
            cells.push(g.count.to_string());
            for total in &g.totals {
                match &total.sum {
                    Some(CellValue::Number(v)) => cells.push(crate::bom::num(*v)),
                    Some(CellValue::Integer(v)) => cells.push(v.to_string()),
                    _ => cells.push(String::new()),
                }
                cells.push(total.unknown.to_string());
            }
            cells
        })
        .collect();
    (labels, rows)
}

/// The group table as CSV: the group columns, the count, and every totalled column's sum
/// with its unknown count.
pub fn groups_csv(template: &Template, schedule: &Schedule) -> String {
    let (labels, rows) = group_rows(template, schedule);
    crate::bom::csv(&header(&labels), rows)
}

/// One HTML-escaped text; `&` goes first so no produced entity is escaped again.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn html_table(labels: &[String], rows: impl IntoIterator<Item = Vec<String>>) -> String {
    let mut out = String::from("<table>\n<tr>");
    for label in labels {
        out.push_str(&format!("<th>{}</th>", escape(label)));
    }
    out.push_str("</tr>\n");
    for row in rows {
        out.push_str("<tr>");
        for cell in row {
            out.push_str(&format!("<td>{}</td>", escape(&cell)));
        }
        out.push_str("</tr>\n");
    }
    out.push_str("</table>\n");
    out
}

/// Both tables as one deterministic HTML document: fixed headings, plain tables, the five
/// HTML entities escaped, no styles, scripts or timestamps.
pub fn html(template: &Template, schedule: &Schedule) -> String {
    let mut out = String::from(concat!(
        "<!doctype html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n",
        "<title>Schedule</title>\n</head>\n<body>\n<h1>Schedule</h1>\n",
    ));
    out.push_str("<h2>Elements</h2>\n");
    let labels: Vec<String> = template.columns.iter().map(|c| c.label.clone()).collect();
    out.push_str(&html_table(&labels, schedule.rows.iter().cloned()));
    let (group_labels, rows) = group_rows(template, schedule);
    out.push_str("<h2>Groups</h2>\n");
    out.push_str(&html_table(&group_labels, rows));
    out.push_str("</body>\n</html>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u8) -> ElementId {
        ElementId::parse(&format!("019a7bf0-0000-7000-8000-0000000000{n:02x}")).expect("fixed id")
    }

    fn text(v: &str) -> Option<CellValue> {
        Some(CellValue::Text(v.to_owned()))
    }

    fn number(v: f64) -> Option<CellValue> {
        Some(CellValue::Number(v))
    }

    fn integer(v: i64) -> CellValue {
        CellValue::Integer(v)
    }

    fn column(label: &str, field: Field) -> Column {
        Column {
            label: label.to_owned(),
            field,
            total: false,
        }
    }

    fn template(columns: Vec<Column>) -> Template {
        Template {
            columns,
            group_by: Vec::new(),
            sort_by: Vec::new(),
        }
    }

    fn row(n: u8, cells: Vec<Option<CellValue>>) -> InputRow {
        InputRow { id: id(n), cells }
    }

    fn cmp(key: &str, op: Operator, literal: CellValue) -> Rule {
        Rule::Compare {
            key: key.to_owned(),
            op,
            literal,
        }
    }

    fn refused(t: &Template, rows: &[InputRow], cap: usize) -> ErrorCode {
        build(t, rows, cap).unwrap_err().code
    }

    #[test]
    fn rows_sort_by_the_template_keys_then_the_id() {
        // Sort by a number attribute: missing first, then 2.0, then the 10.0 rows by id.
        let mut t = template(vec![
            column("mark", Field::Standard(StandardField::Mark)),
            column("Length", Field::Attribute(CellKind::Number)),
        ]);
        t.sort_by = vec![1];
        let rows = [
            row(3, vec![text("c"), number(10.0)]),
            row(1, vec![text("a"), None]),
            row(2, vec![text("b"), number(10.0)]),
            row(4, vec![text("d"), number(2.0)]),
        ];
        let schedule = build(&t, &rows, 100).expect("admitted");
        let marks: Vec<&str> = schedule.rows.iter().map(|r| r[0].as_str()).collect();
        assert_eq!(marks, ["a", "d", "b", "c"]);
        assert_eq!(schedule.rows[1][1], "2");
    }

    #[test]
    fn groups_total_known_values_and_count_unknown_cells() {
        let mut t = template(vec![
            column("profile", Field::Standard(StandardField::Profile)),
            column("Length", Field::Attribute(CellKind::Number)),
            column("Count", Field::Attribute(CellKind::Integer)),
        ]);
        t.columns[1].total = true;
        t.columns[2].total = true;
        t.group_by = vec![0];
        let rows = [
            row(
                1,
                vec![text("PL10"), number(2.5), Some(integer(9_000_000_000_000))],
            ),
            row(2, vec![text("PL10"), None, Some(integer(1))]),
            row(3, vec![text("PL12"), number(1.0), None]),
        ];
        let schedule = build(&t, &rows, 100).expect("admitted");
        assert_eq!(schedule.groups.len(), 2);
        let pl10 = &schedule.groups[0];
        assert_eq!(pl10.count, 2);
        assert_eq!(pl10.totals[0].sum, Some(CellValue::Number(2.5)));
        assert_eq!(pl10.totals[0].unknown, 1);
        // Exact integer sums, no float drift.
        assert_eq!(
            pl10.totals[1].sum,
            Some(CellValue::Integer(9_000_000_000_001))
        );
        let pl12 = &schedule.groups[1];
        assert_eq!(pl12.totals[0].sum, Some(CellValue::Number(1.0)));
        assert_eq!(pl12.totals[1].sum, None);
        assert_eq!(pl12.totals[1].unknown, 1);
    }

    #[test]
    fn filters_admit_against_the_template_and_match_cells() {
        let t = template(vec![
            column("profile", Field::Standard(StandardField::Profile)),
            column("Paint", Field::Attribute(CellKind::Text)),
            column("Count", Field::Attribute(CellKind::Integer)),
        ]);
        let rows = [
            row(1, vec![text("PL10"), text("New"), Some(integer(2))]),
            row(2, vec![text("PL12"), None, None]),
        ];
        // Refusals: unknown column, an ordering over text, wrong literal types.
        for rule in [
            cmp("Ghost", Operator::Eq, CellValue::Text(String::new())),
            cmp("Paint", Operator::Lt, CellValue::Text("New".to_owned())),
            cmp("Count", Operator::Gt, CellValue::Boolean(true)),
            cmp("Count", Operator::Eq, CellValue::Number(2.0)),
        ] {
            assert_eq!(rule.admit(&t).unwrap_err().code, ErrorCode::InvalidArgument);
        }
        // The conjunction matches the valued row; the row with missing cells never matches,
        // `ne` included.
        let rule = Rule::All(vec![
            cmp(
                "profile",
                Operator::StartsWith,
                CellValue::Text("PL".to_owned()),
            ),
            cmp("Count", Operator::Ne, integer(3)),
        ]);
        rule.admit(&t).expect("admitted");
        assert!(rule.matches(&t, &rows[0]).expect("admitted rule"));
        assert!(!rule.matches(&t, &rows[1]).expect("admitted rule"));
        // A substring or an ordered comparison, through a disjunction.
        let rule = Rule::Any(vec![
            cmp("Paint", Operator::Contains, CellValue::Text("e".to_owned())),
            cmp("Count", Operator::Ge, integer(5)),
        ]);
        rule.admit(&t).expect("admitted");
        assert!(rule.matches(&t, &rows[0]).expect("admitted rule"));
        assert!(!rule.matches(&t, &rows[1]).expect("admitted rule"));
    }

    #[test]
    fn admission_refuses_broken_templates_and_cells() {
        let text_column = template(vec![column("Status", Field::Attribute(CellKind::Text))]);
        let rows = [row(1, vec![text("New")])];
        // A total on a text column.
        let mut total = text_column.clone();
        total.columns[0].total = true;
        assert_eq!(refused(&total, &rows, 10), ErrorCode::InvalidArgument);
        // A cell that does not match its column type.
        let numbers = [row(1, vec![number(1.0)])];
        assert_eq!(
            refused(&text_column, &numbers, 10),
            ErrorCode::InvalidArgument
        );
        // An unknown column carrying a cell.
        let unknown = template(vec![column("Ghost", Field::Unknown)]);
        assert_eq!(refused(&unknown, &numbers, 10), ErrorCode::InvalidArgument);
        // Out-of-range and repeated keys.
        let mut bad = text_column.clone();
        bad.group_by = vec![1];
        assert_eq!(refused(&bad, &rows, 10), ErrorCode::InvalidArgument);
        let mut dup = text_column.clone();
        dup.group_by = vec![0, 0];
        assert_eq!(refused(&dup, &rows, 10), ErrorCode::InvalidArgument);
        // Empty columns and a cells/template mismatch.
        assert_eq!(
            refused(&template(Vec::new()), &[], 10),
            ErrorCode::InvalidArgument
        );
        let mismatched = [row(1, vec![])];
        assert_eq!(
            refused(&text_column, &mismatched, 10),
            ErrorCode::InvalidArgument
        );
        // The row cap lands before any rendering.
        assert_eq!(refused(&text_column, &rows, 0), ErrorCode::LimitExceeded);
    }

    #[test]
    fn csv_headers_quote_labels_like_cells() {
        // An unknown-attribute selector is arbitrary template text; the header quotes it.
        let t = template(vec![
            column("mark", Field::Standard(StandardField::Mark)),
            column("Ghost, \"name\"", Field::Unknown),
        ]);
        let schedule = build(&t, &[], 10).expect("admitted");
        assert!(elements_csv(&t, &schedule).starts_with("mark,\"Ghost, \"\"name\"\"\"\n"));
    }

    #[test]
    fn csv_and_html_render_deterministically_and_escape() {
        let mut t = template(vec![
            column("name", Field::Standard(StandardField::Name)),
            column("Status", Field::Attribute(CellKind::Text)),
        ]);
        t.group_by = vec![0];
        let rows = [
            row(1, vec![text("He said \"stop\", <then>"), text("a,b")]),
            row(2, vec![text("Plain"), None]),
        ];
        let first = build(&t, &rows, 10).expect("admitted");
        let second = build(&t, &rows, 10).expect("admitted");
        // Same input, byte-identical output.
        assert_eq!(elements_csv(&t, &first), elements_csv(&t, &second));
        assert_eq!(html(&t, &first), html(&t, &second));
        let text_csv = elements_csv(&t, &first);
        assert!(
            text_csv.contains(r#""He said ""stop"", <then>""#),
            "{text_csv}"
        );
        assert!(text_csv.contains(r#""a,b""#), "{text_csv}");
        // Missing values are empty cells, never zero.
        assert!(
            text_csv
                .lines()
                .nth(2)
                .expect("two rows")
                .ends_with("Plain,"),
            "{text_csv}"
        );
        let page = html(&t, &first);
        assert!(page.contains("&lt;then&gt;"), "{page}");
        assert!(page.contains("<h2>Groups</h2>"), "{page}");
        assert!(page.contains("<th>count</th>"), "{page}");
        let groups = groups_csv(&t, &first);
        assert_eq!(groups.lines().next(), Some("name,count"));
        // One row per group key in first-seen order, count only while nothing totals.
        assert!(
            groups.contains(r#""He said ""stop"", <then>",1"#) && groups.contains("Plain,1"),
            "{groups}"
        );
    }
}
