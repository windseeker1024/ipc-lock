//! Windows implementation: wraps a named kernel mutex (`CreateMutexW`).
//!
//! The mutex is created in the `Global\` namespace so it is visible across
//! all user sessions on the machine.

use std::io;

use crate::sys::LockAcquisition;
use windows::Win32::Foundation::{
    CloseHandle, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::System::Threading::{
    CreateMutexW, INFINITE, ReleaseMutex, WaitForSingleObject,
};
use windows::core::HSTRING;

/// Windows named-mutex exclusive lock.
#[derive(Debug)]
pub(crate) struct OsLock {
    handle: HANDLE,
}

// HANDLE is a raw pointer type; the mutex itself is thread-safe.
unsafe impl Send for OsLock {}
unsafe impl Sync for OsLock {}

impl OsLock {
    /// Open (or create) a named mutex identified by `name`.
    pub(crate) fn open(name: &str) -> io::Result<Self> {
        let handle =
            unsafe { CreateMutexW(None, false, &HSTRING::from(name)).map_err(io::Error::from)? };
        Ok(OsLock { handle })
    }

    /// Block until the mutex is acquired.
    ///
    /// Returns [`LockAcquisition::Abandoned`] when the previous owner terminated
    /// without releasing the mutex.
    pub(crate) fn lock(&self) -> io::Result<LockAcquisition> {
        let rc = unsafe { WaitForSingleObject(self.handle, INFINITE) };
        if rc == WAIT_OBJECT_0 {
            Ok(LockAcquisition::Normal)
        } else if rc == WAIT_ABANDONED {
            Ok(LockAcquisition::Abandoned)
        } else {
            Err(io::Error::last_os_error())
        }
    }

    /// Attempt a non-blocking acquire.
    ///
    /// Returns `Err` with [`io::ErrorKind::WouldBlock`] when the mutex is held.
    /// Returns [`LockAcquisition::Abandoned`] when the previous owner terminated
    /// without releasing the mutex.
    pub(crate) fn try_lock(&self) -> io::Result<LockAcquisition> {
        let rc = unsafe { WaitForSingleObject(self.handle, 0) };
        if rc == WAIT_OBJECT_0 {
            Ok(LockAcquisition::Normal)
        } else if rc == WAIT_ABANDONED {
            Ok(LockAcquisition::Abandoned)
        } else if rc == WAIT_TIMEOUT {
            Err(io::Error::from(io::ErrorKind::WouldBlock))
        } else {
            Err(io::Error::last_os_error())
        }
    }

    /// Release the mutex.
    pub(crate) fn unlock(&self) -> io::Result<()> {
        unsafe { ReleaseMutex(self.handle).map_err(io::Error::from) }
    }
}

impl Drop for OsLock {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}
