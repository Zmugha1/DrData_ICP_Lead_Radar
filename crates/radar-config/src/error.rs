use std::io;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("missing config file: {path}")]
    FileMissing { path: PathBuf },
    #[error("unreadable config file {path}: {source}")]
    FileUnreadable { path: PathBuf, source: io::Error },
    #[error("failed to parse {path}: {message}")]
    ParseFailed { path: PathBuf, message: String },
    #[error("unknown field {field} in {path}")]
    UnknownField { path: PathBuf, field: String },
}
