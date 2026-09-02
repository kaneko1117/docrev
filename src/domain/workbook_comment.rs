/// Read-only: lives in the document, has no id, never written back.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkbookComment {
    pub row: usize,
    pub col: usize,
    pub author: String,
    pub body: String,
    /// Always false for legacy notes.
    pub resolved: bool,
    pub replies: Vec<WorkbookReply>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkbookReply {
    pub author: String,
    pub body: String,
}
