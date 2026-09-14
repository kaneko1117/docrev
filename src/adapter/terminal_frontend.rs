use std::io::Write;
use std::time::Duration;

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{
    self, Event as TermEvent, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};

use crate::app::error::FrontendError;
use crate::app::viewer::{EditTarget, Event, Frontend, Mode, Shown, Viewer};
use crate::domain::anchor::Anchor;
use crate::domain::sheet::Sheet;
use crate::domain::text_document::TextDocument;
use crate::infra::clipboard;
use crate::ui::grid::{
    self, EditorView, GridView, Hit, HitMap, NoteView, NotesView, PickerItem, PickerView, Scroll,
    SearchView,
};
use crate::ui::text_pane::{self, TextHits, TextView};
use crate::ui::theme::Theme;

/// Input wait before a `Tick`.
const TICK: Duration = Duration::from_millis(500);

/// Which key table applies; remembered at draw time.
#[derive(Clone, Copy, PartialEq)]
enum InputMode {
    Grid,
    Editing,
    Picker,
    Search,
    Notes,
}

pub struct TerminalFrontend {
    terminal: DefaultTerminal,
    theme: Theme,
    scrolls: Vec<Scroll>,
    /// First line on screen of a text document.
    text_top: usize,
    page_rows: usize,
    input_mode: InputMode,
    hits: Hits,
    drag: DragState,
}

impl TerminalFrontend {
    pub fn new(terminal: DefaultTerminal, theme: Theme) -> Self {
        Self {
            terminal,
            theme,
            scrolls: Vec::new(),
            text_top: 0,
            page_rows: 1,
            input_mode: InputMode::Grid,
            hits: Hits::Grid(HitMap::default()),
            drag: DragState::default(),
        }
    }
}

/// `dragged` turns true once the drag reaches another cell, or on text a line other than the press.
#[derive(Default)]
struct DragState {
    pressed: bool,
    dragged: bool,
    /// The 0-based line a press on text started on.
    pressed_line: Option<usize>,
}

/// The click targets of the last drawn screen.
enum Hits {
    Grid(HitMap),
    Text(TextHits),
}

fn map_mouse(hits: &Hits, mode: InputMode, drag: &mut DragState, mouse: MouseEvent) -> Event {
    let shift = mouse.modifiers.contains(KeyModifiers::SHIFT);
    let step: isize = match mode {
        InputMode::Picker | InputMode::Search => 1,
        _ => 3,
    };
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => match hits {
            Hits::Text(text) => match text.at(mouse.column, mouse.row) {
                Some(line) => {
                    drag.pressed = true;
                    drag.dragged = false;
                    drag.pressed_line = Some(line);
                    Event::SelectLine(line)
                }
                None => Event::Noop,
            },
            Hits::Grid(grid) => match grid.at(mouse.column, mouse.row) {
                Some(Hit::Cell { row, col }) => {
                    drag.pressed = true;
                    drag.dragged = false;
                    Event::SelectCell { row, col }
                }
                Some(Hit::Tab(index)) => Event::SelectSheet(index),
                Some(Hit::PrevTabs) => Event::PrevSheet,
                Some(Hit::NextTabs) => Event::NextSheet,
                None => Event::Noop,
            },
        },
        MouseEventKind::Drag(MouseButton::Left) if drag.pressed => match hits {
            Hits::Text(text) => match text.at(mouse.column, mouse.row) {
                Some(line) => {
                    drag.dragged |= drag.pressed_line != Some(line);
                    Event::DragToLine(line)
                }
                None => Event::Noop,
            },
            Hits::Grid(grid) => match grid.at(mouse.column, mouse.row) {
                Some(Hit::Cell { row, col }) => {
                    drag.dragged = true;
                    Event::DragTo { row, col }
                }
                _ => Event::Noop,
            },
        },
        MouseEventKind::Up(MouseButton::Left) if drag.pressed => {
            drag.pressed = false;
            // every release must reach the viewer or the selection outlives the press
            Event::DragEnd {
                copy: std::mem::take(&mut drag.dragged),
            }
        }
        MouseEventKind::ScrollUp => Event::Move {
            rows: if shift { 0 } else { -step },
            cols: if shift { -step } else { 0 },
        },
        MouseEventKind::ScrollDown => Event::Move {
            rows: if shift { 0 } else { step },
            cols: if shift { step } else { 0 },
        },
        MouseEventKind::ScrollLeft => Event::Move {
            rows: 0,
            cols: -step,
        },
        MouseEventKind::ScrollRight => Event::Move {
            rows: 0,
            cols: step,
        },
        _ => Event::Noop,
    }
}

