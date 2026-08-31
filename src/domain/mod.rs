//! docrev's own model of a reviewable document. No IO, no dependencies
//! on other layers, and no other format's grammar — see CLAUDE.md.

pub mod anchor;
pub mod cell;
pub mod comment;
pub mod document;
pub mod sheet;
pub mod workbook_comment;
