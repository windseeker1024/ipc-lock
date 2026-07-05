//! Platform-specific OS locking primitives.
//!
//! Each platform module exposes a single type:
//!
//! ```text
//! pub(crate) struct OsLock { … }
//! impl OsLock {
//!     fn lock(&self) -> io::Result<LockAcquisition>
//!     fn try_lock(&self) -> io::Result<LockAcquisition>   // Err(WouldBlock) when held
//!     fn unlock(&self) -> io::Result<()>
//! }
//! ```

/// Result of acquiring the OS-level lock.
///
/// On Windows a named mutex may be abandoned when the previous owner terminates
/// without releasing it; on Unix this situation does not exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LockAcquisition {
    /// Acquired normally.
    Normal,
    /// Acquired from an abandoned owner (Windows only).
    Abandoned,
}

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub(crate) use unix::OsLock;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(crate) use windows::OsLock;
