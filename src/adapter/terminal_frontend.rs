use std::time::Duration;

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event as TermEvent, KeyCode, KeyEvent, KeyModifiers};

use crate::app::error::FrontendError;
use crate::app::viewer::{EditTarget, Event, Frontend, Mode, Viewer};
use crate::domain::anchor::Anchor;
use crate::ui::grid::{self, EditorView, GridView, PickerItem, PickerView, Scroll};
use crate::ui::theme::Theme;

/// How long the viewer waits for input before checking for outside changes.
const TICK: Duration = Duration::from_millis(500);

/// Which key table applies — remembered at draw time for the next event.
#[derive(Clone, Copy, PartialEq)]
enum InputMode {
    Grid,
    Editing,
    Picker,
}

pub struct TerminalFrontend {
    terminal: DefaultTerminal,
    theme: Theme,
    scrolls: Vec<Scroll>,
    page_rows: usize,
    input_mode: InputMode,
}

impl TerminalFrontend {
    pub fn new(terminal: DefaultTerminal, theme: Theme) -> Self {
        Self {
            terminal,
            theme,
            scrolls: Vec::new(),
            page_rows: 1,
            input_mode: InputMode::Grid,
        }
    }
}

impl Frontend for TerminalFrontend {
    fn draw(&mut self, viewer: &Viewer) -> Result<(), FrontendError> {
        let Self {
            terminal,
            theme,
            scrolls,
            page_rows,
            input_mode,
        } = self;
        if scrolls.len() < viewer.sheet_count() {
            scrolls.resize(viewer.sheet_count(), Scroll::default());
        }
        let mut fallback = Scroll::default();
        let scroll = scrolls.get_mut(viewer.active()).unwrap_or(&mut fallback);
        let (row, col) = viewer.cursor();
        let address = Anchor::cell(viewer.sheet().name(), row as u32, col as u32).cell_ref();
        let editor = match viewer.mode() {
            Mode::Editing { target, buffer } => Some(EditorView {
                title: match target {
                    EditTarget::NewThread => format!(" Comment on {address} "),
                    EditTarget::Reply { .. } => format!(" Reply on {address} "),
                },
                buffer,
            }),
            _ => None,
        };
        let picker = viewer.picker_state().map(|state| {
            let names = viewer.sheet_names();
            let counts = viewer.unresolved_counts();
            PickerView {
                query: state.query.to_string(),
                selected: state.selected,
                total: viewer.sheet_count(),
                items: state
                    .candidates
                    .iter()
                    .map(|&i| PickerItem {
                        name: names[i].to_string(),
                        count: counts[i],
                        active: i == viewer.active(),
                    })
                    .collect(),
            }
        });
        *input_mode = match viewer.mode() {
            Mode::Grid => InputMode::Grid,
            Mode::Editing { .. } => InputMode::Editing,
            Mode::SheetPicker { .. } => InputMode::Picker,
        };
        let view = GridView {
            sheet: viewer.sheet(),
            sheet_names: viewer.sheet_names(),
            active: viewer.active(),
            cursor: viewer.cursor(),
            markers: viewer.unresolved_on_active_sheet().into_iter().collect(),
            notice: viewer.notice(),
            thread: viewer.thread_at_cursor(),
            editor,
            picker,
            theme: *theme,
            col_widths: (0..viewer.sheet().col_count())
                .map(|c| {
                    viewer
                        .sheet()
                        .col_width(c)
                        .map(usize::from)
                        .unwrap_or(grid::DEFAULT_CELL_WIDTH)
                })
                .collect(),
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
            // waking up regularly is what lets agent replies appear without
            // a keypress; the tick itself costs one `stat` upstream
            if !event::poll(TICK).map_err(|e| FrontendError(e.to_string()))? {
                return Ok(Event::Tick);
            }
            match event::read().map_err(|e| FrontendError(e.to_string()))? {
                TermEvent::Key(key) if key.is_press() => {
                    return Ok(map_key(key, self.page_rows as isize, self.input_mode));
                }
                TermEvent::Resize(..) => return Ok(Event::Noop),
                _ => {}
            }
        }
    }
}

