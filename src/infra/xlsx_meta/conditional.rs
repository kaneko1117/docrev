use std::collections::{HashMap, HashSet};

use quick_xml::Reader;
use quick_xml::events::Event;

use crate::domain::anchor::Anchor;

use super::MetaError;
use super::archive::{attr_value, reference_piece, text_piece};
use super::styles::Dxf;

/// 0-based inclusive; `u32::MAX` as an end marks a whole column or row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellRange {
    pub start_row: u32,
    pub start_col: u32,
    pub end_row: u32,
    pub end_col: u32,
}

impl CellRange {
    fn contains(&self, row: u32, col: u32) -> bool {
        (self.start_row..=self.end_row).contains(&row)
            && (self.start_col..=self.end_col).contains(&col)
    }

    /// `None` when nothing of the range lies within `rows` × `cols`.
    fn clamp(&self, rows: u32, cols: u32) -> Option<CellRange> {
        if rows == 0 || cols == 0 || self.start_row >= rows || self.start_col >= cols {
            return None;
        }
        Some(CellRange {
            start_row: self.start_row,
            start_col: self.start_col,
            end_row: self.end_row.min(rows - 1),
            end_col: self.end_col.min(cols - 1),
        })
    }

    fn cells(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        (self.start_row..=self.end_row)
            .flat_map(move |r| (self.start_col..=self.end_col).map(move |c| (r, c)))
    }
}

/// One `<conditionalFormatting>` block: its `sqref` ranges and rules.
#[derive(Debug, Clone, PartialEq)]
pub struct CondFormat {
    pub ranges: Vec<CellRange>,
    pub rules: Vec<CondRule>,
}

/// Attributes as written; `formulas` are the `<formula>` children in order.
#[derive(Debug, Clone, PartialEq)]
pub struct CondRule {
    pub kind: String,
    pub operator: Option<String>,
    pub text: Option<String>,
    pub formulas: Vec<String>,
    pub dxf: Option<usize>,
    pub priority: i64,
    pub stop_if_true: bool,
    pub rank: u32,
    pub percent: bool,
    pub bottom: bool,
}

impl Default for CondRule {
    fn default() -> Self {
        Self {
            kind: String::new(),
            operator: None,
            text: None,
            formulas: Vec::new(),
            dxf: None,
            // a rule without a priority sorts after every rule that has one
            priority: i64::MAX,
            stop_if_true: false,
            rank: 10,
            percent: false,
            bottom: false,
        }
    }
}

