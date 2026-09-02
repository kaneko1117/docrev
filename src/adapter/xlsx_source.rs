use std::collections::HashMap;
use std::path::Path;

use calamine::{Data, ExcelDateTime, ExcelDateTimeType, Range};

use crate::app::error::LoadError;
use crate::app::ports::DocumentSource;
use crate::domain::cell::CellValue;
use crate::domain::document::Document;
use crate::domain::sheet::{MergedRange, Rgb, Sheet, TextColor};
use crate::domain::workbook_comment::{WorkbookComment, WorkbookReply};
use crate::infra::datetime::{DateTimeKind, DateTimeParts};
use crate::infra::number_format::NumberFormat;
use crate::infra::{xlsx, xlsx_meta};

pub struct XlsxSource;

impl DocumentSource for XlsxSource {
    fn load(&self, path: &Path) -> Result<Document, LoadError> {
        let raw = xlsx::read_workbook(path).map_err(|e| match e {
            xlsx::ReadError::Open { .. } => LoadError::Open(e.to_string()),
            xlsx::ReadError::Sheet { .. } => LoadError::Sheet(e.to_string()),
        })?;
        let is_1904 = raw.is_1904;
        let raw = raw.sheets;
        let meta = xlsx_meta::read_meta(path);
        let (widths, styles, frozen) = (meta.widths, meta.styles, meta.frozen);
        let mut workbook_comments = xlsx_meta::workbook_comments(path).unwrap_or_default();
        // parse each format once per workbook
        let formats: Vec<Option<NumberFormat>> = styles
            .styles
            .iter()
            .map(|s| s.format.as_deref().map(NumberFormat::parse))
            .collect();
        let sheets = raw
            .into_iter()
            .map(|raw_sheet| {
                let cols = widths.get(&raw_sheet.name);
                let cells = styles.sheets.get(&raw_sheet.name);
                let native = workbook_comments
                    .remove(&raw_sheet.name)
                    .unwrap_or_default();
                let merges: Vec<MergedRange> = raw_sheet
                    .merges
                    .iter()
                    .map(|(start, end)| MergedRange {
                        start_row: start.0 as usize,
                        start_col: start.1 as usize,
                        end_row: end.0 as usize,
                        end_col: end.1 as usize,
                    })
                    .collect();
                let (frozen_rows, frozen_cols) =
                    frozen.get(&raw_sheet.name).copied().unwrap_or((0, 0));
                let formulas: HashMap<(usize, usize), String> = raw_sheet
                    .formulas
                    .into_iter()
                    .map(|((row, col), f)| ((row as usize, col as usize), f))
                    .collect();
                let sheet = to_sheet(
                    raw_sheet.name,
                    raw_sheet.cells,
                    cells,
                    &styles.styles,
                    &formats,
                    is_1904,
                )
                .with_merges(merges)
                .with_frozen(frozen_rows, frozen_cols)
                .with_formulas(formulas)
                .with_workbook_comments(native.into_iter().map(to_workbook_comment).collect());
                match cols {
                    Some(cols) => {
                        let expanded = expand_widths(cols, sheet.col_count());
                        sheet.with_col_widths(expanded)
                    }
                    None => sheet,
                }
            })
            .collect();
        Ok(Document::new(sheets))
    }

    /// mtime mixed with size; a missing file is `Some(0)`.
    fn revision(&self, path: &Path) -> Option<u64> {
        let metadata = match std::fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Some(0),
            Err(_) => return None,
        };
        let modified = metadata
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_millis() as u64;
        Some(modified.wrapping_mul(31).wrapping_add(metadata.len()))
    }
}

fn to_workbook_comment(raw: xlsx_meta::RawWorkbookComment) -> WorkbookComment {
    WorkbookComment {
        row: raw.row as usize,
        col: raw.col as usize,
        author: raw.author,
        body: raw.body,
        resolved: raw.resolved,
        replies: raw
            .replies
            .into_iter()
            .map(|(author, body)| WorkbookReply { author, body })
            .collect(),
    }
}

