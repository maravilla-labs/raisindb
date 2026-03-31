//! Configuration presets: development, production, and high-performance.

use super::{CompressionType, ReplicationPeerConfig, RocksDBConfig};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

impl RocksDBConfig {
    /// Development configuration with minimal resources
    ///
    /// Optimized for fast startup and low resource usage during development.
    /// - Smaller caches (128MB block cache, 16MB write buffer)
    /// - No background jobs or auto-healing
    /// - Minimal parallelism
    /// - No statistics collection
    pub fn development() -> Self {
        Self {
            path: PathBuf::from("./data"),
            create_if_missing: true,
            block_cache_size: 128 * 1024 * 1024, // 128MB
            write_buffer_size: 16 * 1024 * 1024, // 16MB
            max_write_buffer_number: 2,
            bloom_filter_bits: 10.0,
            compression: CompressionType::Snappy,
            enable_statistics: false,
            enable_checksums: false,
            integrity_check_interval: Duration::from_secs(24 * 60 * 60), // 24 hours
            auto_heal_enabled: false,
            background_jobs_enabled: false,
            tenant_resource_limits: HashMap::new(),
            worker_pool_size: 10,
            target_file_size_base: 64 * 1024 * 1024,
            max_background_compactions: 1,
            max_background_flushes: 1,
            max_open_files: 256,
            cluster_node_id: None,      // Will be auto-generated
            replication_enabled: false, // Disabled by default in development
            replication_peers: Vec::new(),
            async_operation_queue: false, // Disabled for development (simpler debugging)
            operation_queue_capacity: 1_000,
            operation_queue_batch_size: 50,
            operation_queue_batch_timeout_ms: 100,
            oplog_compaction_enabled: false, // Disabled by default in development
            oplog_compaction_interval_secs: 21600, // 6 hours
            oplog_compaction_min_age_secs: 3600, // 1 hour
            oplog_merge_property_updates: true,
            oplog_compaction_batch_size: 100_000,
        }
    }

    /// Production configuration with optimized settings
    ///
    /// Balanced configuration for production deployments.
    /// - Medium caches (512MB block cache, 64MB write buffer)
    /// - Background jobs and auto-healing enabled
    /// - Statistics and checksums enabled
    /// - Moderate parallelism (4 compactions, 2 flushes)
    pub fn production() -> Self {
        Self {
            path: PathBuf::from("./data"),
            create_if_missing: true,
            block_cache_size: 512 * 1024 * 1024, // 512MB
            write_buffer_size: 64 * 1024 * 1024, // 64MB
            max_write_buffer_number: 4,
            bloom_filter_bits: 10.0,
            compression: CompressionType::Snappy,
            enable_statistics: true,
            enable_checksums: true,
            integrity_check_interval: Duration::from_secs(6 * 60 * 60), // 6 hours
            auto_heal_enabled: true,
            background_jobs_enabled: true,
            tenant_resource_limits: HashMap::new(),
            worker_pool_size: 20,
            target_file_size_base: 64 * 1024 * 1024,
            max_background_compactions: 4,
            max_background_flushes: 2,
            max_open_files: 5000,
            cluster_node_id: None, // Should be configured explicitly in production
            replication_enabled: true, // Enabled by default in production
            replication_peers: Vec::new(),
            async_operation_queue: true, // Enabled for production high-throughput
            operation_queue_capacity: 10_000,
            operation_queue_batch_size: 100,
            operation_queue_batch_timeout_ms: 100,
            oplog_compaction_enabled: true, // Enabled for production
            oplog_compaction_interval_secs: 21600, // 6 hours
            oplog_compaction_min_age_secs: 3600, // 1 hour
            oplog_merge_property_updates: true,
            oplog_compaction_batch_size: 100_000,
        }
    }

    /// High-performance configuration for large deployments
    ///
    /// Optimized for maximum throughput and performance.
    /// - Large caches (2GB block cache, 128MB write buffer)
    /// - Aggressive parallelism (8 compactions, 4 flushes)
    /// - LZ4 compression for better CPU/compression tradeoff
    /// - Statistics and checksums enabled
    pub fn high_performance() -> Self {
        Self {
            path: PathBuf::from("./data"),
            create_if_missing: true,
            block_cache_size: 2048 * 1024 * 1024, // 2GB
            write_buffer_size: 128 * 1024 * 1024, // 128MB
            max_write_buffer_number: 6,
            bloom_filter_bits: 10.0,
            compression: CompressionType::Lz4,
            enable_statistics: true,
            enable_checksums: true,
            integrity_check_interval: Duration::from_secs(3 * 60 * 60), // 3 hours
            auto_heal_enabled: true,
            background_jobs_enabled: true,
            tenant_resource_limits: HashMap::new(),
            worker_pool_size: 40,
            target_file_size_base: 128 * 1024 * 1024,
            max_background_compactions: 8,
            max_background_flushes: 4,
            max_open_files: 10000,
            cluster_node_id: None,     // Should be configured explicitly
            replication_enabled: true, // Enabled for high-performance clusters
            replication_peers: Vec::new(),
            async_operation_queue: true, // Critical for high-throughput scenarios
            operation_queue_capacity: 50_000,
            operation_queue_batch_size: 500,
            operation_queue_batch_timeout_ms: 50,
            oplog_compaction_enabled: true, // Enabled for high-performance
            oplog_compaction_interval_secs: 10800, // 3 hours (more frequent for high-volume)
            oplog_compaction_min_age_secs: 1800, // 30 minutes (shorter for high-volume)
            oplog_merge_property_updates: true,
            oplog_compaction_batch_size: 500_000, // Larger batch for high-performance
        }
    }
}
