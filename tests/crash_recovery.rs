//! Crash-recovery integration test.
//!
//! A child process acquires the lock and is then forcibly terminated. The
//! parent verifies that the lock becomes available again. This works because:
//!
//! * Unix: `flock(2)` is tied to the file descriptor; the kernel releases it
//!   when the process exits.
//! * Windows: an abandoned named mutex is treated as acquired by the current
//!   implementation.

use std::io::{self, BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use ipc_lock::{Lock, Result};

const LOCK_NAME: &str = "ipc-lock-crash-recovery-test";
const WORKER_ENV: &str = "IPC_LOCK_CRASH_RECOVERY_WORKER";
const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);

#[test]
fn crash_recovery() -> Result<()> {
    // When the environment variable is set we are the child worker: acquire the
    // lock, signal the parent, and keep the process alive until killed.
    if std::env::var(WORKER_ENV).is_ok() {
        let lock = Lock::new(LOCK_NAME)?;
        let _guard = lock.lock()?;
        println!("LOCK_ACQUIRED");
        io::stdout().flush().expect("flush stdout");
        loop {
            thread::sleep(Duration::from_secs(60));
        }
    }

    // Parent: spawn a copy of the test binary as the worker.
    let mut child = Command::new(std::env::current_exe().expect("current exe unknown"))
        .env(WORKER_ENV, "1")
        .arg("--nocapture")
        .stdout(Stdio::piped())
        .spawn()
        .expect("failed to spawn worker");

    let stdout = child.stdout.take().expect("no stdout");
    let reader = BufReader::new(stdout);
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        for line in reader.lines() {
            if let Ok(line) = line {
                if line.trim() == "LOCK_ACQUIRED" {
                    let _ = tx.send(());
                    return;
                }
            }
        }
    });

    if rx.recv_timeout(STARTUP_TIMEOUT).is_err() {
        let _ = child.kill();
        panic!("worker did not acquire the lock in time");
    }

    // Give the OS a moment to finish setting up the lock in the child.
    thread::sleep(Duration::from_millis(100));

    child.kill().expect("failed to kill worker");
    child.wait().expect("worker did not exit");

    // The lock must be acquirable after the child is terminated.
    let lock = Lock::new(LOCK_NAME)?;
    let _guard = lock.lock()?;
    Ok(())
}
