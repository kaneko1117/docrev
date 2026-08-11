use crate::domain::comment::CommentThread;

use super::text::sanitize;

/// Human-readable `comment list` output (agents use `--json`).
pub fn render(threads: &[CommentThread]) -> String {
    if threads.is_empty() {
        return "no comments\n".to_string();
    }
    let mut out = String::new();
    for thread in threads {
        let mark = if thread.resolved { "✓" } else { "●" };
        let first_line = sanitize(thread.body.lines().next().unwrap_or(""));
        out.push_str(&format!(
            "{mark} {}!{} [{}] {first_line}",
            thread.anchor.sheet(),
            thread.anchor.cell_ref(),
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
        let threads = vec![CommentThread {
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
        assert_eq!(render(&threads), "● 売上!B3 [user] line1 (1 reply)\n");
        assert_eq!(render(&[]), "no comments\n");
    }
}