/// One entry per 0-based column, values as the file states them.
fn expand_widths(cols: &[xlsx_meta::ColumnWidth], col_count: usize) -> Vec<Option<f64>> {
    let mut widths = vec![None; col_count];
    for col in cols {
        // NaN/inf (reachable via parse()) must not overwrite an earlier valid width
        if !col.width.is_finite() {
            continue;
        }
        let from = col.min.saturating_sub(1) as usize;
        let to = (col.max as usize).min(col_count);
        for slot in widths.iter_mut().take(to).skip(from) {
            *slot = Some(col.width);
        }
    }
    widths
}

/// Pads with `Empty` up to the used range's offset so (0, 0) stays A1.
fn to_sheet(
    name: String,
    range: Range<Data>,
    cells: Option<&HashMap<(u32, u32), usize>>,
    styles: &[xlsx_meta::CellStyle],
    formats: &[Option<NumberFormat>],
    is_1904: bool,
) -> Sheet {
    let Some((start_row, start_col)) = range.start() else {
        return Sheet::new(name, Vec::new());
    };
    let mut rows: Vec<Vec<CellValue>> = Vec::with_capacity(start_row as usize + range.height());
    rows.resize_with(start_row as usize, Vec::new);
    let mut date_parts: HashMap<(usize, usize), DateTimeParts> = HashMap::new();
    for (r, raw) in range.rows().enumerate() {
        let mut row = vec![CellValue::Empty; start_col as usize];
        for (c, data) in raw.iter().enumerate() {
            if let Data::DateTime(dt) = data {
                let parts = to_parts(dt);
                date_parts.insert((start_row as usize + r, start_col as usize + c), parts);
                let text = parts.fallback_text();
                row.push(CellValue::DateTime {
                    raw: text.clone(),
                    text,
                });
            } else {
                row.push(to_cell(data));
            }
        }
        rows.push(row);
    }
    let (fills, text_colors) = match cells {
        Some(cells) => apply_styles(&mut rows, cells, styles, formats, &date_parts, is_1904),
        None => (HashMap::new(), HashMap::new()),
    };
    Sheet::new(name, rows)
        .with_fills(fills)
        .with_text_colors(text_colors)
}

type Fills = HashMap<(usize, usize), Rgb>;
type TextColors = HashMap<(usize, usize), TextColor>;

/// An unsupported date format keeps the fallback text; fills and colors are collected for empty cells too.
fn apply_styles(
    rows: &mut [Vec<CellValue>],
    cells: &HashMap<(u32, u32), usize>,
    styles: &[xlsx_meta::CellStyle],
    formats: &[Option<NumberFormat>],
    date_parts: &HashMap<(usize, usize), DateTimeParts>,
    is_1904: bool,
) -> (Fills, TextColors) {
    let mut fills = HashMap::new();
    let mut text_colors = HashMap::new();
    for (&(row, col), &idx) in cells {
        let (row, col) = (row as usize, col as usize);
        let Some(style) = styles.get(idx) else {
            continue;
        };
        if let Some((r, g, b)) = style.fill {
            fills.insert((row, col), Rgb { r, g, b });
        }
        // a `[Red]` code colors nothing unless the format rendered a value
        let mut named = None;
        if let Some(Some(format)) = formats.get(idx)
            && !format.is_general()
            && let Some(cell) = rows.get_mut(row).and_then(|r| r.get_mut(col))
        {
            match cell {
                // calamine does not know the ja builtin ids (27-36): those
                // date cells arrive as plain numbers
                CellValue::Number(value) if format.is_date() => {
                    if let Some(parts) = serial_parts(*value, is_1904) {
                        let formatted = format.format_datetime(&parts);
                        named = formatted.color;
                        *cell = CellValue::DateTime {
                            text: formatted.text,
                            raw: parts.fallback_text(),
                        };
                    }
                }
                CellValue::Number(value) => {
                    let formatted = format.format(*value);
                    named = formatted.color;
                    *cell = CellValue::FormattedNumber {
                        value: *value,
                        text: formatted.text,
                    };
                }
                CellValue::DateTime { text, .. } => {
                    if let Some(parts) = date_parts.get(&(row, col)) {
                        let formatted = format.format_datetime(parts);
                        named = formatted.color;
                        *text = formatted.text;
                    }
                }
                _ => {}
            }
        }
        // the format's named color beats the font color
        let literal = style.font.map(|(r, g, b)| Rgb { r, g, b });
        if let Some(color) = named
            .map(TextColor::Named)
            .or(literal.map(TextColor::Literal))
        {
            text_colors.insert((row, col), color);
        }
    }
    (fills, text_colors)
}

