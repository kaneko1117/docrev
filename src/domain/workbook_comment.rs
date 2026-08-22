//! Comments stored in the workbook itself — Excel's legacy notes and
//! threaded comments. Deliberately not `CommentThread`: they are read-only,
//! live in the document rather than the sidecar, carry no id, and can never
//! be replied to or resolved through docrev (writing back is #7).

/// One workbook comment on a cell: a legacy note, or a threaded comment
/// with its replies.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkbookComment {
    pub row: usize,
    pub col: usize,
    pub author: String,
    pub body: String,
    /// Threaded comments carry Excel's "resolved" flag; notes never do.
    pub resolved: bool,
    pub replies: Vec<WorkbookReply>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkbookReply {
    pub author: String,
    pub body: String,
}
