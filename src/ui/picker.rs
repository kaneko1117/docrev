//! Pure sheet-picker layout: which candidates are visible around the
//! selection and how each row is formatted. No ratatui; `grid.rs` renders.

use super::text::{clip, pad_left, pad_right};

/// What the frontend hands the renderer, already filtered by the viewer.
pub struct PickerView {
    pub query: String,
    /// Index into `items`.
    pub selected: usize,
    /// Sheets in the workbook, for the `5/32` counter.
    pub total: usize,
    pub items: Vec<PickerItem>,
}

pub struct PickerItem {
    pub name: String,
    /// Unresolved threads on the sheet; 0 renders blank.
    pub count: usize,
    /// The sheet open behind the picker.
    pub active: bool,
}

pub(crate) struct PickerLayout {
    pub lines: Vec<PickerLine>,
    /// `5/32` — matches / sheets in the workbook.
    pub counter: String,
    pub no_match: bool,
}

/// One candidate row: `▸ name    ● 2`, pre-padded to the popup width.
pub(crate) struct PickerLine {
    pub name: String,
    /// Right-aligned marker + count, empty when the sheet has no threads.
    pub count: String,
    pub selected: bool,
    pub active: bool,
}

const COUNT_WIDTH: usize = 4;

pub(crate) fn picker_layout(view: &PickerView, width: usize, visible: usize) -> PickerLayout {
    let counter = format!("{}/{}", view.items.len(), view.total);
    let rows = window(view.items.len(), view.selected, visible);
    let lines = rows
        .clone()
        .map(|i| {
            let item = &view.items[i];
            let prefix = if i == view.selected { "▸ " } else { "  " };
            let count = if item.count > 0 {
                pad_left(&format!("● {}", item.count), COUNT_WIDTH)
            } else {
                String::new()
            };
            // name and count split the row: their widths always sum to `width`
            let count_width = unicode_width::UnicodeWidthStr::width(count.as_str());
            let name_width = width.saturating_sub(count_width);
            let name = clip(&item.name, name_width.saturating_sub(2));
            PickerLine {
                name: pad_right(&format!("{prefix}{name}"), name_width),
                count,
                selected: i == view.selected,
                active: item.active,
            }
        })
        .collect();
    PickerLayout {
        lines,
        counter,
        no_match: view.items.is_empty(),
    }
}

/// The slice of `len` rows that keeps `selected` visible, centered when
/// possible — opening on sheet 20 of 32 shows it mid-list, not at an edge.
fn window(len: usize, selected: usize, visible: usize) -> std::ops::Range<usize> {
    if visible == 0 || len == 0 {
        return 0..0;
    }
    let selected = selected.min(len - 1);
    let start = selected
        .saturating_sub(visible / 2)
        .min(len.saturating_sub(visible));
    start..(start + visible).min(len)
}

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
