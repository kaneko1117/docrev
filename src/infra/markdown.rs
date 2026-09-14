//! Line-preserving Markdown: one source line always yields one styled line.

use crate::domain::text_document::{Face, Run};

/// What a line is, decided by looking at the lines around it; a table line carries its table's id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Block {
    Normal,
    /// Inside a ``` fence, the fence lines included.
    Code,
    /// Inside the leading `---` block, the delimiters included.
    FrontMatter,
    /// The row above a table's separator.
    TableHeader(usize),
    TableRow(usize),
    /// The `|---|---|` under the header.
    TableRule(usize),
}

/// Every line's block, plus the rendered column widths of each table found.
pub(crate) struct Blocks {
    kinds: Vec<Block>,
    tables: Vec<Vec<usize>>,
}

impl Blocks {
    pub(crate) fn kind(&self, line: usize) -> Block {
        self.kinds.get(line).copied().unwrap_or(Block::Normal)
    }

    /// Empty unless the line belongs to a table.
    fn columns(&self, line: usize) -> &[usize] {
        match self.kind(line) {
            Block::TableHeader(id) | Block::TableRow(id) | Block::TableRule(id) => {
                self.tables.get(id).map_or(&[][..], Vec::as_slice)
            }
            _ => &[],
        }
    }

    pub(crate) fn render(&self, text: &str, line: usize) -> Vec<Run> {
        render_line(text, self.kind(line), self.columns(line))
    }
}

/// Every line as it is shown, one entry per line; blocks are read from the source as written and
/// only the shown text has control characters turned into spaces.
pub fn shown_lines(lines: &[String]) -> Vec<Vec<Run>> {
    let blocks = blocks(lines.iter().map(String::as_str));
    lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let clean: String = line
                .chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect();
            blocks.render(&clean, i)
        })
        .collect()
}

/// One entry per line; blocks are decided in order, so a fence wins over a table.
pub(crate) fn blocks<'a>(lines: impl Iterator<Item = &'a str>) -> Blocks {
    let lines: Vec<&str> = lines.collect();
    let mut blocks = vec![Block::Normal; lines.len()];
    let mut tables: Vec<Vec<usize>> = Vec::new();
    let mut start = 0;
    // front matter only counts when the file opens with it and closes it
    let closing = lines
        .first()
        .filter(|line| line.trim_end() == "---")
        .and_then(|_| lines[1..].iter().position(|line| line.trim_end() == "---"));
    if let Some(close) = closing {
        let end = close + 1;
        for block in &mut blocks[..=end] {
            *block = Block::FrontMatter;
        }
        start = end + 1;
    }
    let mut fenced = false;
    for i in start..lines.len() {
        if lines[i].trim_start().starts_with("```") {
            fenced = !fenced;
            blocks[i] = Block::Code;
        } else if fenced {
            blocks[i] = Block::Code;
        }
    }
    for i in 0..lines.len() {
        if blocks[i] != Block::Normal || !is_table_rule(lines[i]) {
            continue;
        }
        let Some(header) = i.checked_sub(1) else {
            continue;
        };
        if blocks[header] != Block::Normal || !lines[header].contains('|') {
            continue;
        }
        let id = tables.len();
        blocks[header] = Block::TableHeader(id);
        blocks[i] = Block::TableRule(id);
        let mut rows = vec![lines[header]];
        for (j, line) in lines.iter().enumerate().skip(i + 1) {
            if blocks[j] != Block::Normal || !line.contains('|') {
                break;
            }
            blocks[j] = Block::TableRow(id);
            rows.push(line);
        }
        tables.push(column_widths(&rows));
    }
    Blocks {
        kinds: blocks,
        tables,
    }
}

/// The widest rendered cell of each column; the separator row is not measured.
fn column_widths(rows: &[&str]) -> Vec<usize> {
    let mut widths: Vec<usize> = Vec::new();
    for row in rows {
        for (i, cell) in cells(row).enumerate() {
            let width = rendered_width(cell);
            match widths.get_mut(i) {
                Some(current) => *current = (*current).max(width),
                None => widths.push(width),
            }
        }
    }
    widths
}

