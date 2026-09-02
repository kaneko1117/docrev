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

pub(crate) fn range_selected(p: &Palette) -> Style {
    if p.reverse_selection {
        return canvas(p).add_modifier(Modifier::REVERSED | Modifier::DIM);
    }
    Style::new().bg(p.range_bg).fg(p.text)
}

pub(crate) fn chrome(p: &Palette) -> Style {
    if p.dim_chrome {
        canvas(p).add_modifier(Modifier::DIM)
    } else {
        header(p)
    }
}

pub(crate) fn ruled(p: &Palette, style: Style) -> Style {
    if p.dim_chrome {
        return style.add_modifier(Modifier::UNDERLINED);
    }
    style
        .add_modifier(Modifier::UNDERLINED)
        .underline_color(p.gridline)
}

pub(crate) fn filled_canvas(p: &Palette, fill: Option<Rgb>) -> Style {
    match fill.filter(|_| p.paint_workbook_colors) {
        Some(f) => canvas(p).bg(Color::Rgb(f.r, f.g, f.b)),
        None => canvas(p),
    }
}

pub(crate) fn dialog(p: &Palette) -> Style {
    Style::new().bg(p.header_bg).fg(p.text)
}

pub(crate) fn freeze_ruled(p: &Palette, style: Style) -> Style {
    if p.dim_chrome {
        return style
            .add_modifier(Modifier::UNDERLINED)
            .remove_modifier(Modifier::DIM);
    }
    style
        .add_modifier(Modifier::UNDERLINED)
        .underline_color(p.header_fg)
}

pub(crate) fn freeze_gridline(p: &Palette) -> Style {
    if p.dim_chrome {
        return canvas(p);
    }
    canvas(p).fg(p.header_fg)
}

pub(crate) fn note_corner(p: &Palette) -> Style {
    if p.dim_chrome {
        return canvas(p).fg(p.notice_fg).add_modifier(Modifier::REVERSED);
    }
    Style::new().bg(p.notice_fg).fg(p.canvas_bg)
}
