# ipc-lock

[![crates.io](https://img.shields.io/crates/v/ipc-lock)](https://crates.io/crates/ipc-lock)
[![docs.rs](https://docs.rs/ipc-lock/badge.svg)](https://docs.rs/ipc-lock)
[![license](https://img.shields.io/crates/l/ipc-lock)](LICENSE)

Cross-process named locks for Rust.

Works across both **threads** and **processes** on the same machine. The
cross-platform code uses no `unsafe`; platform-specific Win32 calls are
contained in a small, scoped module.

**MSRV:** Rust 1.89+

## Platform support

| Platform | Mechanism |
|---|---|
| Unix (Linux, macOS, …) | `flock(2)` via `std::fs::File::lock()` |
| Windows | `CreateMutexW` (kernel named mutex, `Global\` namespace) |

## Usage

```toml
[dependencies]
ipc-lock = "0.1"
```

```rust
use ipc_lock::{Lock, Result};

fn main() -> Result<()> {
    let lock = Lock::new("my-app")?;
    let _guard = lock.lock()?;   // blocks until the lock is free
    // critical section …
    Ok(())                        // _guard dropped → lock released automatically
}
```

`Lock` is cheaply cloneable — all clones share the same underlying state:

```rust
let lock  = Lock::new("my-app")?;
let other = lock.clone();        // O(1) Arc clone, same lock

let _guard = lock.lock()?;
assert!(other.try_lock().is_err()); // WouldBlock
```

## Name rules

- Must not be empty.
- Must not contain `\0`, `/`, or `\`.

On Unix the lock file is placed at `$TMPDIR/<name>.lock` (falling back to
`/tmp/<name>.lock`).  Use [`Lock::with_path`] to specify an exact path.

## Platform notes

### Unix lock-file cleanup

The library intentionally leaves the lock file in place after the lock is
released. You can retrieve the path with [`Lock::path`] and delete it when you
know no other process is using the lock:

```rust
let lock = Lock::new("my-app")?;
// ... use the lock ...
drop(lock);
std::fs::remove_file(lock.path())?;
```

Deleting the file while another process may still be using the lock can break
mutual exclusion, because a new process would create a fresh file at the same
path.

### Windows abandoned mutexes

If a Windows process terminates without releasing the named mutex, the next
waiter still acquires the lock successfully, but
[`LockGuard::is_abandoned()`] returns `true`. This signals that any shared
state protected by the lock may be inconsistent.

## License

MIT — see [LICENSE](LICENSE).
