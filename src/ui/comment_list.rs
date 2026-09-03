use crate::domain::comment::CommentThread;

use super::text::sanitize;

/// `hidden` marks threads whose anchor the workbook hides.
pub fn render(threads: &[(&CommentThread, bool)]) -> String {
    if threads.is_empty() {
        return "no comments\n".to_string();
    }
    let mut out = String::new();
    for &(thread, hidden) in threads {
        let mark = if thread.resolved { "✓" } else { "●" };
        let first_line = sanitize(thread.body.lines().next().unwrap_or(""));
        out.push_str(&format!(
            "{mark} {}!{}{} [{}] {first_line}",
            thread.anchor.sheet(),
            thread.anchor.cell_ref(),
            if hidden { " (hidden)" } else { "" },
            thread.author,
        ));
        match thread.replies.len() {
            0 => {}
            1 => out.push_str(" (1 reply)"),
            n => out.push_str(&format!(" ({n} replies)")),
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::anchor::Anchor;
    use crate::domain::comment::Reply;

    #[test]
    fn renders_one_line_per_thread() {
        let threads = [CommentThread {
            id: "t".into(),
            anchor: Anchor::cell("売上", 2, 1),
            author: "user".into(),
            body: "line1\nline2".into(),
            created_at: "".into(),
            resolved: false,
            replies: vec![Reply {
                id: "r".into(),
                author: "claude".into(),
                body: "done".into(),
                created_at: "".into(),
            }],
        }];
        assert_eq!(
            render(&[(&threads[0], false)]),
            "● 売上!B3 [user] line1 (1 reply)\n"
        );
        assert_eq!(
            render(&[(&threads[0], true)]),
            "● 売上!B3 (hidden) [user] line1 (1 reply)\n"
        );
        assert_eq!(render(&[]), "no comments\n");
    }
}
