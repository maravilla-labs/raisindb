//! Resumable, bounded, crash-consistent progress for a streaming repair.
//!
//! A repair streams one CF of one branch and commits in bounded batches. The
//! per-node state record — `{tenant}\0{repo}\0{branch}\0repair_state\0{repair}\0{node_id}`
//! in `INDEX_STATUS` — is written IN THE SAME BATCH as the repair's own writes,
//! so after a crash the cursor names exactly the last committed key: a resumed
//! run neither skips nor (beyond idempotent re-writes) repeats work.
//!
//! The record is tenant-first, so a tenant wipe reaches it, and it carries the
//! cluster node id so an operator can see which nodes have not repaired (the
//! fan-out endpoint reads it). It is a progress record, not a "done" flag: a
//! repair always finds its targets from the data, so it is correct after a
//! checkpoint ingest and after a crash regardless of what the record says.

use crate::{cf, cf_handle};
use raisin_error::Result;
use rocksdb::{WriteBatch, DB};
use serde::{Deserialize, Serialize};

/// Where a repair is, as persisted in its state record.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepairState {
    /// `running` while unfinished (including after a crash), `done` after.
    pub status: String,
    /// The pass the cursor belongs to.
    pub pass: String,
    /// Hex of the last key whose work is committed; resume strictly after it.
    pub cursor: Option<String>,
    /// Tombstones (or rewrites) committed so far by this run.
    pub written: u64,
    /// RFC 3339 time of the last update.
    pub updated_at: String,
}

/// `{tenant}\0{repo}\0{branch}\0repair_state\0{repair}\0{node_id}`.
pub fn state_key(
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    repair: &str,
    node_id: &str,
) -> Vec<u8> {
    crate::keys::KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push("repair_state")
        .push(repair)
        .push(node_id)
        .build()
}

/// The persisted state, if any.
pub fn load_state(
    db: &DB,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    repair: &str,
    node_id: &str,
) -> Result<Option<RepairState>> {
    let cf = cf_handle(db, cf::INDEX_STATUS)?;
    let key = state_key(tenant_id, repo_id, branch, repair, node_id);
    match db
        .get_cf(cf, key)
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?
    {
        Some(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| raisin_error::Error::storage(format!("repair state decode: {e}"))),
        None => Ok(None),
    }
}

/// Accumulates one repair's writes and commits them in bounded batches, each
/// together with the state record that names its last key.
pub(super) struct BoundedWriter<'a> {
    db: &'a DB,
    batch: WriteBatch,
    state_key: Vec<u8>,
    state: RepairState,
    batch_bytes: usize,
    dry_run: bool,
    /// Stop (as if crashed) after this many commits — a test hook.
    stop_after_batches: Option<usize>,
    /// Bytes per second the commits may average; 0 = unlimited.
    max_bytes_per_sec: u64,
    started: std::time::Instant,
    committed_bytes: u64,
    pub(super) report: BatchReport,
}

/// What the writer committed (or, in a dry run, would have).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchReport {
    /// Entries written: tombstones, or `\0` -> `T` rewrites.
    pub written: u64,
    /// Bytes of those writes (key + value).
    pub bytes: u64,
    /// Batches committed.
    pub batches: u64,
    /// The largest committed batch, in bytes — bounded by the configured size
    /// plus one entry.
    pub max_batch_bytes: u64,
}

