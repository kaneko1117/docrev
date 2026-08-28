//! Pure grid layout (#36): decides what goes where — merges, wrapping,
//! variable row heights, scroll following, alignment, semantic styling.
//! No ratatui, no colors, no `Span`s; `grid.rs` translates the result.

use std::collections::HashSet;

use crate::domain::anchor::Anchor;
use crate::domain::sheet::{Rgb, Sheet, TextColor};

use super::text::{cell_text, center, clip, pad_left, pad_right, wrap};

pub const DEFAULT_CELL_WIDTH: usize = 12;

/// Excel widths are fractional character counts; a terminal cell wants an
/// integer column count, clamped to a sane range. Absent or non-finite
/// widths (NaN in the file) fall back to the default.
fn display_width(excel: Option<f64>) -> usize {
    match excel {
        Some(w) if w.is_finite() => w.round().clamp(4.0, 60.0) as usize,
        _ => DEFAULT_CELL_WIDTH,
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Scroll {
    pub top: usize,
    pub left: usize,
}

pub(crate) struct LayoutInput<'a> {
    pub sheet: &'a Sheet,
    pub cursor: (usize, usize),
    pub markers: &'a HashSet<(usize, usize)>,
    /// Cells carrying the workbook's own comments — tinted in the corner.
    pub notes: &'a HashSet<(usize, usize)>,
    /// Widths as the file states them; `display_width` turns each into
    /// terminal cells.
    pub col_widths: &'a [Option<f64>],
    /// A drag in progress, as (press cell, current cell) — highlighted like
    /// the cursor.
    pub selection: Option<((usize, usize), (usize, usize))>,
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
    /// Scrolling-body columns currently on screen, and how many the sheet
    /// has — the status bar tells the user where they are in a wide sheet.
    pub visible_cols: std::ops::Range<usize>,
    /// Columns pinned left of the body — they count as visible too.
    pub frozen_cols: usize,
    pub col_count: usize,
    /// Centered column letters: frozen columns first, then the visible body.
    pub header: Vec<String>,
    /// Index into `header` whose left separator is the freeze boundary.
    pub header_boundary: Option<usize>,
    /// Each visible column with its x-span inside the grid area (separator
    /// included), frozen columns first — the hit map for mouse clicks.
    pub col_spans: Vec<(usize, std::ops::Range<usize>)>,
    pub lines: Vec<BodyLine>,
}

/// One terminal line of the grid body; a sheet row wraps into several.
pub(crate) struct BodyLine {
    /// The sheet row this line belongs to — the hit map for mouse clicks.
    pub row: usize,
    /// Row number on the first line of a sheet row, blanks below.
    pub label: String,
    /// Horizontal gridline under this line (a sheet row's last line);
    /// applies to the label, the separators and non-merged slots.
    pub ruled: bool,
    /// This line's gridline is the frozen-rows boundary — drawn emphasized.
    pub freeze_boundary: bool,
    pub slots: Vec<Slot>,
}

pub(crate) struct Slot {
    pub separator: Separator,
    /// The left separator is the frozen-columns boundary — drawn emphasized.
    pub freeze_boundary: bool,
    /// Clipped and aligned (padded) text for this line of the slot.
    pub text: String,
    /// Cursor styling (the cursor is on — or, for merges, inside — it).
    pub cursor: bool,
    /// Inside the dragged rectangle — drawn a step lighter than the cursor.
    pub selected: bool,
    /// The cell carries a workbook comment: its top-right corner is tinted,
    /// like Sheets. Only on the first line of the sheet row.
    pub note: bool,
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
            visible_cols: 0..0,
            frozen_cols: 0,
            col_count: 0,
            header: Vec::new(),
            header_boundary: None,
            col_spans: Vec::new(),
            lines: Vec::new(),
        };
    }

    let label_width = sheet.row_count().to_string().len().max(2);
    let rows_visible = viewport.rows;
    let avail = viewport.width.saturating_sub(label_width);
    let width_of = |c: usize| display_width(input.col_widths.get(c).copied().flatten());

    // Frozen columns pin to the left of the body. Dropped for the frame
    // when they would leave the body no room at all (separator + one cell).
    let mut frozen_cols = sheet.frozen_cols().min(sheet.col_count().saturating_sub(1));
    let span_of = |cols: std::ops::Range<usize>| cols.map(|c| width_of(c) + 1).sum::<usize>();
    if frozen_cols > 0 && span_of(0..frozen_cols) + 2 > avail {
        frozen_cols = 0;
    }
    let body_avail = avail - span_of(0..frozen_cols);

    let (cursor_row, cursor_col) = input.cursor;
    scroll.left = scroll.left.max(frozen_cols);
    // a cursor inside the frozen columns is always visible — only a body
    // cursor drives horizontal scrolling
    if cursor_col >= frozen_cols {
        follow_col(
            &mut scroll.left,
            cursor_col,
            sheet.col_count(),
            body_avail,
            &width_of,
            frozen_cols,
        );
    } else {
        // the no-blank-space-on-the-right invariant holds regardless of
        // where the cursor is — widening the grid must still scroll back
        scroll_back_col(
            &mut scroll.left,
            sheet.col_count(),
            body_avail,
            &width_of,
            frozen_cols,
        );
    }
    let last_col = last_visible_col(scroll.left, sheet.col_count(), body_avail, &width_of);
    let body_left = scroll.left;
    let segments = move || [(0, frozen_cols), (body_left, last_col)].into_iter();

    // Wrapped text makes row heights variable: a sheet row is as tall as
    // its tallest visible cell — frozen columns included. Numbers stay
    // single-line (#33), and a merged region wraps at the merged width,
    // counted on its anchor row.
    let height_of = |row: usize| -> usize {
        let mut height = 1;
        for (start, end) in segments() {
            let mut col = start;
            while col < end {
                if let Some(merge) = sheet.merge_at(row, col) {
                    let segment_end = (merge.end_col + 1).min(end);
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
        }
        height
    };

    // Frozen rows pin above the body. Dropped when they would leave no
    // line for the body to scroll in. Heights are at least 1, so more
    // frozen rows than viewport lines can never fit — checked first, or a
    // hostile ySplit would make this sum walk the whole sheet every frame.
    let mut frozen_rows = sheet.frozen_rows().min(sheet.row_count().saturating_sub(1));
    if frozen_rows >= rows_visible
        || (0..frozen_rows).map(&height_of).sum::<usize>() >= rows_visible
    {
        frozen_rows = 0;
    }
    let frozen_height: usize = (0..frozen_rows).map(&height_of).sum();
    scroll.top = scroll.top.max(frozen_rows);
    if cursor_row >= frozen_rows {
        follow_row_wrapped(
            &mut scroll.top,
            cursor_row,
            rows_visible - frozen_height,
            &height_of,
            frozen_rows,
        );
    }
    let body_top = scroll.top;

    let header: Vec<String> = segments()
        .flat_map(|(start, end)| start..end)
        .map(|col| center(&Anchor::column_label(col as u32), width_of(col)))
        .collect();
    let header_boundary = (frozen_cols > 0).then_some(frozen_cols);

    // every visible column's x-span (its left separator included), so a
    // click can be mapped back to a cell without re-deriving the geometry
    let mut col_spans = Vec::new();
    let mut x = label_width;
    for col in segments().flat_map(|(start, end)| start..end) {
        let width = width_of(col) + 1;
        col_spans.push((col, x..x + width));
        x += width;
    }

    // the drag rectangle highlights a step lighter than the cursor cell
    let in_selection = |row: usize, col: usize| match input.selection {
        Some(((r0, c0), (r1, c1))) => {
            (r0.min(r1)..=r0.max(r1)).contains(&row) && (c0.min(c1)..=c0.max(c1)).contains(&col)
        }
        None => false,
    };

    // the first visible line of a merge carries its ● marker; with a freeze
    // the pinned rows are visible from the top, not from the scroll offset
    let first_visible_row = |merge: &crate::domain::sheet::MergedRange| {
        if merge.start_row < frozen_rows {
            merge.start_row
        } else {
            merge.start_row.max(body_top)
        }
    };

    let build_row = |row: usize, boundary_row: bool| -> Vec<BodyLine> {
        let height = height_of(row);
        let mut out = Vec::with_capacity(height);
        for sub in 0..height {
            // the horizontal gridline sits under a sheet row's LAST line
            let last_line = sub + 1 == height;
            let label = if sub == 0 {
                pad_left(&(row + 1).to_string(), label_width)
            } else {
                " ".repeat(label_width)
            };
            let mut slots = Vec::new();
            for (seg_index, (start, end)) in segments().enumerate() {
                let mut col = start;
                while col < end {
                    let freeze_boundary = frozen_cols > 0 && seg_index == 1 && col == start;
                    // A merged region lays out as one cell: the value on its
                    // anchor row wrapped to the merged width, no gridlines
                    // inside, and the whole region highlights when the cursor
                    // is anywhere in it. A thread on ANY of its cells shows
                    // one ● on the first visible line.
                    if let Some(merge) = sheet.merge_at(row, col) {
                        // a merge that began inside the frozen columns has
                        // already drawn its text and marker in the pinned
                        // segment — the body segment shows only its blank
                        // continuation, never a second copy
                        let continuation = seg_index == 1 && merge.start_col < frozen_cols;
                        let region_marked = !continuation
                            && row == first_visible_row(merge)
                            && sub == 0
                            && input.markers.iter().any(|(r, c)| merge.contains(*r, *c));
                        let separator = if region_marked {
                            Separator::Marker { fill: None }
                        } else {
                            Separator::Gridline
                        };
                        let segment_end = (merge.end_col + 1).min(end);
                        let span_cols = segment_end - col;
                        let span_width: usize =
                            (col..segment_end).map(&width_of).sum::<usize>() + (span_cols - 1);
                        let text = if row == merge.start_row && !continuation {
                            wrap(&cell_text(sheet.display_cell(row, col)), span_width)
                                .get(sub)
                                .cloned()
                                .unwrap_or_default()
                        } else {
                            String::new()
                        };
                        let on_cursor = merge.contains(cursor_row, cursor_col);
                        let in_range = (col..segment_end).any(|c| in_selection(row, c));
                        let font = if row == merge.start_row && !continuation {
                            visible_color(sheet.text_color_at(row, col), on_cursor || in_range)
                        } else {
                            None
                        };
                        let note =
                            sub == 0 && input.notes.iter().any(|(r, c)| merge.contains(*r, *c));
                        slots.push(Slot {
                            separator,
                            freeze_boundary,
                            text: pad_right(&clip(&text, span_width), span_width),
                            cursor: on_cursor,
                            selected: in_range,
                            note,
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
                    let in_range = in_selection(row, col);

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
                        freeze_boundary,
                        text: aligned,
                        cursor: on_cursor,
                        selected: in_range,
                        note: sub == 0 && input.notes.contains(&(row, col)),
                        fill,
                        font: visible_color(sheet.text_color_at(row, col), on_cursor || in_range),
                        ruled: last_line,
                    });
                    col += 1;
                }
            }
            out.push(BodyLine {
                row,
                label,
                ruled: last_line,
                freeze_boundary: boundary_row && last_line,
                slots,
            });
        }
        out
    };

    let mut lines = Vec::with_capacity(rows_visible);
    for row in 0..frozen_rows {
        lines.extend(build_row(row, row + 1 == frozen_rows));
    }
    let mut row = body_top;
    while lines.len() < rows_visible && row < sheet.row_count() {
        let mut body = build_row(row, false);
        body.truncate(rows_visible - lines.len());
        lines.extend(body);
        row += 1;
    }

    GridLayout {
        empty: false,
        label_width,
        visible_cols: body_left..last_col,
        frozen_cols,
        col_count: sheet.col_count(),
        header,
        header_boundary,
        col_spans,
        lines,
    }
}

/// The slice of sheet tabs that fits, always including the active one, plus
/// whether tabs are hidden on either side.
pub(crate) struct TabStrip {
    pub more_left: bool,
    pub more_right: bool,
    /// (sheet index, rendered label) in display order.
    pub tabs: Vec<(usize, String)>,
}

pub(crate) fn tab_strip(names: &[&str], active: usize, width: usize) -> TabStrip {
    let labels: Vec<String> = names.iter().map(|n| format!("[{n}]")).collect();
    if labels.is_empty() || width == 0 {
        return TabStrip {
            more_left: false,
            more_right: false,
            tabs: Vec::new(),
        };
    }
    let w = |i: usize| unicode_width::UnicodeWidthStr::width(labels[i].as_str());
    let active = active.min(labels.len() - 1);

    // grow around the active tab, preferring the tabs before it so the user
    // keeps the context they scrolled through
    let (mut start, mut end) = (active, active + 1);
    let mut used = w(active);
    loop {
        // one column per arrow, only while tabs are actually hidden
        let budget = width
            .saturating_sub(usize::from(start > 0))
            .saturating_sub(usize::from(end < labels.len()));
        let mut grew = false;
        if start > 0 && used + w(start - 1) <= budget {
            start -= 1;
            used += w(start);
            grew = true;
        }
        if end < labels.len() && used + w(end) <= budget {
            used += w(end);
            end += 1;
            grew = true;
        }
        if !grew {
            break;
        }
    }
    TabStrip {
        more_left: start > 0,
        more_right: end < labels.len(),
        tabs: (start..end).map(|i| (i, labels[i].clone())).collect(),
    }
}

/// Keeps the cursor row inside the visible window when rows have variable
/// heights. A cursor row taller than the window scrolls to its first line.
/// `floor` is the first scrollable row — frozen rows never scroll away.
fn follow_row_wrapped(
    top: &mut usize,
    cursor: usize,
    visible: usize,
    height_of: &impl Fn(usize) -> usize,
    floor: usize,
) {
    if visible == 0 {
        return;
    }
    if cursor < *top {
        *top = cursor.max(floor);
        return;
    }
    // heights are at least 1, so more than `visible` rows above the cursor
    // can never fit — jump close before fine-tuning
    if cursor >= *top + visible {
        *top = (cursor + 1 - visible).max(floor);
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

/// Horizontal variant for variable column widths; `floor` is the first
/// scrollable column.
fn follow_col(
    left: &mut usize,
    cursor: usize,
    col_count: usize,
    avail: usize,
    width_of: &impl Fn(usize) -> usize,
    floor: usize,
) {
    if cursor < *left {
        *left = cursor.max(floor);
        return;
    }
    while *left < cursor {
        let span: usize = (*left..=cursor).map(|c| width_of(c) + 1).sum();
        if span <= avail {
            break;
        }
        *left += 1;
    }
    scroll_back_col(left, col_count, avail, width_of, floor);
}

/// Never leave blank space on the right while columns hide on the left —
/// otherwise widening the grid (closing the sidebar) would keep the view
/// scrolled where the narrower grid had pushed it.
fn scroll_back_col(
    left: &mut usize,
    col_count: usize,
    avail: usize,
    width_of: &impl Fn(usize) -> usize,
    floor: usize,
) {
    while *left > floor {
        let span: usize = (*left - 1..col_count).map(|c| width_of(c) + 1).sum();
        if span > avail {
            break;
        }
        *left -= 1;
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

    #[test]
    fn display_width_rounds_clamps_and_defaults() {
        assert_eq!(display_width(Some(18.5)), 19, "rounds to a whole cell");
        assert_eq!(display_width(Some(3.0)), 4, "clamped up");
        assert_eq!(display_width(Some(100.0)), 60, "clamped down");
        assert_eq!(display_width(Some(f64::NAN)), DEFAULT_CELL_WIDTH);
        assert_eq!(display_width(Some(f64::INFINITY)), DEFAULT_CELL_WIDTH);
        assert_eq!(display_width(None), DEFAULT_CELL_WIDTH);
    }
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
        let notes = HashSet::new();
        let input = LayoutInput {
            sheet,
            cursor,
            markers,
            notes: &notes,
            col_widths: &[],
            selection: None,
        };
        let mut scroll = Scroll::default();
        grid_layout(&input, &Viewport { width, rows }, &mut scroll)
    }

    #[test]
    fn follow_row_wrapped_accounts_for_tall_rows() {
        let heights = [3usize, 1, 1, 1, 1];
        let h = |r: usize| heights[r];

        let mut top = 0;
        follow_row_wrapped(&mut top, 3, 4, &h, 0);
        assert_eq!(top, 1, "row 0 is 3 lines tall; rows 0-3 exceed 4 lines");

        follow_row_wrapped(&mut top, 0, 4, &h, 0);
        assert_eq!(top, 0, "scrolling back up");

        let mut top = 0;
        follow_row_wrapped(&mut top, 0, 2, &h, 0);
        assert_eq!(top, 0, "a row taller than the window shows its top");
    }

    #[test]
    fn widening_the_grid_scrolls_back_instead_of_leaving_blank_space() {
        // 8 columns of the default width; the cursor sits on the 7th
        let sheet = Sheet::new("s", vec![vec![CellValue::Text("x".into()); 8]]);
        let markers = HashSet::new();
        let notes = HashSet::new();
        let input = LayoutInput {
            sheet: &sheet,
            cursor: (0, 6),
            markers: &markers,
            notes: &notes,
            col_widths: &[],
            selection: None,
        };
        let mut scroll = Scroll::default();

        // narrow viewport (the sidebar is open) pushes the view right
        grid_layout(&input, &Viewport { width: 46, rows: 5 }, &mut scroll);
        let narrow_left = scroll.left;
        assert!(narrow_left > 0, "the cursor forced a scroll");

        // closing the sidebar widens it again: the view must come back
        grid_layout(&input, &Viewport { width: 80, rows: 5 }, &mut scroll);
        let wide_left = scroll.left;
        assert!(
            wide_left < narrow_left,
            "widening must scroll back, got {wide_left} (was {narrow_left})"
        );

        // and it must be stable: reopening and closing lands on the same place
        grid_layout(&input, &Viewport { width: 46, rows: 5 }, &mut scroll);
        grid_layout(&input, &Viewport { width: 80, rows: 5 }, &mut scroll);
        assert_eq!(scroll.left, wide_left, "the view must not drift");
    }

    #[test]
    fn the_tab_strip_always_shows_the_active_sheet() {
        let names = ["売上", "経費", "集計", "備考", "参考", "旧データ"];
        let refs: Vec<&str> = names.to_vec();

        // everything fits: no arrows
        let all = tab_strip(&refs, 0, 200);
        assert_eq!(all.tabs.len(), names.len());
        assert!(!all.more_left && !all.more_right);

        // narrow: the active tab is present wherever it is
        for active in 0..names.len() {
            let strip = tab_strip(&refs, active, 24);
            assert!(
                strip.tabs.iter().any(|(i, _)| *i == active),
                "active {active} must be visible: {:?}",
                strip.tabs
            );
            assert_eq!(strip.more_left, strip.tabs[0].0 > 0);
            let last = strip.tabs.last().unwrap().0;
            assert_eq!(strip.more_right, last + 1 < names.len());
        }
    }

    #[test]
    fn the_tab_strip_fits_its_width() {
        let names = vec!["とても長い名前のシート1", "2月度実績データ", "集計"];
        let strip = tab_strip(&names, 1, 30);
        let used: usize = strip
            .tabs
            .iter()
            .map(|(_, l)| unicode_width::UnicodeWidthStr::width(l.as_str()))
            .sum::<usize>()
            + usize::from(strip.more_left)
            + usize::from(strip.more_right);
        assert!(used <= 30, "strip must fit: {used} > 30");
        assert!(strip.tabs.iter().any(|(i, _)| *i == 1));
    }

    #[test]
    fn a_single_tab_wider_than_the_bar_is_still_shown() {
        let names = vec!["これは画面よりずっと長い名前のシートです"];
        let strip = tab_strip(&names, 0, 10);
        assert_eq!(strip.tabs.len(), 1);
        assert!(!strip.more_left && !strip.more_right);
    }

    #[test]
    fn the_layout_reports_the_visible_column_range() {
        let sheet = Sheet::new("s", vec![vec![CellValue::Text("x".into()); 30]]);
        let markers = HashSet::new();
        let notes = HashSet::new();
        let input = LayoutInput {
            sheet: &sheet,
            cursor: (0, 0),
            markers: &markers,
            notes: &notes,
            col_widths: &[],
            selection: None,
        };
        let mut scroll = Scroll::default();
        let layout = grid_layout(&input, &Viewport { width: 80, rows: 5 }, &mut scroll);
        assert_eq!(layout.col_count, 30);
        assert_eq!(layout.visible_cols.start, 0);
        assert!(layout.visible_cols.end < 30, "a wide sheet is clipped");
    }

    fn tall_sheet_frozen(rows: usize, frozen_rows: usize, frozen_cols: usize) -> Sheet {
        let grid: Vec<Vec<CellValue>> = (0..rows)
            .map(|r| {
                vec![
                    CellValue::Text(format!("r{r}a")),
                    CellValue::Text(format!("r{r}b")),
                    CellValue::Text(format!("r{r}c")),
                    CellValue::Text(format!("r{r}d")),
                ]
            })
            .collect();
        Sheet::new("s", grid).with_frozen(frozen_rows, frozen_cols)
    }

    #[test]
    fn frozen_rows_stay_pinned_while_the_body_scrolls() {
        let sheet = tall_sheet_frozen(100, 1, 0);
        let layout = run_layout(&sheet, (60, 0), &HashSet::new(), 60, 5);
        assert_eq!(layout.lines[0].label.trim(), "1", "header row pinned");
        assert!(layout.lines[0].freeze_boundary, "boundary under the pin");
        assert_eq!(layout.lines[1].label.trim(), "58", "body scrolled to fit");
        assert!(!layout.lines[1].freeze_boundary);
        assert!(
            layout.lines.last().unwrap().slots[0]
                .text
                .starts_with("r60"),
            "cursor row visible at the bottom"
        );
    }

    #[test]
    fn frozen_cols_stay_pinned_while_the_body_scrolls_right() {
        let sheet = tall_sheet_frozen(3, 0, 1);
        let notes = HashSet::new();
        let input = LayoutInput {
            sheet: &sheet,
            cursor: (0, 3),
            markers: &HashSet::new(),
            notes: &notes,
            col_widths: &[],
            selection: None,
        };
        let mut scroll = Scroll::default();
        // narrow: room for the frozen column plus one body column
        let layout = grid_layout(&input, &Viewport { width: 30, rows: 4 }, &mut scroll);
        assert_eq!(layout.header.len(), 2, "one pinned + one body column");
        assert_eq!(layout.header[0].trim(), "A", "column A pinned");
        assert_eq!(layout.header[1].trim(), "D", "body scrolled to the cursor");
        assert_eq!(layout.header_boundary, Some(1));
        let line = &layout.lines[0];
        assert!(line.slots[0].text.starts_with("r0a"), "pinned cell");
        assert!(line.slots[1].freeze_boundary, "boundary on the body side");
        assert!(line.slots[1].text.starts_with("r0d"));
        assert_eq!(layout.visible_cols, 3..4, "status reports the body range");
    }

    #[test]
    fn a_freeze_that_leaves_no_body_is_dropped_for_the_frame() {
        // frozen rows as tall as the viewport: no line left to scroll
        let sheet = tall_sheet_frozen(50, 4, 0);
        let layout = run_layout(&sheet, (30, 0), &HashSet::new(), 60, 4);
        assert_eq!(
            layout.lines[0].label.trim(),
            "28",
            "no pinning, plain scroll"
        );
        assert!(layout.lines.iter().all(|l| !l.freeze_boundary));

        // frozen columns wider than the viewport
        let wide = Sheet::new("s", vec![vec![CellValue::Text("x".into()); 6]]).with_frozen(0, 5);
        let notes = HashSet::new();
        let input = LayoutInput {
            sheet: &wide,
            cursor: (0, 5),
            markers: &HashSet::new(),
            notes: &notes,
            col_widths: &[],
            selection: None,
        };
        let mut scroll = Scroll::default();
        let layout = grid_layout(&input, &Viewport { width: 20, rows: 3 }, &mut scroll);
        assert_eq!(layout.header_boundary, None, "column freeze dropped");
    }

    #[test]
    fn a_merge_crossing_the_column_freeze_renders_its_text_and_marker_once() {
        let sheet = Sheet::new(
            "s",
            vec![
                vec![CellValue::Text("TITLE".into()); 5],
                vec![CellValue::Text("x".into()); 5],
            ],
        )
        .with_merges(vec![MergedRange {
            start_row: 0,
            start_col: 0,
            end_row: 0,
            end_col: 4,
        }])
        .with_frozen(0, 1);
        let markers = HashSet::from([(0usize, 2usize)]);
        let layout = run_layout(&sheet, (1, 1), &markers, 41, 4);
        let line = &layout.lines[0];
        assert!(line.slots[0].text.starts_with("TITLE"), "pinned side draws");
        assert_eq!(
            line.slots[1].text.trim(),
            "",
            "the body side is a blank continuation"
        );
        let markers_drawn = line
            .slots
            .iter()
            .filter(|s| matches!(s.separator, Separator::Marker { .. }))
            .count();
        assert_eq!(markers_drawn, 1, "one ● for the region, not two");
    }

    #[test]
    fn widening_scrolls_back_even_while_the_cursor_sits_in_frozen_columns() {
        let sheet = Sheet::new("s", vec![vec![CellValue::Text("x".into()); 8]]).with_frozen(0, 1);
        let markers = HashSet::new();
        // cursor in the pinned column, but the view was scrolled right
        let notes = HashSet::new();
        let input = LayoutInput {
            sheet: &sheet,
            cursor: (0, 0),
            markers: &markers,
            notes: &notes,
            col_widths: &[],
            selection: None,
        };
        let mut scroll = Scroll { top: 0, left: 5 };
        grid_layout(
            &input,
            &Viewport {
                width: 120,
                rows: 3,
            },
            &mut scroll,
        );
        assert_eq!(
            scroll.left, 1,
            "everything fits: the body scrolls back to the floor"
        );
    }

    #[test]
    fn a_hostile_freeze_count_does_not_scan_the_whole_sheet() {
        let sheet = tall_sheet_frozen(200, usize::MAX, 0);
        let layout = run_layout(&sheet, (100, 0), &HashSet::new(), 60, 5);
        assert!(
            layout.lines.iter().all(|l| !l.freeze_boundary),
            "an impossible freeze is dropped, not honored"
        );
    }

    #[test]
    fn scrolling_up_stops_at_the_freeze_floor() {
        let sheet = tall_sheet_frozen(100, 2, 0);
        let markers = HashSet::new();
        let mut scroll = Scroll { top: 50, left: 0 };
        let notes = HashSet::new();
        let input = LayoutInput {
            sheet: &sheet,
            cursor: (2, 0),
            markers: &markers,
            notes: &notes,
            col_widths: &[],
            selection: None,
        };
        grid_layout(&input, &Viewport { width: 60, rows: 6 }, &mut scroll);
        assert_eq!(scroll.top, 2, "the body scrolls back to the floor, not 0");
    }

    #[test]
    fn the_drag_rectangle_highlights_a_step_lighter_than_the_cursor() {
        let sheet = Sheet::new("s", vec![vec![CellValue::Text("x".into()); 3]; 3]);
        let markers = HashSet::new();
        let notes = HashSet::new();
        let input = LayoutInput {
            sheet: &sheet,
            cursor: (2, 2),
            markers: &markers,
            notes: &notes,
            col_widths: &[],
            // dragged backwards on purpose: the rectangle still normalizes
            selection: Some(((1, 1), (0, 0))),
        };
        let mut scroll = Scroll::default();
        let layout = grid_layout(&input, &Viewport { width: 60, rows: 5 }, &mut scroll);
        assert!(layout.lines[0].slots[0].selected, "A1 inside the rectangle");
        assert!(layout.lines[1].slots[1].selected, "B2 inside the rectangle");
        assert!(!layout.lines[0].slots[2].selected, "C1 outside");
        assert!(
            layout.lines[2].slots[2].cursor && !layout.lines[2].slots[2].selected,
            "the cursor keeps its own, stronger style"
        );
        assert!(
            !layout.lines[0].slots[0].cursor,
            "range cells do not borrow the cursor style"
        );
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
