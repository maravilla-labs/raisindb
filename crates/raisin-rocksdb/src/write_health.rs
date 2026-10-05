// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Whether RocksDB is accepting writes, and getting it to accept them again.
//!
//! # Why this exists
//!
//! When a write fails with an IO error (a WAL append on a full disk), RocksDB
//! records a hard background error and STOPS: every later write returns that
//! same error. It schedules one automatic recovery right away, but if the disk
//! is still full that recovery's flush fails too, and RocksDB then turns
//! auto-recovery off for good (its LOG says `auto_recovery=0`, "Failed to
//! resume DB"). From there only `DB::Resume()` or a restart clears it — so a
//! server that ran out of disk kept refusing every write after the space came
//! back, until someone restarted it (local server, 2026-09-29).
//!
//! [`WriteHealth`] notices the stop from the commit path and runs a recovery
//! loop that calls `Resume()` with backoff until it succeeds. `Resume()` on a
//! database that is not stopped is a no-op, so a false alarm costs one call.
//! The state is reported by `/health` and the storage health check.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, Utc};
use rocksdb::DB;
use serde::Serialize;

/// First wait between recovery attempts; doubles up to [`MAX_RETRY`].
const FIRST_RETRY: Duration = Duration::from_secs(2);
const MAX_RETRY: Duration = Duration::from_secs(30);

/// A point-in-time view of the write state, for health reporting.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct WriteHealthSnapshot {
    /// False while RocksDB refuses writes.
    pub writable: bool,
    /// When writes stopped.
    pub stopped_since: Option<DateTime<Utc>>,
    /// The write error that stopped them.
    pub error: Option<String>,
    /// Why the last recovery attempt failed (still stopped).
    pub last_attempt_error: Option<String>,
    pub last_attempt_at: Option<DateTime<Utc>>,
    /// Recoveries completed since the process started.
    pub recoveries: u64,
    pub last_recovered_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Default)]
struct State {
    stopped_since: Option<DateTime<Utc>>,
    error: Option<String>,
    last_attempt_error: Option<String>,
    last_attempt_at: Option<DateTime<Utc>>,
    recoveries: u64,
    last_recovered_at: Option<DateTime<Utc>>,
}

/// Write-availability tracker for one RocksDB instance.
#[derive(Debug, Default)]
pub struct WriteHealth {
    state: Mutex<State>,
    recovering: AtomicBool,
}

/// Does this write error mean RocksDB has stopped accepting writes?
///
/// Any IO error from a write is recorded by RocksDB as a background error
/// that stops the database; a stopped database answers every later write
/// with that stored error (or "Writer has previous error"), which are IO
/// errors too. `Incomplete` is the write-stall form of the same stop.
pub fn is_stopping_write_error(message: &str) -> bool {
    message.contains("IO error") || message.starts_with("Incomplete")
}

impl WriteHealth {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn snapshot(&self) -> WriteHealthSnapshot {
        let s = self.state.lock().unwrap();
        WriteHealthSnapshot {
            writable: s.stopped_since.is_none(),
            stopped_since: s.stopped_since,
            error: s.error.clone(),
            last_attempt_error: s.last_attempt_error.clone(),
            last_attempt_at: s.last_attempt_at,
            recoveries: s.recoveries,
            last_recovered_at: s.last_recovered_at,
        }
    }

    /// Record a failed write. Returns true when it marks the database stopped
    /// (the first stopping error since the last recovery).
    pub fn record_write_error(&self, message: &str) -> bool {
        if !is_stopping_write_error(message) {
            return false;
        }
        let mut s = self.state.lock().unwrap();
        if s.stopped_since.is_some() {
            return false;
        }
        s.stopped_since = Some(Utc::now());
        s.error = Some(message.to_string());
        s.last_attempt_error = None;
        s.last_attempt_at = None;
        true
    }

    /// One recovery attempt. `resume` is `DB::resume` in production.
    ///
    /// Returns true when the database is writable afterwards.
    pub fn attempt_recovery(&self, resume: impl FnOnce() -> Result<(), String>) -> bool {
        let result = resume();
        let mut s = self.state.lock().unwrap();
        let now = Utc::now();
        match result {
            Ok(()) => {
                if let Some(since) = s.stopped_since.take() {
                    s.recoveries += 1;
                    s.last_recovered_at = Some(now);
                    tracing::warn!(
                        stopped_for_secs = (now - since).num_seconds(),
                        error = s.error.as_deref().unwrap_or_default(),
                        "RocksDB accepts writes again: recovered from a stopped state without a restart"
                    );
                }
                s.error = None;
                s.last_attempt_error = None;
                s.last_attempt_at = Some(now);
                true
            }
            Err(e) => {
                s.last_attempt_error = Some(e);
                s.last_attempt_at = Some(now);
                false
            }
        }
    }

    /// Record a failed write and, if it stopped the database, start the
    /// recovery loop on the current Tokio runtime (at most one at a time).
    pub fn on_write_error(self: &Arc<Self>, db: &Arc<DB>, message: &str) {
        if self.record_write_error(message) {
            tracing::error!(
                error = %message,
                "RocksDB stopped accepting writes; retrying DB::Resume() in the background \
                 (free disk space if the error is 'No space left on device')"
            );
        }
        if self.snapshot().writable {
            return;
        }
        self.spawn_recovery(db.clone());
    }

