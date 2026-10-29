//! Error type for the whole crate.

use std::path::PathBuf;

/// Everything that can go wrong while opening or reading a help package.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("`{path}` is not a readable zip archive: {source}")]
    Zip {
        path: PathBuf,
        #[source]
        source: zip::result::ZipError,
    },

    #[error("`{path}` is a Microsoft cabinet (.cab) archive, not an .mshc package: \
extract it first, e.g. `cabextract {name}` and open the resulting .mshc files")]
    Cab { path: PathBuf, name: String },

    #[error("`{path}` is not a readable CHM archive: {reason}")]
    Chm { path: PathBuf, reason: String },

    #[error("`{path}` is not a help package: {reason}")]
    NotPackage { path: PathBuf, reason: String },

    #[error("package `{path}` contains no help topics (looked for .htm/.html/.xhtml parts)")]
    NoTopics { path: PathBuf },

    #[error("malformed XML in {part}: {reason}")]
    Xml { part: String, reason: String },

    #[error("{0} not found in this content set")]
    NotFound(String),

    #[error("{0}")]
    Other(String),
}

impl Error {
    /// Convenience constructor for I/O errors (keeps the path in the message).
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }

    /// Convenience constructor for zip errors.
    pub fn zip(path: impl Into<PathBuf>, source: zip::result::ZipError) -> Self {
        Error::Zip {
            path: path.into(),
            source,
        }
    }

    /// Convenience constructor for CHM errors.
    pub fn chm(path: impl Into<PathBuf>, reason: impl Into<String>) -> Self {
        Error::Chm {
            path: path.into(),
            reason: reason.into(),
        }
    }

    pub fn other(msg: impl Into<String>) -> Self {
        Error::Other(msg.into())
    }
}

/// Crate result alias.
pub type Result<T> = std::result::Result<T, Error>;
