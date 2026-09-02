use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;

use crate::domain::cell::CellValue;
use crate::domain::sheet::Sheet;

use super::grid::{GridView, Scroll, draw};

pub(crate) fn sheet_3x3() -> Sheet {
    Sheet::new(
        "売上",
        vec![
            vec![
                CellValue::Text("項目".into()),
                CellValue::Text("単価".into()),
                CellValue::Text("数量".into()),
            ],
            vec![
                CellValue::Text("りんご".into()),
                CellValue::Number(120.0),
                CellValue::Number(3.0),
            ],
            vec![
                CellValue::Text("みかん".into()),
                CellValue::Number(80.0),
                CellValue::Number(5.0),
            ],
        ],
    )
}

pub(crate) fn render_text(view: &GridView, scroll: &mut Scroll, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|f| {
            draw(f, view, scroll);
        })
        .unwrap();
    buffer_text(terminal.backend().buffer())
}

pub(crate) fn buffer_text(buffer: &Buffer) -> String {
    (0..buffer.area.height)
        .map(|y| {
            let mut line = String::new();
            let mut x = 0;
            // wide graphemes occupy a continuation cell — skip it
            while x < buffer.area.width {
                let symbol = buffer.cell((x, y)).map(|c| c.symbol()).unwrap_or(" ");
                line.push_str(symbol);
                x += unicode_width::UnicodeWidthStr::width(symbol).max(1) as u16;
            }
            line.trim_end().to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
