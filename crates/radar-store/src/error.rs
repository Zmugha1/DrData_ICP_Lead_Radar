#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("migration checksum mismatch: {filename}")]
    ChecksumMismatch { filename: String },
    #[error("parcel fetch: {0}")]
    ParcelFetch(String),
    #[error("parcel assertion {code}: expected {expected} actual {actual}")]
    ParcelAssertion {
        code: String,
        expected: String,
        actual: String,
    },
    #[error("fred fetch: {0}")]
    FredFetch(String),
    #[error("fred assertion {code}: {message}")]
    FredAssertion { code: String, message: String },
    #[error("assignment load: {0}")]
    AssignmentLoad(String),
    #[error("assignment assertion {code}: {message}")]
    AssignmentAssertion { code: String, message: String },
    #[error("outcome: {0}")]
    Outcome(String),
    #[error("liquor: {0}")]
    Liquor(String),
    #[error("lira: {0}")]
    Lira(String),
    #[error("lira pdf: {0}")]
    LiraPdf(String),
    #[error("anthropic: {0}")]
    Anthropic(String),
}
