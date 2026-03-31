//! Production-ready configuration for RocksDB storage
//!
//! This module provides configuration management for RocksDB with support for:
//! - Performance tuning (caching, compression, parallelism)
//! - Development/Production/High-Performance presets
//! - Tenant resource limits
//! - Atomic counter merge operators
//! - Integrity checking and background jobs

mod presets;
mod rocksdb_options;
#[cfg(test)]
mod tests;

pub use presets::*;

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

/// Configuration for a replication peer
#[derive(Debug, Clone)]
pub struct ReplicationPeerConfig {
    /// Unique identifier for this peer
    pub peer_id: String,
    /// Base URL for the peer's HTTP API
    pub url: String,
    /// Whether sync with this peer is enabled
    pub enabled: bool,
    /// Sync interval in seconds (default: 60)
    pub sync_interval_secs: u64,
    /// Batch size for fetching operations (default: 1000)
    pub batch_size: usize,
}

impl ReplicationPeerConfig {
    /// Create a new peer configuration
    pub fn new(peer_id: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            peer_id: peer_id.into(),
            url: url.into(),
            enabled: true,
            sync_interval_secs: 60,
            batch_size: 1000,
        }
    }
}

/// Production-ready configuration for RocksDB storage
#[derive(Debug, Clone)]
pub struct RocksDBConfig {
    // Basic configuration
    /// Path to the RocksDB data directory
    pub path: PathBuf,
    /// Create database if it doesn't exist
    pub create_if_missing: bool,

    // Performance tuning
    /// Block cache size in bytes (default: 512MB)
    pub block_cache_size: usize,
    /// Write buffer size in bytes (default: 64MB)
    pub write_buffer_size: usize,
    /// Maximum number of write buffers (default: 4)
    pub max_write_buffer_number: i32,
    /// Bits per key for bloom filter (default: 10)
    pub bloom_filter_bits: f64,
    /// Compression type for data blocks
    pub compression: CompressionType,
    /// Enable statistics collection
    pub enable_statistics: bool,
    /// Enable paranoid checks (checksums on reads)
    pub enable_checksums: bool,

    // Management features
    /// Interval for integrity checks
    pub integrity_check_interval: Duration,
    /// Enable automatic self-healing
    pub auto_heal_enabled: bool,
    /// Enable background jobs
    pub background_jobs_enabled: bool,
    /// Per-tenant resource limits
    pub tenant_resource_limits: HashMap<String, TenantLimits>,
    /// Number of worker threads in the unified job worker pool
    pub worker_pool_size: usize,

    // Performance tuning
    /// Target file size for Level-0 (default: 64MB)
    pub target_file_size_base: u64,
    /// Maximum number of concurrent background compactions
    pub max_background_compactions: i32,
    /// Maximum number of concurrent background flushes
    pub max_background_flushes: i32,
    /// Maximum number of open files
    pub max_open_files: i32,

    // Replication configuration
    /// Unique cluster node ID for this server instance (for CRDT replication)
    /// If None, a random ID will be generated
    pub cluster_node_id: Option<String>,
    /// Enable operation capture for replication
    pub replication_enabled: bool,
    /// List of replication peers (for pull-based sync)
    pub replication_peers: Vec<ReplicationPeerConfig>,

    // Operation queue configuration (async capture for high throughput)
    /// Enable async operation queue for non-blocking operation capture
    pub async_operation_queue: bool,
    /// Queue capacity (maximum operations in queue before backpressure)
    pub operation_queue_capacity: usize,
    /// Batch size for queue processing (operations per batch)
    pub operation_queue_batch_size: usize,
    /// Batch timeout in milliseconds (max wait for full batch)
    pub operation_queue_batch_timeout_ms: u64,

    // Operation log compaction configuration
    /// Enable periodic operation log compaction
    pub oplog_compaction_enabled: bool,
    /// Compaction interval in seconds (default: 21600 = 6 hours)
    pub oplog_compaction_interval_secs: u64,
    /// Minimum age of operations to compact in seconds (default: 3600 = 1 hour)
    pub oplog_compaction_min_age_secs: u64,
    /// Whether to merge consecutive SetProperty operations
    pub oplog_merge_property_updates: bool,
    /// Maximum operations to process per compaction run
    pub oplog_compaction_batch_size: usize,
}

/// Compression types supported by RocksDB
#[derive(Debug, Clone, Copy)]
pub enum CompressionType {
    None,
    Snappy,
    Zlib,
    Bz2,
    Lz4,
    Lz4hc,
    Zstd,
}

/// Configuration for a single job worker pool category
#[derive(Debug, Clone)]
pub struct JobPoolConfig {
    /// Number of lightweight dispatcher workers
    pub dispatcher_workers: usize,
    /// Number of tokio threads in the pool's dedicated runtime
    pub runtime_threads: usize,
    /// Maximum concurrent handler tasks (semaphore permits)
    pub max_concurrent_handlers: usize,
}