/// The width a cell takes once its markers are hidden.
fn rendered_width(cell: &str) -> usize {
    inline(cell.trim(), Face::Plain)
        .iter()
        .map(|(text, _)| unicode_width::UnicodeWidthStr::width(text.as_str()))
        .sum()
}

/// The cells of a row, outer pipes aside; the text is not trimmed.
fn cells(row: &str) -> impl Iterator<Item = &str> {
    let trimmed = row.trim();
    let trimmed = trimmed.strip_prefix('|').unwrap_or(trimmed);
    let trimmed = trimmed.strip_suffix('|').unwrap_or(trimmed);
    trimmed.split('|')
}

/// `columns` are the table's rendered column widths, empty outside a table.
pub(crate) fn render_line(line: &str, block: Block, columns: &[usize]) -> Vec<Run> {
    match block {
        Block::Code => return vec![(line.to_string(), Face::CodeBlock)],
        Block::FrontMatter => return vec![(line.to_string(), Face::FrontMatter)],
        Block::TableRule(_) => return vec![(table_rule(columns), Face::TableEdge)],
        Block::TableHeader(_) => return table_row(line, columns, Face::Bold),
        Block::TableRow(_) => return table_row(line, columns, Face::Plain),
        Block::Normal => {}
    }
    let (indent, rest) = split_indent(line);
    // the pane fills the row, so a rule drops its indent
    if is_rule(rest) {
        return vec![(rest.to_string(), Face::Rule)];
    }
    let mut runs: Vec<Run> = Vec::new();
    if !indent.is_empty() {
        runs.push((indent.to_string(), Face::Plain));
    }
    if let Some((level, text)) = heading(rest) {
        runs.extend(inline(text, Face::Heading(level)));
        return runs;
    }
    if let Some(text) = rest.strip_prefix('>') {
        runs.push((">".to_string(), Face::Quote));
        runs.extend(inline(text, Face::Plain));
        return runs;
    }
    if let Some(text) = list_item(rest) {
        let (marker, text) = match task(text) {
            Some((true, rest)) => ("☑ ", rest),
            Some((false, rest)) => ("☐ ", rest),
            None => ("• ", text),
        };
        runs.push((marker.to_string(), Face::ListMarker));
        runs.extend(inline(text, Face::Plain));
        return runs;
    }
    runs.extend(inline(rest, Face::Plain));
    runs
}

fn split_indent(line: &str) -> (&str, &str) {
    let end = line.len() - line.trim_start().len();
    line.split_at(end)
}

/// Three or more of the same `-`, `*` or `_`, spaces aside.
fn is_rule(text: &str) -> bool {
    let marks: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
    marks.len() >= 3
        && matches!(marks[0], '-' | '*' | '_')
        && marks.iter().all(|mark| *mark == marks[0])
}

/// A separator of dashes, colons and at least one `|`.
fn is_table_rule(text: &str) -> bool {
    let text = text.trim();
    text.contains('|')
        && text.contains('-')
        && text.chars().all(|c| matches!(c, '-' | ':' | '|' | ' '))
}

/// Built from the column widths, so it lines up with the rows.
fn table_rule(columns: &[usize]) -> String {
    let mut out = String::from("├");
    for (i, width) in columns.iter().enumerate() {
        out.push_str(&"─".repeat(width + 2));
        out.push(if i + 1 == columns.len() { '┤' } else { '┼' });
    }
    out
}

/// Every cell is padded to its column's width, so all rows of a table line up; cells keep their
/// inline markup and `cell` is the face they start from.
fn table_row(line: &str, columns: &[usize], cell: Face) -> Vec<Run> {
    let mut runs = vec![("│".to_string(), Face::TableEdge)];
    let mut contents = cells(line);
    for width in columns {
        let text = contents.next().unwrap_or("");
        let inner = inline(text.trim(), cell);
        let used: usize = inner
            .iter()
            .map(|(text, _)| unicode_width::UnicodeWidthStr::width(text.as_str()))
            .sum();
        runs.push((" ".to_string(), cell));
        runs.extend(inner);
        runs.push((" ".repeat(width.saturating_sub(used) + 1), cell));
        runs.push(("│".to_string(), Face::TableEdge));
    }
    runs
}

