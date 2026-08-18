#[cfg(test)]
mod tests {
    use super::*;

    fn items(names: &[&str]) -> Vec<PickerItem> {
        names
            .iter()
            .map(|n| PickerItem {
                name: (*n).to_string(),
                count: 0,
                active: false,
            })
            .collect()
    }

    #[test]
    fn window_keeps_the_selection_visible() {
        assert_eq!(window(32, 0, 10), 0..10, "top stays at the top");
        assert_eq!(window(32, 31, 10), 22..32, "bottom stays at the bottom");
        let mid = window(32, 20, 10);
        assert!(mid.contains(&20), "selection inside: {mid:?}");
        assert_eq!(mid.len(), 10);
        assert_eq!(window(3, 1, 10), 0..3, "short lists are whole");
        assert_eq!(window(0, 0, 10), 0..0);
        assert_eq!(window(5, 9, 3), 2..5, "an out-of-range selection clamps");
    }

    #[test]
    fn rows_mark_the_selection_and_carry_counts() {
        let mut view = PickerView {
            query: String::new(),
            selected: 1,
            total: 3,
            items: items(&["概要", "IT-01", "IT-02"]),
        };
        view.items[1].count = 2;
        view.items[2].active = true;
        let layout = picker_layout(&view, 20, 10);
        assert_eq!(layout.lines.len(), 3);
        assert!(layout.lines[1].selected && layout.lines[1].name.starts_with("▸ IT-01"));
        assert_eq!(layout.lines[1].count, " ● 2");
        for line in &layout.lines {
            let used = unicode_width::UnicodeWidthStr::width(line.name.as_str())
                + unicode_width::UnicodeWidthStr::width(line.count.as_str());
            assert_eq!(used, 20, "name and count fill the row exactly");
        }
        assert!(!layout.lines[0].selected && layout.lines[0].name.starts_with("  概要"));
        assert!(layout.lines[2].active);
        assert_eq!(layout.counter, "3/3");
        assert!(!layout.no_match);
    }

    #[test]
    fn long_names_clip_instead_of_wrapping() {
        let view = PickerView {
            query: String::new(),
            selected: 0,
            total: 1,
            items: items(&["これは画面よりずっと長い名前のシートです"]),
        };
        let layout = picker_layout(&view, 16, 5);
        let name = &layout.lines[0].name;
        assert!(name.contains('…'), "clipped: {name}");
        assert!(unicode_width::UnicodeWidthStr::width(name.as_str()) <= 16);
    }

    #[test]
    fn the_query_clips_from_the_front_keeping_the_cursor() {
        assert_eq!(query_line("abc", 10), "abc█");
        assert_eq!(query_line("", 10), "█");
        assert_eq!(query_line("abcdefgh", 5), "efgh█", "the tail stays");
        // CJK: a char that would half-fit is dropped whole
        assert_eq!(query_line("あいうえお", 5), "えお█");
        assert_eq!(query_line("abc", 0), "█", "never empty, renderer clips");
    }

    #[test]
    fn no_match_is_reported() {
        let view = PickerView {
            query: "zzz".into(),
            selected: 0,
            total: 32,
            items: Vec::new(),
        };
        let layout = picker_layout(&view, 20, 10);
        assert!(layout.no_match);
        assert!(layout.lines.is_empty());
        assert_eq!(layout.counter, "0/32");
    }
}
