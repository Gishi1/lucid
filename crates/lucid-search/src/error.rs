//! Errors for the search crate.

/// Everything that can go wrong loading or saving an index.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("index data is corrupt: {0}")]
    Format(String),

    #[error("index format version {found} is not supported (this build understands {expected})")]
    Version { found: u32, expected: u32 },

    #[error("serialization failed: {0}")]
    Encode(String),
}

impl Error {
    /// Convenience constructor for corrupt-index errors.
    pub fn corrupt(message: impl Into<String>) -> Self {
        Error::Format(message.into())
    }
}

/// Result alias for the crate.
pub type Result<T> = std::result::Result<T, Error>;