impl TerminalFrontend {
    fn draw_grid(
        &mut self,
        viewer: &Viewer,
        sheet: &Sheet,
        editor: Option<EditorView>,
    ) -> Result<(), FrontendError> {
        let Self {
            terminal,
            theme,
            scrolls,
            page_rows,
            hits,
            ..
        } = self;
        if scrolls.len() < viewer.sheet_count() {
            scrolls.resize(viewer.sheet_count(), Scroll::default());
        }
        let mut fallback = Scroll::default();
        let scroll = scrolls.get_mut(viewer.active()).unwrap_or(&mut fallback);
        let picker = viewer.picker_state().map(|state| {
            let names = viewer.sheet_names();
            let counts = viewer.unresolved_counts();
            PickerView {
                query: state.query.to_string(),
                selected: state.selected,
                total: viewer.shown_sheets().len(),
                items: state
                    .candidates
                    .iter()
                    .filter_map(|&i| {
                        Some(PickerItem {
                            name: names.get(i)?.to_string(),
                            count: counts.get(i).copied()?,
                            active: i == viewer.active(),
                        })
                    })
                    .collect(),
            }
        });
        let search = viewer.search_state().map(|state| SearchView {
            query: state.query.to_string(),
            current: state.current,
            total: state.total,
        });
        let notes_view = viewer.notes_state().map(|(comments, scroll)| {
            let (row, col) = viewer.cursor();
            NotesView {
                cell_ref: Anchor::a1(row as u32, col as u32),
                comments: comments
                    .iter()
                    .map(|c| NoteView {
                        author: c.author.clone(),
                        body: c.body.clone(),
                        resolved: c.resolved,
                        replies: c
                            .replies
                            .iter()
                            .map(|r| (r.author.clone(), r.body.clone()))
                            .collect(),
                    })
                    .collect(),
                scroll,
            }
        });
        let names = viewer.sheet_names();
        let view = GridView {
            sheet,
            tabs: viewer
                .shown_sheets()
                .into_iter()
                .filter_map(|i| Some((i, *names.get(i)?)))
                .collect(),
            active: viewer.active(),
            cursor: viewer.cursor(),
            markers: viewer.unresolved_on_active_sheet().into_iter().collect(),
            notes: viewer.workbook_comment_cells().into_iter().collect(),
            notes_view,
            notice: viewer.notice(),
            thread: viewer.thread_at_cursor(),
            editor,
            picker,
            search,
            selection: viewer.selection(),
            theme: *theme,
            col_widths: (0..sheet.col_count()).map(|c| sheet.col_width(c)).collect(),
        };
        terminal
            .draw(|frame| {
                *page_rows = frame.area().height.saturating_sub(grid::CHROME_ROWS).max(1) as usize;
                *hits = Hits::Grid(grid::draw(frame, &view, scroll));
            })
            .map(|_| ())
            .map_err(|e| FrontendError(e.to_string()))
    }

