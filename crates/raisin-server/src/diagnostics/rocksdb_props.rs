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

//! Per-column-family RocksDB memory properties.
//!
//! RocksDB's heap does not go through Rust's global allocator, so it is
//! invisible to [`super::allocator`] — it shows up in process RSS and nowhere
//! else. These properties are the only way to attribute that portion, and
//! nothing in this workspace read a single one of them before 2026-08.
//!
//! # What to look at
//!
//! - **`estimate_table_readers_mem`** — index and bloom-filter blocks held by
//!   open SSTs. If `cache_index_and_filter_blocks` is off this is unbounded
//!   except by `max_open_files` and grows with SST count over *days*, entirely
//!   independent of traffic. That is the classic slow-creep signature, and it
//!   was our state until the CF options were fixed. It should now plateau, with
//!   the memory charged to `block_cache_usage` instead.
//! - **`cur_size_all_mem_tables`** — unflushed writes. With ~49 column families
//!   each holding its own memtable, this is the other way write-side memory
//!   grows without any single CF looking busy.
//! - **`block_cache_usage`** — should approach the configured cache size and
//!   then STOP. It is bounded by construction; if it is far below the
//!   configured size, some CF is not sharing the cache.
//! - **`num_live_versions`** high with `compaction_pending` true means
//!   compaction is falling behind and superseded revisions are accumulating.
//!
//! Sum across CFs and compare the total against `process_rss_bytes`: whatever
//! is left over is Rust heap, Tantivy mmaps, or thread stacks.

use serde::Serialize;
use std::collections::BTreeMap;

use raisin_rocksdb::RocksDBStorage;

/// Memory-relevant properties for one column family.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ColumnFamilyProps {
    /// Index + filter blocks pinned by open table readers.
    pub estimate_table_readers_mem: Option<u64>,
    /// Bytes in active (unflushed) memtables.
    pub cur_size_all_mem_tables: Option<u64>,
    /// Bytes in all memtables, including those being flushed.
    pub size_all_mem_tables: Option<u64>,
    /// Block cache bytes charged by this CF. Shared, so summing across CFs
    /// over-counts — read the max, not the total.
    pub block_cache_usage: Option<u64>,
    /// Block cache bytes that cannot be evicted.
    pub block_cache_pinned_usage: Option<u64>,
    /// Live versions; high values mean iterators or compaction are holding
    /// obsolete files open.
    pub num_live_versions: Option<u64>,
    /// On-disk size of live SSTs.
    pub live_sst_files_size: Option<u64>,
    /// Number of live SST files — the thing `estimate_table_readers_mem`
    /// scales with.
    pub num_live_sst_files: Option<u64>,
    pub estimate_num_keys: Option<u64>,
    /// 1 when compaction is needed but has not run.
    pub compaction_pending: Option<u64>,
    pub num_running_compactions: Option<u64>,
    pub num_running_flushes: Option<u64>,
}

/// A whole-database sample: per-CF properties plus roll-ups.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RocksDbMemory {
    /// Summed `estimate_table_readers_mem`. **The number to watch** for the
    /// slow creep.
    pub total_table_readers_bytes: u64,
    /// Summed `cur_size_all_mem_tables` across all column families.
    pub total_memtable_bytes: u64,
    /// Largest `block_cache_usage` seen. The cache is shared, so this is the
    /// usage, not a per-CF figure to add up.
    pub block_cache_usage_bytes: u64,
    pub block_cache_pinned_bytes: u64,
    /// Summed on-disk live SST size.
    pub total_live_sst_bytes: u64,
    pub total_live_sst_files: u64,
    /// Column families with a pending compaction — a persistent nonzero value
    /// means garbage is accumulating faster than it is reclaimed.
    pub column_families_pending_compaction: u64,
    /// Per-CF detail, only for CFs that report a nonzero value somewhere, so an
    /// idle database does not emit 49 blocks of zeros.
    pub column_families: BTreeMap<String, ColumnFamilyProps>,
    /// Column families that could not be read at all.
    pub unreadable_column_families: Vec<String>,
}

/// Read every memory-relevant property for every column family.
///
/// All reads are O(1) property lookups — no scans, no iterators — so this is
/// safe to poll on a monitoring interval.
pub fn sample(storage: &RocksDBStorage) -> RocksDbMemory {
    let db = storage.db();
    let mut out = RocksDbMemory::default();

    for cf_name in raisin_rocksdb::all_column_family_names() {
        let Some(cf) = db.cf_handle(cf_name) else {
            out.unreadable_column_families.push(cf_name.to_string());
            continue;
        };

        let read = |prop: &str| db.property_int_value_cf(&cf, prop).ok().flatten();

        let props = ColumnFamilyProps {
            estimate_table_readers_mem: read("rocksdb.estimate-table-readers-mem"),
            cur_size_all_mem_tables: read("rocksdb.cur-size-all-mem-tables"),
            size_all_mem_tables: read("rocksdb.size-all-mem-tables"),
            block_cache_usage: read("rocksdb.block-cache-usage"),
            block_cache_pinned_usage: read("rocksdb.block-cache-pinned-usage"),
            num_live_versions: read("rocksdb.num-live-versions"),
            live_sst_files_size: read("rocksdb.live-sst-files-size"),
            // RocksDB exposes no single "live SST count" integer property;
            // it is per level, so sum the levels.
            num_live_sst_files: (0..7)
                .filter_map(|level| read(&format!("rocksdb.num-files-at-level{level}")))
                .reduce(|a, b| a + b),
            estimate_num_keys: read("rocksdb.estimate-num-keys"),
            compaction_pending: read("rocksdb.compaction-pending"),
            num_running_compactions: read("rocksdb.num-running-compactions"),
            num_running_flushes: read("rocksdb.num-running-flushes"),
        };

        out.total_table_readers_bytes += props.estimate_table_readers_mem.unwrap_or(0);
        out.total_memtable_bytes += props.cur_size_all_mem_tables.unwrap_or(0);
        out.total_live_sst_bytes += props.live_sst_files_size.unwrap_or(0);
        out.total_live_sst_files += props.num_live_sst_files.unwrap_or(0);
        // Shared cache: take the max rather than summing, or a 512MB cache
        // reads as 25GB across 49 CFs.
        out.block_cache_usage_bytes = out
            .block_cache_usage_bytes
            .max(props.block_cache_usage.unwrap_or(0));
        out.block_cache_pinned_bytes = out
            .block_cache_pinned_bytes
            .max(props.block_cache_pinned_usage.unwrap_or(0));
        if props.compaction_pending.unwrap_or(0) > 0 {
            out.column_families_pending_compaction += 1;
        }

        // Skip CFs that hold nothing worth looking at — an idle database would
        // otherwise emit 49 near-identical zero blocks and bury the two that
        // matter, on a path groundcrew polls every 30s.
        //
        // The memtable threshold is NOT zero: RocksDB reports ~2 KB of arena
        // for a completely empty memtable, so `> 0` matches every column family
        // and filters nothing. Only a memtable with real content counts.
        const EMPTY_MEMTABLE_ARENA_CEILING: u64 = 64 * 1024;
        let interesting = props.estimate_table_readers_mem.unwrap_or(0) > 0
            || props.cur_size_all_mem_tables.unwrap_or(0) > EMPTY_MEMTABLE_ARENA_CEILING
            || props.live_sst_files_size.unwrap_or(0) > 0;
        if interesting {
            out.column_families.insert(cf_name.to_string(), props);
        }
    }

    out
}
