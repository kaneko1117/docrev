use super::anchor::Anchor;

#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    pub id: String,
    pub author: String,
    pub body: String,
    /// ISO 8601 UTC.
    pub created_at: String,
}

/// `resolved` applies to the whole thread.
#[derive(Debug, Clone, PartialEq)]
pub struct CommentThread {
    pub id: String,
    pub anchor: Anchor,
    pub author: String,
    pub body: String,
    /// ISO 8601 UTC.
    pub created_at: String,
    pub resolved: bool,
    pub replies: Vec<Reply>,
}
