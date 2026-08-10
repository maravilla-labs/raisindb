// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Runtime memory diagnostics.
//!
//! # Why this exists
//!
//! Production RSS climbed steadily for months and the server could not report
//! one number about its own memory: no allocator counters (the `stats` feature
//! was compiled in but nothing read it), no RocksDB properties (zero
//! `GetProperty` calls in the entire workspace), and a `/metrics` endpoint
//! whose RocksDB implementation returned hardcoded zeros. Every investigation
//! therefore had to watch cgroup RSS from outside and guess at the mechanism.
//!
//! # How to read the payload
//!
//! The three plausible causes have **disjoint signatures**, so one set of
//! samples separates them without changing any code:
//!
//! - `allocator.allocated_bytes` climbing → a genuine Rust-heap leak. Look at
//!   `gauges` to see which collection.
//! - `allocated_bytes` flat while `resident_bytes` / `retained_bytes` climb →
//!   allocator retention or fragmentation, not a leak. Reach for
//!   `/diagnostics/malloc-stats` and the `dirty_decay_ms` knobs.
//! - both flat while `rocksdb.total_table_readers_bytes` or
//!   `total_memtable_bytes` track the RSS slope → the storage engine.
//!
//! `unattributed_bytes` is the residue: process RSS minus what the allocator
//! and RocksDB admit to. Tantivy's mmap'd index segments and thread stacks land
//! there, so a large and growing residue points at those rather than at
//! anything the first three lines cover.
//!
//! Sample repeatedly. A single reading tells you almost nothing; the slope
//! across a few hours tells you everything.

pub mod allocator;
pub mod gauges;
#[cfg(feature = "storage-rocksdb")]
pub mod rocksdb_props;

use serde::Serialize;

/// One complete memory sample.
#[derive(Debug, Clone, Serialize)]
pub struct MemoryDiagnostics {
    /// Milliseconds since the Unix epoch, so a collector can build a series
    /// without trusting its own clock for ordering.
    pub sampled_at_ms: u64,
    pub allocator: allocator::AllocatorStats,
    #[cfg(feature = "storage-rocksdb")]
    pub rocksdb: rocksdb_props::RocksDbMemory,
    pub gauges: gauges::Gauges,
    /// Process RSS minus jemalloc `resident` minus RocksDB's reported totals.
    /// `None` when process RSS is unavailable (non-Linux). Negative residues
    /// are clamped to zero — the inputs are sampled at slightly different
    /// instants and RocksDB's figures are estimates.
    pub unattributed_bytes: Option<u64>,
}

/// Take a full sample.
#[cfg(feature = "storage-rocksdb")]
pub async fn sample(storage: &raisin_rocksdb::RocksDBStorage) -> MemoryDiagnostics {
    let allocator = allocator::sample();
    let rocksdb = rocksdb_props::sample(storage);
    let gauges = gauges::sample(Some(storage.job_registry())).await;

    let unattributed_bytes = allocator.process_rss_bytes.map(|rss| {
        let accounted = allocator.resident_bytes.unwrap_or(0)
            + rocksdb.total_table_readers_bytes
            + rocksdb.total_memtable_bytes
            + rocksdb.block_cache_usage_bytes;
        rss.saturating_sub(accounted)
    });

    MemoryDiagnostics {
        sampled_at_ms: now_ms(),
        allocator,
        rocksdb,
        gauges,
        unattributed_bytes,
    }
}

/// Take a full sample without a RocksDB backend.
#[cfg(not(feature = "storage-rocksdb"))]
pub async fn sample() -> MemoryDiagnostics {
    let allocator = allocator::sample();
    let gauges = gauges::sample(None).await;

    let unattributed_bytes = allocator
        .process_rss_bytes
        .map(|rss| rss.saturating_sub(allocator.resident_bytes.unwrap_or(0)));

    MemoryDiagnostics {
        sampled_at_ms: now_ms(),
        allocator,
        gauges,
        unattributed_bytes,
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