    fn draw_text(
        &mut self,
        viewer: &Viewer,
        document: &TextDocument,
        editor: Option<EditorView>,
    ) -> Result<(), FrontendError> {
        let Self {
            terminal,
            theme,
            text_top,
            page_rows,
            hits,
            ..
        } = self;
        let name = viewer
            .path()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("(text)");
        let search = viewer.search_state().map(|state| SearchView {
            query: state.query.to_string(),
            current: state.current,
            total: state.total,
        });
        let view = TextView {
            name,
            document,
            cursor: viewer.cursor().0,
            markers: viewer.unresolved_lines().into_iter().collect(),
            notice: viewer.notice(),
            thread: viewer.thread_at_cursor(),
            editor,
            search,
            selection: viewer
                .selection()
                .map(|((a, _), (b, _))| (a.min(b), a.max(b))),
            theme: *theme,
        };
        terminal
            .draw(|frame| {
                *page_rows = frame
                    .area()
                    .height
                    .saturating_sub(text_pane::CHROME_ROWS)
                    .max(1) as usize;
                *hits = Hits::Text(text_pane::draw(frame, &view, text_top));
            })
            .map(|_| ())
            .map_err(|e| FrontendError(e.to_string()))
    }
}

impl Frontend for TerminalFrontend {
    fn draw(&mut self, viewer: &Viewer) -> Result<(), FrontendError> {
        self.input_mode = match viewer.mode() {
            Mode::Grid => InputMode::Grid,
            Mode::Editing { .. } => InputMode::Editing,
            Mode::SheetPicker { .. } => InputMode::Picker,
            Mode::Search { .. } => InputMode::Search,
            Mode::Notes { .. } => InputMode::Notes,
        };
        let editor = match viewer.mode() {
            Mode::Editing { target, at, buffer } => Some(EditorView {
                kind: match target {
                    EditTarget::NewThread => grid::EditorKind::Comment,
                    EditTarget::Reply => grid::EditorKind::Reply,
                },
                address: at.position(),
                buffer,
            }),
            _ => None,
        };
        match viewer.shown() {
            Shown::Sheet(sheet) => self.draw_grid(viewer, sheet, editor),
            Shown::Text(document) => self.draw_text(viewer, document, editor),
        }
    }

    fn next_event(&mut self) -> Result<Event, FrontendError> {
        loop {
            if !event::poll(TICK).map_err(|e| FrontendError(e.to_string()))? {
                return Ok(Event::Tick);
            }
            match event::read().map_err(|e| FrontendError(e.to_string()))? {
                TermEvent::Key(key) if key.is_press() => {
                    return Ok(map_key(key, self.page_rows as isize, self.input_mode));
                }
                TermEvent::Mouse(mouse) => {
                    return Ok(map_mouse(
                        &self.hits,
                        self.input_mode,
                        &mut self.drag,
                        mouse,
                    ));
                }
                TermEvent::Resize(..) => return Ok(Event::Noop),
                _ => {}
            }
        }
    }

    fn copy(&mut self, text: &str) -> Result<(), FrontendError> {
        let mut out = std::io::stdout();
        out.write_all(clipboard::osc52(text).as_bytes())
            .and_then(|()| out.flush())
            .map_err(|e| FrontendError(e.to_string()))
    }
}

fn map_key(key: KeyEvent, page: isize, mode: InputMode) -> Event {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    if mode == InputMode::Editing {
        return match key.code {
            KeyCode::Esc => Event::CancelEdit,
            // raw mode disables XOFF, so Ctrl+S arrives as a key
            KeyCode::Char('s') if ctrl => Event::Submit,
            KeyCode::Enter => Event::Newline,
            KeyCode::Backspace => Event::Backspace,
            KeyCode::Char(c) if !ctrl => Event::Insert(c),
            // a pasted tab arrives as the Tab key and must not switch sheets mid-paste
            KeyCode::Tab => Event::Insert(' '),
            KeyCode::BackTab => Event::Noop,
            code => movement(code, ctrl, page).unwrap_or(Event::Noop),
        };
    }
    if mode == InputMode::Notes {
        return match key.code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('n') => Event::CancelEdit,
            KeyCode::Up => Event::Move { rows: -1, cols: 0 },
            KeyCode::Down => Event::Move { rows: 1, cols: 0 },
            KeyCode::Char('q') => Event::Quit,
            KeyCode::Char('c') if ctrl => Event::Quit,
            _ => Event::Noop,
        };
    }
    if mode == InputMode::Picker || mode == InputMode::Search {
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
        // raw mode turns Ctrl+C into a plain key event
        KeyCode::Char('c') if ctrl => Event::Quit,
        KeyCode::Char('c') => Event::StartComment,
        KeyCode::Char('g') if ctrl => Event::OpenSheetPicker,
        KeyCode::F(5) => Event::OpenSheetPicker,
        KeyCode::Char('f') if ctrl => Event::OpenSearch,
        KeyCode::Char('n') => Event::OpenNotes,
        code => movement(code, ctrl, page).unwrap_or(Event::Noop),
    }
}

