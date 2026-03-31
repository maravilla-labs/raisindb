//! Conversion of RocksDBConfig to RocksDB options and merge operators.

use super::RocksDBConfig;

impl RocksDBConfig {
    /// Convert configuration to RocksDB options
    ///
    /// Applies all configuration settings to create a RocksDB Options object.
    /// This includes:
    /// - Block cache and bloom filters
    /// - Write buffer configuration
    /// - Compression settings
    /// - Parallelism based on CPU cores
    /// - Merge operator for atomic counters
    /// - Production durability settings (fsync, WAL recovery)
    pub fn to_rocksdb_options(&self) -> rocksdb::Options {
        let mut opts = rocksdb::Options::default();
        opts.create_if_missing(self.create_if_missing);
        opts.create_missing_column_families(true);
        opts.set_paranoid_checks(self.enable_checksums);

        // Block cache configuration
        let cache = rocksdb::Cache::new_lru_cache(self.block_cache_size);
        let mut block_opts = rocksdb::BlockBasedOptions::default();
        block_opts.set_block_cache(&cache);
        block_opts.set_bloom_filter(self.bloom_filter_bits, false);
        opts.set_block_based_table_factory(&block_opts);

        // Add this line to cache metadata blocks as well
        block_opts.set_cache_index_and_filter_blocks(true);

        // Write buffer configuration
        opts.set_write_buffer_size(self.write_buffer_size);
        opts.set_max_write_buffer_number(self.max_write_buffer_number);

        // Compression
        opts.set_compression_type(self.compression.to_rocksdb());

        // Performance tuning
        opts.set_target_file_size_base(self.target_file_size_base);
        // RocksDB now uses max_background_jobs instead of separate compaction/flush settings
        opts.set_max_background_jobs(self.max_background_compactions + self.max_background_flushes);
        opts.set_max_open_files(self.max_open_files);

        // Statistics
        if self.enable_statistics {
            opts.enable_statistics();
        }

        // Merge operator for atomic counter increments (used for revision allocation)
        opts.set_merge_operator_associative("uint64_add", Self::merge_uint64_add);

        // Additional production settings
        opts.set_use_fsync(true); // Ensure data durability
        opts.set_wal_recovery_mode(rocksdb::DBRecoveryMode::PointInTime);
        opts.increase_parallelism(num_cpus::get() as i32);
        // Convert bytes to MB for optimize_for_point_lookup
        opts.optimize_for_point_lookup((self.block_cache_size / (1024 * 1024)) as u64);

        opts
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
