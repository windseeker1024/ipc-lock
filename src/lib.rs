#![cfg_attr(docsrs, feature(doc_cfg))]

//! Cross-process named locks.
//!
//! `ipc-lock` provides mutual exclusion that works across **both threads and
//! processes** on the same machine.
//!
//! # How it works
//!
//! Two locking layers work together:
//!
//! 1. **OS-level** — keeps different *processes* out.
//!    - Unix: `flock(2)` via [`std::fs::File::lock`] on a file under `$TMPDIR`.
//!    - Windows: a `Global\` named kernel mutex via `CreateMutexW`.
//!
//! 2. **Thread-level** — keeps different threads in the same process from
//!    entering concurrently, because `flock` and `CreateMutexW` are
//!    process-granular primitives that allow re-entry from the same process
//!    without blocking. Implemented with a [`Mutex<bool>`] gate and a [`Condvar`].
//!
//! # Example
//!
//! ```rust,no_run
//! use ipc_lock::{Lock, Result};
//!
//! fn main() -> Result<()> {
//!     let lock = Lock::new("my-app")?;
//!     let _guard = lock.lock()?;   // blocks until available
//!     // critical section …
//!     Ok(())                        // _guard dropped → lock released
//! }
//! ```
//!
//! `Lock` is cheap to clone — all clones share the same underlying state.
//!
//! ```rust,no_run
//! # use ipc_lock::{Lock, Result};
//! # fn main() -> Result<()> {
//! let lock = Lock::new("shared")?;
//! let other = lock.clone();        // cheap Arc clone
//! # Ok(())
//! # }
//! ```

use std::collections::HashMap;
use std::fmt;
use std::io;
use std::sync::{Arc, Condvar, LazyLock, Mutex, Weak};

#[cfg(unix)]
use std::path::{Path, PathBuf};

mod error;
mod sys;

pub use error::{Error, Result};

// ── Platform key type ─────────────────────────────────────────────────────────
//
// The registry key is the canonical OS-level identifier for the lock.
// On Unix it is the full path to the lock file; on Windows the mutex name.

#[cfg(unix)]
type Key = PathBuf;
#[cfg(windows)]
type Key = String;

/// Derive the OS-level key from a user-supplied name.
#[cfg(unix)]
fn key_from_name(name: &str) -> Key {
    std::env::var_os("TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(format!("{name}.lock"))
}

#[cfg(windows)]
fn key_from_name(name: &str) -> Key {
    format!("Global\\{name}")
}

// ── Validation ────────────────────────────────────────────────────────────────

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(Error::InvalidName);
    }
    // Null bytes break OS APIs on both platforms.
    // Slashes are reserved (Unix path separator / Windows mutex namespace).
    if name.bytes().any(|b| matches!(b, b'\0' | b'/' | b'\\')) {
        return Err(Error::InvalidName);
    }
    Ok(())
}

// ── Internal shared state ─────────────────────────────────────────────────────

/// Combined OS primitive + thread coordination for one named lock.
struct LockState {
    /// The underlying OS lock (file or named mutex).
    os: sys::OsLock,
    /// `true` while a [`LockGuard`] for this state exists in this process.
    held: Mutex<bool>,
    /// Notified when `held` transitions from `true` to `false`.
    released: Condvar,
}

impl fmt::Debug for LockState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LockState")
            .field("os", &self.os)
            .finish_non_exhaustive()
    }
}

// ── Process-wide registry ─────────────────────────────────────────────────────
//
// Ensures that every `Lock` for the same key within a single process shares
// the same `LockState`.  A `Weak` reference is stored; when all `Lock` handles
// and outstanding `LockGuard`s for a key are dropped the entry naturally
// becomes dead and is recycled on the next `create` call for that key.

static REGISTRY: LazyLock<Mutex<HashMap<Key, Weak<LockState>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Return an existing live `LockState` for `key`, or create a new one by
/// calling `create`.
///
/// `create` receives a reference to `key` so it can use the key value without
/// a clone — ownership of `key` is transferred to the registry on insertion.
fn registry_get_or_create(
    key: Key,
    create: impl FnOnce(&Key) -> io::Result<sys::OsLock>,
) -> Result<Arc<LockState>> {
    let mut map = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());

    // Fast path: a live state already exists.
    if let Some(state) = map.get(&key).and_then(Weak::upgrade) {
        return Ok(state);
    }

    // Slow path: open the OS primitive and mint a new state.
    let os = create(&key).map_err(Error::Io)?;
    let state = Arc::new(LockState {
        os,
        held: Mutex::new(false),
        released: Condvar::new(),
    });
    map.insert(key, Arc::downgrade(&state));
    Ok(state)
}