/// `Data::DateTime` is the caller's job.
fn to_cell(data: &Data) -> CellValue {
    match data {
        Data::Empty => CellValue::Empty,
        Data::String(s) => CellValue::Text(s.clone()),
        Data::Float(f) => CellValue::Number(*f),
        Data::Int(i) => CellValue::Number(*i as f64),
        Data::Bool(b) => CellValue::Bool(*b),
        Data::DateTime(dt) => {
            let text = to_parts(dt).fallback_text();
            CellValue::DateTime {
                raw: text.clone(),
                text,
            }
        }
        Data::DateTimeIso(s) => CellValue::DateTime {
            text: s.clone(),
            raw: s.clone(),
        },
        Data::DurationIso(s) => CellValue::Text(s.clone()),
        Data::Error(e) => CellValue::Error(e.to_string()),
    }
}

/// `None` outside Excel's serial range (0 to 9999-12-31).
fn serial_parts(value: f64, is_1904: bool) -> Option<DateTimeParts> {
    const MAX_SERIAL: f64 = 2_958_465.0;
    if !(0.0..=MAX_SERIAL).contains(&value) {
        return None;
    }
    Some(to_parts(&ExcelDateTime::new(
        value,
        ExcelDateTimeType::DateTime,
        is_1904,
    )))
}

