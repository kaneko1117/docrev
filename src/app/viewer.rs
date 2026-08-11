use std::path::Path;

use crate::domain::document::Document;
use crate::domain::sheet::Sheet;

use super::error::{DocumentError, FrontendError};
use super::ports::DocumentSource;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    Move { rows: isize, cols: isize },
    Top,
    Bottom,
    RowStart,
    RowEnd,
    NextSheet,
    PrevSheet,
    Quit,
    Noop,
}

pub trait Frontend {
    fn draw(&mut self, viewer: &Viewer) -> Result<(), FrontendError>;
    fn next_event(&mut self) -> Result<Event, FrontendError>;
}

/// Non-empty by construction: `first` always exists.
struct Sheets {
    first: Sheet,
    rest: Vec<Sheet>,
}

impl Sheets {
    fn get(&self, index: usize) -> &Sheet {
        match index.checked_sub(1) {
            None => &self.first,
            Some(i) => self.rest.get(i).unwrap_or(&self.first),
        }
    }

    fn len(&self) -> usize {
        1 + self.rest.len()
    }
}

pub struct Viewer {
    sheets: Sheets,
    cursors: Vec<(usize, usize)>,
    active: usize,
    quit: bool,
}

impl Viewer {
    pub fn open(source: &impl DocumentSource, path: &Path) -> Result<Self, DocumentError> {
        let document = source.load(path)?;
        Self::from_document(document)
    }

    fn from_document(document: Document) -> Result<Self, DocumentError> {
        let mut sheets = document.into_sheets().into_iter();
        let Some(first) = sheets.next() else {
            return Err(DocumentError::EmptyDocument);
        };
        let rest: Vec<Sheet> = sheets.collect();
        let cursors = vec![(0, 0); 1 + rest.len()];
        Ok(Self {
            sheets: Sheets { first, rest },
            cursors,
            active: 0,
            quit: false,
        })
    }

    pub fn sheet(&self) -> &Sheet {
        self.sheets.get(self.active)
    }

    pub fn sheet_names(&self) -> Vec<&str> {
        std::iter::once(self.sheets.first.name())
            .chain(self.sheets.rest.iter().map(Sheet::name))
            .collect()
    }

    pub fn sheet_count(&self) -> usize {
        self.sheets.len()
    }

    pub fn active(&self) -> usize {
        self.active
    }

    pub fn cursor(&self) -> (usize, usize) {
        self.cursors.get(self.active).copied().unwrap_or((0, 0))
    }

    pub fn wants_quit(&self) -> bool {
        self.quit
    }

    pub fn apply(&mut self, event: Event) {
        let (row, col) = self.cursor();
        let max_row = self.sheet().row_count().saturating_sub(1);
        let max_col = self.sheet().col_count().saturating_sub(1);
        let next = match event {
            Event::Move { rows, cols } => (
                add_clamped(row, rows, max_row),
                add_clamped(col, cols, max_col),
            ),
            Event::Top => (0, col),
            Event::Bottom => (max_row, col),
            Event::RowStart => (row, 0),
            Event::RowEnd => (row, max_col),
            Event::NextSheet => {
                self.active = (self.active + 1) % self.sheets.len();
                return;
            }
            Event::PrevSheet => {
                self.active = (self.active + self.sheets.len() - 1) % self.sheets.len();
                return;
            }
            Event::Quit => {
                self.quit = true;
                return;
            }
            Event::Noop => return,
        };
        if let Some(cursor) = self.cursors.get_mut(self.active) {
            *cursor = next;
        }
    }
}

fn add_clamped(value: usize, delta: isize, max: usize) -> usize {
    (value as isize + delta).clamp(0, max as isize) as usize
}

pub fn run(mut viewer: Viewer, frontend: &mut impl Frontend) -> Result<(), FrontendError> {
    while !viewer.wants_quit() {
        frontend.draw(&viewer)?;
        let event = frontend.next_event()?;
        viewer.apply(event);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;
    use crate::domain::cell::CellValue;

    fn viewer(rows: usize, cols: usize) -> Viewer {
        let grid = vec![vec![CellValue::Number(1.0); cols]; rows];
        let doc = Document::new(vec![
            Sheet::new("one", grid),
            Sheet::new("two", vec![vec![CellValue::Bool(true)]]),
        ]);
        Viewer::from_document(doc).unwrap()
    }

    #[test]
    fn empty_document_is_rejected() {
        assert!(Viewer::from_document(Document::new(vec![])).is_err());
    }

    #[test]
    fn cursor_clamps_at_edges() {
        let mut v = viewer(3, 2);
        v.apply(Event::Move { rows: -5, cols: -5 });
        assert_eq!(v.cursor(), (0, 0));
        v.apply(Event::Move {
            rows: 100,
            cols: 100,
        });
        assert_eq!(v.cursor(), (2, 1));
    }

    #[test]
    fn jump_events() {
        let mut v = viewer(10, 4);
        v.apply(Event::Move { rows: 5, cols: 2 });
        v.apply(Event::Top);
        assert_eq!(v.cursor(), (0, 2));
        v.apply(Event::Bottom);
        assert_eq!(v.cursor(), (9, 2));
        v.apply(Event::RowStart);
        assert_eq!(v.cursor(), (9, 0));
        v.apply(Event::RowEnd);
        assert_eq!(v.cursor(), (9, 3));
    }

    #[test]
    fn sheet_switching_wraps_and_keeps_cursor_per_sheet() {
        let mut v = viewer(5, 5);
        v.apply(Event::Move { rows: 2, cols: 2 });
        v.apply(Event::NextSheet);
        assert_eq!(v.sheet().name(), "two");
        assert_eq!(v.cursor(), (0, 0));
        v.apply(Event::NextSheet);
        assert_eq!(v.sheet().name(), "one");
        assert_eq!(v.cursor(), (2, 2), "cursor remembered per sheet");
        v.apply(Event::PrevSheet);
        assert_eq!(v.sheet().name(), "two");
    }

    struct FakeFrontend {
        events: VecDeque<Event>,
        draws: usize,
    }

    impl Frontend for FakeFrontend {
        fn draw(&mut self, _: &Viewer) -> Result<(), FrontendError> {
            self.draws += 1;
            Ok(())
        }

        fn next_event(&mut self) -> Result<Event, FrontendError> {
            Ok(self.events.pop_front().unwrap_or(Event::Quit))
        }
    }

    #[test]
    fn run_draws_until_quit() {
        let mut frontend = FakeFrontend {
            events: VecDeque::from([Event::Move { rows: 1, cols: 0 }, Event::Quit]),
            draws: 0,
        };
        run(viewer(3, 3), &mut frontend).unwrap();
        assert_eq!(frontend.draws, 2);
    }
}
