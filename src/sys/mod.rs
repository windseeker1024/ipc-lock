//! Platform-specific OS locking primitives.
//!
//! Each platform module exposes a single type:
//!
//! ```text
//! pub(crate) struct OsLock { … }
//! impl OsLock {
//!     fn lock(&self) -> io::Result<()>
//!     fn try_lock(&self) -> io::Result<()>   // Err(WouldBlock) when held
//!     fn unlock(&self) -> io::Result<()>
//! }
//! ```

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub(crate) use unix::OsLock;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(crate) use windows::OsLock;
