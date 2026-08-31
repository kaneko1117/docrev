//! docrev's own model of a reviewable document: what a document, a location
//! and a comment are. No IO and no dependencies on other layers; what else
//! belongs here is settled by the criterion in CLAUDE.md.

pub mod anchor;
pub mod cell;
pub mod comment;
pub mod document;
pub mod sheet;
pub mod workbook_comment;
