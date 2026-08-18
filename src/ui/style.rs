//! Shared style helpers: how each palette role maps to a ratatui `Style`.

use ratatui::style::{Color, Modifier, Style};

use crate::domain::sheet::Rgb;

use super::theme::Palette;

pub(crate) fn canvas(p: &Palette) -> Style {
    Style::new().bg(p.canvas_bg).fg(p.text)
}

pub(crate) fn header(p: &Palette) -> Style {
    Style::new().bg(p.header_bg).fg(p.header_fg)
}

pub(crate) fn gridline_style(p: &Palette) -> Style {
    if p.dim_chrome {
        canvas(p).add_modifier(Modifier::DIM)
    } else {
        canvas(p).fg(p.gridline)
    }
}

pub(crate) fn selected(p: &Palette) -> Style {
    if p.reverse_selection {
        return canvas(p).add_modifier(Modifier::REVERSED);
    }
    Style::new().bg(p.selection_bg).fg(p.text)
}

pub(crate) fn chrome(p: &Palette) -> Style {
    if p.dim_chrome {
        canvas(p).add_modifier(Modifier::DIM)
    } else {
        header(p)
    }
}

/// Horizontal gridlines without spending screen rows: a colored underline.
pub(crate) fn ruled(p: &Palette, style: Style) -> Style {
    if p.dim_chrome {
        return style.add_modifier(Modifier::UNDERLINED);
    }
    style
        .add_modifier(Modifier::UNDERLINED)
        .underline_color(p.gridline)
}

/// Canvas painted with the workbook fill when the cell has one — only for
/// palettes whose background the workbook's absolute colors were meant for.
pub(crate) fn filled_canvas(p: &Palette, fill: Option<Rgb>) -> Style {
    match fill.filter(|_| p.paint_workbook_colors) {
        Some(f) => canvas(p).bg(Color::Rgb(f.r, f.g, f.b)),
        None => canvas(p),
    }
}

/// The dialog surface: the header gray, so it reads as a layer above the
/// white cells instead of blending into them. On the terminal palette both
/// colors are `Reset` and the scrim alone carries the depth.
pub(crate) fn dialog(p: &Palette) -> Style {
    Style::new().bg(p.header_bg).fg(p.text)
}