/// `"## Title"` -> `(2, "Title")`; a `#` run without a space is not a heading.
fn heading(text: &str) -> Option<(u8, &str)> {
    let hashes = text.chars().take_while(|&c| c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &text[hashes..];
    let body = rest.strip_prefix(' ')?;
    Some((hashes as u8, body.trim_end_matches(['#', ' '])))
}

/// `"- item"`, `"* item"`, `"+ item"`, `"3. item"` -> `"item"`.
fn list_item(text: &str) -> Option<&str> {
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = text.strip_prefix(marker) {
            return Some(rest);
        }
    }
    let digits = text.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    text[digits..].strip_prefix(". ")
}

/// `"[x] done"` -> `(true, "done")`.
fn task(text: &str) -> Option<(bool, &str)> {
    let rest = text.strip_prefix('[')?;
    let mut chars = rest.chars();
    let mark = chars.next()?;
    let rest = chars.as_str().strip_prefix("] ")?;
    match mark {
        ' ' => Some((false, rest)),
        'x' | 'X' => Some((true, rest)),
        _ => None,
    }
}

/// Inline markers toggle `**bold**`, `*italic*` / `_italic_`, `~~strike~~`, `` `code` ``,
/// `[text](url)` and `![alt](url)`; an unmatched marker is kept as text.
fn inline(text: &str, base: Face) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    let mut plain = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let push = |runs: &mut Vec<Run>, plain: &mut String| {
        if !plain.is_empty() {
            runs.push((std::mem::take(plain), base));
        }
    };
    while i < chars.len() {
        let rest = &chars[i..];
        if let Some((body, len)) = delimited(rest, &['`'], &['`']) {
            push(&mut runs, &mut plain);
            runs.push((body, Face::Code));
            i += len;
        } else if let Some((body, len)) = delimited(rest, &['*', '*'], &['*', '*']) {
            push(&mut runs, &mut plain);
            runs.push((body, emphasis(base, Face::Bold)));
            i += len;
        } else if let Some((body, len)) = delimited(rest, &['~', '~'], &['~', '~']) {
            push(&mut runs, &mut plain);
            runs.push((body, emphasis(base, Face::Strike)));
            i += len;
        } else if let Some((body, len)) = delimited(rest, &['*'], &['*'])
            .or_else(|| delimited(rest, &['_'], &['_']).filter(|_| word_boundary(&chars, i)))
        {
            push(&mut runs, &mut plain);
            runs.push((body, emphasis(base, Face::Italic)));
            i += len;
        } else if let Some((label, len)) = image(rest).or_else(|| link(rest)) {
            push(&mut runs, &mut plain);
            runs.push((label, Face::Link));
            i += len;
        } else {
            plain.push(chars[i]);
            i += 1;
        }
    }
    push(&mut runs, &mut plain);
    runs
}

/// `_` opens emphasis only after a non-word character, so `snake_case` stays plain.
fn word_boundary(chars: &[char], i: usize) -> bool {
    i == 0 || !chars[i - 1].is_alphanumeric()
}

/// A heading keeps its face under emphasis so its color survives.
fn emphasis(base: Face, wanted: Face) -> Face {
    match base {
        Face::Heading(_) => base,
        _ => wanted,
    }
}

/// `(body, chars consumed)` when `chars` starts with `open` and `close` follows a non-empty body.
fn delimited(chars: &[char], open: &[char], close: &[char]) -> Option<(String, usize)> {
    if !chars.starts_with(open) {
        return None;
    }
    let body_start = open.len();
    let mut j = body_start;
    while j + close.len() <= chars.len() {
        if chars[j..].starts_with(close) {
            if j == body_start {
                return None;
            }
            return Some((chars[body_start..j].iter().collect(), j + close.len()));
        }
        j += 1;
    }
    None
}

