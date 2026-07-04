//! Unix implementation: wraps `std::fs::File::lock()` / `try_lock()` / `unlock()`.
//!
//! Internally the OS uses `flock(2)` with `LOCK_EX`.  The standard library
//! handles `EINTR` retries automatically.

use std::fs::{File, OpenOptions, TryLockError};
use std::io;
use std::path::Path;

/// Unix file-based exclusive lock.
#[derive(Debug)]
pub(crate) struct OsLock {
    file: File,
}

impl OsLock {
    /// Open (or create) the lock file at `path`.
    pub(crate) fn open(path: &Path) -> io::Result<Self> {
        // `create(true)` without `truncate(true)` opens an existing file or
        // creates a new one without erasing any content — ideal for a lock file
        // whose content is irrelevant. `truncate(false)` is explicit to silence
        // the `suspicious_open_options` lint.
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        Ok(OsLock { file })
    }

    /// Block until the exclusive lock is acquired.
    pub(crate) fn lock(&self) -> io::Result<()> {
        self.file.lock()
    }

    /// Attempt a non-blocking acquire.
    ///
    /// Returns `Err` with [`io::ErrorKind::WouldBlock`] when the lock is held
    /// by another process.
    pub(crate) fn try_lock(&self) -> io::Result<()> {
        self.file.try_lock().map_err(|e| match e {
            TryLockError::WouldBlock => io::Error::from(io::ErrorKind::WouldBlock),
            TryLockError::Error(e) => e,
        })
    }

    /// Release the lock.
    pub(crate) fn unlock(&self) -> io::Result<()> {
        self.file.unlock()
    }
}
