# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.3] - 2026-07-05

### Added

- Added `LockGuard::is_abandoned()` to report when a Windows named mutex was
  acquired from an owner that terminated without releasing it.
- Added `Lock::path()` (Unix only) to retrieve the filesystem path of the
  backing lock file, useful for manual cleanup.

## [0.1.2] - 2026-07-05

### Fixed

- Added `#![cfg_attr(docsrs, feature(doc_cfg))]` so docs.rs can build the
  `#[cfg_attr(docsrs, doc(cfg(unix)))]` annotation on `Lock::with_path`.

## [0.1.1] - 2026-07-05

### Added

- Expanded test coverage: try_lock success, distinct names, heavy contention,
  same-thread re-entry, blocking-until-released, valid/invalid names, error
  display/source, Unix `with_path`, and Unix lock-file creation.
- Added `tests/crash_recovery.rs` integration test for lock recovery after a
  child process is forcibly terminated.

### Fixed

- Synchronised `Error::WouldBlock` `Display` text with its doc comment.

## [0.1.0] - 2026-07-05

### Added

- Initial release of `ipc-lock`.
- `Lock` handle and `LockGuard` RAII guard for cross-process named locks.
- Unix implementation using `flock(2)` via `std::fs::File::lock()` on a file under `$TMPDIR` (fallback `/tmp`).
- Windows implementation using a named kernel mutex in the `Global\` namespace via `CreateMutexW`.
- Process-wide registry so handles and clones within the same process share state.
- `Lock::with_path` (Unix only) for specifying an explicit lock file path.
- `Lock::try_lock` for non-blocking acquire returning `Error::WouldBlock`.
