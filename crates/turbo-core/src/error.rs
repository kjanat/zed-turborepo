//! Common error types for turbo-core

use std::path::PathBuf;

/// Result type alias using [`Error`]
pub type Result<T> = std::result::Result<T, Error>;

/// Errors that can occur in turbo-core operations
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// turbo.json not found in directory hierarchy
    #[error("No turbo.json found starting from {0}")]
    ConfigNotFound(PathBuf),

    /// Failed to read file
    #[error("Failed to read {path}: {source}")]
    ReadFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Failed to parse JSON
    #[error("Failed to parse JSON in {path}: {message}")]
    ParseJson { path: PathBuf, message: String },

    /// Failed to parse JSONC (JSON with comments)
    #[error("Failed to parse JSONC in {path}: {message}")]
    ParseJsonc { path: PathBuf, message: String },

    /// Invalid path
    #[error("Invalid path: {0}")]
    InvalidPath(PathBuf),

    /// Command execution failed
    #[error("Command `{command}` failed: {message}")]
    CommandFailed { command: String, message: String },

    /// Package not found
    #[error("Package not found: {0}")]
    PackageNotFound(String),

    /// Task not found
    #[error("Task not found: {0}")]
    TaskNotFound(String),
}
