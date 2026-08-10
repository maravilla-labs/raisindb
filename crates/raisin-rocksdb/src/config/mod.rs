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
    ///
    /// ONE `Cache` of this size is shared by every column family, and it also
    /// holds the index and filter blocks (see `to_rocksdb_options`). It is
    /// therefore the real ceiling on RocksDB read-side memory, not a per-CF
    /// figure.
    pub block_cache_size: usize,
    /// Per-column-family write buffer (memtable) size in bytes.
    ///
    /// Multiply by `max_write_buffer_number` AND by the ~49 column families
    /// before reading this as a memory budget — that product is why
    /// `db_write_buffer_size` exists.
    pub write_buffer_size: usize,
    /// Maximum number of write buffers per column family
    pub max_write_buffer_number: i32,
    /// Global memtable budget across ALL column families, in bytes.
    ///
    /// `write_buffer_size * max_write_buffer_number` is a PER-CF bound, and
    /// this database has ~49 column families, so the per-CF settings alone
    /// permit multi-GB of unflushed memtables. This is the only setting that
    /// caps the total: once the sum crosses it RocksDB force-flushes the
    /// largest memtable. `0` disables the cap (RocksDB's default, and how this
    /// ran before 2026-08).
    pub db_write_buffer_size: usize,
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

    /// Trigger circuit breaker configuration — guards against a single
    /// tenant's runaway trigger/function loop growing the job registry
    /// without bound. See `jobs::TriggerBreaker`.
    pub trigger_safety: crate::jobs::TriggerSafetyConfig,
    /// Physical backstop behind `trigger_safety`: maximum non-terminal
    /// (Scheduled/Running/Executing) jobs one tenant may have registered at
    /// once, across ALL job types (not just triggers). `None` disables the
    /// check. This is the direct answer to "the dispatch queue's capacity
    /// doesn't actually bound anything" — registration into the job
    /// registry, not dispatch, is what grows without bound, so the cap has
    /// to live there. See `raisin_storage::jobs::JobRegistry::with_tenant_job_cap`.
    pub max_active_jobs_per_tenant: Option<usize>,

    /// Spatial index compaction filter — prunes superseded spatial index
    /// entries from `cf::SPATIAL_INDEX`, which otherwise only ever grows
    /// because the revision is part of the key. See
    /// [`crate::spatial::compaction`] for the retention trade-off, and set
    /// `enabled = false` (or `RAISIN_SPATIAL_COMPACTION_FILTER=off`) to turn it
    /// off. Applied at CF-open time, so a change needs a restart.
    pub spatial_compaction: crate::spatial::SpatialCompactionConfig,

    /// Hard ceiling on spatial index entries visited inside ONE geohash cell
    /// prefix before the index gives up on answering the query.
    ///
    /// Exceeding it is NOT a query failure: the repository returns
    /// [`raisin_error::Error::SpatialBudgetExceeded`] and the SQL executor
    /// degrades to a row scan, which is slow and exact. Configurable mainly so a
    /// test can reach the degradation path without writing a quarter of a
    /// million entries.
    pub spatial_max_entries_per_cell: usize,
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

    /// Override the trigger circuit breaker configuration
    pub fn with_trigger_safety(mut self, trigger_safety: crate::jobs::TriggerSafetyConfig) -> Self {
        self.trigger_safety = trigger_safety;
        self
    }

    /// Override the per-tenant active-job registry cap
    pub fn with_max_active_jobs_per_tenant(mut self, max: Option<usize>) -> Self {
        self.max_active_jobs_per_tenant = max;
        self
    }

    /// Override the per-cell spatial scan budget (see
    /// [`Self::spatial_max_entries_per_cell`]).
    pub fn with_spatial_max_entries_per_cell(mut self, max_entries: usize) -> Self {
        self.spatial_max_entries_per_cell = max_entries;
        self
    }

    /// Override the spatial index compaction filter configuration.
    pub fn with_spatial_compaction(
        mut self,
        spatial_compaction: crate::spatial::SpatialCompactionConfig,
    ) -> Self {
        self.spatial_compaction = spatial_compaction;
        self
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
