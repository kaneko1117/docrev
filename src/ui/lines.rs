use crate::domain::text_document::TextDocument;

/// `cat -n`: a width-6 right-aligned 1-based number, a tab, the line.
pub fn render(text: &TextDocument) -> String {
    let mut out = String::new();
    for (index, line) in text.lines().iter().enumerate() {
        out.push_str(&format!("{:>6}\t{line}\n", index + 1));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_every_line_like_cat_n() {
        let text = TextDocument::new("# title\n\nbody\n");
        assert_eq!(render(&text), "     1\t# title\n     2\t\n     3\tbody\n");
        assert_eq!(render(&TextDocument::new("")), "");
    }
}
