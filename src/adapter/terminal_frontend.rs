use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event as TermEvent, KeyCode, KeyEvent, KeyModifiers};

use crate::app::error::FrontendError;
use crate::app::viewer::{Event, Frontend, Viewer};
use crate::ui::grid::{self, GridView, Scroll};

pub struct TerminalFrontend {
    terminal: DefaultTerminal,
    scrolls: Vec<Scroll>,
    page_rows: usize,
}

impl TerminalFrontend {
    pub fn new(terminal: DefaultTerminal) -> Self {
        Self {
            terminal,
            scrolls: Vec::new(),
            page_rows: 1,
        }
    }
}

impl Frontend for TerminalFrontend {
    fn draw(&mut self, viewer: &Viewer) -> Result<(), FrontendError> {
        let Self {
            terminal,
            scrolls,
            page_rows,
        } = self;
        if scrolls.len() < viewer.sheet_count() {
            scrolls.resize(viewer.sheet_count(), Scroll::default());
        }
        let mut fallback = Scroll::default();
        let scroll = scrolls.get_mut(viewer.active()).unwrap_or(&mut fallback);
        let view = GridView {
            sheet: viewer.sheet(),
            sheet_names: viewer.sheet_names(),
            active: viewer.active(),
            cursor: viewer.cursor(),
        };
        terminal
            .draw(|frame| {
                *page_rows = frame.area().height.saturating_sub(grid::CHROME_ROWS).max(1) as usize;
                grid::draw(frame, &view, scroll);
            })
            .map_err(|e| FrontendError(e.to_string()))?;
        Ok(())
    }

    fn next_event(&mut self) -> Result<Event, FrontendError> {
        loop {
            match event::read().map_err(|e| FrontendError(e.to_string()))? {
                TermEvent::Key(key) if key.is_press() => {
                    return Ok(map_key(key, self.page_rows as isize));
                }
                TermEvent::Resize(..) => return Ok(Event::Noop),
                _ => {}
            }
        }
    }
}

fn map_key(key: KeyEvent, page: isize) -> Event {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('q') => Event::Quit,
        // raw mode turns Ctrl+C into a plain key event — without this, users
        // who don't know 'q' cannot exit
        KeyCode::Char('c') if ctrl => Event::Quit,
        KeyCode::Home if ctrl => Event::Top,
        KeyCode::End if ctrl => Event::Bottom,
        KeyCode::Up => Event::Move { rows: -1, cols: 0 },
        KeyCode::Down => Event::Move { rows: 1, cols: 0 },
        KeyCode::Left => Event::Move { rows: 0, cols: -1 },
        KeyCode::Right => Event::Move { rows: 0, cols: 1 },
        KeyCode::PageDown => Event::Move {
            rows: page,
            cols: 0,
        },
        KeyCode::PageUp => Event::Move {
            rows: -page,
            cols: 0,
        },
        KeyCode::Home => Event::RowStart,
        KeyCode::End => Event::RowEnd,
        KeyCode::Tab => Event::NextSheet,
        KeyCode::BackTab => Event::PrevSheet,
        _ => Event::Noop,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn maps_movement_keys() {
        assert_eq!(
            map_key(key(KeyCode::Down), 10),
            Event::Move { rows: 1, cols: 0 }
        );
        assert_eq!(
            map_key(key(KeyCode::Up), 10),
            Event::Move { rows: -1, cols: 0 }
        );
        assert_eq!(
            map_key(key(KeyCode::Right), 10),
            Event::Move { rows: 0, cols: 1 }
        );
        assert_eq!(
            map_key(key(KeyCode::PageDown), 10),
            Event::Move { rows: 10, cols: 0 }
        );
        assert_eq!(map_key(key(KeyCode::Home), 10), Event::RowStart);
        assert_eq!(map_key(key(KeyCode::End), 10), Event::RowEnd);
    }

    #[test]
    fn ctrl_home_and_end_jump_to_sheet_edges() {
        let ctrl_home = KeyEvent::new(KeyCode::Home, KeyModifiers::CONTROL);
        assert_eq!(map_key(ctrl_home, 10), Event::Top);
        let ctrl_end = KeyEvent::new(KeyCode::End, KeyModifiers::CONTROL);
        assert_eq!(map_key(ctrl_end, 10), Event::Bottom);
    }

    #[test]
    fn maps_sheet_and_quit_keys() {
        assert_eq!(map_key(key(KeyCode::Tab), 10), Event::NextSheet);
        assert_eq!(map_key(key(KeyCode::BackTab), 10), Event::PrevSheet);
        assert_eq!(map_key(key(KeyCode::Char('q')), 10), Event::Quit);
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(map_key(ctrl_c, 10), Event::Quit);
        assert_eq!(map_key(key(KeyCode::Char('x')), 10), Event::Noop);
    }

    #[test]
    fn vim_keys_are_not_bound() {
        for code in ['h', 'j', 'k', 'l', 'g', 'G'] {
            assert_eq!(map_key(key(KeyCode::Char(code)), 10), Event::Noop);
        }
    }
}