    fn spawn_recovery(self: &Arc<Self>, db: Arc<DB>) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        if self.recovering.swap(true, Ordering::AcqRel) {
            return;
        }
        let health = self.clone();
        runtime.spawn(async move {
            let mut wait = FIRST_RETRY;
            loop {
                tokio::time::sleep(wait).await;
                let db = db.clone();
                let resumed =
                    tokio::task::spawn_blocking(move || db.resume().map_err(|e| e.into_string()))
                        .await
                        .unwrap_or_else(|e| Err(format!("resume task failed: {e}")));
                if health.attempt_recovery(|| resumed) {
                    break;
                }
                wait = (wait * 2).min(MAX_RETRY);
            }
            health.recovering.store(false, Ordering::Release);
            // A write may have stopped the database again between the
            // successful resume and clearing the flag; pick it up.
            if !health.snapshot().writable {
                health.spawn_recovery(db);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_io_errors_stop_writes() {
        assert!(is_stopping_write_error(
            "IO error: No space left on device: While appending to file: 000026.log"
        ));
        assert!(is_stopping_write_error(
            "IO error: Writer has previous error."
        ));
        assert!(!is_stopping_write_error("Corruption: bad block"));
        assert!(!is_stopping_write_error(
            "Invalid argument: column family not found"
        ));
    }

    #[test]
    fn stop_then_failed_attempt_then_recovery() {
        let health = WriteHealth::new();
        assert!(health.snapshot().writable);

        assert!(health.record_write_error("IO error: No space left on device"));
        assert!(
            !health.record_write_error("IO error: Writer has previous error."),
            "a second error while stopped is the same stop"
        );
        let stopped = health.snapshot();
        assert!(!stopped.writable);
        assert_eq!(
            stopped.error.as_deref(),
            Some("IO error: No space left on device"),
            "the first error is the one reported"
        );

        assert!(!health.attempt_recovery(|| Err("IO error: No space left on device".into())));
        let still = health.snapshot();
        assert!(!still.writable);
        assert!(still.last_attempt_error.is_some());
        assert_eq!(still.recoveries, 0);

        assert!(health.attempt_recovery(|| Ok(())));
        let recovered = health.snapshot();
        assert!(recovered.writable);
        assert_eq!(recovered.recoveries, 1);
        assert!(recovered.error.is_none() && recovered.last_attempt_error.is_none());
        assert!(recovered.last_recovered_at.is_some());
    }

    #[test]
    fn a_non_io_error_does_not_mark_stopped() {
        let health = WriteHealth::new();
        assert!(!health.record_write_error("Invalid argument: bad key"));
        assert!(health.snapshot().writable);
    }

    /// The vendored `DB::resume()` binding links and is a no-op on a
    /// database that is not stopped.
    #[test]
    fn resume_on_a_healthy_database_is_ok() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(true);
        let db = DB::open(&opts, dir.path()).unwrap();
        db.put(b"k", b"v").unwrap();
        db.resume().expect("resume on a healthy DB");
        db.put(b"k2", b"v2").unwrap();
    }

    /// A real full disk: fill a small volume until RocksDB stops, free the
    /// space, and check that `resume()` alone makes it writable again.
    ///
    /// Needs a small dedicated volume, so it is ignored by default. On macOS:
    ///
    /// ```sh
    /// hdiutil create -size 40m -fs HFS+ -volname rdbtest /tmp/rdbtest.dmg
    /// hdiutil attach /tmp/rdbtest.dmg -mountpoint /tmp/rdbmnt -nobrowse
    /// RAISIN_ENOSPC_DIR=/tmp/rdbmnt cargo test -p raisin-rocksdb --lib \
    ///     write_health::tests::recovers_from_a_full_disk -- --ignored --nocapture
    /// hdiutil detach /tmp/rdbmnt
    /// ```
    #[test]
    #[ignore = "needs a small volume in RAISIN_ENOSPC_DIR"]
    fn recovers_from_a_full_disk() {
        use std::io::Write;

        let root = std::path::PathBuf::from(
            std::env::var("RAISIN_ENOSPC_DIR").expect("set RAISIN_ENOSPC_DIR"),
        );
        let db_dir = root.join("db");
        let filler = root.join("filler");
        let _ = std::fs::remove_dir_all(&db_dir);
        let _ = std::fs::remove_file(&filler);

        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(true);
        let db = DB::open(&opts, &db_dir).unwrap();
        db.put(b"before", b"ok").unwrap();

        // Leave a few hundred KB free, then write until RocksDB fails.
        {
            let mut f = std::fs::File::create(&filler).unwrap();
            let chunk = vec![0u8; 1 << 20];
            while f.write_all(&chunk).is_ok() {}
            let _ = f.sync_all();
        }
        let value = vec![7u8; 64 * 1024];
        let mut stop_error = None;
        for i in 0..100_000u32 {
            let mut wo = rocksdb::WriteOptions::default();
            wo.set_sync(true);
            if let Err(e) = db.put_opt(i.to_be_bytes(), &value, &wo) {
                stop_error = Some(e.into_string());
                break;
            }
        }
        let stop_error = stop_error.expect("the volume never filled up");
        println!("stopped with: {stop_error}");
        let health = WriteHealth::new();
        assert!(health.record_write_error(&stop_error));

        // Still full: the database stays stopped, and resume says why.
        assert!(
            db.put(b"while-full", b"x").is_err(),
            "a stopped DB refuses writes"
        );
        assert!(!health.attempt_recovery(|| db.resume().map_err(|e| e.into_string())));

        // Space is back. Before this change only a restart cleared the stop.
        std::fs::remove_file(&filler).unwrap();
        let mut recovered = false;
        for _ in 0..20 {
            if health.attempt_recovery(|| db.resume().map_err(|e| e.into_string())) {
                recovered = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        assert!(recovered, "resume failed: {:?}", health.snapshot());
        db.put(b"after", b"ok")
            .expect("writable again without a restart");
        assert_eq!(db.get(b"before").unwrap().as_deref(), Some(&b"ok"[..]));
    }
}