/// calamine has already absorbed the 1904 epoch and the 1900 quirk; sub-second precision is dropped.
fn to_parts(dt: &ExcelDateTime) -> DateTimeParts {
    let (year, month, day, hour, minute, second, _milli) = dt.to_ymd_hms_milli();
    let serial = dt.as_f64();
    let kind = if dt.is_duration() {
        DateTimeKind::Duration
    } else if serial < 1.0 {
        DateTimeKind::TimeOnly
    } else {
        DateTimeKind::DateTime
    };
    DateTimeParts {
        year,
        month,
        day,
        hour,
        minute,
        second,
        serial,
        kind,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_pass_through_raw_and_non_finite_ones_are_absent() {
        let cols = vec![
            xlsx_meta::ColumnWidth {
                min: 1,
                max: 1,
                width: 10.0,
            },
            xlsx_meta::ColumnWidth {
                min: 1,
                max: 1,
                width: f64::NAN,
            },
            xlsx_meta::ColumnWidth {
                min: 3,
                max: 3,
                width: 18.5,
            },
        ];
        assert_eq!(expand_widths(&cols, 3), vec![Some(10.0), None, Some(18.5)]);
    }

    #[test]
    fn styles_format_numbers_collect_fills_and_resolve_text_colors() {
        use crate::domain::sheet::NamedColor;

        let mut rows = vec![vec![
            CellValue::Number(0.15),
            CellValue::Number(-1234.0),
            CellValue::Number(3.0),
            CellValue::Text("x".into()),
            CellValue::Empty,
            CellValue::Text("y".into()),
        ]];
        let styles = vec![
            xlsx_meta::CellStyle {
                format: Some("0%".into()),
                fill: Some((255, 255, 0)),
                font: None,
            },
            xlsx_meta::CellStyle {
                format: Some("#,##0;[Red]▲#,##0".into()),
                fill: None,
                font: Some((0, 0, 255)),
            },
            xlsx_meta::CellStyle {
                format: Some("General".into()),
                fill: None,
                font: Some((255, 255, 255)),
            },
            xlsx_meta::CellStyle {
                format: None,
                fill: Some((0, 128, 0)),
                font: Some((0, 0, 0)),
            },
        ];
        let formats: Vec<Option<NumberFormat>> = styles
            .iter()
            .map(|s| s.format.as_deref().map(NumberFormat::parse))
            .collect();
        let cells = [
            ((0u32, 0u32), 0usize),
            ((0, 1), 1),
            ((0, 2), 2),
            ((0, 3), 0),
            ((0, 4), 3),  // fill on an empty cell
            ((0, 5), 1),  // the [Red] format on a text cell
            ((8, 25), 0), // outside the grid: must not panic
        ]
        .into();
        let (fills, text_colors) =
            apply_styles(&mut rows, &cells, &styles, &formats, &HashMap::new(), false);
        assert_eq!(
            rows[0][0],
            CellValue::FormattedNumber {
                value: 0.15,
                text: "15%".into(),
            }
        );
        assert_eq!(
            rows[0][1],
            CellValue::FormattedNumber {
                value: -1234.0,
                text: "▲1,234".into(),
            }
        );
        assert_eq!(rows[0][2], CellValue::Number(3.0), "General stays raw");
        assert_eq!(rows[0][3], CellValue::Text("x".into()), "text is untouched");
        assert_eq!(
            fills.get(&(0, 0)),
            Some(&Rgb {
                r: 255,
                g: 255,
                b: 0
            })
        );
        assert_eq!(
            fills.get(&(0, 4)),
            Some(&Rgb { r: 0, g: 128, b: 0 }),
            "fills apply to empty cells too"
        );
        assert_eq!(fills.get(&(0, 1)), None);
        assert_eq!(
            text_colors.get(&(0, 1)),
            Some(&TextColor::Named(NamedColor::Red)),
            "the number format's named color beats the cell's font color"
        );
        assert_eq!(
            text_colors.get(&(0, 5)),
            Some(&TextColor::Literal(Rgb { r: 0, g: 0, b: 255 })),
            "the same [Red] format renders no number on a text cell, so the \
             font color applies"
        );
        assert_eq!(
            text_colors.get(&(0, 2)),
            Some(&TextColor::Literal(Rgb {
                r: 255,
                g: 255,
                b: 255
            })),
            "a real font color is inherited"
        );
        assert_eq!(
            text_colors.get(&(0, 4)),
            Some(&TextColor::Literal(Rgb { r: 0, g: 0, b: 0 })),
            "an explicit black on a non-default font is an author's choice \
             (the workbook default is filtered by font id in infra)"
        );
        assert_eq!(
            text_colors.get(&(0, 0)),
            None,
            "a format with no color and no font leaves the cell uncolored"
        );
    }

    fn date_cell(text: &str) -> CellValue {
        CellValue::DateTime {
            text: text.into(),
            raw: text.into(),
        }
    }

    #[test]
    fn date_cells_render_through_their_format_and_keep_raw() {
        let full = DateTimeParts {
            year: 2026,
            month: 8,
            day: 31,
            hour: 0,
            minute: 0,
            second: 0,
            serial: 46_265.0,
            kind: DateTimeKind::DateTime,
        };
        let time_only = DateTimeParts {
            year: 1899,
            month: 12,
            day: 31,
            hour: 13,
            minute: 5,
            second: 0,
            serial: 47_100.0 / 86_400.0,
            kind: DateTimeKind::TimeOnly,
        };
        let mut rows = vec![vec![
            date_cell("2026-08-31 00:00:00"),
            date_cell("13:05:00"),
            date_cell("2026-08-31 00:00:00"),
        ]];
        let styles = vec![
            xlsx_meta::CellStyle {
                format: Some("yyyy\"年\"m\"月\"d\"日\"(aaa)".into()),
                fill: None,
                font: None,
            },
            xlsx_meta::CellStyle {
                format: Some("mm:ss.00".into()),
                fill: None,
                font: None,
            },
            xlsx_meta::CellStyle {
                format: Some("0.00".into()),
                fill: None,
                font: None,
            },
        ];
        let formats: Vec<Option<NumberFormat>> = styles
            .iter()
            .map(|s| s.format.as_deref().map(NumberFormat::parse))
            .collect();
        let date_parts = HashMap::from([
            ((0usize, 0usize), full),
            ((0, 1), time_only),
            ((0, 2), full),
        ]);
        let cells = [((0u32, 0u32), 0usize), ((0, 1), 1), ((0, 2), 2)].into();
        apply_styles(&mut rows, &cells, &styles, &formats, &date_parts, false);
        assert_eq!(
            rows[0][0],
            CellValue::DateTime {
                text: "2026年8月31日(月)".into(),
                raw: "2026-08-31 00:00:00".into(),
            },
            "the format renders, raw keeps the machine-readable value"
        );
        assert_eq!(
            rows[0][1],
            date_cell("13:05:00"),
            "an unsupported format keeps the fallback — no epoch date leaks"
        );
        assert_eq!(
            rows[0][2],
            CellValue::DateTime {
                text: "46265.00".into(),
                raw: "2026-08-31 00:00:00".into(),
            },
            "a numeric format paints the serial value"
        );
    }

    #[test]
    fn a_number_wearing_a_date_format_is_promoted_to_a_date() {
        // 27-36 arrive as plain numbers
        let mut rows = vec![vec![
            CellValue::Number(46_265.0),
            CellValue::Number(-5.0),
            CellValue::Number(3.0e9),
        ]];
        let styles = vec![xlsx_meta::CellStyle {
            format: Some("[$-411]ggge\"年\"m\"月\"d\"日\"".into()),
            fill: None,
            font: None,
        }];
        let formats: Vec<Option<NumberFormat>> = styles
            .iter()
            .map(|s| s.format.as_deref().map(NumberFormat::parse))
            .collect();
        let cells = [((0u32, 0u32), 0usize), ((0, 1), 0), ((0, 2), 0)].into();
        apply_styles(&mut rows, &cells, &styles, &formats, &HashMap::new(), false);
        assert_eq!(
            rows[0][0],
            CellValue::DateTime {
                text: "令和8年8月31日".into(),
                raw: "2026-08-31 00:00:00".into(),
            },
            "promoted from the serial value"
        );
        assert_eq!(
            rows[0][1],
            CellValue::Number(-5.0),
            "a value no calendar can hold stays a number"
        );
        assert_eq!(
            rows[0][2],
            CellValue::Number(3.0e9),
            "beyond Excel's last date (9999-12-31) stays a number"
        );
    }

    #[test]
    fn a_general_sectioned_format_never_promotes_a_number() {
        let mut rows = vec![vec![CellValue::Number(46_265.0)]];
        let styles = vec![xlsx_meta::CellStyle {
            format: Some("General;[Red]-General".into()),
            fill: None,
            font: None,
        }];
        let formats: Vec<Option<NumberFormat>> = styles
            .iter()
            .map(|s| s.format.as_deref().map(NumberFormat::parse))
            .collect();
        let cells = [((0u32, 0u32), 0usize)].into();
        apply_styles(&mut rows, &cells, &styles, &formats, &HashMap::new(), false);
        assert_eq!(
            rows[0][0],
            CellValue::FormattedNumber {
                value: 46_265.0,
                text: "46265".into(),
            }
        );
    }

    #[test]
    fn to_parts_classifies_dates_times_and_durations() {
        use calamine::ExcelDateTimeType;

        let noon = ExcelDateTime::new(1.5, ExcelDateTimeType::DateTime, false);
        let parts = to_parts(&noon);
        assert_eq!(
            (parts.year, parts.month, parts.day),
            (1900, 1, 1),
            "calamine owns the serial-to-calendar conversion"
        );
        assert_eq!(parts.kind, DateTimeKind::DateTime);
        assert_eq!(parts.fallback_text(), "1900-01-01 12:00:00");

        let afternoon = ExcelDateTime::new(47_100.0 / 86_400.0, ExcelDateTimeType::DateTime, false);
        let parts = to_parts(&afternoon);
        assert_eq!(parts.kind, DateTimeKind::TimeOnly);
        assert_eq!(parts.fallback_text(), "13:05:00");

        let elapsed = ExcelDateTime::new(1.5, ExcelDateTimeType::TimeDelta, false);
        let parts = to_parts(&elapsed);
        assert_eq!(parts.kind, DateTimeKind::Duration);
        assert_eq!(parts.fallback_text(), "36:00:00");
    }
}
