use std::fmt;
use std::str::FromStr;

use ratatui::style::Color;

use crate::domain::number_format::FormatColor;

/// Which palette the viewer paints with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    /// A Google Sheets-flavored white canvas, the same on every terminal.
    #[default]
    Sheets,
    /// Leave the background alone and use the terminal's own 16 colors, so
    /// the viewer matches the surrounding shell (dark themes included).
    Terminal,
}

impl Theme {
    /// Reads `DOCREV_THEME`; an unknown value falls back to the default
    /// rather than refusing to start over a cosmetic setting.
    pub fn from_env() -> Self {
        std::env::var("DOCREV_THEME")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or_default()
    }

    pub(crate) fn palette(self) -> Palette {
        match self {
            Theme::Sheets => Palette {
                text: Color::Rgb(32, 33, 36),
                canvas_bg: Color::Rgb(255, 255, 255),
                header_bg: Color::Rgb(241, 243, 244),
                header_fg: Color::Rgb(95, 99, 104),
                selection_bg: Color::Rgb(210, 227, 252),
                marker_fg: Color::Rgb(242, 153, 0),
                notice_fg: Color::Rgb(217, 48, 37),
                gridline: Color::Rgb(218, 220, 224),
                user_fg: Color::Rgb(146, 64, 14),
                agent_fg: Color::Rgb(11, 87, 208),
                paint_workbook_colors: true,
            },
            // `Reset` means "whatever the terminal uses", and the named
            // colors follow the user's palette instead of fighting it.
            Theme::Terminal => Palette {
                text: Color::Reset,
                canvas_bg: Color::Reset,
                header_bg: Color::Reset,
                header_fg: Color::DarkGray,
                selection_bg: Color::Blue,
                marker_fg: Color::Yellow,
                notice_fg: Color::Red,
                gridline: Color::DarkGray,
                user_fg: Color::Yellow,
                agent_fg: Color::Cyan,
                // workbook fills and font colors are absolute RGB tuned for
                // white paper; on an unknown background they can vanish
                paint_workbook_colors: false,
            },
        }
    }
}

impl FromStr for Theme {
    type Err = UnknownTheme;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "sheets" => Ok(Theme::Sheets),
            "terminal" => Ok(Theme::Terminal),
            other => Err(UnknownTheme(other.to_string())),
        }
    }
}

#[derive(Debug)]
pub struct UnknownTheme(String);

impl fmt::Display for UnknownTheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unknown theme \"{}\" (expected sheets or terminal)",
            self.0
        )
    }
}

impl std::error::Error for UnknownTheme {}

impl PartialEq for UnknownTheme {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

pub(crate) struct Palette {
    pub text: Color,
    pub canvas_bg: Color,
    pub header_bg: Color,
    pub header_fg: Color,
    pub selection_bg: Color,
    pub marker_fg: Color,
    pub notice_fg: Color,
    pub gridline: Color,
    pub user_fg: Color,
    pub agent_fg: Color,
    /// Whether cell fills and font colors from the workbook are painted.
    pub paint_workbook_colors: bool,
}

impl Palette {
    /// Number-format colors (`[Red]` sections). The Sheets palette darkens
    /// yellow and white, which would be unreadable on white paper; the
    /// terminal palette hands them to the user's own 16 colors.
    pub fn format_fg(&self, color: FormatColor) -> Color {
        if self.canvas_bg == Color::Reset {
            return match color {
                FormatColor::Red => Color::Red,
                FormatColor::Blue => Color::Blue,
                FormatColor::Green => Color::Green,
                FormatColor::Yellow => Color::Yellow,
                FormatColor::Magenta => Color::Magenta,
                FormatColor::Cyan => Color::Cyan,
                FormatColor::Black => Color::Black,
                FormatColor::White => Color::White,
            };
        }
        match color {
            FormatColor::Red => Color::Rgb(217, 48, 37),
            FormatColor::Blue => Color::Rgb(11, 87, 208),
            FormatColor::Green => Color::Rgb(19, 115, 51),
            FormatColor::Yellow => Color::Rgb(178, 138, 0),
            FormatColor::Magenta => Color::Rgb(168, 37, 168),
            FormatColor::Cyan => Color::Rgb(0, 131, 143),
            FormatColor::Black => self.text,
            FormatColor::White => Color::Rgb(128, 134, 139),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_both_themes_case_insensitively() {
        assert_eq!("sheets".parse(), Ok(Theme::Sheets));
        assert_eq!(" Terminal ".parse(), Ok(Theme::Terminal));
        assert!("dark".parse::<Theme>().is_err());
    }

    #[test]
    fn the_error_names_the_valid_values() {
        let err = "dark".parse::<Theme>().unwrap_err().to_string();
        assert!(err.contains("dark"), "{err}");
        assert!(err.contains("sheets") && err.contains("terminal"), "{err}");
    }

    #[test]
    fn sheets_paints_its_own_canvas_terminal_does_not() {
        let sheets = Theme::Sheets.palette();
        assert_ne!(sheets.canvas_bg, Color::Reset);
        assert!(sheets.paint_workbook_colors);

        let terminal = Theme::Terminal.palette();
        assert_eq!(
            terminal.canvas_bg,
            Color::Reset,
            "the user's background shows through"
        );
        assert!(
            !terminal.paint_workbook_colors,
            "workbook RGB assumes white paper"
        );
    }

    #[test]
    fn format_colors_follow_the_palette() {
        assert!(matches!(
            Theme::Sheets.palette().format_fg(FormatColor::Red),
            Color::Rgb(..)
        ));
        assert_eq!(
            Theme::Terminal.palette().format_fg(FormatColor::Red),
            Color::Red,
            "named colors follow the terminal's own palette"
        );
    }
}