/// Configuration for the three-pool job system
#[derive(Debug, Clone)]
pub struct JobPoolsConfig {
    /// Realtime pool: triggers, functions, AI, flows
    pub realtime: JobPoolConfig,
    /// Background pool: indexing, embedding, replication
    pub background: JobPoolConfig,
    /// System pool: auth, packages, cleanup
    pub system: JobPoolConfig,
}

impl JobPoolsConfig {
    /// Development preset — minimal resources
    pub fn development() -> Self {
        Self {
            realtime: JobPoolConfig {
                dispatcher_workers: 3,
                runtime_threads: 32,
                max_concurrent_handlers: 30,
            },
            background: JobPoolConfig {
                dispatcher_workers: 2,
                runtime_threads: 8,
                max_concurrent_handlers: 10,
            },
            system: JobPoolConfig {
                dispatcher_workers: 2,
                runtime_threads: 8,
                max_concurrent_handlers: 10,
            },
        }
    }

    /// Production preset — balanced for typical workloads
    pub fn production() -> Self {
        Self {
            realtime: JobPoolConfig {
                dispatcher_workers: 4,
                runtime_threads: 64,
                max_concurrent_handlers: 50,
            },
            background: JobPoolConfig {
                dispatcher_workers: 4,
                runtime_threads: 16,
                max_concurrent_handlers: 20,
            },
            system: JobPoolConfig {
                dispatcher_workers: 2,
                runtime_threads: 8,
                max_concurrent_handlers: 10,
            },
        }
    }

    /// High-performance preset — maximum throughput
    pub fn high_performance() -> Self {
        Self {
            realtime: JobPoolConfig {
                dispatcher_workers: 8,
                runtime_threads: 128,
                max_concurrent_handlers: 100,
            },
            background: JobPoolConfig {
                dispatcher_workers: 8,
                runtime_threads: 32,
                max_concurrent_handlers: 50,
            },
            system: JobPoolConfig {
                dispatcher_workers: 4,
                runtime_threads: 16,
                max_concurrent_handlers: 20,
            },
        }
    }
}

impl Default for JobPoolsConfig {
    fn default() -> Self {
        Self::development()
    }
}

/// Per-tenant resource limits
#[derive(Debug, Clone, Default)]
pub struct TenantLimits {
    /// Maximum storage size in bytes for this tenant
    pub max_storage_bytes: Option<u64>,
    /// Maximum number of nodes for this tenant
    pub max_nodes: Option<u64>,
    /// Maximum operations per second
    pub max_ops_per_second: Option<u32>,
    /// Maximum memory usage in bytes
    pub max_memory_bytes: Option<u64>,
}

impl Default for RocksDBConfig {
    fn default() -> Self {
        Self::development()
    }
}

impl RocksDBConfig {
    /// Set a custom path for the database
    pub fn with_path<P: Into<PathBuf>>(mut self, path: P) -> Self {
        self.path = path.into();
        self
    }

    /// Set the number of worker threads in the job worker pool
    pub fn with_worker_pool_size(mut self, size: usize) -> Self {
        self.worker_pool_size = size;
        self
    }

    /// Set the cluster node ID for replication
    pub fn with_cluster_node_id(mut self, cluster_node_id: impl Into<String>) -> Self {
        self.cluster_node_id = Some(cluster_node_id.into());
        self
    }

    /// Enable replication
    pub fn with_replication_enabled(mut self, enabled: bool) -> Self {
        self.replication_enabled = enabled;
        self
    }

    /// Add a replication peer
    pub fn with_peer(mut self, peer: ReplicationPeerConfig) -> Self {
        self.replication_peers.push(peer);
        self
    }

    /// Set all replication peers
    pub fn with_peers(mut self, peers: Vec<ReplicationPeerConfig>) -> Self {
        self.replication_peers = peers;
        self
    }

    /// Set tenant-specific resource limits
    pub fn set_tenant_limit(&mut self, tenant: String, limits: TenantLimits) {
        self.tenant_resource_limits.insert(tenant, limits);
    }

    /// Get tenant-specific resource limits
    pub fn get_tenant_limit(&self, tenant: &str) -> Option<&TenantLimits> {
        self.tenant_resource_limits.get(tenant)
    }
}

impl CompressionType {
    /// Convert to RocksDB compression type
    pub fn to_rocksdb(&self) -> rocksdb::DBCompressionType {
        match self {
            CompressionType::None => rocksdb::DBCompressionType::None,
            CompressionType::Snappy => rocksdb::DBCompressionType::Snappy,
            CompressionType::Zlib => rocksdb::DBCompressionType::Zlib,
            CompressionType::Bz2 => rocksdb::DBCompressionType::Bz2,
            CompressionType::Lz4 => rocksdb::DBCompressionType::Lz4,
            CompressionType::Lz4hc => rocksdb::DBCompressionType::Lz4hc,
            CompressionType::Zstd => rocksdb::DBCompressionType::Zstd,
        }
    }
}
