//! Pure grid layout (#36): decides what goes where — merges, wrapping,
//! variable row heights, scroll following, alignment, semantic styling.
//! No ratatui, no colors, no `Span`s; `grid.rs` translates the result.

use std::collections::HashSet;

use crate::domain::anchor::Anchor;
use crate::domain::sheet::{Rgb, Sheet, TextColor};

use super::text::{cell_text, center, clip, pad_left, pad_right, wrap};

pub const DEFAULT_CELL_WIDTH: usize = 12;

#[derive(Debug, Default, Clone, Copy)]
pub struct Scroll {
    pub top: usize,
    pub left: usize,
}

pub(crate) struct LayoutInput<'a> {
    pub sheet: &'a Sheet,
    pub cursor: (usize, usize),
    pub markers: &'a HashSet<(usize, usize)>,
    pub col_widths: &'a [usize],
}

pub(crate) struct Viewport {
    /// Grid area width in terminal cells (including the row-label gutter).
    pub width: usize,
    /// Grid line capacity below the column-header line.
    pub rows: usize,
}

pub(crate) struct GridLayout {
    pub empty: bool,
    pub label_width: usize,
    /// Centered column letters, one per visible column.
    pub header: Vec<String>,
    pub lines: Vec<BodyLine>,
}

/// One terminal line of the grid body; a sheet row wraps into several.
pub(crate) struct BodyLine {
    /// Row number on the first line of a sheet row, blanks below.
    pub label: String,
    /// Horizontal gridline under this line (a sheet row's last line);
    /// applies to the label, the separators and non-merged slots.
    pub ruled: bool,
    pub slots: Vec<Slot>,
}

pub(crate) struct Slot {
    pub separator: Separator,
    /// Clipped and aligned (padded) text for this line of the slot.
    pub text: String,
    /// Selection styling (the cursor is on — or, for merges, inside — it).
    pub cursor: bool,
    /// Workbook fill; ignored under the cursor.
    pub fill: Option<Rgb>,
    /// Text color as resolved by the workbook; suppressed under the cursor
    /// unless it comes from the number format.
    pub font: Option<TextColor>,
    /// Horizontal gridline under this slot (merges only rule their last row).
    pub ruled: bool,
}

pub(crate) enum Separator {
    Gridline,
    Marker { fill: Option<Rgb> },
}

/// Font colors are dropped under the selection so light fonts stay readable
/// on the selection blue; format colors carry meaning and are kept.
fn visible_color(color: Option<TextColor>, on_cursor: bool) -> Option<TextColor> {
    match color {
        Some(TextColor::Font(_)) if on_cursor => None,
        other => other,
    }
}

