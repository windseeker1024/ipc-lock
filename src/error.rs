use std::fmt;
use std::io;

/// Specialized `Result` for this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors that can occur when using [`Lock`](crate::Lock).
#[derive(Debug)]
pub enum Error {
    /// The lock name is invalid.
    ///
    /// Names must be non-empty and must not contain `\0`, `/`, or `\`.
    InvalidName,

    /// The lock is currently held by another thread or process and a
    /// non-blocking acquire was requested.
    WouldBlock,

    /// An underlying I/O error occurred.
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidName => f.write_str(
                "invalid lock name: must be non-empty and contain no '\\0', '/', or '\\'",
            ),
            Error::WouldBlock => f.write_str("lock is currently held by another holder"),
            Error::Io(e) => write!(f, "I/O error: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}
