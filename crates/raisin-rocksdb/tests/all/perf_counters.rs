//! RocksDB `PerfContext` counters around a piece of work, so a test can assert
//! how MANY seeks, steps and reads an operation costs rather than how long it
//! took on this machine.
//!
//! The perf context is THREAD-LOCAL. Only work that runs on the calling thread
//! is counted: `count` for synchronous code, `count_async` for a future polled
//! on this thread — which is what `#[tokio::test]`'s default current-thread
//! runtime does. Anything moved to `spawn_blocking` or another worker thread is
//! invisible here.
//!
//! Which counter to assert on:
//!
//! - `iter_read_bytes` is the robust one. It grows with every key and value an
//!   iterator positions on, whether the data sits in a memtable or an SST, so
//!   a walk over N revisions costs N entries' bytes and a seek costs one.
//!   RocksDB only feeds it when DB STATISTICS are enabled
//!   (`RocksDBConfig::enable_statistics`, off in `development()`); without
//!   them it reads 0 and any bound on it passes vacuously.
//! - `next_on_memtable` / `seek_on_memtable` count iterator steps and seeks,
//!   but only against memtables: after a flush they read 0 for both a walk and
//!   a seek. Use them as a supplement, not alone.
//! - `seek_child` counts seeks on every child iterator (memtables and SST
//!   files), so it scales with the LSM shape as well as with the seeks issued.

use rocksdb::perf::{set_perf_stats, PerfContext, PerfMetric, PerfStatsLevel};
use std::future::Future;

/// A snapshot of the counters one operation moved.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PerfCounts {
    /// Seeks against memtables (one per iterator seek per memtable).
    pub seek_on_memtable: u64,
    /// Seeks against every child iterator — memtables and SST files.
    pub seek_child: u64,
    /// Forward steps against memtables.
    pub next_on_memtable: u64,
    /// Point lookups answered from memtables (`get_cf` and friends).
    pub get_from_memtable: u64,
    /// Internal keys stepped over (overwritten or deleted versions).
    pub internal_key_skipped: u64,
    /// Bytes of keys and values iterators positioned on.
    pub iter_read_bytes: u64,
    /// Bytes returned by point lookups.
    pub get_read_bytes: u64,
    pub block_cache_hit: u64,
    pub block_read: u64,
}

impl PerfCounts {
    fn read(ctx: &PerfContext) -> Self {
        Self {
            seek_on_memtable: ctx.metric(PerfMetric::SeekOnMemtableCount),
            seek_child: ctx.metric(PerfMetric::SeekChildSeekCount),
            next_on_memtable: ctx.metric(PerfMetric::NextOnMemtableCount),
            get_from_memtable: ctx.metric(PerfMetric::GetFromMemtableCount),
            internal_key_skipped: ctx.metric(PerfMetric::InternalKeySkippedCount),
            iter_read_bytes: ctx.metric(PerfMetric::IterReadBytes),
            get_read_bytes: ctx.metric(PerfMetric::GetReadBytes),
            block_cache_hit: ctx.metric(PerfMetric::BlockCacheHitCount),
            block_read: ctx.metric(PerfMetric::BlockReadCount),
        }
    }
}

/// Run `f` and return its result with the counters it moved on this thread.
pub fn count<R>(f: impl FnOnce() -> R) -> (R, PerfCounts) {
    set_perf_stats(PerfStatsLevel::EnableCount);
    let mut ctx = PerfContext::default();
    ctx.reset();
    let out = f();
    let counts = PerfCounts::read(&ctx);
    set_perf_stats(PerfStatsLevel::Disable);
    (out, counts)
}

/// Await `fut` and return its output with the counters it moved on this
/// thread. The future must be polled on the calling thread (see module docs).
pub async fn count_async<F: Future>(fut: F) -> (F::Output, PerfCounts) {
    set_perf_stats(PerfStatsLevel::EnableCount);
    let mut ctx = PerfContext::default();
    ctx.reset();
    let out = fut.await;
    let counts = PerfCounts::read(&ctx);
    set_perf_stats(PerfStatsLevel::Disable);
    (out, counts)
}