// ── Lock ──────────────────────────────────────────────────────────────────────

/// A cross-process named lock.
///
/// `Lock` is a lightweight handle backed by an [`Arc`]; cloning it is O(1)
/// and all clones share the same underlying state — including the
/// process-level mutual-exclusion guarantee.
///
/// # Name rules
///
/// * Must not be empty.
/// * Must not contain `\0`, `/`, or `\`.
#[derive(Clone, Debug)]
pub struct Lock {
    state: Arc<LockState>,
}

impl Lock {
    /// Open (or create) a named lock identified by `name`.
    ///
    /// # Platform behaviour
    ///
    /// * **Unix** — creates/opens `$TMPDIR/<name>.lock` (falls back to
    ///   `/tmp/<name>.lock` when `TMPDIR` is unset).
    /// * **Windows** — creates/opens a kernel mutex named `Global\<name>`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidName`] for illegal names, or [`Error::Io`] if
    /// the OS operation fails.
    pub fn new(name: &str) -> Result<Self> {
        validate_name(name)?;
        let key = key_from_name(name);
        // `key` is only borrowed by the closure. On the fast path an existing
        // live state is returned and `key` remains owned by this function; on
        // the slow path the closure borrows it and ownership then moves into
        // the registry.
        let state = registry_get_or_create(key, |k| sys::OsLock::open(k))?;
        Ok(Lock { state })
    }

    /// Open (or create) a named lock at an explicit filesystem path.
    ///
    /// Unlike [`Lock::new`], no `.lock` suffix is appended and the location
    /// is not constrained to `$TMPDIR`.  Parent directories must already exist.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] if the path cannot be opened or created.
    #[cfg(unix)]
    #[cfg_attr(docsrs, doc(cfg(unix)))]
    pub fn with_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        let key: PathBuf = path.as_ref().to_owned();
        let state = registry_get_or_create(key, |p| sys::OsLock::open(p))?;
        Ok(Lock { state })
    }

    /// Acquire the lock, **blocking** until it is available.
    ///
    /// Returns a [`LockGuard`] that releases the lock when dropped.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Io`] if the underlying OS call fails.
    pub fn lock(&self) -> Result<LockGuard> {
        acquire(Arc::clone(&self.state), true)
    }

    /// Try to acquire the lock **without blocking**.
    ///
    /// Returns a [`LockGuard`] if the lock is free, or
    /// [`Error::WouldBlock`] if it is currently held.
    ///
    /// # Errors
    ///
    /// Returns [`Error::WouldBlock`] when the lock is held, or [`Error::Io`]
    /// for any other OS-level failure.
    pub fn try_lock(&self) -> Result<LockGuard> {
        acquire(Arc::clone(&self.state), false)
    }
}

// ── Acquire helper ────────────────────────────────────────────────────────────

/// Core acquire logic shared by [`Lock::lock`] and [`Lock::try_lock`].
///
/// When `blocking` is `true` this function waits indefinitely; when `false`
/// it returns [`Error::WouldBlock`] immediately if either layer is busy.
fn acquire(state: Arc<LockState>, blocking: bool) -> Result<LockGuard> {
    // ── Layer 1: thread gate ──────────────────────────────────────────────────
    //
    // Claim `held` before touching the OS primitive.  This prevents two
    // threads in the same process from both entering `os.lock()`.
    {
        let mut held = state.held.lock().unwrap_or_else(|e| e.into_inner());
        if blocking {
            while *held {
                held = state.released.wait(held).unwrap_or_else(|e| e.into_inner());
            }
        } else if *held {
            return Err(Error::WouldBlock);
        }
        *held = true;
        // Intentionally drop the MutexGuard here. `held == true` is now the
        // logical claim; the actual OS lock is acquired below.
    }

    // ── Layer 2: OS lock ──────────────────────────────────────────────────────
    let os_result = if blocking {
        state.os.lock()
    } else {
        state.os.try_lock()
    };

    match os_result {
        Ok(()) => Ok(LockGuard { state }),

        Err(e) => {
            // Release the thread gate so waiting threads can retry.
            let mut held = state.held.lock().unwrap_or_else(|p| p.into_inner());
            *held = false;
            state.released.notify_one();

            if e.kind() == io::ErrorKind::WouldBlock {
                Err(Error::WouldBlock)
            } else {
                Err(Error::Io(e))
            }
        }
    }
}

// ── LockGuard ─────────────────────────────────────────────────────────────────