/// `[label](url)` -> `(label, url, chars consumed)`.
fn bracketed(chars: &[char]) -> Option<(String, String, usize)> {
    if chars.first() != Some(&'[') {
        return None;
    }
    let close = chars.iter().position(|&c| c == ']')?;
    if chars.get(close + 1) != Some(&'(') {
        return None;
    }
    let end = chars[close + 2..].iter().position(|&c| c == ')')? + close + 2;
    Some((
        chars[1..close].iter().collect(),
        chars[close + 2..end].iter().collect(),
        end + 1,
    ))
}

/// `[label](url)` -> `(label, chars consumed)`.
fn link(chars: &[char]) -> Option<(String, usize)> {
    let (label, _, len) = bracketed(chars)?;
    Some((label, len))
}

/// `![alt](url)` -> `(alt, chars consumed)`; an empty alt shows `[image: <file name>]` instead.
fn image(chars: &[char]) -> Option<(String, usize)> {
    if chars.first() != Some(&'!') {
        return None;
    }
    let (alt, url, len) = bracketed(&chars[1..])?;
    let label = if alt.trim().is_empty() {
        match file_name(&url) {
            Some(name) => format!("[image: {name}]"),
            None => "[image]".to_string(),
        }
    } else {
        alt
    };
    Some((label, len + 1))
}