pub(crate) fn grid_layout(
    input: &LayoutInput,
    viewport: &Viewport,
    scroll: &mut Scroll,
) -> GridLayout {
    let sheet = input.sheet;
    if sheet.row_count() == 0 || sheet.col_count() == 0 {
        return GridLayout {
            empty: true,
            label_width: 0,
            header: Vec::new(),
            lines: Vec::new(),
        };
    }

    let label_width = sheet.row_count().to_string().len().max(2);
    let rows_visible = viewport.rows;
    let avail = viewport.width.saturating_sub(label_width);
    let width_of = |c: usize| {
        input
            .col_widths
            .get(c)
            .copied()
            .unwrap_or(DEFAULT_CELL_WIDTH)
    };

    let (cursor_row, cursor_col) = input.cursor;
    follow_col(&mut scroll.left, cursor_col, avail, &width_of);
    let last_col = last_visible_col(scroll.left, sheet.col_count(), avail, &width_of);

    // Wrapped text makes row heights variable: a sheet row is as tall as
    // its tallest visible cell. Numbers stay single-line (#33), and a
    // merged region wraps at the merged width, counted on its anchor row.
    let height_of = |row: usize| -> usize {
        let mut height = 1;
        let mut col = scroll.left;
        while col < last_col {
            if let Some(merge) = sheet.merge_at(row, col) {
                let segment_end = (merge.end_col + 1).min(last_col);
                if row == merge.start_row {
                    let span_cols = segment_end - col;
                    let span_width: usize =
                        (col..segment_end).map(&width_of).sum::<usize>() + (span_cols - 1);
                    let text = cell_text(sheet.display_cell(row, col));
                    height = height.max(wrap(&text, span_width).len());
                }
                col = segment_end;
                continue;
            }
            let cell = sheet.cell(row, col);
            if !cell.is_number() && !cell.is_empty() {
                height = height.max(wrap(&cell_text(cell), width_of(col)).len());
            }
            col += 1;
        }
        height
    };
    follow_row_wrapped(&mut scroll.top, cursor_row, rows_visible, &height_of);

    let header = (scroll.left..last_col)
        .map(|col| center(&Anchor::column_label(col as u32), width_of(col)))
        .collect();

    let mut lines = Vec::with_capacity(rows_visible);
    let mut used = 0;
    let mut row = scroll.top;
    while used < rows_visible && row < sheet.row_count() {
        let height = height_of(row);
        for sub in 0..height {
            if used >= rows_visible {
                break;
            }
            // the horizontal gridline sits under a sheet row's LAST line
            let last_line = sub + 1 == height;
            let label = if sub == 0 {
                pad_left(&(row + 1).to_string(), label_width)
            } else {
                " ".repeat(label_width)
            };
            let mut slots = Vec::new();
            let mut col = scroll.left;
            while col < last_col {
                // A merged region lays out as one cell: the value on its
                // anchor row wrapped to the merged width, no gridlines
                // inside, and the whole region highlights when the cursor
                // is anywhere in it. A thread on ANY of its cells shows
                // one ● on the first visible line.
                if let Some(merge) = sheet.merge_at(row, col) {
                    let first_visible_row = merge.start_row.max(scroll.top);
                    let region_marked = row == first_visible_row
                        && sub == 0
                        && input.markers.iter().any(|(r, c)| merge.contains(*r, *c));
                    let separator = if region_marked {
                        Separator::Marker { fill: None }
                    } else {
                        Separator::Gridline
                    };
                    let segment_end = (merge.end_col + 1).min(last_col);
                    let span_cols = segment_end - col;
                    let span_width: usize =
                        (col..segment_end).map(&width_of).sum::<usize>() + (span_cols - 1);
                    let text = if row == merge.start_row {
                        wrap(&cell_text(sheet.display_cell(row, col)), span_width)
                            .get(sub)
                            .cloned()
                            .unwrap_or_default()
                    } else {
                        String::new()
                    };
                    let on_cursor = merge.contains(cursor_row, cursor_col);
                    let font = if row == merge.start_row {
                        visible_color(sheet.text_color_at(row, col), on_cursor)
                    } else {
                        None
                    };
                    slots.push(Slot {
                        separator,
                        text: pad_right(&clip(&text, span_width), span_width),
                        cursor: on_cursor,
                        // the anchor's fill paints the whole merged region
                        fill: sheet.display_fill_at(row, col),
                        font,
                        // the horizontal gridline only under the region's last row
                        ruled: row == merge.end_row && last_line,
                    });
                    col += span_cols;
                    continue;
                }

                let fill = sheet.display_fill_at(row, col);
                let separator = if sub == 0 && input.markers.contains(&(row, col)) {
                    Separator::Marker { fill }
                } else {
                    Separator::Gridline
                };
                let cell = sheet.cell(row, col);
                let own_width = width_of(col);
                let is_number = cell.is_number();
                let on_cursor = (row, col) == input.cursor;

                // text wraps within the column (#33); numbers stay on one
                // right-aligned line so digit groups never split
                let line_text = if is_number {
                    if sub == 0 {
                        cell_text(cell)
                    } else {
                        String::new()
                    }
                } else {
                    wrap(&cell_text(cell), own_width)
                        .get(sub)
                        .cloned()
                        .unwrap_or_default()
                };
                let clipped = clip(&line_text, own_width);
                let aligned = if is_number {
                    pad_left(&clipped, own_width)
                } else {
                    pad_right(&clipped, own_width)
                };
                slots.push(Slot {
                    separator,
                    text: aligned,
                    cursor: on_cursor,
                    fill,
                    font: visible_color(sheet.text_color_at(row, col), on_cursor),
                    ruled: last_line,
                });
                col += 1;
            }
            lines.push(BodyLine {
                label,
                ruled: last_line,
                slots,
            });
            used += 1;
        }
        row += 1;
    }

    GridLayout {
        empty: false,
        label_width,
        header,
        lines,
    }
}

