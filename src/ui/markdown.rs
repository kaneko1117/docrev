//! Line-preserving Markdown: one source line always yields one styled line.

/// How a run of text is drawn; markers that produced it are dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Face {
    Plain,
    /// 1 to 6.
    Heading(u8),
    Bold,
    Italic,
    Code,
    /// A whole line inside a fenced block, or the fence itself.
    CodeBlock,
    /// The `•` put in place of `-`, `*`, `+` or `1.`.
    ListMarker,
    Link,
    /// The `>` of a quote, kept but dimmed.
    Quote,
}

pub(crate) type Run = (String, Face);

/// Which lines sit inside ``` fences, fence lines included.
pub(crate) fn fenced_lines<'a>(lines: impl Iterator<Item = &'a str>) -> Vec<bool> {
    let mut inside = false;
    lines
        .map(|line| {
            if line.trim_start().starts_with("```") {
                inside = !inside;
                true
            } else {
                inside
            }
        })
        .collect()
}

/// `fenced` lines are code verbatim; everything else is block markup then inline markup.
pub(crate) fn render_line(line: &str, fenced: bool) -> Vec<Run> {
    if fenced {
        return vec![(line.to_string(), Face::CodeBlock)];
    }
    let (indent, rest) = split_indent(line);
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
        runs.push(("• ".to_string(), Face::ListMarker));
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

/// Inline markers toggle `**bold**`, `*italic*` / `_italic_`, `` `code` `` and `[text](url)`; an
/// unmatched marker is kept as text.
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
        } else if let Some((body, len)) = delimited(rest, &['*'], &['*'])
            .or_else(|| delimited(rest, &['_'], &['_']).filter(|_| word_boundary(&chars, i)))
        {
            push(&mut runs, &mut plain);
            runs.push((body, emphasis(base, Face::Italic)));
            i += len;
        } else if let Some((label, len)) = link(rest) {
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

/// `[label](url)` -> `(label, chars consumed)`.
fn link(chars: &[char]) -> Option<(String, usize)> {
    if chars.first() != Some(&'[') {
        return None;
    }
    let close = chars.iter().position(|&c| c == ']')?;
    if close == 0 || chars.get(close + 1) != Some(&'(') {
        return None;
    }
    let end = chars[close + 2..].iter().position(|&c| c == ')')? + close + 2;
    Some((chars[1..close].iter().collect(), end + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runs(line: &str) -> Vec<(&'static str, Face)> {
        render_line(line, false)
            .into_iter()
            .map(|(text, face)| (Box::leak(text.into_boxed_str()) as &str, face))
            .collect()
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
    fn links_show_their_label_and_quotes_keep_their_mark() {
        assert_eq!(
            runs("see [the docs](https://x.y) now"),
            vec![
                ("see ", Face::Plain),
                ("the docs", Face::Link),
                (" now", Face::Plain)
            ]
        );
        assert_eq!(runs("[broken](nope"), vec![("[broken](nope", Face::Plain)]);
        assert_eq!(
            runs("> quoted"),
            vec![(">", Face::Quote), (" quoted", Face::Plain)]
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
    fn fences_cover_their_lines_and_render_verbatim() {
        let text = "a\n```rust\nlet **x** = 1;\n```\nb\n```\nopen";
        assert_eq!(
            fenced_lines(text.lines()),
            vec![false, true, true, true, false, true, true]
        );
        assert_eq!(
            render_line("let **x** = 1;", true),
            vec![("let **x** = 1;".to_string(), Face::CodeBlock)]
        );
    }
}