/// The last path segment of a link target, its title, query and fragment aside.
fn file_name(url: &str) -> Option<&str> {
    let target = url.split_whitespace().next()?;
    let path = target.split(['?', '#']).next().unwrap_or(target);
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runs(line: &str) -> Vec<(&'static str, Face)> {
        faces(render_line(line, Block::Normal, &[]))
    }

    /// Every line of `text` rendered as one string, the way the pane draws it.
    fn drawn(text: &str) -> Vec<String> {
        let blocks = blocks(text.lines());
        text.lines()
            .enumerate()
            .map(|(i, line)| {
                blocks
                    .render(line, i)
                    .into_iter()
                    .map(|(run, _)| run)
                    .collect()
            })
            .collect()
    }

    fn kinds(text: &str) -> Vec<Block> {
        let blocks = blocks(text.lines());
        (0..text.lines().count()).map(|i| blocks.kind(i)).collect()
    }

    fn faces(runs: Vec<Run>) -> Vec<(&'static str, Face)> {
        runs.into_iter()
            .map(|(text, face)| (Box::leak(text.into_boxed_str()) as &str, face))
            .collect()
    }

    #[test]
    fn shown_lines_render_every_line_with_its_block() {
        let lines: Vec<String> = "# T\n|a|\n|-|\n\tx".lines().map(str::to_string).collect();
        let shown = shown_lines(&lines);
        assert_eq!(shown.len(), 4);
        assert_eq!(shown[0], vec![("T".to_string(), Face::Heading(1))]);
        assert_eq!(shown[2], vec![("├───┤".to_string(), Face::TableEdge)]);
        assert_eq!(
            shown[3],
            vec![
                (" ".to_string(), Face::Plain),
                ("x".to_string(), Face::Plain)
            ],
            "a tab is shown as a space"
        );
    }

    #[test]
    fn blocks_are_read_before_control_characters_are_blanked() {
        let lines = |text: &str| text.lines().map(str::to_string).collect::<Vec<_>>();
        let table = shown_lines(&lines("| a | b |\n|-\t-|---|\n| 1 | 2 |"));
        assert_eq!(
            table[1],
            vec![("|- -|---|".to_string(), Face::Plain)],
            "a tab in the separator row means there is no table"
        );
        let fence = shown_lines(&lines("a\n\0```\n**x**\n\0```"));
        assert_eq!(
            fence[2],
            vec![("x".to_string(), Face::Bold)],
            "a control character before ``` means there is no fence"
        );
        let front = shown_lines(&lines("---\x1b\ntitle: x\n---\n# H"));
        assert_eq!(
            front[1],
            vec![("title: x".to_string(), Face::Plain)],
            "a control character after --- means there is no front matter"
        );
    }

    #[test]
    fn headings_drop_their_hashes_and_keep_their_level() {
        assert_eq!(runs("# Title"), vec![("Title", Face::Heading(1))]);
        assert_eq!(runs("### Sub ##"), vec![("Sub", Face::Heading(3))]);
        assert_eq!(runs("#hashtag"), vec![("#hashtag", Face::Plain)]);
        assert_eq!(runs("####### seven"), vec![("####### seven", Face::Plain)]);
    }

    #[test]
    fn list_markers_become_bullets_and_indent_is_kept() {
        assert_eq!(
            runs("- item"),
            vec![("• ", Face::ListMarker), ("item", Face::Plain)]
        );
        assert_eq!(
            runs("3. third"),
            vec![("• ", Face::ListMarker), ("third", Face::Plain)]
        );
        assert_eq!(
            runs("  * nested"),
            vec![
                ("  ", Face::Plain),
                ("• ", Face::ListMarker),
                ("nested", Face::Plain)
            ]
        );
        assert_eq!(runs("-no space"), vec![("-no space", Face::Plain)]);
    }

    #[test]
    fn task_items_become_boxes() {
        assert_eq!(
            runs("- [ ] todo"),
            vec![("☐ ", Face::ListMarker), ("todo", Face::Plain)]
        );
        assert_eq!(
            runs("- [x] done"),
            vec![("☑ ", Face::ListMarker), ("done", Face::Plain)]
        );
        assert_eq!(
            runs("- [X] done"),
            vec![("☑ ", Face::ListMarker), ("done", Face::Plain)]
        );
        assert_eq!(
            runs("- [y] not a box"),
            vec![("• ", Face::ListMarker), ("[y] not a box", Face::Plain)]
        );
    }

    #[test]
    fn inline_markers_are_hidden_and_unmatched_ones_stay() {
        assert_eq!(
            runs("run `docrev` **now** or *later*"),
            vec![
                ("run ", Face::Plain),
                ("docrev", Face::Code),
                (" ", Face::Plain),
                ("now", Face::Bold),
                (" or ", Face::Plain),
                ("later", Face::Italic),
            ]
        );
        assert_eq!(
            runs("~~gone~~ soon"),
            vec![("gone", Face::Strike), (" soon", Face::Plain)]
        );
        assert_eq!(runs("2 * 3 = 6"), vec![("2 * 3 = 6", Face::Plain)]);
        assert_eq!(runs("a ** b"), vec![("a ** b", Face::Plain)]);
        assert_eq!(
            runs("snake_case_name"),
            vec![("snake_case_name", Face::Plain)]
        );
        assert_eq!(
            runs("say _hi_"),
            vec![("say ", Face::Plain), ("hi", Face::Italic)]
        );
    }

    #[test]
    fn links_and_images_show_their_label_and_quotes_keep_their_mark() {
        assert_eq!(
            runs("see [the docs](https://x.y) now"),
            vec![
                ("see ", Face::Plain),
                ("the docs", Face::Link),
                (" now", Face::Plain)
            ]
        );
        assert_eq!(
            runs("![a diagram](d.png) below"),
            vec![("a diagram", Face::Link), (" below", Face::Plain)]
        );
        assert_eq!(runs("[broken](nope"), vec![("[broken](nope", Face::Plain)]);
        assert_eq!(
            runs("> quoted"),
            vec![(">", Face::Quote), (" quoted", Face::Plain)]
        );
    }

    #[test]
    fn an_image_without_alt_text_shows_its_file_name() {
        assert_eq!(
            runs("![](empty.png)"),
            vec![("[image: empty.png]", Face::Link)]
        );
        assert_eq!(
            runs("see ![ ](https://x.com/a/b.png?v=1#top) here"),
            vec![
                ("see ", Face::Plain),
                ("[image: b.png]", Face::Link),
                (" here", Face::Plain)
            ]
        );
        assert_eq!(
            runs(r#"![](pics/c.png "a title")"#),
            vec![("[image: c.png]", Face::Link)]
        );
        assert_eq!(runs("![]()"), vec![("[image]", Face::Link)]);
        assert_eq!(
            runs("![named](d.png)"),
            vec![("named", Face::Link)],
            "alt text still wins"
        );
    }

    #[test]
    fn thematic_breaks_are_one_rule_run_without_their_indent() {
        for line in ["---", "***", "___", "- - -", "  ----"] {
            let rendered = render_line(line, Block::Normal, &[]);
            assert_eq!(rendered.len(), 1, "{line}");
            assert_eq!(rendered[0].1, Face::Rule, "{line}");
        }
        assert_eq!(runs("--"), vec![("--", Face::Plain)]);
        assert_eq!(
            runs("-*-"),
            vec![("-*-", Face::Plain)],
            "mixed marks are not a rule"
        );
    }

    #[test]
    fn a_heading_keeps_its_face_under_emphasis() {
        assert_eq!(
            runs("# A **big** title"),
            vec![
                ("A ", Face::Heading(1)),
                ("big", Face::Heading(1)),
                (" title", Face::Heading(1))
            ]
        );
    }

    #[test]
    fn fences_and_front_matter_cover_their_lines_and_render_verbatim() {
        let text = "a\n```rust\nlet **x** = 1;\n```\nb\n```\nopen";
        assert_eq!(
            kinds(text),
            vec![
                Block::Normal,
                Block::Code,
                Block::Code,
                Block::Code,
                Block::Normal,
                Block::Code,
                Block::Code
            ]
        );
        assert_eq!(
            render_line("let **x** = 1;", Block::Code, &[]),
            vec![("let **x** = 1;".to_string(), Face::CodeBlock)]
        );

        let front = "---\nname: x\n---\n# Title\n---";
        assert_eq!(
            kinds(front),
            vec![
                Block::FrontMatter,
                Block::FrontMatter,
                Block::FrontMatter,
                Block::Normal,
                Block::Normal
            ],
            "only the leading block counts"
        );
        assert_eq!(
            render_line("name: x", Block::FrontMatter, &[]),
            vec![("name: x".to_string(), Face::FrontMatter)]
        );
    }

    #[test]
    fn an_unclosed_leading_rule_is_not_front_matter() {
        assert_eq!(
            kinds("---\n# Title\n- item"),
            vec![Block::Normal, Block::Normal, Block::Normal]
        );
        assert_eq!(kinds("---"), vec![Block::Normal]);
        assert_eq!(runs("# Title"), vec![("Title", Face::Heading(1))]);
    }

    #[test]
    fn a_table_is_its_header_its_rule_and_its_rows() {
        let text = "before\n| a | b |\n|---|:--|\n| 1 | 2 |\n\nafter";
        assert_eq!(
            kinds(text),
            vec![
                Block::Normal,
                Block::TableHeader(0),
                Block::TableRule(0),
                Block::TableRow(0),
                Block::Normal,
                Block::Normal
            ]
        );
        assert_eq!(
            kinds("| a | b |\nnot a rule"),
            vec![Block::Normal, Block::Normal],
            "a row without a rule under it is not a table"
        );
    }

    #[test]
    fn every_column_is_as_wide_as_its_widest_rendered_cell() {
        let drawn = drawn("|item|qty|\n|-|-|\n|**apple**|3|\n|fig|12|");
        assert_eq!(
            drawn,
            vec![
                "│ item  │ qty │",
                "├───────┼─────┤",
                "│ apple │ 3   │",
                "│ fig   │ 12  │",
            ]
        );
        for line in &drawn {
            assert_eq!(
                unicode_width::UnicodeWidthStr::width(line.as_str()),
                15,
                "{line}"
            );
        }
    }

    #[test]
    fn wide_characters_and_ragged_rows_still_line_up() {
        let drawn = drawn("| 項目 | 数 |\n|---|---|\n| りんご |\n| a | b | c |");
        for line in &drawn {
            assert_eq!(
                unicode_width::UnicodeWidthStr::width(line.as_str()),
                unicode_width::UnicodeWidthStr::width(drawn[0].as_str()),
                "{line}"
            );
        }
        assert!(drawn[2].starts_with("│ りんご"), "{}", drawn[2]);
        assert!(drawn[3].ends_with("c │"), "a ragged row keeps every cell");
    }
}
