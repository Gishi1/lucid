//! The error type shared by the packer and the validator.

use std::path::PathBuf;

/// Everything that can go wrong while building or reading a package.
///
/// The variants are deliberately coarse: callers either print the message (the CLI) or match
/// on the interesting cases ([`Error::MissingId`], [`Error::NoTopics`]).
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A file could not be read or written.
    #[error("I/O error on {}: {source}", .path.display())]
    Io {
        /// The file the operation was performed on.
        path: PathBuf,
        /// The underlying operating system error.
        #[source]
        source: std::io::Error,
    },
    /// The package is not a readable/writable zip archive.
    #[error("zip error: {0}")]
    Zip(#[from] zip::result::ZipError),
    /// A topic has no `Microsoft.Help.Id` and ids may not be derived.
    #[error(
        "topic {0} has no Microsoft.Help.Id meta tag (pass --allow-missing-ids to derive an id \
         from the file name)"
    )]
    MissingId(String),
    /// The input directory does not contain a single `.htm`/`.html`/`.xhtml` file.
    #[error("no topic files (.htm/.html/.xhtml) found in {0}")]
    NoTopics(String),
    /// The input is not something the operation can work with (not a directory, a `.cab`
    /// archive, duplicate part names, …).
    #[error("{0}")]
    Input(String),
}

impl Error {
    /// Build an [`Error::Io`] for `path`.
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }
}