pub(super) fn parse_conditional_formats(xml: &str) -> Result<Vec<CondFormat>, MetaError> {
    let mut reader = Reader::from_str(xml);
    let mut out = Vec::new();
    let mut block: Option<CondFormat> = None;
    let mut rule: Option<CondRule> = None;
    // `<formula>` text arrives in pieces: entity references are separate events
    let mut formula: Option<String> = None;
    loop {
        let event: Event<'_> = reader.read_event().map_err(|e| MetaError(e.to_string()))?;
        match &event {
            Event::Start(e) | Event::Empty(e) => {
                let empty = matches!(event, Event::Empty(_));
                match e.local_name().as_ref() {
                    b"conditionalFormatting" if !empty => {
                        let sqref = e
                            .attributes()
                            .flatten()
                            .find(|a| a.key.as_ref() == b"sqref")
                            .map(|a| attr_value(&a, reader.decoder()))
                            .unwrap_or_default();
                        block = Some(CondFormat {
                            ranges: parse_sqref(&sqref),
                            rules: Vec::new(),
                        });
                    }
                    b"cfRule" if block.is_some() => {
                        let mut current = CondRule::default();
                        for attr in e.attributes().flatten() {
                            let value = attr_value(&attr, reader.decoder());
                            match attr.key.as_ref() {
                                b"type" => current.kind = value,
                                b"operator" => current.operator = Some(value),
                                b"text" => current.text = Some(value),
                                b"dxfId" => current.dxf = value.parse().ok(),
                                b"priority" => current.priority = value.parse().unwrap_or(i64::MAX),
                                b"stopIfTrue" => current.stop_if_true = is_true(&value),
                                b"rank" => current.rank = value.parse().unwrap_or(10),
                                b"percent" => current.percent = is_true(&value),
                                b"bottom" => current.bottom = is_true(&value),
                                _ => {}
                            }
                        }
                        if empty {
                            if let Some(block) = &mut block {
                                block.rules.push(current);
                            }
                        } else {
                            rule = Some(current);
                        }
                    }
                    b"formula" if rule.is_some() && !empty => formula = Some(String::new()),
                    _ => {}
                }
            }
            Event::Text(t) if formula.is_some() => {
                if let Some(formula) = &mut formula {
                    formula.push_str(&text_piece(t)?);
                }
            }
            Event::GeneralRef(r) if formula.is_some() => {
                if let Some(formula) = &mut formula {
                    formula.push_str(&reference_piece(r)?);
                }
            }
            Event::End(e) => match e.local_name().as_ref() {
                b"formula" => {
                    if let (Some(text), Some(rule)) = (formula.take(), &mut rule) {
                        rule.formulas.push(text);
                    }
                }
                b"cfRule" => {
                    if let (Some(rule), Some(block)) = (rule.take(), &mut block) {
                        block.rules.push(rule);
                    }
                }
                b"conditionalFormatting" => {
                    if let Some(block) = block.take()
                        && !block.rules.is_empty()
                        && !block.ranges.is_empty()
                    {
                        out.push(block);
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(out)
}

/// `"A1:B5 D:D 3:3 F7"`; malformed parts are dropped.
fn parse_sqref(sqref: &str) -> Vec<CellRange> {
    sqref
        .split_whitespace()
        .filter_map(|part| {
            let (first, second) = part.split_once(':').unwrap_or((part, part));
            let (r1, c1) = parse_ref_part(first)?;
            let (r2, c2) = parse_ref_part(second)?;
            Some(CellRange {
                start_row: r1.unwrap_or(0),
                start_col: c1.unwrap_or(0),
                end_row: r2.unwrap_or(u32::MAX),
                end_col: c2.unwrap_or(u32::MAX),
            })
        })
        .collect()
}

/// (row, col) of a reference; either half may be absent (`B` or `3`), not both.
fn parse_ref_part(part: &str) -> Option<(Option<u32>, Option<u32>)> {
    let part = part.replace('$', "");
    let letters: String = part
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect();
    let digits = &part[letters.len()..];
    if part.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let col = if letters.is_empty() {
        None
    } else {
        Some(Anchor::parse_cell_ref(&format!("{letters}1"))?.1)
    };
    let row = if digits.is_empty() {
        None
    } else {
        Some(digits.parse::<u32>().ok()?.checked_sub(1)?)
    };
    Some((row, col))
}

fn is_true(value: &str) -> bool {
    value == "1" || value == "true"
}

/// A cell's value as the rules see it.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Empty,
    Number(f64),
    Text(String),
    Bool(bool),
    Error,
}

/// What matched rules apply to a cell; colors are sRGB.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Flagged {
    pub fill: Option<(u8, u8, u8)>,
    pub font: Option<(u8, u8, u8)>,
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
}

impl Flagged {
    fn is_plain(&self) -> bool {
        *self == Self::default()
    }

    /// Earlier rules keep what they set.
    fn absorb(&mut self, dxf: &Dxf) {
        self.fill = self.fill.or(dxf.fill);
        self.font = self.font.or(dxf.font);
        self.bold |= dxf.bold;
        self.italic |= dxf.italic;
        self.strike |= dxf.strike;
    }
}

/// Cells the rules flag within `rows` × `cols`. Rules run in `priority` order, the first to set
/// a property wins, `stopIfTrue` ends a cell's walk; a rule docrev cannot evaluate flags nothing.
pub fn evaluate(
    formats: &[CondFormat],
    dxfs: &[Dxf],
    rows: u32,
    cols: u32,
    value_at: &dyn Fn(u32, u32) -> Value,
) -> HashMap<(u32, u32), Flagged> {
    let mut prepared: Vec<Prepared> = Vec::new();
    for format in formats {
        let ranges: Vec<CellRange> = format
            .ranges
            .iter()
            .filter_map(|r| r.clamp(rows, cols))
            .collect();
        if ranges.is_empty() {
            continue;
        }
        // formulas are written relative to the first range's top-left cell
        let origin = (format.ranges[0].start_row, format.ranges[0].start_col);
        for rule in &format.rules {
            let Some(matcher) = Matcher::prepare(rule, &ranges, value_at) else {
                continue;
            };
            // a rule without a format still counts: "no format, stop if true" guards later rules
            let dxf = rule
                .dxf
                .and_then(|i| dxfs.get(i))
                .copied()
                .unwrap_or_default();
            prepared.push(Prepared {
                priority: rule.priority,
                stop: rule.stop_if_true,
                dxf,
                origin,
                ranges: ranges.clone(),
                matcher,
            });
        }
    }
    prepared.sort_by_key(|p| p.priority);

    let mut out = HashMap::new();
    let mut visited = HashSet::new();
    for candidate in &prepared {
        for cell in candidate.ranges.iter().flat_map(CellRange::cells) {
            if !visited.insert(cell) {
                continue;
            }
            let mut flagged = Flagged::default();
            for rule in &prepared {
                if !rule.ranges.iter().any(|r| r.contains(cell.0, cell.1)) {
                    continue;
                }
                if !rule.matcher.matches(cell, rule.origin, value_at) {
                    continue;
                }
                flagged.absorb(&rule.dxf);
                if rule.stop {
                    break;
                }
            }
            if !flagged.is_plain() {
                out.insert(cell, flagged);
            }
        }
    }
    out
}

struct Prepared {
    priority: i64,
    stop: bool,
    dxf: Dxf,
    origin: (u32, u32),
    ranges: Vec<CellRange>,
    matcher: Matcher,
}

enum Matcher {
    CellIs {
        op: Op,
        operands: Vec<Operand>,
    },
    Text {
        mode: TextMode,
        needle: String,
    },
    Blanks(bool),
    Errors(bool),
    /// Cells a range-wide rule (duplicates, unique, top10) picked out.
    Set(HashSet<(u32, u32)>),
}

#[derive(Clone, Copy)]
enum Op {
    LessThan,
    LessThanOrEqual,
    Equal,
    NotEqual,
    GreaterThanOrEqual,
    GreaterThan,
    Between,
    NotBetween,
}

#[derive(Clone, Copy)]
enum TextMode {
    Contains,
    NotContains,
    BeginsWith,
    EndsWith,
}

impl Matcher {
    /// `None` for a rule docrev does not evaluate.
    fn prepare(
        rule: &CondRule,
        ranges: &[CellRange],
        value_at: &dyn Fn(u32, u32) -> Value,
    ) -> Option<Matcher> {
        match rule.kind.as_str() {
            "cellIs" => {
                let op = match rule.operator.as_deref()? {
                    "lessThan" => Op::LessThan,
                    "lessThanOrEqual" => Op::LessThanOrEqual,
                    "equal" => Op::Equal,
                    "notEqual" => Op::NotEqual,
                    "greaterThanOrEqual" => Op::GreaterThanOrEqual,
                    "greaterThan" => Op::GreaterThan,
                    "between" => Op::Between,
                    "notBetween" => Op::NotBetween,
                    _ => return None,
                };
                let needed = if matches!(op, Op::Between | Op::NotBetween) {
                    2
                } else {
                    1
                };
                let operands: Vec<Operand> = rule
                    .formulas
                    .iter()
                    .take(needed)
                    .map(|f| Operand::parse(f))
                    .collect::<Option<_>>()?;
                (operands.len() == needed).then_some(Matcher::CellIs { op, operands })
            }
            "containsText" | "notContainsText" | "beginsWith" | "endsWith" => {
                let mode = match rule.kind.as_str() {
                    "containsText" => TextMode::Contains,
                    "notContainsText" => TextMode::NotContains,
                    "beginsWith" => TextMode::BeginsWith,
                    _ => TextMode::EndsWith,
                };
                Some(Matcher::Text {
                    mode,
                    needle: literal_needle(rule.text.as_deref()?)?.to_lowercase(),
                })
            }
            "containsBlanks" => Some(Matcher::Blanks(true)),
            "notContainsBlanks" => Some(Matcher::Blanks(false)),
            "containsErrors" => Some(Matcher::Errors(true)),
            "notContainsErrors" => Some(Matcher::Errors(false)),
            "duplicateValues" | "uniqueValues" => {
                let mut counts: HashMap<String, u32> = HashMap::new();
                let keyed: Vec<((u32, u32), String)> = ranges
                    .iter()
                    .flat_map(CellRange::cells)
                    .filter_map(|cell| Some((cell, value_key(&value_at(cell.0, cell.1))?)))
                    .collect();
                for (_, key) in &keyed {
                    *counts.entry(key.clone()).or_default() += 1;
                }
                let want_duplicates = rule.kind == "duplicateValues";
                Some(Matcher::Set(
                    keyed
                        .into_iter()
                        .filter(|(_, key)| (counts[key] > 1) == want_duplicates)
                        .map(|(cell, _)| cell)
                        .collect(),
                ))
            }
            "top10" => {
                let mut numbers: Vec<((u32, u32), f64)> = ranges
                    .iter()
                    .flat_map(CellRange::cells)
                    .filter_map(|cell| match value_at(cell.0, cell.1) {
                        Value::Number(n) if n.is_finite() => Some((cell, n)),
                        _ => None,
                    })
                    .collect();
                if numbers.is_empty() {
                    return Some(Matcher::Set(HashSet::new()));
                }
                let count = numbers.len();
                let take = if rule.percent {
                    (count as f64 * f64::from(rule.rank) / 100.0).ceil() as usize
                } else {
                    rule.rank as usize
                }
                .clamp(1, count);
                numbers.sort_by(|a, b| a.1.total_cmp(&b.1));
                if !rule.bottom {
                    numbers.reverse();
                }
                // ties with the last taken value are included, as in Excel
                let threshold = numbers[take - 1].1;
                let picked = numbers
                    .into_iter()
                    .filter(|(_, n)| {
                        if rule.bottom {
                            *n <= threshold
                        } else {
                            *n >= threshold
                        }
                    })
                    .map(|(cell, _)| cell)
                    .collect();
                Some(Matcher::Set(picked))
            }
            _ => None,
        }
    }

    fn matches(
        &self,
        cell: (u32, u32),
        origin: (u32, u32),
        value_at: &dyn Fn(u32, u32) -> Value,
    ) -> bool {
        match self {
            Matcher::CellIs { op, operands } => {
                let value = value_at(cell.0, cell.1);
                let resolved: Vec<Value> = operands
                    .iter()
                    .map(|o| o.resolve(origin, cell, value_at))
                    .collect();
                compare_rule(*op, &value, &resolved)
            }
            Matcher::Text { mode, needle } => {
                let hay = text_of(&value_at(cell.0, cell.1)).to_lowercase();
                match mode {
                    TextMode::Contains => hay.contains(needle.as_str()),
                    TextMode::NotContains => !hay.contains(needle.as_str()),
                    TextMode::BeginsWith => hay.starts_with(needle.as_str()),
                    TextMode::EndsWith => hay.ends_with(needle.as_str()),
                }
            }
            Matcher::Blanks(want) => {
                let blank = match value_at(cell.0, cell.1) {
                    Value::Empty => true,
                    Value::Text(s) => s.trim().is_empty(),
                    _ => false,
                };
                blank == *want
            }
            Matcher::Errors(want) => matches!(value_at(cell.0, cell.1), Value::Error) == *want,
            Matcher::Set(cells) => cells.contains(&cell),
        }
    }
}

enum Operand {
    Literal(Value),
    /// 0-based; a non-absolute half moves with the cell.
    Ref {
        row: u32,
        col: u32,
        abs_row: bool,
        abs_col: bool,
    },
}

impl Operand {
    /// A number, a quoted string, TRUE/FALSE, or one cell reference; anything else is `None`.
    fn parse(formula: &str) -> Option<Operand> {
        let f = formula.trim();
        if f.len() >= 2 && f.starts_with('"') && f.ends_with('"') {
            return Some(Operand::Literal(Value::Text(
                f[1..f.len() - 1].replace("\"\"", "\""),
            )));
        }
        if f.eq_ignore_ascii_case("TRUE") {
            return Some(Operand::Literal(Value::Bool(true)));
        }
        if f.eq_ignore_ascii_case("FALSE") {
            return Some(Operand::Literal(Value::Bool(false)));
        }
        if let Ok(n) = f.parse::<f64>()
            && n.is_finite()
        {
            return Some(Operand::Literal(Value::Number(n)));
        }
        let (abs_col, rest) = match f.strip_prefix('$') {
            Some(rest) => (true, rest),
            None => (false, f),
        };
        let letters: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphabetic())
            .collect();
        let rest = &rest[letters.len()..];
        let (abs_row, digits) = match rest.strip_prefix('$') {
            Some(rest) => (true, rest),
            None => (false, rest),
        };
        if letters.is_empty() || digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let (row, col) = Anchor::parse_cell_ref(&format!("{letters}{digits}"))?;
        Some(Operand::Ref {
            row,
            col,
            abs_row,
            abs_col,
        })
    }

    fn resolve(
        &self,
        origin: (u32, u32),
        cell: (u32, u32),
        value_at: &dyn Fn(u32, u32) -> Value,
    ) -> Value {
        match self {
            Operand::Literal(value) => value.clone(),
            Operand::Ref {
                row,
                col,
                abs_row,
                abs_col,
            } => {
                let shift = |base: u32, from: u32, to: u32, absolute: bool| -> Option<u32> {
                    if absolute {
                        return Some(base);
                    }
                    let moved = i64::from(base) + i64::from(to) - i64::from(from);
                    u32::try_from(moved).ok()
                };
                match (
                    shift(*row, origin.0, cell.0, *abs_row),
                    shift(*col, origin.1, cell.1, *abs_col),
                ) {
                    (Some(r), Some(c)) => value_at(r, c),
                    _ => Value::Empty,
                }
            }
        }
    }
}

fn compare_rule(op: Op, value: &Value, operands: &[Value]) -> bool {
    use std::cmp::Ordering::*;
    let Some(first) = operands.first() else {
        return false;
    };
    match op {
        Op::Between | Op::NotBetween => {
            let Some(second) = operands.get(1) else {
                return false;
            };
            let (lo, hi) = match compare(first, second) {
                Some(Greater) => (second, first),
                _ => (first, second),
            };
            let (Some(from_lo), Some(from_hi)) = (compare(value, lo), compare(value, hi)) else {
                return false;
            };
            let inside = from_lo != Less && from_hi != Greater;
            inside == matches!(op, Op::Between)
        }
        _ => {
            let Some(order) = compare(value, first) else {
                return false;
            };
            match op {
                Op::LessThan => order == Less,
                Op::LessThanOrEqual => order != Greater,
                Op::Equal => order == Equal,
                Op::NotEqual => order != Equal,
                Op::GreaterThanOrEqual => order != Less,
                Op::GreaterThan => order == Greater,
                Op::Between | Op::NotBetween => false,
            }
        }
    }
}

/// Excel's ordering: numbers below text below booleans, text case-insensitive; a blank stands in
/// as 0, "" or FALSE; errors compare with nothing.
fn compare(a: &Value, b: &Value) -> Option<std::cmp::Ordering> {
    use Value::*;
    let coerce = |v: &Value, like: &Value| -> Value {
        match (v, like) {
            (Empty, Number(_)) => Number(0.0),
            (Empty, Text(_)) => Text(String::new()),
            (Empty, Bool(_)) => Bool(false),
            _ => v.clone(),
        }
    };
    let (a, b) = (coerce(a, b), coerce(b, a));
    let rank = |v: &Value| match v {
        Number(_) => 0,
        Text(_) => 1,
        Bool(_) => 2,
        Empty => 3,
        Error => 4,
    };
    match (&a, &b) {
        (Error, _) | (_, Error) => None,
        (Number(x), Number(y)) => x.partial_cmp(y),
        (Text(x), Text(y)) => Some(x.to_lowercase().cmp(&y.to_lowercase())),
        (Bool(x), Bool(y)) => Some(x.cmp(y)),
        (Empty, Empty) => Some(std::cmp::Ordering::Equal),
        _ => Some(rank(&a).cmp(&rank(&b))),
    }
}

/// Excel searches with wildcards: `~` escapes the next char, and a bare `*` or `?` makes the rule
/// one docrev does not evaluate.
fn literal_needle(text: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '~' => out.push(chars.next()?),
            '*' | '?' => return None,
            _ => out.push(ch),
        }
    }
    Some(out)
}