/// The cursor keys, shared by browsing and the comment editor.
fn movement(code: KeyCode, ctrl: bool, page: isize) -> Option<Event> {
    Some(match code {
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
        _ => return None,
    })
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
        assert_eq!(map_key_grid(key(KeyCode::Char('r')), 10), Event::Noop);
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
            Event::Move { rows: -1, cols: 0 },
            "the cursor still moves while editing"
        );
        assert_eq!(
            map_key_editing(key(KeyCode::PageDown)),
            Event::Move { rows: 10, cols: 0 }
        );
        assert_eq!(map_key_editing(key(KeyCode::Home)), Event::RowStart);
        let ctrl_end = KeyEvent::new(KeyCode::End, KeyModifiers::CONTROL);
        assert_eq!(map_key_editing(ctrl_end), Event::Bottom);
        assert_eq!(
            map_key_editing(key(KeyCode::Tab)),
            Event::Insert(' '),
            "a pasted tab stays in the text"
        );
        assert_eq!(map_key_editing(key(KeyCode::BackTab)), Event::Noop);
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
    fn ctrl_f_opens_search_and_search_keys_mirror_the_picker() {
        let ctrl_f = KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL);
        assert_eq!(map_key_grid(ctrl_f, 10), Event::OpenSearch);
        assert_eq!(
            map_key(ctrl_f, 10, InputMode::Editing),
            Event::Noop,
            "not while composing a comment"
        );

        let search = |code| map_key(key(code), 10, InputMode::Search);
        assert_eq!(search(KeyCode::Char('q')), Event::Insert('q'));
        assert_eq!(search(KeyCode::Esc), Event::CancelEdit);
        assert_eq!(search(KeyCode::Enter), Event::Submit);
        assert_eq!(search(KeyCode::Down), Event::Move { rows: 1, cols: 0 });
        assert_eq!(search(KeyCode::Up), Event::Move { rows: -1, cols: 0 });
        assert_eq!(search(KeyCode::Backspace), Event::Backspace);
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

    fn hitmap() -> HitMap {
        HitMap {
            grid: ratatui::layout::Rect {
                x: 0,
                y: 1,
                width: 40,
                height: 6,
            },
            col_spans: vec![(0, 2..15), (1, 15..28)],
            line_rows: vec![0, 1, 2],
            tabs_y: 7,
            tab_spans: vec![(0, 1..7), (1, 7..13)],
            arrow_left: Some(0),
            arrow_right: Some(13),
        }
    }

    fn mouse(kind: MouseEventKind, x: u16, y: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column: x,
            row: y,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn a_click_resolves_to_a_cell_and_a_plain_click_never_copies() {
        let hits = Hits::Grid(hitmap());
        let mut drag = DragState::default();
        // y=1 is the column header, y=2 the first body line
        assert_eq!(
            map_mouse(
                &hits,
                InputMode::Grid,
                &mut drag,
                mouse(MouseEventKind::Down(MouseButton::Left), 3, 2)
            ),
            Event::SelectCell { row: 0, col: 0 }
        );
        assert_eq!(
            map_mouse(
                &hits,
                InputMode::Grid,
                &mut drag,
                mouse(MouseEventKind::Up(MouseButton::Left), 3, 2)
            ),
            Event::DragEnd { copy: false },
            "release still reaches the viewer, but a plain click never copies"
        );
    }

    #[test]
    fn a_drag_crosses_cells_and_release_ends_it() {
        let hits = Hits::Grid(hitmap());
        let mut drag = DragState::default();
        map_mouse(
            &hits,
            InputMode::Grid,
            &mut drag,
            mouse(MouseEventKind::Down(MouseButton::Left), 3, 2),
        );
        assert_eq!(
            map_mouse(
                &hits,
                InputMode::Grid,
                &mut drag,
                mouse(MouseEventKind::Drag(MouseButton::Left), 16, 3)
            ),
            Event::DragTo { row: 1, col: 1 }
        );
        assert_eq!(
            map_mouse(
                &hits,
                InputMode::Grid,
                &mut drag,
                mouse(MouseEventKind::Up(MouseButton::Left), 16, 3)
            ),
            Event::DragEnd { copy: true }
        );
        assert_eq!(
            map_mouse(
                &hits,
                InputMode::Grid,
                &mut drag,
                mouse(MouseEventKind::Drag(MouseButton::Left), 16, 3)
            ),
            Event::Noop
        );
    }

    #[test]
    fn tabs_arrows_and_dead_zones_resolve() {
        let hits = Hits::Grid(hitmap());
        let mut drag = DragState::default();
        let down = |x, y| mouse(MouseEventKind::Down(MouseButton::Left), x, y);
        assert_eq!(
            map_mouse(&hits, InputMode::Grid, &mut drag, down(8, 7)),
            Event::SelectSheet(1)
        );
        assert_eq!(
            map_mouse(&hits, InputMode::Grid, &mut drag, down(0, 7)),
            Event::PrevSheet
        );
        assert_eq!(
            map_mouse(&hits, InputMode::Grid, &mut drag, down(13, 7)),
            Event::NextSheet
        );
        assert_eq!(
            map_mouse(&hits, InputMode::Grid, &mut drag, down(3, 0)),
            Event::Noop,
            "the formula bar is not clickable"
        );
        assert_eq!(
            map_mouse(&hits, InputMode::Grid, &mut drag, down(3, 1)),
            Event::Noop,
            "the column-letter header is not a cell"
        );
    }

    #[test]
    fn a_click_on_the_text_pane_selects_the_line_under_it() {
        let hits = Hits::Text(TextHits {
            pane: ratatui::layout::Rect {
                x: 0,
                y: 1,
                width: 30,
                height: 5,
            },
            lines: vec![0, 1, 1, 2],
        });
        let mut drag = DragState::default();
        let down = |x, y| mouse(MouseEventKind::Down(MouseButton::Left), x, y);
        assert_eq!(
            map_mouse(&hits, InputMode::Grid, &mut drag, down(5, 1)),
            Event::SelectLine(0)
        );
        assert_eq!(
            map_mouse(&hits, InputMode::Grid, &mut drag, down(5, 3)),
            Event::SelectLine(1),
            "a wrapped row belongs to its line"
        );
        assert_eq!(
            map_mouse(&hits, InputMode::Grid, &mut drag, down(5, 5)),
            Event::Noop,
            "below the last row"
        );
        assert_eq!(
            map_mouse(&hits, InputMode::Grid, &mut drag, down(35, 2)),
            Event::Noop,
            "the panel is not the pane"
        );
        assert_eq!(
            map_mouse(
                &hits,
                InputMode::Grid,
                &mut drag,
                mouse(MouseEventKind::Up(MouseButton::Left), 5, 1)
            ),
            Event::DragEnd { copy: false },
            "a plain click on a line never copies"
        );
        assert_eq!(
            map_mouse(
                &hits,
                InputMode::Grid,
                &mut drag,
                mouse(MouseEventKind::ScrollDown, 5, 2)
            ),
            Event::Move { rows: 3, cols: 0 }
        );
    }

    #[test]
    fn a_drag_across_lines_ends_in_a_copy() {
        let hits = Hits::Text(TextHits {
            pane: ratatui::layout::Rect {
                x: 0,
                y: 1,
                width: 30,
                height: 5,
            },
            lines: vec![0, 1, 1, 2],
        });
        let mut drag = DragState::default();
        let at = |kind: MouseEventKind, y: u16| mouse(kind, 5, y);
        assert_eq!(
            map_mouse(
                &hits,
                InputMode::Grid,
                &mut drag,
                at(MouseEventKind::Down(MouseButton::Left), 1)
            ),
            Event::SelectLine(0)
        );
        assert_eq!(
            map_mouse(
                &hits,
                InputMode::Grid,
                &mut drag,
                at(MouseEventKind::Drag(MouseButton::Left), 4)
            ),
            Event::DragToLine(2)
        );
        assert_eq!(
            map_mouse(
                &hits,
                InputMode::Grid,
                &mut drag,
                at(MouseEventKind::Up(MouseButton::Left), 4)
            ),
            Event::DragEnd { copy: true }
        );
        assert_eq!(
            map_mouse(
                &hits,
                InputMode::Grid,
                &mut drag,
                at(MouseEventKind::Drag(MouseButton::Left), 2)
            ),
            Event::Noop,
            "no drag after the release"
        );
    }

    #[test]
    fn a_pointer_slip_on_the_same_line_is_still_a_click() {
        let hits = Hits::Text(TextHits {
            pane: ratatui::layout::Rect {
                x: 0,
                y: 1,
                width: 30,
                height: 5,
            },
            lines: vec![0, 1, 1, 2],
        });
        let mut drag = DragState::default();
        let mut send = |kind: MouseEventKind, x: u16, y: u16| {
            map_mouse(&hits, InputMode::Grid, &mut drag, mouse(kind, x, y))
        };
        send(MouseEventKind::Down(MouseButton::Left), 5, 2);
        send(MouseEventKind::Drag(MouseButton::Left), 6, 3);
        assert_eq!(
            send(MouseEventKind::Up(MouseButton::Left), 6, 3),
            Event::DragEnd { copy: false },
            "moving within the pressed line, even onto its wrapped row, is a click"
        );
        send(MouseEventKind::Down(MouseButton::Left), 5, 2);
        send(MouseEventKind::Drag(MouseButton::Left), 5, 4);
        send(MouseEventKind::Drag(MouseButton::Left), 5, 2);
        assert_eq!(
            send(MouseEventKind::Up(MouseButton::Left), 5, 2),
            Event::DragEnd { copy: true },
            "leaving the line and coming back copies it"
        );
    }

    #[test]
    fn the_wheel_moves_the_cursor_and_prompts_step_by_one() {
        let hits = Hits::Grid(HitMap::default());
        let mut drag = DragState::default();
        assert_eq!(
            map_mouse(
                &hits,
                InputMode::Grid,
                &mut drag,
                mouse(MouseEventKind::ScrollDown, 5, 5)
            ),
            Event::Move { rows: 3, cols: 0 }
        );
        assert_eq!(
            map_mouse(
                &hits,
                InputMode::Picker,
                &mut drag,
                mouse(MouseEventKind::ScrollUp, 5, 5)
            ),
            Event::Move { rows: -1, cols: 0 }
        );
        let mut shifted = mouse(MouseEventKind::ScrollDown, 5, 5);
        shifted.modifiers = KeyModifiers::SHIFT;
        assert_eq!(
            map_mouse(&hits, InputMode::Grid, &mut drag, shifted),
            Event::Move { rows: 0, cols: 3 },
            "shift turns the wheel sideways"
        );
    }
}
