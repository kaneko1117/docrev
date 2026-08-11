use ratatui::DefaultTerminal;

/// Enters the alternate screen / raw mode and installs a panic hook that
/// restores the terminal.
pub fn init() -> std::io::Result<DefaultTerminal> {
    ratatui::try_init()
}

pub fn restore() {
    ratatui::restore();
}