/// What a text rule searches: the value as Excel would display it, unformatted.
fn text_of(value: &Value) -> String {
    match value {
        Value::Text(s) => s.clone(),
        Value::Number(n) => {
            if n.fract() == 0.0 && n.abs() < 1e15 {
                format!("{}", *n as i64)
            } else {
                n.to_string()
            }
        }
        Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        Value::Empty | Value::Error => String::new(),
    }
}

/// Duplicate detection key; blanks and errors never count.
fn value_key(value: &Value) -> Option<String> {
    match value {
        Value::Number(n) => Some(format!("n:{n}")),
        Value::Text(s) if s.trim().is_empty() => None,
        Value::Text(s) => Some(format!("t:{}", s.to_lowercase())),
        Value::Bool(b) => Some(format!("b:{b}")),
        Value::Empty | Value::Error => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(sr: u32, sc: u32, er: u32, ec: u32) -> CellRange {
        CellRange {
            start_row: sr,
            start_col: sc,
            end_row: er,
            end_col: ec,
        }
    }

    #[test]
    fn blocks_carry_ranges_rules_and_formulas() {
        let xml = r#"<worksheet><sheetData/>
        <conditionalFormatting sqref="B2:B10 D:D 3:3 F7">
            <cfRule type="cellIs" dxfId="0" priority="2" operator="between" stopIfTrue="1">
                <formula>1</formula><formula>&quot;x&quot;</formula>
            </cfRule>
            <cfRule type="top10" dxfId="1" priority="1" rank="3" percent="1" bottom="true"/>
            <cfRule type="containsText" dxfId="2" priority="3" operator="containsText" text="NG"/>
        </conditionalFormatting>
        <conditionalFormatting sqref="A1"/>
        <conditionalFormatting sqref="A1: 1A"><cfRule type="cellIs" dxfId="0" priority="9"/></conditionalFormatting>
        </worksheet>"#;
        let blocks = parse_conditional_formats(xml).unwrap();
        assert_eq!(blocks.len(), 1, "empty and malformed blocks are dropped");
        assert_eq!(
            blocks[0].ranges,
            vec![
                range(1, 1, 9, 1),
                range(0, 3, u32::MAX, 3),
                range(2, 0, 2, u32::MAX),
                range(6, 5, 6, 5),
            ]
        );
        let rules = &blocks[0].rules;
        assert_eq!(rules.len(), 3);
        assert_eq!(rules[0].kind, "cellIs");
        assert_eq!(rules[0].operator.as_deref(), Some("between"));
        assert_eq!(rules[0].formulas, vec!["1", "\"x\""]);
        assert!(rules[0].stop_if_true);
        assert_eq!((rules[0].dxf, rules[0].priority), (Some(0), 2));
        assert_eq!(
            (rules[1].rank, rules[1].percent, rules[1].bottom),
            (3, true, true)
        );
        assert_eq!(rules[2].text.as_deref(), Some("NG"));
    }

    fn dxf(fill: Option<(u8, u8, u8)>) -> Dxf {
        Dxf {
            fill,
            ..Dxf::default()
        }
    }

    fn rule(kind: &str, dxf: usize, priority: i64) -> CondRule {
        CondRule {
            kind: kind.into(),
            dxf: Some(dxf),
            priority,
            ..CondRule::default()
        }
    }

    const RED: (u8, u8, u8) = (255, 0, 0);
    const BLUE: (u8, u8, u8) = (0, 0, 255);

    /// A 4x2 grid: column 0 = numbers 50, 500, 80, 120; column 1 = text "OK", "NG", "", "ok".
    fn grid(r: u32, c: u32) -> Value {
        match (r, c) {
            (0, 0) => Value::Number(50.0),
            (1, 0) => Value::Number(500.0),
            (2, 0) => Value::Number(80.0),
            (3, 0) => Value::Number(120.0),
            (0, 1) => Value::Text("OK".into()),
            (1, 1) => Value::Text("NG".into()),
            (2, 1) => Value::Text("".into()),
            (3, 1) => Value::Text("ok".into()),
            _ => Value::Empty,
        }
    }

    fn flagged_cells(formats: &[CondFormat], dxfs: &[Dxf]) -> Vec<((u32, u32), Flagged)> {
        let mut out: Vec<_> = evaluate(formats, dxfs, 4, 2, &grid).into_iter().collect();
        out.sort_by_key(|(cell, _)| *cell);
        out
    }

    #[test]
    fn cell_is_compares_literals_and_shifts_relative_references() {
        let mut gt = rule("cellIs", 0, 1);
        gt.operator = Some("greaterThan".into());
        gt.formulas = vec!["100".into()];
        let mut between = rule("cellIs", 1, 2);
        between.operator = Some("between".into());
        between.formulas = vec!["120".into(), "60".into()];
        let formats = vec![CondFormat {
            ranges: vec![range(0, 0, 3, 0)],
            rules: vec![gt, between],
        }];
        let dxfs = vec![dxf(Some(RED)), dxf(Some(BLUE))];
        let cells: Vec<_> = flagged_cells(&formats, &dxfs)
            .into_iter()
            .map(|(c, f)| (c, f.fill))
            .collect();
        assert_eq!(
            cells,
            vec![
                ((1, 0), Some(RED)),
                ((2, 0), Some(BLUE)),
                ((3, 0), Some(RED))
            ],
            "120 breaches both; the lower priority number wins"
        );

        // "text in column 1 equals the number in column 0 of the same row": never, but the
        // reference must shift per row without panicking
        let mut eq = rule("cellIs", 0, 1);
        eq.operator = Some("equal".into());
        eq.formulas = vec!["A1".into()];
        let mut same = rule("cellIs", 1, 2);
        same.operator = Some("equal".into());
        same.formulas = vec!["$B$2".into()];
        let formats = vec![CondFormat {
            ranges: vec![range(0, 1, 3, 1)],
            rules: vec![eq, same],
        }];
        let cells: Vec<_> = flagged_cells(&formats, &dxfs)
            .into_iter()
            .map(|(c, _)| c)
            .collect();
        assert_eq!(cells, vec![(1, 1)], "only NG equals the absolute B2");
    }

    #[test]
    fn text_blank_duplicate_and_top_rules() {
        let mut contains = rule("containsText", 0, 1);
        contains.text = Some("Ng".into());
        let blanks = rule("containsBlanks", 1, 2);
        let text_block = CondFormat {
            ranges: vec![range(0, 1, 3, 1)],
            rules: vec![contains, blanks],
        };
        let mut top = rule("top10", 0, 3);
        top.rank = 2;
        let dup = rule("duplicateValues", 1, 4);
        let number_block = CondFormat {
            ranges: vec![range(0, 0, 3, 0)],
            rules: vec![top, dup],
        };
        let dxfs = vec![dxf(Some(RED)), dxf(Some(BLUE))];
        let cells: Vec<_> = flagged_cells(&[text_block, number_block], &dxfs)
            .into_iter()
            .map(|(c, f)| (c, f.fill))
            .collect();
        assert_eq!(
            cells,
            vec![
                ((1, 0), Some(RED)),
                ((1, 1), Some(RED)),
                ((2, 1), Some(BLUE)),
                ((3, 0), Some(RED)),
            ],
            "top 2 of the numbers, the NG cell, the blank; no duplicates among the numbers"
        );

        let unique = rule("uniqueValues", 0, 2);
        let mut bottom = rule("top10", 1, 1);
        bottom.rank = 50;
        bottom.percent = true;
        bottom.bottom = true;
        let formats = vec![CondFormat {
            ranges: vec![range(0, 1, 3, 1), range(0, 0, 3, 0)],
            rules: vec![unique, bottom],
        }];
        let cells: Vec<_> = flagged_cells(&formats, &dxfs)
            .into_iter()
            .map(|(c, f)| (c, f.fill))
            .collect();
        assert_eq!(
            cells,
            vec![
                ((0, 0), Some(BLUE)),
                ((1, 0), Some(RED)),
                ((1, 1), Some(RED)),
                ((2, 0), Some(BLUE)),
                ((3, 0), Some(RED)),
            ],
            "bottom half of the numbers first; OK/ok are duplicates and the blank is no value"
        );
    }

    #[test]
    fn stop_if_true_priority_and_unevaluable_rules() {
        let mut first = rule("containsBlanks", 0, 1);
        first.stop_if_true = true;
        let mut second = rule("notContainsBlanks", 1, 2);
        second.kind = "notContainsBlanks".into();
        let mut expression = rule("expression", 0, 0);
        expression.formulas = vec!["MOD(ROW(),2)=0".into()];
        let mut bad_ref = rule("cellIs", 0, 0);
        bad_ref.operator = Some("greaterThan".into());
        bad_ref.formulas = vec!["SUM(A1:A3)".into()];
        let no_dxf = rule("containsBlanks", 7, 0);
        let dxfs = vec![
            Dxf {
                fill: Some(RED),
                bold: true,
                ..Dxf::default()
            },
            Dxf {
                fill: Some(BLUE),
                font: Some(RED),
                ..Dxf::default()
            },
        ];
        let formats = vec![CondFormat {
            ranges: vec![range(0, 1, 3, 1)],
            rules: vec![first, second, expression, bad_ref, no_dxf],
        }];
        let cells = flagged_cells(&formats, &dxfs);
        assert_eq!(cells.len(), 4);
        assert_eq!(
            cells[2],
            (
                (2, 1),
                Flagged {
                    fill: Some(RED),
                    bold: true,
                    ..Flagged::default()
                }
            ),
            "the blank matches the stopping rule and nothing after it"
        );
        assert_eq!(
            cells[0].1,
            Flagged {
                fill: Some(BLUE),
                font: Some(RED),
                ..Flagged::default()
            }
        );
    }

    #[test]
    fn a_format_less_stopping_rule_guards_the_rules_after_it() {
        let mut guard = rule("containsBlanks", 0, 0);
        guard.dxf = None;
        guard.stop_if_true = true;
        let mut less = rule("cellIs", 0, 1);
        less.operator = Some("lessThan".into());
        less.formulas = vec!["50".into()];
        let formats = vec![CondFormat {
            ranges: vec![range(0, 1, 3, 1), range(0, 0, 3, 0)],
            rules: vec![guard, less],
        }];
        let cells: Vec<_> = flagged_cells(&formats, &[dxf(Some(RED))])
            .into_iter()
            .map(|(c, _)| c)
            .collect();
        assert_eq!(
            cells,
            vec![],
            "the blank would read as 0 < 50 without the guard; 50 is not < 50"
        );
    }

    #[test]
    fn between_needs_both_bounds_comparable() {
        let n = |v: f64| Value::Number(v);
        assert!(!compare_rule(
            Op::NotBetween,
            &n(5.0),
            &[n(1.0), Value::Error]
        ));
        assert!(!compare_rule(
            Op::NotBetween,
            &n(5.0),
            &[Value::Error, n(1.0)]
        ));
        assert!(!compare_rule(Op::Between, &n(5.0), &[n(1.0), Value::Error]));
        assert!(
            compare_rule(Op::Between, &n(5.0), &[n(10.0), n(1.0)]),
            "bounds in either order"
        );
        assert!(
            compare_rule(Op::NotBetween, &Value::Empty, &[n(1.0), n(2.0)]),
            "blank is 0"
        );
    }

    #[test]
    fn text_rules_with_wildcards_are_not_evaluated() {
        assert_eq!(literal_needle("a~*b~~c"), Some("a*b~c".into()));
        assert_eq!(literal_needle("a*b"), None);
        assert_eq!(literal_needle("a?"), None);
        assert_eq!(
            literal_needle("trailing~"),
            None,
            "a dangling escape is malformed"
        );
        let mut contains = rule("containsText", 0, 1);
        contains.text = Some("N*".into());
        let formats = vec![CondFormat {
            ranges: vec![range(0, 1, 3, 1)],
            rules: vec![contains],
        }];
        assert!(flagged_cells(&formats, &[dxf(Some(RED))]).is_empty());
    }

    #[test]
    fn excel_ordering_and_blank_coercion() {
        use std::cmp::Ordering::*;
        let n = |v: f64| Value::Number(v);
        let t = |s: &str| Value::Text(s.into());
        assert_eq!(compare(&n(1.0), &n(2.0)), Some(Less));
        assert_eq!(compare(&t("abc"), &t("ABC")), Some(Equal));
        assert_eq!(
            compare(&t("a"), &n(1e9)),
            Some(Greater),
            "text sorts above numbers"
        );
        assert_eq!(compare(&Value::Bool(false), &t("zzz")), Some(Greater));
        assert_eq!(compare(&Value::Empty, &n(0.0)), Some(Equal), "blank is 0");
        assert_eq!(
            compare(&Value::Empty, &t("")),
            Some(Equal),
            "blank is empty text"
        );
        assert_eq!(compare(&Value::Error, &n(1.0)), None);
        assert!(compare_rule(Op::LessThan, &Value::Empty, &[n(1.0)]));
        assert!(!compare_rule(Op::NotEqual, &Value::Error, &[n(1.0)]));
        assert!(compare_rule(Op::NotBetween, &n(5.0), &[n(10.0), n(20.0)]));
    }

    #[test]
    fn operands_accept_literals_and_single_references_only() {
        assert!(
            matches!(Operand::parse(" 12.5 "), Some(Operand::Literal(Value::Number(v))) if v == 12.5)
        );
        assert!(
            matches!(Operand::parse("\"a\"\"b\""), Some(Operand::Literal(Value::Text(s))) if s == "a\"b")
        );
        assert!(matches!(
            Operand::parse("true"),
            Some(Operand::Literal(Value::Bool(true)))
        ));
        assert!(matches!(
            Operand::parse("$C2"),
            Some(Operand::Ref {
                row: 1,
                col: 2,
                abs_row: false,
                abs_col: true
            })
        ));
        assert!(Operand::parse("C2+1").is_none());
        assert!(Operand::parse("SUM(A1)").is_none());
        assert!(Operand::parse("").is_none());
    }

    #[test]
    fn ranges_clamp_to_the_used_range() {
        let whole_column = rule("containsBlanks", 0, 1);
        let formats = vec![CondFormat {
            ranges: vec![range(0, 5, u32::MAX, 5), range(0, 0, u32::MAX, 0)],
            rules: vec![whole_column],
        }];
        let flagged = evaluate(&formats, &[dxf(Some(RED))], 2, 1, &|_, _| Value::Empty);
        let mut cells: Vec<_> = flagged.keys().copied().collect();
        cells.sort_unstable();
        assert_eq!(
            cells,
            vec![(0, 0), (1, 0)],
            "column 5 is outside the used range"
        );
        assert!(evaluate(&formats, &[dxf(Some(RED))], 0, 0, &|_, _| Value::Empty).is_empty());
    }
}
