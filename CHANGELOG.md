# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-07-05

### Added

- Initial release of `ipc-lock`.
- `Lock` handle and `LockGuard` RAII guard for cross-process named locks.
- Unix implementation using `flock(2)` via `std::fs::File::lock()` on a file under `$TMPDIR` (fallback `/tmp`).
- Windows implementation using a named kernel mutex in the `Global\` namespace via `CreateMutexW`.
- Process-wide registry so handles and clones within the same process share state.
- `Lock::with_path` (Unix only) for specifying an explicit lock file path.
- `Lock::try_lock` for non-blocking acquire returning `Error::WouldBlock`.
