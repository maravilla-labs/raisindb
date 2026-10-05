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
    /// The `property_index` rebuild's invalidation epoch when this run
    /// STARTED (kept across a resume): `done` is committed only while it is
    /// still current (`property_state::rebuild_epoch`). For a `failed`
    /// `compound_builds` link: the work the branch still owed when it failed
    /// (`compound_detect::work_fingerprint`), which a targeted request
    /// compares before running it again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch: Option<String>,
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

/// A TEST hook run at the start of every commit, before the repair re-checks
/// what it staged (`node_path`) — the window a concurrent writer can land in.
/// Tests use it to interleave a write there; production never sets it.
#[derive(Clone)]
pub struct CommitHook(pub std::sync::Arc<dyn Fn() + Send + Sync>);

impl std::fmt::Debug for CommitHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CommitHook(..)")
    }
}

/// Accumulates one repair's writes and commits them in bounded batches, each
/// together with the state record that names its last key.
pub(crate) struct BoundedWriter<'a> {
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
    before_commit: Option<CommitHook>,
    started: std::time::Instant,
    committed_bytes: u64,
    /// Bytes read since the last commit, charged to the throttle with it.
    read_bytes: u64,
    pub(crate) report: BatchReport,
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
    pub(crate) fn new(
        db: &'a DB,
        state_key: Vec<u8>,
        state: RepairState,
        batch_bytes: usize,
        dry_run: bool,
        stop_after_batches: Option<usize>,
        max_bytes_per_sec: u64,
        before_commit: Option<CommitHook>,
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
            before_commit,
            started: std::time::Instant::now(),
            committed_bytes: 0,
            read_bytes: 0,
            report: BatchReport::default(),
        }
    }

    pub(crate) fn state(&self) -> &RepairState {
        &self.state
    }

    /// Record the invalidation epoch this run started under.
    pub(crate) fn set_epoch(&mut self, epoch: Option<String>) {
        self.state.epoch = epoch;
    }

    /// Forget the cursor (the next run starts from the beginning).
    pub(crate) fn clear_cursor(&mut self) {
        self.state.cursor = None;
    }

    /// Queue one write.
    pub(crate) fn put(&mut self, cf_name: &str, key: &[u8], value: &[u8]) -> Result<()> {
        self.report.written += 1;
        self.report.bytes += (key.len() + value.len()) as u64;
        if !self.dry_run {
            let cf = cf_handle(self.db, cf_name)?;
            self.batch.put_cf(cf, key, value);
        }
        Ok(())
    }

    /// Queue one delete (run-collapse GC). Counted like a write: the batch
    /// bound and the throttle see its key bytes.
    pub(crate) fn delete(&mut self, cf_name: &str, key: &[u8]) -> Result<()> {
        self.report.written += 1;
        self.report.bytes += key.len() as u64;
        if !self.dry_run {
            let cf = cf_handle(self.db, cf_name)?;
            self.batch.delete_cf(cf, key);
        }
        Ok(())
    }

    /// Charge `bytes` READ to the throttle at the next commit: a pass that
    /// scans far more than it writes (run-collapse over clean data) is held
    /// to the same rate as one that writes.
    pub(crate) fn note_read(&mut self, bytes: u64) {
        self.read_bytes += bytes;
    }

    /// Mark `key` (of `pass`) as fully processed, and commit if the batch has
    /// reached its size. Returns `false` when the run must stop (the crash
    /// hook fired).
    pub(crate) fn checkpoint(&mut self, pass: &str, key: &[u8]) -> Result<bool> {
        self.mark(pass, key);
        if self.batch_full(0) {
            self.commit("running")?;
            if self.stop_requested() {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// [`Self::checkpoint`] for a pass that mostly READS: it also commits
    /// (the cursor, and whatever is queued) once the bytes read since the
    /// last commit reach the batch size, so a scan over clean data is held to
    /// the rate limit and persists its cursor instead of restarting from the
    /// beginning after a crash.
    pub(crate) fn checkpoint_scan(&mut self, pass: &str, key: &[u8]) -> Result<bool> {
        self.mark(pass, key);
        if self.batch_full(0) || self.read_bytes >= self.batch_bytes as u64 {
            self.commit("running")?;
            if self.stop_requested() {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Mark `key` (of `pass`) as fully processed, without committing.
    pub(crate) fn mark(&mut self, pass: &str, key: &[u8]) {
        self.state.pass = pass.to_string();
        self.state.cursor = Some(hex::encode(key));
    }

    /// Whether the queued batch plus `staged` bytes held elsewhere has
    /// reached the configured size.
    pub(crate) fn batch_full(&self, staged: usize) -> bool {
        self.batch.size_in_bytes() + staged >= self.batch_bytes
    }

    /// Whether the crash hook says to stop now.
    pub(crate) fn stop_requested(&self) -> bool {
        self.stop_after_batches
            .is_some_and(|limit| self.report.batches as usize >= limit)
    }

    /// Whether this run only counts.
    pub(crate) fn dry_run(&self) -> bool {
        self.dry_run
    }

    /// Start a new pass: its cursor begins empty.
    pub(crate) fn begin_pass(&mut self, pass: &str) {
        if self.state.pass != pass {
            self.state.pass = pass.to_string();
            self.state.cursor = None;
        }
    }

    /// Commit what is queued, with the state record, as `status`.
    pub(crate) fn commit(&mut self, status: &str) -> Result<()> {
        self.commit_prepared(status, |_| Ok(()))
    }

    /// Commit as `status`, first letting `prepare` queue the writes that are
    /// only valid if re-checked immediately before the batch is written. The
    /// guard `prepare` returns is held until the write is done, and released
    /// before the throttle sleeps.
    pub(crate) fn commit_prepared<G>(
        &mut self,
        status: &str,
        prepare: impl FnOnce(&mut Self) -> Result<G>,
    ) -> Result<()> {
        if let Some(hook) = &self.before_commit {
            (hook.0)();
        }
        let guard = prepare(self)?;
        let size = self.write(status)?;
        drop(guard);
        if size > 0 {
            self.report.batches += 1;
            self.report.max_batch_bytes = self.report.max_batch_bytes.max(size);
        }
        let charged = size + std::mem::take(&mut self.read_bytes);
        if charged > 0 {
            self.throttle(charged);
        }
        Ok(())
    }

    /// Write what is queued, with the state record, as `status`. Returns the
    /// size of the repair's own writes.
    fn write(&mut self, status: &str) -> Result<u64> {
        let size = self.batch.size_in_bytes() as u64;
        self.state.status = status.to_string();
        self.state.written += self.batch.len() as u64;
        self.state.updated_at = chrono::Utc::now().to_rfc3339();
        if self.dry_run {
            self.batch = WriteBatch::default();
            return Ok(0);
        }
        let state_bytes = serde_json::to_vec(&self.state)
            .map_err(|e| raisin_error::Error::storage(format!("repair state encode: {e}")))?;
        let cf_status = cf_handle(self.db, cf::INDEX_STATUS)?;
        let mut batch = std::mem::take(&mut self.batch);
        batch.put_cf(cf_status, &self.state_key, state_bytes);
        self.db
            .write(batch)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
        Ok(size)
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