/// RAII guard returned by [`Lock::lock`] and [`Lock::try_lock`].
///
/// Releases the lock — both the OS primitive and the thread gate — when
/// dropped.  The guard keeps the [`Lock`]'s backing state alive, so it is
/// safe to drop the originating `Lock` while the guard is still live.
pub struct LockGuard {
    state: Arc<LockState>,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        // Release in reverse order of acquisition.
        // Step 1: release the cross-process OS lock.
        let _ = self.state.os.unlock();

        // Step 2: release the thread gate and wake one waiting thread.
        let mut held = self.state.held.lock().unwrap_or_else(|e| e.into_inner());
        *held = false;
        self.state.released.notify_one();
    }
}

impl fmt::Debug for LockGuard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LockGuard").finish_non_exhaustive()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    #[cfg(unix)]
    use std::path::PathBuf;
    use std::process::{Child, Command};
    use std::thread;
    use std::time::{Duration, Instant};
    use uuid::Uuid;

    fn random_name() -> String {
        Uuid::new_v4().as_hyphenated().to_string()
    }

    fn spawn_subprocess(num: u32, uuid: &str) -> Child {
        let exe = env::current_exe().expect("could not locate test binary");
        Command::new(exe)
            .env("IPC_LOCK_TEST_PROC", num.to_string())
            .env("IPC_LOCK_TEST_UUID", uuid)
            .arg("tests::cross_process")
            .spawn()
            .expect("failed to spawn subprocess")
    }

    // ── cross-process ─────────────────────────────────────────────────────────

    /// Orchestrates a three-process mutual-exclusion test:
    ///
    /// * Subprocess 1 holds the lock for a short period.
    /// * Subprocess 2 asserts `try_lock` fails, then waits for the lock.
    /// * The main process confirms both subprocesses exited successfully.
    ///
    /// The orchestrator polls instead of relying on exact sleep timings, so the
    /// test remains stable under CI load on all platforms.
    #[test]
    fn cross_process() -> Result<()> {
        let proc_num: u32 = env::var("IPC_LOCK_TEST_PROC")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let uuid = env::var("IPC_LOCK_TEST_UUID").unwrap_or_else(|_| random_name());

        match proc_num {
            0 => {
                // Orchestrator
                let mut h1 = spawn_subprocess(1, &uuid);
                thread::sleep(Duration::from_millis(50));
                let mut h2 = spawn_subprocess(2, &uuid);

                // Wait until subprocess 1 has actually acquired the OS lock.
                // Polling avoids fragile timing assumptions across platforms.
                let lock = Lock::new(&uuid)?;
                let deadline = Instant::now() + Duration::from_secs(5);
                let mut saw_would_block = false;
                while Instant::now() < deadline {
                    if matches!(lock.try_lock(), Err(Error::WouldBlock)) {
                        saw_would_block = true;
                        break;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                assert!(
                    saw_would_block,
                    "expected WouldBlock while subprocess 1 holds the lock"
                );

                assert!(h1.wait().unwrap().success(), "subprocess 1 failed");
                assert!(h2.wait().unwrap().success(), "subprocess 2 failed");
            }

            1 => {
                // Holds the lock long enough for the orchestrator to observe it.
                let lock = Lock::new(&uuid)?;
                let _guard = lock.lock()?;
                thread::sleep(Duration::from_millis(500));
            }

            2 => {
                // Verifies WouldBlock, then waits for the lock.
                let lock = Lock::new(&uuid)?;
                assert!(matches!(lock.try_lock(), Err(Error::WouldBlock)));
                let _guard = lock.lock()?;
                thread::sleep(Duration::from_millis(50));
            }

            _ => unreachable!(),
        }

        Ok(())
    }

    // ── same-process edge cases ───────────────────────────────────────────────

    /// Two handles for the same name share one `LockState`; holding via one
    /// blocks the other.
    #[test]
    fn shared_state() -> Result<()> {
        let name = random_name();
        let a = Lock::new(&name)?;
        let b = Lock::new(&name)?;

        {
            let _g = a.try_lock()?;
            assert!(matches!(a.try_lock(), Err(Error::WouldBlock)));
            assert!(matches!(b.try_lock(), Err(Error::WouldBlock)));
        }
        // After the guard drops both handles should be acquirable again.
        let _g = b.try_lock()?;
        Ok(())
    }

    /// Cloning a `Lock` yields another handle to the same state.
    #[test]
    fn clone_shares_state() -> Result<()> {
        let name = random_name();
        let original = Lock::new(&name)?;
        let cloned = original.clone();

        let guard = original.try_lock()?;
        assert!(matches!(cloned.try_lock(), Err(Error::WouldBlock)));
        drop(guard);
        let _g = cloned.try_lock()?; // now acquirable
        Ok(())
    }

    /// The guard keeps the lock alive even after the originating `Lock` is
    /// dropped.
    #[test]
    fn guard_outlives_lock() -> Result<()> {
        let name = random_name();
        let a = Lock::new(&name)?;
        let b = Lock::new(&name)?;

        let guard = a.try_lock()?;
        assert!(matches!(b.try_lock(), Err(Error::WouldBlock)));

        drop(a); // drop the handle — NOT the guard
        assert!(
            matches!(b.try_lock(), Err(Error::WouldBlock)),
            "lock should still be held after Lock handle is dropped"
        );

        drop(guard); // now the guard releases
        let _g = b.try_lock()?;
        Ok(())
    }

    /// A second thread in the same process is properly blocked and then woken.
    #[test]
    fn thread_mutual_exclusion() -> Result<()> {
        let name = random_name();
        let lock = Lock::new(&name)?;
        let lock2 = lock.clone();

        let guard = lock.lock()?;

        // Spawn a thread that will block on `lock2.lock()`.
        let handle = thread::spawn(move || -> Result<()> {
            let _g = lock2.lock()?; // blocks until main thread drops guard
            Ok(())
        });

        thread::sleep(Duration::from_millis(50));
        drop(guard); // wake the spawned thread

        handle
            .join()
            .expect("thread panicked")
            .expect("thread returned error");
        Ok(())
    }

    /// `try_lock` succeeds immediately when the lock is free.
    #[test]
    fn try_lock_succeeds_when_free() -> Result<()> {
        let name = random_name();
        let lock = Lock::new(&name)?;
        let _guard = lock.try_lock()?;
        Ok(())
    }

    /// Locks with different names are independent of each other.
    #[test]
    fn distinct_names_are_independent() -> Result<()> {
        let name_a = random_name();
        let name_b = random_name();

        let a = Lock::new(&name_a)?;
        let b = Lock::new(&name_b)?;

        let _guard_a = a.lock()?;
        let _guard_b = b.lock()?; // should not block, different OS keys
        Ok(())
    }

    /// Many threads competing for the same lock must be mutually exclusive.
    #[test]
    fn concurrent_threads() -> Result<()> {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let name = random_name();
        let lock = Lock::new(&name)?;
        let counter = Arc::new(AtomicUsize::new(0));
        const THREADS: usize = 10;
        const ITERATIONS: usize = 100;

        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let lock = lock.clone();
                let counter = Arc::clone(&counter);
                thread::spawn(move || -> Result<()> {
                    for _ in 0..ITERATIONS {
                        let _guard = lock.lock()?;
                        // Increment while holding the lock to guarantee
                        // no lost updates.
                        let prev = counter.load(Ordering::Relaxed);
                        counter.store(prev + 1, Ordering::Relaxed);
                    }
                    Ok(())
                })
            })
            .collect();

        for handle in handles {
            handle
                .join()
                .expect("thread panicked")
                .expect("thread returned error");
        }

        assert_eq!(
            counter.load(Ordering::Relaxed),
            THREADS * ITERATIONS,
            "atomic counter should match total increments"
        );
        Ok(())
    }

    /// Heavier contention test: many threads repeatedly acquiring and releasing the
    /// lock with no sleep. This is a regression guard against deadlocks or lost
    /// wake-ups in the thread-gate / Condvar logic.
    #[test]
    fn heavy_contention() -> Result<()> {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let name = random_name();
        let lock = Lock::new(&name)?;
        let counter = Arc::new(AtomicUsize::new(0));
        const THREADS: usize = 32;
        const ITERATIONS: usize = 1_000;

        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let lock = lock.clone();
                let counter = Arc::clone(&counter);
                thread::spawn(move || -> Result<()> {
                    for _ in 0..ITERATIONS {
                        let _guard = lock.lock()?;
                        let prev = counter.load(Ordering::Relaxed);
                        counter.store(prev + 1, Ordering::Relaxed);
                    }
                    Ok(())
                })
            })
            .collect();

        for handle in handles {
            handle
                .join()
                .expect("thread panicked")
                .expect("thread returned error");
        }

        assert_eq!(
            counter.load(Ordering::Relaxed),
            THREADS * ITERATIONS,
            "counter should equal total increments after heavy contention"
        );
        Ok(())
    }

    /// A second `try_lock` from the same thread while a guard is live must fail
    /// with `WouldBlock`. `Lock` is not re-entrant.
    #[test]
    fn try_lock_fails_while_held_by_same_thread() -> Result<()> {
        let name = random_name();
        let lock = Lock::new(&name)?;

        let _guard = lock.lock()?;
        assert!(
            matches!(lock.try_lock(), Err(Error::WouldBlock)),
            "same-thread re-entry should be rejected"
        );
        Ok(())
    }

    /// `lock()` blocks until the current holder releases the guard.
    #[test]
    fn lock_blocks_until_released() -> Result<()> {
        let name = random_name();
        let lock = Lock::new(&name)?;
        let lock2 = lock.clone();

        let guard = lock.lock()?;
        let start = Instant::now();

        let handle = thread::spawn(move || -> Result<Instant> {
            let _g = lock2.lock()?; // blocks
            Ok(Instant::now())
        });

        // Give the spawned thread time to start waiting.
        thread::sleep(Duration::from_millis(50));
        drop(guard);

        let acquired_after = handle
            .join()
            .expect("thread panicked")
            .expect("thread returned error");

        assert!(
            acquired_after >= start + Duration::from_millis(50),
            "second thread should have blocked until the guard was dropped"
        );
        Ok(())
    }

    // ── invalid names ─────────────────────────────────────────────────────────

    #[test]
    fn invalid_names() {
        for bad in ["", "a/b", "a\\b", "a\0b"] {
            assert!(
                matches!(Lock::new(bad), Err(Error::InvalidName)),
                "expected InvalidName for {bad:?}"
            );
        }
    }

    /// Names containing spaces, dots, dashes, or underscores are accepted.
    #[test]
    fn valid_names() -> Result<()> {
        for good in ["my app", "my-app", "my_app", "my.app", "123"] {
            let lock = Lock::new(good)?;
            let _guard = lock.try_lock()?;
        }
        Ok(())
    }

    // ── error display ─────────────────────────────────────────────────────────

    #[test]
    fn error_display() {
        assert_eq!(
            Error::InvalidName.to_string(),
            "invalid lock name: must be non-empty and contain no '\\0', '/', or '\\'"
        );
        assert_eq!(
            Error::WouldBlock.to_string(),
            "lock is currently held by another thread or process"
        );
        assert!(
            Error::Io(io::Error::new(io::ErrorKind::Other, "boom"))
                .to_string()
                .contains("I/O error"),
            "Io error should mention I/O"
        );
    }

    /// Verify the `std::error::Error::source` implementation.
    #[test]
    fn error_source() {
        use std::error::Error as StdError;

        let io_err = io::Error::new(io::ErrorKind::Other, "boom");
        let err = Error::Io(io_err);
        assert!(
            StdError::source(&err).is_some(),
            "Io error should have a source"
        );
        assert!(StdError::source(&Error::InvalidName).is_none());
        assert!(StdError::source(&Error::WouldBlock).is_none());
    }

    // ── trait bounds ─────────────────────────────────────────────────────────

    fn assert_send_sync<T: Send + Sync>() {}
    fn assert_clone_debug<T: Clone + std::fmt::Debug>() {}

    #[test]
    fn trait_bounds() {
        assert_send_sync::<Lock>();
        assert_send_sync::<LockGuard>();
        assert_clone_debug::<Lock>();
    }

    // ── platform-specific tests ───────────────────────────────────────────────

    /// Unix-only: `Lock::with_path` uses the supplied filesystem path.
    #[cfg(unix)]
    #[test]
    fn unix_with_path() -> Result<()> {
        let path = std::env::temp_dir().join(format!("ipc-lock-test-{}", random_name()));
        let a = Lock::with_path(&path)?;
        let b = Lock::with_path(&path)?;

        let _guard = a.try_lock()?;
        assert!(
            matches!(b.try_lock(), Err(Error::WouldBlock)),
            "two handles for the same path should share state"
        );

        // Clean up the lock file; ignore errors if the OS already removed it.
        let _ = std::fs::remove_file(&path);
        Ok(())
    }

    /// Unix-only: `Lock::new` creates the backing lock file under `$TMPDIR`.
    #[cfg(unix)]
    #[test]
    fn unix_lock_file_created() -> Result<()> {
        let name = random_name();
        let expected_path = std::env::var_os("TMPDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(format!("{name}.lock"));

        let lock = Lock::new(&name)?;
        assert!(
            expected_path.exists(),
            "lock file should be created at {expected_path:?}"
        );

        // The library intentionally leaves the lock file in place; holding and
        // dropping the guard should not remove it.
        {
            let _guard = lock.try_lock()?;
        }
        assert!(
            expected_path.exists(),
            "lock file should remain after the guard is dropped"
        );

        let _ = std::fs::remove_file(&expected_path);
        Ok(())
    }
}
