//! Conversion of RocksDBConfig to RocksDB options and merge operators.

use super::RocksDBConfig;

impl RocksDBConfig {
    /// Build the **DB-level** options.
    ///
    /// # These are NOT the column-family options
    ///
    /// RocksDB splits its settings in two, and `DB::open_cf_descriptors` takes
    /// them from two different places: this object supplies only the DB-wide
    /// settings, while every CF-scoped setting comes **exclusively** from the
    /// [`ColumnFamilyDescriptor`](rocksdb::ColumnFamilyDescriptor)s.
    ///
    /// Until 2026-08 this function also set `write_buffer_size`,
    /// `max_write_buffer_number`, the block cache, compression,
    /// `target_file_size_base` and `periodic_compaction_seconds` — all of which
    /// are CF options, and all of which RocksDB therefore silently discarded
    /// for all ~49 column families. Notably `cache_index_and_filter_blocks`
    /// stayed `false`, which pins every SST's index and bloom blocks in the
    /// table reader OUTSIDE the block cache, where nothing evicts them; that is
    /// a slow RSS climb that tracks SST count rather than traffic.
    ///
    /// So: **anything CF-scoped belongs in `column_family_options`, not here.**
    /// If you add a setter to this function, check the RocksDB docs first —
    /// if it is a CF option it will do nothing.
    pub fn to_rocksdb_options(&self) -> rocksdb::Options {
        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(self.create_if_missing);
        opts.create_missing_column_families(true);
        opts.set_paranoid_checks(self.enable_checksums);

        // Global memtable budget across ALL column families. The per-CF
        // write_buffer_size is charged ~49 times over; this is the only bound
        // on the total, and without it idle CFs each sit on their own
        // partially-filled memtable indefinitely.
        if self.db_write_buffer_size > 0 {
            opts.set_db_write_buffer_size(self.db_write_buffer_size);
        }

        // RocksDB now uses max_background_jobs instead of separate compaction/flush settings
        opts.set_max_background_jobs(self.max_background_compactions + self.max_background_flushes);
        opts.set_max_open_files(self.max_open_files);

        // Statistics
        if self.enable_statistics {
            opts.enable_statistics();
        }

        // Merge operator for atomic counter increments (used for revision allocation)
        opts.set_merge_operator_associative("uint64_add", Self::merge_uint64_add);

        // Disk-growth bounds (2026-07 prod incident: 1.3G of pinned WALs,
        // 266M of rotated info logs, no age-based compaction — all snapshotted
        // into every nightly backup).
        //
        // With 40+ column families and tiny write volume, memtables never fill,
        // so the oldest WAL stays pinned by some un-flushed CF and WALs
        // accumulate (default max_total_wal_size=0 derives a ~40G cap from the
        // summed memtable budgets). A 256MB cap forces the pinning CFs to
        // flush so old WALs can be deleted.
        opts.set_max_total_wal_size(256 * 1024 * 1024);
        // Bound rotated info logs (defaults keep up to 1000 files).
        opts.set_keep_log_file_num(10);
        opts.set_max_log_file_size(64 * 1024 * 1024);
        // NOTE: age-based compaction is a CF option and now lives in
        // `column_family_options`.

        // Additional production settings
        opts.set_use_fsync(true); // Ensure data durability
        opts.set_wal_recovery_mode(rocksdb::DBRecoveryMode::PointInTime);
        opts.increase_parallelism(num_cpus::get() as i32);
        // NOTE: `optimize_for_point_lookup` used to be called here. It is a CF
        // option, so it did nothing for any real CF — but it is also actively
        // wrong for us: it REPLACES the block-based table factory (discarding
        // the shared cache and the bloom filter configured alongside it) and
        // allocates a second, private block cache of its own. Do not restore it.

        opts
    }

    /// Build the **column-family** options, shared by every CF.
    ///
    /// `cache` must be the single process-wide block cache so that all column
    /// families charge one bounded pool; see [`shared_block_cache`].
    ///
    /// Callers layer per-CF extras (prefix extractors, compaction filters) on
    /// top of the returned value — see `create_column_family_descriptors`.
    pub fn column_family_options(&self, cache: &rocksdb::Cache) -> rocksdb::Options {
        let mut opts = rocksdb::Options::default();

        opts.set_block_based_table_factory(
            &self.block_table_options(cache, self.bloom_filter_bits),
        );

        opts.set_write_buffer_size(self.write_buffer_size);
        opts.set_max_write_buffer_number(self.max_write_buffer_number);
        opts.set_compression_type(self.compression.to_rocksdb());
        opts.set_target_file_size_base(self.target_file_size_base);

        // Age-based compaction so obsolete revisions/tombstones are reclaimed
        // even when write volume never triggers size-based compaction. Most of
        // this database's index CFs encode the revision IN the key, so an
        // update writes a NEW key and size-based compaction has nothing to
        // collapse on a low-write deployment.
        opts.set_periodic_compaction_seconds(24 * 60 * 60);

        opts
    }

    /// Block-table options for one column family, bound to the shared cache.
    ///
    /// The ORDER of these calls matters and has bitten us: RocksDB's
    /// `set_block_based_table_factory` SNAPSHOTS the options into the factory,
    /// so any setter called on the `BlockBasedOptions` afterwards is silently
    /// discarded. `cache_index_and_filter_blocks` was set after the snapshot
    /// and was dead for that reason. Configure fully, then install.
    pub fn block_table_options(
        &self,
        cache: &rocksdb::Cache,
        bloom_bits: f64,
    ) -> rocksdb::BlockBasedOptions {
        let mut block_opts = rocksdb::BlockBasedOptions::default();
        block_opts.set_block_cache(cache);
        block_opts.set_bloom_filter(bloom_bits, false);
        // Charge index and filter blocks to the (bounded) block cache instead
        // of pinning them in the table reader forever. Without this they are
        // capped only by `max_open_files` and grow with SST count for the life
        // of the process — the signature of a slow, traffic-independent RSS
        // climb.
        block_opts.set_cache_index_and_filter_blocks(true);
        // ...but keep L0's index/filter blocks pinned, so the change above does
        // not turn every point lookup into extra cache churn on the hottest
        // level. Pinned L0 metadata is still charged to the cache.
        block_opts.set_pin_l0_filter_and_index_blocks_in_cache(true);
        block_opts
    }

    /// Allocate the one block cache shared by every column family.
    pub fn shared_block_cache(&self) -> rocksdb::Cache {
        rocksdb::Cache::new_lru_cache(self.block_cache_size)
    }

    /// Merge operator for atomic u64 addition
    ///
    /// This merge operator enables atomic counter increments without read-modify-write cycles.
    /// Used primarily for revision counter allocation in the versioning system.
    fn merge_uint64_add(
        _key: &[u8],
        existing_val: Option<&[u8]>,
        operands: &rocksdb::MergeOperands,
    ) -> Option<Vec<u8>> {
        let mut counter: u64 = existing_val
            .and_then(|v| {
                if v.len() == 8 {
                    Some(u64::from_le_bytes(v.try_into().ok()?))
                } else {
                    None
                }
            })
            .unwrap_or(0);

        for op in operands {
            if op.len() == 8 {
                if let Ok(bytes) = <[u8; 8]>::try_from(op) {
                    counter = counter.saturating_add(u64::from_le_bytes(bytes));
                }
            }
        }

        Some(counter.to_le_bytes().to_vec())
    }
}