/// Keeps the cursor row inside the visible window when rows have variable
/// heights. A cursor row taller than the window scrolls to its first line.
fn follow_row_wrapped(
    top: &mut usize,
    cursor: usize,
    visible: usize,
    height_of: &impl Fn(usize) -> usize,
) {
    if visible == 0 {
        return;
    }
    if cursor < *top {
        *top = cursor;
        return;
    }
    // heights are at least 1, so more than `visible` rows above the cursor
    // can never fit — jump close before fine-tuning
    if cursor >= *top + visible {
        *top = cursor + 1 - visible;
    }
    while *top < cursor {
        let mut span = 0;
        let fits = (*top..=cursor).all(|r| {
            span += height_of(r);
            span <= visible
        });
        if fits {
            break;
        }
        *top += 1;
    }
}

/// Horizontal variant for variable column widths.
fn follow_col(left: &mut usize, cursor: usize, avail: usize, width_of: &impl Fn(usize) -> usize) {
    if cursor < *left {
        *left = cursor;
        return;
    }
    while *left < cursor {
        let span: usize = (*left..=cursor).map(|c| width_of(c) + 1).sum();
        if span <= avail {
            break;
        }
        *left += 1;
    }
}

/// First column that no longer fits; always shows at least one column.
fn last_visible_col(
    left: usize,
    col_count: usize,
    avail: usize,
    width_of: &impl Fn(usize) -> usize,
) -> usize {
    let mut used = 0;
    let mut col = left;
    while col < col_count {
        let needed = width_of(col) + 1;
        if used + needed > avail && col > left {
            break;
        }
        used += needed;
        col += 1;
    }
    col
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::cell::CellValue;
    use crate::domain::number_format::FormatColor;
    use crate::domain::sheet::MergedRange;
    use std::collections::HashMap;

    fn run_layout(
        sheet: &Sheet,
        cursor: (usize, usize),
        markers: &HashSet<(usize, usize)>,
        width: usize,
        rows: usize,
    ) -> GridLayout {
        let input = LayoutInput {
            sheet,
            cursor,
            markers,
            col_widths: &[],
        };
        let mut scroll = Scroll::default();
        grid_layout(&input, &Viewport { width, rows }, &mut scroll)
    }

    #[test]
    fn follow_row_wrapped_accounts_for_tall_rows() {
        let heights = [3usize, 1, 1, 1, 1];
        let h = |r: usize| heights[r];

        let mut top = 0;
        follow_row_wrapped(&mut top, 3, 4, &h);
        assert_eq!(top, 1, "row 0 is 3 lines tall; rows 0-3 exceed 4 lines");

        follow_row_wrapped(&mut top, 0, 4, &h);
        assert_eq!(top, 0, "scrolling back up");

        let mut top = 0;
        follow_row_wrapped(&mut top, 0, 2, &h);
        assert_eq!(top, 0, "a row taller than the window shows its top");
    }

    #[test]
    fn empty_sheet_yields_an_empty_layout() {
        let sheet = Sheet::new("s", vec![]);
        let layout = run_layout(&sheet, (0, 0), &HashSet::new(), 40, 5);
        assert!(layout.empty);
        assert!(layout.lines.is_empty());
    }

    #[test]
    fn a_merged_region_is_one_slot() {
        let sheet = Sheet::new(
            "s",
            vec![
                vec![CellValue::Text("title".into())],
                vec![
                    CellValue::Text("a".into()),
                    CellValue::Text("b".into()),
                    CellValue::Text("c".into()),
                ],
            ],
        )
        .with_merges(vec![MergedRange {
            start_row: 0,
            start_col: 0,
            end_row: 0,
            end_col: 2,
        }]);
        let layout = run_layout(&sheet, (1, 0), &HashSet::new(), 50, 5);
        assert_eq!(layout.lines[0].slots.len(), 1, "merged row: one slot");
        assert!(layout.lines[0].slots[0].text.starts_with("title"));
        assert_eq!(layout.lines[1].slots.len(), 3, "plain row: three slots");
    }

    #[test]
    fn cursor_inside_a_merge_flags_the_whole_region() {
        let sheet = Sheet::new(
            "s",
            vec![
                vec![CellValue::Text("title".into())],
                vec![
                    CellValue::Text("a".into()),
                    CellValue::Text("b".into()),
                    CellValue::Text("c".into()),
                ],
            ],
        )
        .with_merges(vec![MergedRange {
            start_row: 0,
            start_col: 0,
            end_row: 0,
            end_col: 2,
        }]);
        let layout = run_layout(&sheet, (0, 2), &HashSet::new(), 50, 5);
        assert!(layout.lines[0].slots[0].cursor);
    }

    #[test]
    fn wrapping_makes_tall_rows_with_blank_continuation_labels() {
        let sheet = Sheet::new(
            "s",
            vec![vec![CellValue::Text(
                "a very long text that wraps across lines".into(),
            )]],
        );
        let layout = run_layout(&sheet, (0, 0), &HashSet::new(), 20, 6);
        assert!(layout.lines.len() > 1, "the row must wrap");
        assert!(layout.lines[0].label.contains('1'));
        assert_eq!(
            layout.lines[1].label.trim(),
            "",
            "continuation lines have no label"
        );
        assert!(!layout.lines[0].ruled, "gridline only under the last line");
        assert!(layout.lines.last().is_some_and(|l| l.ruled));
    }

    #[test]
    fn numbers_stay_single_line_and_right_aligned() {
        let sheet = Sheet::new(
            "s",
            vec![vec![
                CellValue::Text("a long wrapping text here".into()),
                CellValue::Number(42.0),
            ]],
        );
        let layout = run_layout(&sheet, (0, 0), &HashSet::new(), 40, 6);
        assert!(layout.lines.len() > 1);
        let first = &layout.lines[0].slots[1];
        assert!(
            first.text.ends_with("42"),
            "right aligned: {:?}",
            first.text
        );
        let second = &layout.lines[1].slots[1];
        assert_eq!(second.text.trim(), "", "no digits on continuation lines");
    }

    #[test]
    fn markers_only_on_the_first_line_of_a_row() {
        let sheet = Sheet::new(
            "s",
            vec![vec![CellValue::Text(
                "a very long text that wraps across lines".into(),
            )]],
        );
        let markers = HashSet::from([(0usize, 0usize)]);
        let layout = run_layout(&sheet, (0, 0), &markers, 20, 6);
        assert!(matches!(
            layout.lines[0].slots[0].separator,
            Separator::Marker { .. }
        ));
        assert!(matches!(
            layout.lines[1].slots[0].separator,
            Separator::Gridline
        ));
    }

    #[test]
    fn format_color_beats_font_color_and_survives_the_cursor() {
        let cell = || CellValue::FormattedNumber {
            value: -1.0,
            text: "▲1".into(),
            color: Some(FormatColor::Red),
        };
        let fonts = HashMap::from([((0usize, 0usize), Rgb { r: 1, g: 2, b: 3 })]);
        let sheet = Sheet::new("s", vec![vec![cell(), CellValue::Text("x".into())]])
            .with_font_colors(fonts);

        // off-cursor: format color wins over the font color
        let layout = run_layout(&sheet, (0, 1), &HashSet::new(), 40, 3);
        assert_eq!(
            layout.lines[0].slots[0].font,
            Some(TextColor::Format(FormatColor::Red))
        );

        // on the cursor the format color survives (font colors would not)
        let layout = run_layout(&sheet, (0, 0), &HashSet::new(), 40, 3);
        assert_eq!(
            layout.lines[0].slots[0].font,
            Some(TextColor::Format(FormatColor::Red))
        );
    }

    #[test]
    fn font_color_is_suppressed_under_the_cursor() {
        let fonts = HashMap::from([((0usize, 0usize), Rgb { r: 1, g: 2, b: 3 })]);
        let sheet =
            Sheet::new("s", vec![vec![CellValue::Text("x".into())]]).with_font_colors(fonts);

        let layout = run_layout(&sheet, (0, 0), &HashSet::new(), 40, 3);
        assert_eq!(layout.lines[0].slots[0].font, None, "cursor suppresses it");

        let sheet2 = Sheet::new(
            "s",
            vec![vec![
                CellValue::Text("x".into()),
                CellValue::Text("y".into()),
            ]],
        )
        .with_font_colors(HashMap::from([(
            (0usize, 0usize),
            Rgb { r: 1, g: 2, b: 3 },
        )]));
        let layout = run_layout(&sheet2, (0, 1), &HashSet::new(), 40, 3);
        assert_eq!(
            layout.lines[0].slots[0].font,
            Some(TextColor::Font(Rgb { r: 1, g: 2, b: 3 }))
        );
    }
}
