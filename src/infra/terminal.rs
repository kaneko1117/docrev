use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use ratatui::crossterm::execute;

/// Enters the alternate screen / raw mode with mouse capture, and installs
/// a panic hook that gives both back before ratatui's own restore runs.
pub fn init() -> std::io::Result<DefaultTerminal> {
    let terminal = ratatui::try_init()?;
    if let Err(e) = execute!(std::io::stdout(), EnableMouseCapture) {
        // never leave the shell in raw mode over a half-finished init
        ratatui::restore();
        return Err(e);
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(std::io::stdout(), DisableMouseCapture);
        previous(info);
    }));
    Ok(terminal)
}

pub fn restore() {
    let _ = execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
}