fn map_key(key: KeyEvent, page: isize, mode: InputMode) -> Event {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    if mode == InputMode::Editing {
        return match key.code {
            KeyCode::Esc => Event::CancelEdit,
            // raw mode disables XOFF flow control, so Ctrl+S arrives as a key
            KeyCode::Char('s') if ctrl => Event::Submit,
            KeyCode::Enter => Event::Newline,
            KeyCode::Backspace => Event::Backspace,
            KeyCode::Char(c) if !ctrl => Event::Insert(c),
            _ => Event::Noop,
        };
    }
    if mode == InputMode::Picker {
        return match key.code {
            KeyCode::Esc => Event::CancelEdit,
            KeyCode::Enter => Event::Submit,
            KeyCode::Backspace => Event::Backspace,
            KeyCode::Up => Event::Move { rows: -1, cols: 0 },
            KeyCode::Down => Event::Move { rows: 1, cols: 0 },
            KeyCode::Char('c') if ctrl => Event::Quit,
            KeyCode::Char(c) if !ctrl => Event::Insert(c),
            _ => Event::Noop,
        };
    }
    match key.code {
        KeyCode::Char('q') => Event::Quit,
        // raw mode turns Ctrl+C into a plain key event — without this, users
        // who don't know 'q' cannot exit
        KeyCode::Char('c') if ctrl => Event::Quit,
        KeyCode::Char('c') => Event::StartComment,
        KeyCode::Char('r') => Event::StartReply,
        // Excel's Go To key, plus F5 for the same muscle memory
        KeyCode::Char('g') if ctrl => Event::OpenSheetPicker,
        KeyCode::F(5) => Event::OpenSheetPicker,
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

    fn map_key_grid(key: KeyEvent, page: isize) -> Event {
        map_key(key, page, InputMode::Grid)
    }

    fn map_key_editing(key: KeyEvent) -> Event {
        map_key(key, 10, InputMode::Editing)
    }

    fn map_key_picker(key: KeyEvent) -> Event {
        map_key(key, 10, InputMode::Picker)
    }

    #[test]
    fn grid_mode_maps_comment_keys() {
        assert_eq!(
            map_key_grid(key(KeyCode::Char('c')), 10),
            Event::StartComment
        );
        assert_eq!(map_key_grid(key(KeyCode::Char('r')), 10), Event::StartReply);
    }

    #[test]
    fn editing_mode_maps_editor_keys() {
        assert_eq!(map_key_editing(key(KeyCode::Char('q'))), Event::Insert('q'));
        assert_eq!(map_key_editing(key(KeyCode::Char('c'))), Event::Insert('c'));
        assert_eq!(map_key_editing(key(KeyCode::Enter)), Event::Newline);
        assert_eq!(map_key_editing(key(KeyCode::Backspace)), Event::Backspace);
        assert_eq!(map_key_editing(key(KeyCode::Esc)), Event::CancelEdit);
        let ctrl_s = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL);
        assert_eq!(map_key_editing(ctrl_s), Event::Submit);
        assert_eq!(
            map_key_editing(key(KeyCode::Up)),
            Event::Noop,
            "navigation is off while editing"
        );
    }

    #[test]
    fn maps_movement_keys() {
        assert_eq!(
            map_key_grid(key(KeyCode::Down), 10),
            Event::Move { rows: 1, cols: 0 }
        );
        assert_eq!(
            map_key_grid(key(KeyCode::Up), 10),
            Event::Move { rows: -1, cols: 0 }
        );
        assert_eq!(
            map_key_grid(key(KeyCode::Right), 10),
            Event::Move { rows: 0, cols: 1 }
        );
        assert_eq!(
            map_key_grid(key(KeyCode::PageDown), 10),
            Event::Move { rows: 10, cols: 0 }
        );
        assert_eq!(map_key_grid(key(KeyCode::Home), 10), Event::RowStart);
        assert_eq!(map_key_grid(key(KeyCode::End), 10), Event::RowEnd);
    }

    #[test]
    fn ctrl_home_and_end_jump_to_sheet_edges() {
        let ctrl_home = KeyEvent::new(KeyCode::Home, KeyModifiers::CONTROL);
        assert_eq!(map_key_grid(ctrl_home, 10), Event::Top);
        let ctrl_end = KeyEvent::new(KeyCode::End, KeyModifiers::CONTROL);
        assert_eq!(map_key_grid(ctrl_end, 10), Event::Bottom);
    }

    #[test]
    fn picker_mode_maps_filter_and_selection_keys() {
        assert_eq!(map_key_picker(key(KeyCode::Char('q'))), Event::Insert('q'));
        assert_eq!(
            map_key_picker(key(KeyCode::Char('達'))),
            Event::Insert('達')
        );
        assert_eq!(map_key_picker(key(KeyCode::Backspace)), Event::Backspace);
        assert_eq!(map_key_picker(key(KeyCode::Esc)), Event::CancelEdit);
        assert_eq!(map_key_picker(key(KeyCode::Enter)), Event::Submit);
        assert_eq!(
            map_key_picker(key(KeyCode::Up)),
            Event::Move { rows: -1, cols: 0 }
        );
        assert_eq!(
            map_key_picker(key(KeyCode::Down)),
            Event::Move { rows: 1, cols: 0 }
        );
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(map_key_picker(ctrl_c), Event::Quit, "the exit hatch stays");
        assert_eq!(map_key_picker(key(KeyCode::Tab)), Event::Noop);
    }

    #[test]
    fn ctrl_g_and_f5_open_the_sheet_picker() {
        let ctrl_g = KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL);
        assert_eq!(map_key_grid(ctrl_g, 10), Event::OpenSheetPicker);
        assert_eq!(map_key_grid(key(KeyCode::F(5)), 10), Event::OpenSheetPicker);
        assert_eq!(
            map_key_editing(key(KeyCode::F(5))),
            Event::Noop,
            "not while composing a comment"
        );
    }

    #[test]
    fn maps_sheet_and_quit_keys() {
        assert_eq!(map_key_grid(key(KeyCode::Tab), 10), Event::NextSheet);
        assert_eq!(map_key_grid(key(KeyCode::BackTab), 10), Event::PrevSheet);
        assert_eq!(map_key_grid(key(KeyCode::Char('q')), 10), Event::Quit);
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(map_key_grid(ctrl_c, 10), Event::Quit);
        assert_eq!(map_key_grid(key(KeyCode::Char('x')), 10), Event::Noop);
    }

    #[test]
    fn vim_keys_are_not_bound() {
        for code in ['h', 'j', 'k', 'l', 'g', 'G'] {
            assert_eq!(map_key_grid(key(KeyCode::Char(code)), 10), Event::Noop);
        }
    }
}