impl<'a> BoundedWriter<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        db: &'a DB,
        state_key: Vec<u8>,
        state: RepairState,
        batch_bytes: usize,
        dry_run: bool,
        stop_after_batches: Option<usize>,
        max_bytes_per_sec: u64,
    ) -> Self {
        Self {
            db,
            batch: WriteBatch::default(),
            state_key,
            state,
            batch_bytes,
            dry_run,
            stop_after_batches,
            max_bytes_per_sec,
            started: std::time::Instant::now(),
            committed_bytes: 0,
            report: BatchReport::default(),
        }
    }

    pub(super) fn state(&self) -> &RepairState {
        &self.state
    }

    /// Queue one write.
    pub(super) fn put(&mut self, cf_name: &str, key: &[u8], value: &[u8]) -> Result<()> {
        self.report.written += 1;
        self.report.bytes += (key.len() + value.len()) as u64;
        if !self.dry_run {
            let cf = cf_handle(self.db, cf_name)?;
            self.batch.put_cf(cf, key, value);
        }
        Ok(())
    }

    /// Mark `key` (of `pass`) as fully processed, and commit if the batch has
    /// reached its size. Returns `false` when the run must stop (the crash
    /// hook fired).
    pub(super) fn checkpoint(&mut self, pass: &str, key: &[u8]) -> Result<bool> {
        self.state.pass = pass.to_string();
        self.state.cursor = Some(hex::encode(key));
        if self.batch.size_in_bytes() >= self.batch_bytes {
            self.commit("running")?;
            if self
                .stop_after_batches
                .is_some_and(|limit| self.report.batches as usize >= limit)
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Whether this run only counts.
    pub(super) fn dry_run(&self) -> bool {
        self.dry_run
    }

    /// Start a new pass: its cursor begins empty.
    pub(super) fn begin_pass(&mut self, pass: &str) {
        if self.state.pass != pass {
            self.state.pass = pass.to_string();
            self.state.cursor = None;
        }
    }

    /// Commit what is queued, with the state record, as `status`.
    pub(super) fn commit(&mut self, status: &str) -> Result<()> {
        let size = self.batch.size_in_bytes() as u64;
        self.state.status = status.to_string();
        self.state.written += self.batch.len() as u64;
        self.state.updated_at = chrono::Utc::now().to_rfc3339();
        if self.dry_run {
            self.batch = WriteBatch::default();
            return Ok(());
        }
        let state_bytes = serde_json::to_vec(&self.state)
            .map_err(|e| raisin_error::Error::storage(format!("repair state encode: {e}")))?;
        let cf_status = cf_handle(self.db, cf::INDEX_STATUS)?;
        let mut batch = std::mem::take(&mut self.batch);
        batch.put_cf(cf_status, &self.state_key, state_bytes);
        self.db
            .write(batch)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
        if size > 0 {
            self.report.batches += 1;
            self.report.max_batch_bytes = self.report.max_batch_bytes.max(size);
            self.throttle(size);
        }
        Ok(())
    }

    /// Hold the average write rate at `max_bytes_per_sec`: after a commit, sleep
    /// until the bytes written so far are no faster than that. The repair runs
    /// on a blocking thread, so a sleep here stalls nothing else.
    fn throttle(&mut self, bytes: u64) {
        self.committed_bytes += bytes;
        if self.max_bytes_per_sec == 0 {
            return;
        }
        let due = std::time::Duration::from_secs_f64(
            self.committed_bytes as f64 / self.max_bytes_per_sec as f64,
        );
        let elapsed = self.started.elapsed();
        if due > elapsed {
            std::thread::sleep(due - elapsed);
        }
    }
}

/// Refuse to start without free space of at least twice `cf_name`'s on-disk
/// size on the data volume: the repair's writes are small, but the compaction
/// that reclaims the superseded entries afterwards rewrites the CF.
pub fn check_headroom(db: &DB, cf_name: &str) -> Result<()> {
    let cf = cf_handle(db, cf_name)?;
    let cf_bytes = db
        .property_int_value_cf(cf, "rocksdb.total-sst-files-size")
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?
        .unwrap_or(0);
    let available = available_bytes(db.path())?;
    if available < cf_bytes.saturating_mul(2) {
        return Err(raisin_error::Error::Validation(format!(
            "repair refused: {available} bytes free on the data volume, need at least \
             {} (2x the {cf_name} column family)",
            cf_bytes.saturating_mul(2)
        )));
    }
    Ok(())
}

/// Free bytes on the volume holding `path`, from `df -Pk` (POSIX output).
fn available_bytes(path: &std::path::Path) -> Result<u64> {
    let output = std::process::Command::new("df")
        .arg("-Pk")
        .arg(path)
        .output()
        .map_err(|e| raisin_error::Error::storage(format!("cannot run df: {e}")))?;
    let text = String::from_utf8_lossy(&output.stdout);
    // Header, then: filesystem, 1024-blocks, used, available, capacity, mount.
    text.lines()
        .nth(1)
        .and_then(|line| line.split_whitespace().nth(3))
        .and_then(|kb| kb.parse::<u64>().ok())
        .map(|kb| kb * 1024)
        .ok_or_else(|| {
            raisin_error::Error::storage(format!("cannot read free space from df: {text}"))
        })
}
