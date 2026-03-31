# Vector Embeddings Implementation Plan (pgvector-style)

## Overview
Implement vector embeddings with **pgvector-style `<->` operator** and **KNN table-valued function** in RaisinSQL, using existing background worker infrastructure, RocksDB storage, and **Moka LRU cache** for multi-tenant memory management.

---

## 🔥 Phase 0: Production Infrastructure (CRITICAL)

**Duration:** 2-3 days

This phase prepares RaisinDB for production/enterprise deployment by:
1. Fixing multi-tenant cache issues (memory safety)
2. Reactivating management interface (operations tooling)

---

### 0.1 Fix Multi-Tenant Cache Issues

**Duration:** 1 day

**Problem:** Both Tantivy (fulltext) and HNSW (vectors) use unbounded in-memory caches
- 500 tenants × 5 repos × 3 branches = **7,500 indexes**
- Tantivy: 10-50MB each = **75-375GB RAM** 🔥
- HNSW: 60MB each = **450GB RAM** 🔥
- Total: **525-825GB RAM needed** for 4GB machine!

**Solution:** Replace unbounded HashMap with **Moka LRU cache** (size-based eviction)

**Tasks:**

**1. Add Dependencies**
```toml
# crates/raisin-indexer/Cargo.toml
[dependencies]
moka = { version = "0.12", features = ["sync"] }
```

**2. Create Shared IndexCacheConfig**
```rust
// crates/raisin-indexer/src/config.rs
pub struct IndexCacheConfig {
    /// Fulltext (Tantivy) cache size in bytes
    pub fulltext_cache_size: usize,

    /// Vector (HNSW) cache size in bytes (for Phase 3)
    pub hnsw_cache_size: usize,
}

impl IndexCacheConfig {
    pub fn development() -> Self {
        Self {
            fulltext_cache_size: 256 * 1024 * 1024,  // 256MB
            hnsw_cache_size: 512 * 1024 * 1024,      // 512MB
        }
    }

    pub fn production() -> Self {
        Self {
            fulltext_cache_size: 1 * 1024 * 1024 * 1024,  // 1GB
            hnsw_cache_size: 2 * 1024 * 1024 * 1024,      // 2GB
        }
    }
}
```

**3. Update TantivyIndexingEngine with Moka**
```rust
// crates/raisin-indexer/src/tantivy_engine.rs
use moka::sync::Cache;

pub struct TantivyIndexingEngine {
    base_path: PathBuf,

    // OLD: indexes: Arc<RwLock<HashMap<String, Arc<CachedIndex>>>>,
    // NEW: LRU cache with size limits
    index_cache: Cache<String, Arc<CachedIndex>>,
}

impl TantivyIndexingEngine {
    pub fn new(base_path: PathBuf, cache_size: usize) -> Result<Self> {
        let index_cache = Cache::builder()
            .weigher(|_key: &String, index: &Arc<CachedIndex>| -> u32 {
                // Estimate: ~10-50MB per Tantivy index
                // Use fixed estimate or query index stats
                (30 * 1024 * 1024) // 30MB estimate
            })
            .max_capacity(cache_size as u64)
            .eviction_listener(|key, _value, cause| {
                tracing::info!(
                    "Evicted Tantivy index: {} (cause: {:?})",
                    key, cause
                );
            })
            .build();

        Ok(Self { base_path, index_cache })
    }

    fn get_or_create_index(&self, tenant_id: &str, repo_id: &str, branch: &str)
        -> Result<Arc<CachedIndex>>
    {
        let cache_key = format!("{}/{}/{}", tenant_id, repo_id, branch);

        // Try cache first (O(1))
        if let Some(cached) = self.index_cache.get(&cache_key) {
            return Ok(cached);
        }

        // Load from disk
        let index_path = self.base_path.join(tenant_id).join(repo_id).join(branch);
        // ... existing load logic ...

        let cached = Arc::new(CachedIndex { index, reader });

        // Insert into cache (may evict LRU)
        self.index_cache.insert(cache_key, Arc::clone(&cached));

        Ok(cached)
    }
}
```

**4. Wire Up in main.rs**
```rust
// crates/raisin-server/src/main.rs
let cache_config = IndexCacheConfig::production();

let tantivy_engine = Arc::new(
    TantivyIndexingEngine::new(
        index_path,
        cache_config.fulltext_cache_size  // NEW: pass cache size
    ).expect("Failed to create indexing engine")
);
```

**5. Testing**
- [ ] Test with 100+ tenant/repo/branch combinations
- [ ] Verify memory stays bounded
- [ ] Check eviction logs
- [ ] Ensure hot tenants stay in cache

**Memory Budget After Phase 0.1:**
```
Total RAM: 4GB
├─ OS + System: 1GB
├─ RocksDB block cache: 512MB
├─ Tantivy fulltext cache: 1GB     ← BOUNDED with LRU
├─ HNSW vector cache: 2GB          ← Will add in Phase 3
└─ Application: 512MB
```

---

### 0.2 Reactivate Management Interface for Production Operations

**Duration:** 1-2 days

**Problem:** Management interface exists but is based on old API and incomplete
- Missing Tantivy (fulltext) index operations
- Missing HNSW (vector) index operations (prepare for Phase 3)
- No clear separation between global vs tenant/database level operations
- SSE infrastructure needs extension for new job types

**Solution:** Refactor and extend management API for production/enterprise readiness

**Architecture:**

```
/api/management/
├── global/                    ← Global operations (all tenants)
│   ├── health                → System-wide health check
│   ├── metrics               → Aggregate metrics
│   ├── compact               → Global RocksDB compaction
│   ├── backup                → Backup all tenants
│   └── jobs                  → List all background jobs
│
├── tenant/{tenant}/           ← Tenant-level operations
│   ├── health                → Tenant health check
│   ├── metrics               → Tenant metrics
│   ├── integrity/check       → Check data integrity
│   ├── integrity/repair      → Auto-repair issues
│   ├── cleanup/orphans       → Remove orphaned nodes
│   ├── compact               → Compact tenant data
│   └── backup                → Backup tenant
│
├── database/{tenant}/{repo}/  ← Database (repo) level operations
│   ├── indexes/rocksdb/
│   │   ├── verify            → Verify RocksDB indexes (property, child_order)
│   │   ├── rebuild           → Rebuild RocksDB indexes
│   │   └── health            → Check index health
│   │
│   ├── indexes/fulltext/      ← Tantivy fulltext indexes
│   │   ├── verify            → Check Tantivy index consistency
│   │   ├── rebuild           → Rebuild from nodes
│   │   ├── optimize          → Merge segments, optimize storage
│   │   ├── health            → Index health & stats
│   │   └── purge             → Delete and recreate index
│   │
│   └── indexes/vector/        ← HNSW vector indexes (for Phase 3)
│       ├── verify            → Check HNSW index consistency
│       ├── rebuild           → Rebuild from embeddings CF
│       ├── optimize          → Rebalance HNSW graph
│       ├── health            → Index stats (node count, memory usage)
│       └── restore           → Restore from RocksDB embeddings
│
└── events/                    ← SSE streams
    ├── jobs                  → Real-time job updates
    ├── health                → Health monitoring
    └── metrics               → Metrics streaming
```

**Backend Tasks:**

**1. Extend Index Management Traits**

```rust
// crates/raisin-storage/src/management.rs
pub trait IndexManagement: Send + Sync {
    // RocksDB index operations
    async fn verify_rocksdb_indexes(&self, tenant: &str, repo: &str) -> Result<IndexReport>;
    async fn rebuild_rocksdb_indexes(&self, tenant: &str, repo: &str) -> Result<RebuildStats>;

    // Fulltext (Tantivy) operations
    async fn verify_fulltext_index(&self, tenant: &str, repo: &str, branch: &str) -> Result<IndexReport>;
    async fn rebuild_fulltext_index(&self, tenant: &str, repo: &str, branch: &str) -> Result<RebuildStats>;
    async fn optimize_fulltext_index(&self, tenant: &str, repo: &str, branch: &str) -> Result<OptimizeStats>;
    async fn purge_fulltext_index(&self, tenant: &str, repo: &str, branch: &str) -> Result<()>;
    async fn fulltext_index_health(&self, tenant: &str, repo: &str, branch: &str) -> Result<IndexHealth>;

    // Vector (HNSW) operations (prepare for Phase 3)
    async fn verify_vector_index(&self, tenant: &str, repo: &str, branch: &str) -> Result<IndexReport>;
    async fn rebuild_vector_index(&self, tenant: &str, repo: &str, branch: &str) -> Result<RebuildStats>;
    async fn optimize_vector_index(&self, tenant: &str, repo: &str, branch: &str) -> Result<OptimizeStats>;
    async fn restore_vector_index(&self, tenant: &str, repo: &str, branch: &str) -> Result<RestoreStats>;
    async fn vector_index_health(&self, tenant: &str, repo: &str, branch: &str) -> Result<IndexHealth>;
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IndexReport {
    pub index_type: IndexType,
    pub status: IndexStatus,
    pub issues: Vec<IndexIssue>,
    pub health_score: f32,
    pub total_entries: u64,
    pub corrupted_entries: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IndexHealth {
    pub index_type: IndexType,
    pub memory_usage_bytes: u64,
    pub disk_usage_bytes: u64,
    pub entry_count: u64,
    pub cache_hit_rate: Option<f32>,
    pub last_optimized: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OptimizeStats {
    pub bytes_before: u64,
    pub bytes_after: u64,
    pub duration_ms: u64,
    pub segments_merged: u32, // For Tantivy
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RestoreStats {
    pub entries_restored: u64,
    pub entries_skipped: u64,
    pub duration_ms: u64,
}
```

**2. Implement Tantivy Index Operations**

```rust
// crates/raisin-indexer/src/management.rs (new file)
use tantivy::Index;

pub struct TantivyManagement {
    base_path: PathBuf,
    engine: Arc<TantivyIndexingEngine>,
}

impl TantivyManagement {
    pub async fn verify_index(&self, tenant: &str, repo: &str, branch: &str) -> Result<IndexReport> {
        let index_path = self.base_path.join(tenant).join(repo).join(branch);

        if !index_path.exists() {
            return Ok(IndexReport {
                index_type: IndexType::FullText,
                status: IndexStatus::Missing,
                issues: vec![],
                health_score: 0.0,
                total_entries: 0,
                corrupted_entries: 0,
            });
        }

        // Load index and check integrity
        let index = Index::open_in_dir(&index_path)?;
        let reader = index.reader()?;
        let searcher = reader.searcher();

        // Count documents
        let total_docs = searcher.num_docs() as u64;

        // Check for corruption (attempt to read all segments)
        let mut corrupted = 0u64;
        for segment_reader in searcher.segment_readers() {
            if let Err(_) = segment_reader.alive_bitset() {
                corrupted += segment_reader.num_docs() as u64;
            }
        }

        let health_score = if total_docs > 0 {
            1.0 - (corrupted as f32 / total_docs as f32)
        } else {
            1.0
        };

        Ok(IndexReport {
            index_type: IndexType::FullText,
            status: if health_score >= 0.99 { IndexStatus::Healthy } else { IndexStatus::Degraded },
            issues: vec![],
            health_score,
            total_entries: total_docs,
            corrupted_entries: corrupted,
        })
    }

    pub async fn rebuild_index(&self, tenant: &str, repo: &str, branch: &str) -> Result<RebuildStats> {
        // Delete existing index
        let index_path = self.base_path.join(tenant).join(repo).join(branch);
        if index_path.exists() {
            std::fs::remove_dir_all(&index_path)?;
        }

        // Re-index all nodes (trigger fulltext indexing jobs)
        // This will be done via the existing indexer worker
        let start = std::time::Instant::now();

        // Enqueue jobs for all nodes in this branch
        // ... implementation ...

        Ok(RebuildStats {
            index_type: IndexType::FullText,
            items_processed: 0, // Updated by worker
            errors: 0,
            duration_ms: start.elapsed().as_millis() as u64,
            success: true,
        })
    }

    pub async fn optimize_index(&self, tenant: &str, repo: &str, branch: &str) -> Result<OptimizeStats> {
        let index_path = self.base_path.join(tenant).join(repo).join(branch);
        let index = Index::open_in_dir(&index_path)?;

        // Get size before optimization
        let bytes_before = Self::get_index_size(&index_path)?;

        // Merge segments
        let start = std::time::Instant::now();
        let mut writer = index.writer(128_000_000)?; // 128MB
        let segment_ids_before = index.searchable_segment_ids()?;

        writer.merge(&segment_ids_before).wait()?;
        writer.commit()?;
        writer.wait_merging_threads()?;

        // Get size after optimization
        let bytes_after = Self::get_index_size(&index_path)?;
        let segment_ids_after = index.searchable_segment_ids()?;

        Ok(OptimizeStats {
            bytes_before,
            bytes_after,
            duration_ms: start.elapsed().as_millis() as u64,
            segments_merged: (segment_ids_before.len() - segment_ids_after.len()) as u32,
        })
    }

    pub async fn get_health(&self, tenant: &str, repo: &str, branch: &str) -> Result<IndexHealth> {
        let index_path = self.base_path.join(tenant).join(repo).join(branch);

        if !index_path.exists() {
            return Ok(IndexHealth {
                index_type: IndexType::FullText,
                memory_usage_bytes: 0,
                disk_usage_bytes: 0,
                entry_count: 0,
                cache_hit_rate: None,
                last_optimized: None,
            });
        }

        let index = Index::open_in_dir(&index_path)?;
        let reader = index.reader()?;
        let searcher = reader.searcher();

        Ok(IndexHealth {
            index_type: IndexType::FullText,
            memory_usage_bytes: Self::estimate_memory_usage(&index)?,
            disk_usage_bytes: Self::get_index_size(&index_path)?,
            entry_count: searcher.num_docs() as u64,
            cache_hit_rate: None, // Could track if needed
            last_optimized: Self::get_last_modified(&index_path).ok(),
        })
    }
}
```

**3. Prepare HNSW Management (Stubbed for Phase 3)**

```rust
// crates/raisin-embeddings/src/management.rs (create during Phase 3)
pub struct HnswManagement {
    base_path: PathBuf,
    engine: Arc<HnswIndexingEngine>,
}

impl HnswManagement {
    pub async fn verify_index(&self, tenant: &str, repo: &str, branch: &str) -> Result<IndexReport> {
        // Check HNSW index file exists and is valid
        // Verify node count matches embeddings CF
        // Check graph connectivity
        unimplemented!("Phase 3")
    }

    pub async fn rebuild_index(&self, tenant: &str, repo: &str, branch: &str) -> Result<RebuildStats> {
        // Rebuild HNSW from RocksDB embeddings CF
        // This recovers from corrupted HNSW index files
        unimplemented!("Phase 3")
    }

    pub async fn restore_from_embeddings(&self, tenant: &str, repo: &str, branch: &str) -> Result<RestoreStats> {
        // Rebuild HNSW index from embeddings CF
        // Useful after data recovery or migration
        unimplemented!("Phase 3")
    }

    pub async fn optimize_index(&self, tenant: &str, repo: &str, branch: &str) -> Result<OptimizeStats> {
        // Rebalance HNSW graph for better search performance
        // This is expensive but improves query speed
        unimplemented!("Phase 3")
    }

    pub async fn get_health(&self, tenant: &str, repo: &str, branch: &str) -> Result<IndexHealth> {
        // Report HNSW stats: memory usage, node count, average connectivity
        unimplemented!("Phase 3")
    }
}
```

**4. Update API Routes**

```rust
// crates/raisin-server/src/management.rs
pub fn management_router(storage: Arc<RocksDBStorage>) -> Router {
    Router::new()
        // Global operations
        .route("/api/management/global/health", get(get_global_health))
        .route("/api/management/global/metrics", get(get_global_metrics))
        .route("/api/management/global/compact", post(start_global_compaction))
        .route("/api/management/global/backup", post(start_global_backup))
        .route("/api/management/global/jobs", get(list_all_jobs))

        // Tenant operations
        .route("/api/management/tenant/:tenant/health", get(get_tenant_health))
        .route("/api/management/tenant/:tenant/metrics", get(get_tenant_metrics))
        .route("/api/management/tenant/:tenant/integrity/check", post(start_integrity_check))
        .route("/api/management/tenant/:tenant/integrity/repair", post(start_repair))
        .route("/api/management/tenant/:tenant/cleanup/orphans", post(start_cleanup))
        .route("/api/management/tenant/:tenant/compact", post(start_tenant_compaction))
        .route("/api/management/tenant/:tenant/backup", post(start_tenant_backup))

        // Database (repo) level - RocksDB indexes
        .route("/api/management/database/:tenant/:repo/indexes/rocksdb/verify", post(verify_rocksdb_indexes))
        .route("/api/management/database/:tenant/:repo/indexes/rocksdb/rebuild", post(rebuild_rocksdb_indexes))
        .route("/api/management/database/:tenant/:repo/indexes/rocksdb/health", get(rocksdb_indexes_health))

        // Database level - Fulltext (Tantivy) indexes
        .route("/api/management/database/:tenant/:repo/:branch/indexes/fulltext/verify", post(verify_fulltext_index))
        .route("/api/management/database/:tenant/:repo/:branch/indexes/fulltext/rebuild", post(rebuild_fulltext_index))
        .route("/api/management/database/:tenant/:repo/:branch/indexes/fulltext/optimize", post(optimize_fulltext_index))
        .route("/api/management/database/:tenant/:repo/:branch/indexes/fulltext/purge", post(purge_fulltext_index))
        .route("/api/management/database/:tenant/:repo/:branch/indexes/fulltext/health", get(fulltext_index_health))

        // Database level - Vector (HNSW) indexes (Phase 3)
        .route("/api/management/database/:tenant/:repo/:branch/indexes/vector/verify", post(verify_vector_index))
        .route("/api/management/database/:tenant/:repo/:branch/indexes/vector/rebuild", post(rebuild_vector_index))
        .route("/api/management/database/:tenant/:repo/:branch/indexes/vector/optimize", post(optimize_vector_index))
        .route("/api/management/database/:tenant/:repo/:branch/indexes/vector/restore", post(restore_vector_index))
        .route("/api/management/database/:tenant/:repo/:branch/indexes/vector/health", get(vector_index_health))

        // Job management
        .route("/api/management/jobs", get(list_jobs))
        .route("/api/management/jobs/:id", get(get_job_info))
        .route("/api/management/jobs/:id/cancel", post(cancel_job))

        // SSE streams
        .route("/api/management/events/jobs", get(job_events_stream))
        .route("/api/management/events/health", get(health_events_stream))
        .route("/api/management/events/metrics", get(metrics_events_stream))

        .with_state(state)
}
```

**5. Update SSE for New Job Types**

```rust
// crates/raisin-server/src/sse.rs
#[derive(Debug, Serialize, Clone)]
#[serde(tag = "type")]
pub enum JobType {
    // Existing
    IntegrityCheck,
    IndexRebuild,
    Compaction,
    Backup,
    Cleanup,
    Repair,

    // New for Tantivy
    FulltextVerify,
    FulltextRebuild,
    FulltextOptimize,
    FulltextPurge,

    // New for HNSW (Phase 3)
    VectorVerify,
    VectorRebuild,
    VectorOptimize,
    VectorRestore,
}
```

**Frontend Tasks:**

**6. Update Admin Console Management UI**

```tsx
// packages/admin-console/src/pages/management/DatabaseManagement.tsx (NEW)
import { Database, Search, Sparkles } from 'lucide-react'
import { useState } from 'react'

export default function DatabaseManagement() {
  const [tenant, setTenant] = useState('default')
  const [repo, setRepo] = useState('')
  const [branch, setBranch] = useState('main')

  return (
    <div className="space-y-8">
      <h1 className="text-4xl font-bold text-white">Database Management</h1>

      {/* RocksDB Indexes */}
      <GlassCard>
        <h2 className="text-2xl font-semibold text-white mb-4 flex items-center gap-2">
          <Database className="w-6 h-6" />
          RocksDB Indexes
        </h2>

        <div className="space-y-3">
          <ActionButton onClick={handleVerifyRocksDBIndexes}>
            Verify Property & Child Order Indexes
          </ActionButton>

          <ActionButton onClick={handleRebuildRocksDBIndexes} variant="secondary">
            Rebuild All RocksDB Indexes
          </ActionButton>
        </div>

        {rocksdbHealth && (
          <IndexHealthDisplay health={rocksdbHealth} />
        )}
      </GlassCard>

      {/* Fulltext (Tantivy) Indexes */}
      <GlassCard>
        <h2 className="text-2xl font-semibold text-white mb-4 flex items-center gap-2">
          <Search className="w-6 h-6" />
          Fulltext Search Indexes
        </h2>

        <BranchSelector value={branch} onChange={setBranch} />

        <div className="grid grid-cols-2 gap-3 mt-4">
          <ActionButton onClick={handleVerifyFulltext}>
            Verify Index
          </ActionButton>

          <ActionButton onClick={handleRebuildFulltext} variant="secondary">
            Rebuild Index
          </ActionButton>

          <ActionButton onClick={handleOptimizeFulltext} variant="secondary">
            Optimize & Merge Segments
          </ActionButton>

          <ActionButton onClick={handlePurgeFulltext} variant="danger">
            Purge & Recreate
          </ActionButton>
        </div>

        {fulltextHealth && (
          <IndexHealthDisplay health={fulltextHealth} />
        )}
      </GlassCard>

      {/* Vector (HNSW) Indexes - Phase 3 */}
      <GlassCard className="opacity-50">
        <h2 className="text-2xl font-semibold text-white mb-4 flex items-center gap-2">
          <Sparkles className="w-6 h-6" />
          Vector Search Indexes
          <span className="text-sm text-zinc-500 ml-2">(Coming in Phase 3)</span>
        </h2>

        <div className="grid grid-cols-2 gap-3">
          <ActionButton disabled>Verify HNSW Index</ActionButton>
          <ActionButton disabled>Rebuild from Embeddings</ActionButton>
          <ActionButton disabled>Optimize Graph</ActionButton>
          <ActionButton disabled>Restore from RocksDB</ActionButton>
        </div>
      </GlassCard>
    </div>
  )
}

// Helper component for displaying index health
function IndexHealthDisplay({ health }: { health: IndexHealth }) {
  return (
    <div className="mt-4 p-4 bg-white/5 rounded-lg">
      <div className="grid grid-cols-2 md:grid-cols-4 gap-4 text-sm">
        <div>
          <div className="text-zinc-400">Memory Usage</div>
          <div className="text-white font-medium">{formatBytes(health.memory_usage_bytes)}</div>
        </div>
        <div>
          <div className="text-zinc-400">Disk Usage</div>
          <div className="text-white font-medium">{formatBytes(health.disk_usage_bytes)}</div>
        </div>
        <div>
          <div className="text-zinc-400">Entry Count</div>
          <div className="text-white font-medium">{health.entry_count.toLocaleString()}</div>
        </div>
        <div>
          <div className="text-zinc-400">Last Optimized</div>
          <div className="text-white font-medium">
            {health.last_optimized ? formatDate(health.last_optimized) : 'Never'}
          </div>
        </div>
      </div>
    </div>
  )
}
```

**7. Add Database Management to Navigation**

```tsx
// packages/admin-console/src/App.tsx
<Route path="/admin/management/global" element={<GlobalManagement />} />
<Route path="/admin/management/tenant/:tenant" element={<TenantManagement />} />
<Route path="/admin/management/database/:tenant/:repo" element={<DatabaseManagement />} />
```

**Checklist:**

- [ ] Add `IndexManagement` trait to raisin-storage
- [ ] Implement `TantivyManagement` in raisin-indexer
- [ ] Create stubs for `HnswManagement` (Phase 3)
- [ ] Refactor management API routes (global/tenant/database separation)
- [ ] Extend SSE job types for new operations
- [ ] Update frontend management pages
- [ ] Add database-level management UI
- [ ] Test all operations with SSE progress tracking
- [ ] Add comprehensive logging for all operations
- [ ] Update documentation

**Integration with Existing Infrastructure:**

The management interface now provides:
✅ **Global operations** - RocksDB compaction, backup across all tenants
✅ **Tenant operations** - Integrity checks, repairs, cleanup
✅ **Database operations** - Index-specific maintenance per repo/branch
✅ **Fulltext search** - Tantivy index management
✅ **Vector search** - HNSW index management (prepared for Phase 3)
✅ **Real-time monitoring** - SSE progress tracking for all jobs
✅ **Production-ready** - Comprehensive tooling for enterprise deployment

---

## Phase 1: Tenant-Level Configuration Foundation (1 week)

### 1.1 Tenant-Level Embedding Configuration Storage

**Key Decision:** API keys stored at **TENANT level** (not repo level)
- One API key per tenant
- Shared across all repositories under that tenant
- Stored encrypted in RocksDB

**New Column Family:**
```rust
// Add to crates/raisin-rocksdb/src/lib.rs
pub mod cf {
    // ... existing CFs ...
    pub const TENANT_EMBEDDING_CONFIG: &str = "tenant_embedding_config";
}
```

**Configuration Model:**
```rust
// crates/raisin-embeddings/src/config.rs
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantEmbeddingConfig {
    pub tenant_id: String,
    pub enabled: bool,

    // Provider settings
    pub provider: EmbeddingProvider,  // OpenAI | Claude | Ollama
    pub model: String,                // e.g., "text-embedding-3-small"
    pub dimensions: usize,            // 1536 for OpenAI

    // Encrypted API key (using ring crate)
    pub api_key_encrypted: Option<Vec<u8>>,

    // Content generation defaults (can be overridden per repo)
    pub include_name: bool,
    pub include_path: bool,

    // Per-node-type settings (global defaults)
    pub node_type_settings: HashMap<String, NodeTypeEmbeddingConfig>,

    // Limits
    pub max_embeddings_per_repo: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EmbeddingProvider {
    OpenAI,
    Claude,    // Uses Voyage embeddings via Anthropic
    Ollama,    // Coming Soon
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeTypeEmbeddingConfig {
    pub enabled: bool,
    pub properties_to_embed: Vec<String>,  // Only indexed/translatable props
}
```

**Storage API:**
```rust
// crates/raisin-rocksdb/src/repositories/tenant_embedding_config.rs
pub trait TenantEmbeddingConfigStore: Send + Sync {
    fn get_config(&self, tenant_id: &str) -> Result<Option<TenantEmbeddingConfig>>;
    fn set_config(&self, config: &TenantEmbeddingConfig) -> Result<()>;
    fn delete_config(&self, tenant_id: &str) -> Result<()>;
}

impl TenantEmbeddingConfigStore for RocksDBStorage {
    fn get_config(&self, tenant_id: &str) -> Result<Option<TenantEmbeddingConfig>> {
        let cf = cf_handle(&self.db, cf::TENANT_EMBEDDING_CONFIG)?;
        let key = tenant_id.as_bytes();

        match self.db.get_cf(cf, key)? {
            Some(bytes) => {
                let config = rmp_serde::from_slice(&bytes)?;
                Ok(Some(config))
            }
            None => Ok(None),
        }
    }

    fn set_config(&self, config: &TenantEmbeddingConfig) -> Result<()> {
        let cf = cf_handle(&self.db, cf::TENANT_EMBEDDING_CONFIG)?;
        let key = config.tenant_id.as_bytes();
        let value = rmp_serde::to_vec(config)?;

        self.db.put_cf(cf, key, value)?;
        Ok(())
    }
}
```

**API Key Encryption:**
```rust
// crates/raisin-embeddings/src/crypto.rs
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
use ring::rand::{SecureRandom, SystemRandom};

pub struct ApiKeyEncryptor {
    key: LessSafeKey,
    rng: SystemRandom,
}

impl ApiKeyEncryptor {
    pub fn new(master_key: &[u8; 32]) -> Result<Self> {
        let unbound = UnboundKey::new(&AES_256_GCM, master_key)?;
        let key = LessSafeKey::new(unbound);

        Ok(Self {
            key,
            rng: SystemRandom::new(),
        })
    }

    pub fn encrypt(&self, plaintext: &str) -> Result<Vec<u8>> {
        let mut nonce_bytes = [0u8; 12];
        self.rng.fill(&mut nonce_bytes)?;

        let nonce = Nonce::assume_unique_for_key(nonce_bytes);
        let mut in_out = plaintext.as_bytes().to_vec();

        self.key.seal_in_place_append_tag(nonce, Aad::empty(), &mut in_out)?;

        // Prepend nonce to ciphertext
        let mut result = nonce_bytes.to_vec();
        result.extend_from_slice(&in_out);
        Ok(result)
    }

    pub fn decrypt(&self, encrypted: &[u8]) -> Result<String> {
        let (nonce_bytes, ciphertext) = encrypted.split_at(12);
        let nonce = Nonce::assume_unique_for_key(*nonce_bytes);

        let mut in_out = ciphertext.to_vec();
        let plaintext = self.key.open_in_place(nonce, Aad::empty(), &mut in_out)?;

        Ok(String::from_utf8(plaintext.to_vec())?)
    }
}
```

### 1.2 HTTP API Endpoints

**New routes:**
```rust
// crates/raisin-transport-http/src/handlers/embeddings.rs
use axum::{extract::State, Json};

// GET /api/tenants/{tenant_id}/embeddings/config
pub async fn get_tenant_embedding_config(
    Path(tenant_id): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<TenantEmbeddingConfig>, ApiError> {
    let config = state.storage()
        .get_tenant_embedding_config(&tenant_id)?
        .ok_or_else(|| ApiError::not_found("Embedding config not found"))?;

    // Don't expose encrypted API key in response
    let mut safe_config = config;
    safe_config.api_key_encrypted = None;

    Ok(Json(safe_config))
}

// POST /api/tenants/{tenant_id}/embeddings/config
pub async fn set_tenant_embedding_config(
    Path(tenant_id): Path<String>,
    State(state): State<AppState>,
    Json(mut config): Json<TenantEmbeddingConfig>,
) -> Result<Json<SuccessResponse>, ApiError> {
    // Encrypt API key if provided
    if let Some(plain_key) = config.api_key_plain.take() {
        let encryptor = ApiKeyEncryptor::new(&state.master_key)?;
        config.api_key_encrypted = Some(encryptor.encrypt(&plain_key)?);
    }

    state.storage().set_tenant_embedding_config(&config)?;

    Ok(Json(SuccessResponse { success: true }))
}

// POST /api/tenants/{tenant_id}/embeddings/config/test
pub async fn test_embedding_connection(
    Path(tenant_id): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<TestResult>, ApiError> {
    let config = state.storage()
        .get_tenant_embedding_config(&tenant_id)?
        .ok_or_else(|| ApiError::not_found("Config not found"))?;

    // Decrypt API key
    let encryptor = ApiKeyEncryptor::new(&state.master_key)?;
    let api_key = config.api_key_encrypted
        .map(|enc| encryptor.decrypt(&enc))
        .transpose()?
        .ok_or_else(|| ApiError::validation("No API key configured"))?;

    // Test embedding generation
    let provider = create_provider(&config.provider, &api_key, &config.model)?;
    let embedding = provider.generate_embedding("test").await?;

    Ok(Json(TestResult {
        success: true,
        dimensions: embedding.len(),
        model: config.model,
    }))
}
```

### 1.3 Admin Console - Tenant Settings Page

**New Page:** `/admin/tenants/{tenant}/settings/embeddings`

**UI Components:**

```tsx
// packages/admin-console/src/pages/TenantEmbeddingSettings.tsx
import { useState, useEffect } from 'react'
import { Save, Key, CheckCircle, AlertCircle } from 'lucide-react'
import GlassCard from '../components/GlassCard'

interface TenantEmbeddingConfig {
  enabled: boolean
  provider: 'OpenAI' | 'Claude' | 'Ollama'
  model: string
  dimensions: number
  include_name: boolean
  include_path: boolean
}

export default function TenantEmbeddingSettings() {
  const [config, setConfig] = useState<TenantEmbeddingConfig | null>(null)
  const [apiKey, setApiKey] = useState('')
  const [showApiKey, setShowApiKey] = useState(false)
  const [testing, setTesting] = useState(false)
  const [testResult, setTestResult] = useState<any>(null)

  const PROVIDERS = [
    { id: 'OpenAI', name: 'OpenAI', available: true },
    { id: 'Claude', name: 'Claude (Voyage)', available: true },
    { id: 'Ollama', name: 'Ollama (Local)', available: false }, // Coming Soon
  ]

  const OPENAI_MODELS = [
    { id: 'text-embedding-3-small', name: 'text-embedding-3-small (1536 dims)', dims: 1536 },
    { id: 'text-embedding-3-large', name: 'text-embedding-3-large (3072 dims)', dims: 3072 },
  ]

  async function handleTestConnection() {
    setTesting(true)
    try {
      const result = await fetch(`/api/tenants/${tenantId}/embeddings/config/test`, {
        method: 'POST',
      }).then(r => r.json())

      setTestResult(result)
    } catch (error) {
      setTestResult({ success: false, error: error.message })
    } finally {
      setTesting(false)
    }
  }

  async function handleSave() {
    // Save config with API key
    await fetch(`/api/tenants/${tenantId}/embeddings/config`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        ...config,
        api_key_plain: apiKey, // Will be encrypted server-side
      }),
    })
  }

  return (
    <div className="max-w-4xl">
      <h1 className="text-4xl font-bold text-white mb-8">
        Embedding Configuration
      </h1>

      {/* Provider Selection */}
      <GlassCard className="mb-6">
        <h2 className="text-2xl font-semibold text-white mb-4">Provider</h2>

        <div className="grid grid-cols-3 gap-4">
          {PROVIDERS.map(provider => (
            <button
              key={provider.id}
              disabled={!provider.available}
              onClick={() => setConfig({ ...config, provider: provider.id })}
              className={`p-4 rounded-lg border-2 transition-all ${
                config?.provider === provider.id
                  ? 'border-primary-500 bg-primary-500/20'
                  : 'border-zinc-700 bg-zinc-800/30 hover:border-zinc-600'
              } ${!provider.available ? 'opacity-50 cursor-not-allowed' : ''}`}
            >
              <div className="text-white font-semibold">{provider.name}</div>
              {!provider.available && (
                <div className="text-xs text-zinc-500 mt-1">Coming Soon</div>
              )}
            </button>
          ))}
        </div>
      </GlassCard>

      {/* API Key Input */}
      <GlassCard className="mb-6">
        <h2 className="text-2xl font-semibold text-white mb-4">API Key</h2>

        <div className="relative">
          <input
            type={showApiKey ? 'text' : 'password'}
            value={apiKey}
            onChange={(e) => setApiKey(e.target.value)}
            className="w-full px-4 py-3 bg-zinc-800/50 border border-zinc-700 rounded-lg text-white"
            placeholder="sk-..."
          />
          <button
            onClick={() => setShowApiKey(!showApiKey)}
            className="absolute right-3 top-1/2 -translate-y-1/2 text-zinc-400"
          >
            {showApiKey ? 'Hide' : 'Show'}
          </button>
        </div>

        <button
          onClick={handleTestConnection}
          disabled={!apiKey || testing}
          className="mt-4 px-4 py-2 bg-zinc-700 hover:bg-zinc-600 text-white rounded-lg"
        >
          {testing ? 'Testing...' : 'Test Connection'}
        </button>

        {testResult && (
          <div className={`mt-4 p-3 rounded-lg flex items-center gap-2 ${
            testResult.success
              ? 'bg-green-500/10 text-green-400'
              : 'bg-red-500/10 text-red-400'
          }`}>
            {testResult.success ? <CheckCircle /> : <AlertCircle />}
            <span>
              {testResult.success
                ? `Success! Generated ${testResult.dimensions}d embedding using ${testResult.model}`
                : `Error: ${testResult.error}`
              }
            </span>
          </div>
        )}
      </GlassCard>

      {/* Content Settings */}
      <GlassCard className="mb-6">
        <h2 className="text-2xl font-semibold text-white mb-4">Content Settings</h2>

        <div className="space-y-3">
          <label className="flex items-center gap-3">
            <input
              type="checkbox"
              checked={config?.include_name}
              onChange={(e) => setConfig({ ...config, include_name: e.target.checked })}
              className="w-5 h-5"
            />
            <span className="text-white">Include node name in embeddings</span>
          </label>

          <label className="flex items-center gap-3">
            <input
              type="checkbox"
              checked={config?.include_path}
              onChange={(e) => setConfig({ ...config, include_path: e.target.checked })}
              className="w-5 h-5"
            />
            <span className="text-white">Include node path in embeddings</span>
          </label>
        </div>
      </GlassCard>

      {/* Save Button */}
      <button
        onClick={handleSave}
        className="px-6 py-3 bg-primary-500 hover:bg-primary-600 text-white rounded-lg flex items-center gap-2"
      >
        <Save className="w-5 h-5" />
        Save Configuration
      </button>
    </div>
  )
}
```

### 1.4 Repository Query UI - Vector Search Toggle

**When tenant has API key configured, show toggle in repository search UI:**

```tsx
// packages/admin-console/src/components/RepositorySearch.tsx
import { Search, Sparkles } from 'lucide-react'

interface RepositorySearchProps {
  tenantId: string
  repoId: string
}

export default function RepositorySearch({ tenantId, repoId }: RepositorySearchProps) {
  const [query, setQuery] = useState('')
  const [includeVectorSearch, setIncludeVectorSearch] = useState(false)
  const [hasEmbeddings, setHasEmbeddings] = useState(false)

  useEffect(() => {
    // Check if tenant has embedding config
    fetch(`/api/tenants/${tenantId}/embeddings/config`)
      .then(r => r.json())
      .then(config => {
        setHasEmbeddings(config.enabled && !!config.api_key_encrypted)
      })
      .catch(() => setHasEmbeddings(false))
  }, [tenantId])

  async function handleSearch() {
    const params = new URLSearchParams({
      q: query,
      include_vector: includeVectorSearch.toString(),
    })

    const results = await fetch(`/api/repository/${repoId}/search?${params}`)
      .then(r => r.json())

    // Display results...
  }

  return (
    <div className="space-y-4">
      {/* Search Input */}
      <div className="relative">
        <Search className="absolute left-3 top-1/2 -translate-y-1/2 text-zinc-400" />
        <input
          type="text"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          className="w-full pl-10 pr-4 py-3 bg-zinc-800/50 border border-zinc-700 rounded-lg text-white"
          placeholder="Search nodes..."
        />
      </div>

      {/* Vector Search Toggle (only if embeddings configured) */}
      {hasEmbeddings && (
        <label className="flex items-center gap-3 p-3 bg-zinc-800/30 rounded-lg border border-zinc-700">
          <input
            type="checkbox"
            checked={includeVectorSearch}
            onChange={(e) => setIncludeVectorSearch(e.target.checked)}
            className="w-5 h-5"
          />
          <Sparkles className="w-5 h-5 text-primary-400" />
          <span className="text-white">Include AI semantic search (vector embeddings)</span>
        </label>
      )}

      <button
        onClick={handleSearch}
        className="w-full px-4 py-3 bg-primary-500 hover:bg-primary-600 text-white rounded-lg"
      >
        Search
      </button>
    </div>
  )
}
```

**Backend handler for combined search:**

```rust
// crates/raisin-transport-http/src/handlers/search.rs
pub async fn search_repository(
    Path((repo, branch, workspace)): Path<(String, String, String)>,
    Query(params): Query<SearchParams>,
    State(state): State<AppState>,
) -> Result<Json<SearchResults>, ApiError> {
    let tenant_id = "default"; // TODO: from auth

    let mut results = Vec::new();

    // 1. Fulltext search (always enabled)
    if !params.q.is_empty() {
        let fulltext_results = state.indexing_engine
            .search(&FullTextSearchQuery {
                tenant_id: tenant_id.to_string(),
                repo_id: repo.clone(),
                branch: branch.clone(),
                workspace_id: workspace.clone(),
                language: "en".to_string(), // TODO: from config
                query: params.q.clone(),
                limit: 50,
            })?;

        results.extend(fulltext_results.into_iter().map(|r| SearchResult {
            node_id: r.node_id,
            score: r.score,
            search_type: SearchType::FullText,
        }));
    }

    // 2. Vector search (if enabled and query text provided)
    if params.include_vector {
        // Check if tenant has embedding config
        if let Some(config) = state.storage().get_tenant_embedding_config(tenant_id)? {
            if config.enabled {
                // Generate embedding for query text
                let provider = create_provider_from_config(&config)?;
                let query_embedding = provider.generate_embedding(&params.q).await?;

                // Search HNSW index
                let vector_results = state.hnsw_engine
                    .search(tenant_id, &repo, &branch, &query_embedding, 20)?;

                results.extend(vector_results.into_iter().map(|r| SearchResult {
                    node_id: r.node_id,
                    score: 1.0 - r.distance, // Convert distance to score
                    search_type: SearchType::Vector,
                }));
            }
        }
    }

    // 3. Deduplicate and merge results
    let merged = merge_search_results(results);

    Ok(Json(SearchResults { items: merged }))
}
```

---

## Phase 2: Storage & Job Infrastructure (1 week)

### 2.1 RocksDB Column Families

```rust
// Add to crates/raisin-rocksdb/src/lib.rs
pub mod cf {
    // ... existing ...
    pub const EMBEDDINGS: &str = "embeddings";
    pub const EMBEDDING_JOBS: &str = "embedding_jobs";
    pub const TENANT_EMBEDDING_CONFIG: &str = "tenant_embedding_config";
}
```

### 2.2 Job Store (Same Pattern as Fulltext)

```rust
// crates/raisin-embeddings/src/job_store.rs
pub struct EmbeddingJob {
    pub job_id: String,
    pub kind: EmbeddingJobKind,
    pub tenant_id: String,
    pub repo_id: String,
    pub branch: String,
    pub workspace_id: String,
    pub revision: u64,
    pub node_id: Option<String>,
    pub source_branch: Option<String>, // For BranchCreated
}

pub enum EmbeddingJobKind {
    AddNode,
    DeleteNode,
    BranchCreated,
}
```

---

## Phase 3: HNSW Engine with Moka Cache (1-2 weeks)

### 3.1 HNSW Engine with LRU Cache (Same as Tantivy fix)

```rust
// crates/raisin-embeddings/src/hnsw_engine.rs
use moka::sync::Cache;
use instant_distance::{Builder, HnswMap};

pub struct HnswIndexingEngine {
    base_path: PathBuf,

    // LRU cache (same pattern as Tantivy)
    index_cache: Cache<String, Arc<HnswIndex>>,
}

impl HnswIndexingEngine {
    pub fn new(base_path: PathBuf, cache_size: usize) -> Result<Self> {
        let index_cache = Cache::builder()
            .weigher(|_key: &String, index: &Arc<HnswIndex>| -> u32 {
                index.estimated_memory_bytes()
                    .try_into()
                    .unwrap_or(u32::MAX)
            })
            .max_capacity(cache_size as u64)
            .eviction_listener(|key, _value, cause| {
                tracing::info!(
                    "Evicted HNSW index: {} (cause: {:?})",
                    key, cause
                );
            })
            .build();

        Ok(Self { base_path, index_cache })
    }

    // Lazy load (same as Tantivy)
    fn get_or_load_index(&self, tenant_id: &str, repo_id: &str, branch: &str)
        -> Result<Arc<HnswIndex>>
    {
        let key = format!("{}/{}/{}", tenant_id, repo_id, branch);

        if let Some(index) = self.index_cache.get(&key) {
            return Ok(index);
        }

        // Load from disk
        let index_path = self.base_path
            .join(tenant_id)
            .join(repo_id)
            .join(branch)
            .join("hnsw.bin");

        let index = if index_path.exists() {
            Arc::new(HnswIndex::load_from_file(&index_path)?)
        } else {
            Arc::new(HnswIndex::new())
        };

        self.index_cache.insert(key, Arc::clone(&index));
        Ok(index)
    }
}
```

### 3.2 Branch Copy (Same as Tantivy)

```rust
impl EmbeddingEngine for HnswIndexingEngine {
    fn do_branch_created(&self, job: &EmbeddingJob) -> Result<()> {
        let source_branch = job.source_branch.as_ref().ok_or_else(|| {
            Error::Validation("source_branch required".to_string())
        })?;

        let source_path = self.base_path
            .join(&job.tenant_id)
            .join(&job.repo_id)
            .join(source_branch);

        let target_path = self.base_path
            .join(&job.tenant_id)
            .join(&job.repo_id)
            .join(&job.branch);

        if !source_path.exists() {
            return Err(Error::NotFound(format!(
                "Source HNSW index not found: {}", source_branch
            )));
        }

        // Copy directory (same as Tantivy)
        Self::copy_dir_recursive(&source_path, &target_path)?;

        Ok(())
    }
}
```

---

## Phase 4: Embedding Providers (1 week)

### 4.1 Provider Implementations

```rust
// crates/raisin-embeddings/src/providers/openai.rs
pub struct OpenAIProvider {
    api_key: String,
    model: String,
    client: reqwest::Client,
}

impl OpenAIProvider {
    pub fn new(api_key: String, model: String) -> Self {
        Self {
            api_key,
            model,
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl EmbeddingProvider for OpenAIProvider {
    async fn generate_embedding(&self, text: &str) -> Result<Vec<f32>> {
        let response = self.client
            .post("https://api.openai.com/v1/embeddings")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&serde_json::json!({
                "input": text,
                "model": self.model,
            }))
            .send()
            .await?;

        let data: EmbeddingResponse = response.json().await?;
        Ok(data.data[0].embedding)
    }

    fn dimensions(&self) -> usize {
        match self.model.as_str() {
            "text-embedding-3-small" => 1536,
            "text-embedding-3-large" => 3072,
            _ => 1536,
        }
    }
}
```

---

## Phase 5: Background Worker (3-4 days)

```rust
// Reuse existing IndexerWorker infrastructure
// Just add EmbeddingEventHandler

pub struct EmbeddingEventHandler {
    job_store: Arc<dyn EmbeddingJobStore>,
}

impl EventHandler for EmbeddingEventHandler {
    async fn handle(&self, event: &Event) -> Result<()> {
        match event {
            Event::NodeCreated { tenant_id, repo_id, branch, node_id, revision, .. } => {
                // Enqueue embedding job
                let job = EmbeddingJob { /* ... */ };
                self.job_store.enqueue(&job)?;
            }
            Event::BranchCreated { tenant_id, repo_id, branch, source_branch, .. } => {
                let job = EmbeddingJob {
                    kind: EmbeddingJobKind::BranchCreated,
                    source_branch: Some(source_branch.clone()),
                    // ... other fields ...
                };
                self.job_store.enqueue(&job)?;
            }
            _ => {}
        }
        Ok(())
    }
}
```

---

## Phase 6: SQL Integration (1 week)

### 6.1 Add `<->` Operator to Parser

```rust
// crates/raisin-sql/src/analyzer/typed_expr.rs
pub enum BinaryOperator {
    // ... existing ...
    VectorL2Distance,      // <->
    VectorCosineDistance,  // <=>
    VectorInnerProduct,    // <#>
}
```

### 6.2 KNN Table Function

```sql
-- Already in tests/sql/04_vector_graph.sql
SELECT n.id, n.name, knn.distance
FROM KNN(:query_vec, 10) AS knn
JOIN nodes n ON n.id = knn.node_id;
```

---

## Phase 7: Testing & Documentation (1 week)

- [ ] Multi-tenant cache eviction tests
- [ ] Tantivy + HNSW memory usage benchmarks
- [ ] Tenant API key encryption tests
- [ ] Repository search toggle UI tests
- [ ] Branch copy tests
- [ ] Documentation

---

## Key Design Decisions

1. ✅ **Moka LRU cache** for both Tantivy and HNSW (multi-tenant safe)
2. ✅ **Tenant-level API keys** (shared across all repos)
3. ✅ **Repository search toggle** (when API key exists)
4. ✅ **Same patterns** as fulltext indexing (consistency)
5. ✅ **Bounded memory** (production-ready for 500+ tenants)

## Final Memory Budget (4GB Machine)

```
Total RAM: 4GB
├─ OS + System: 1GB
├─ RocksDB block cache: 512MB
├─ Tantivy cache (LRU): 1GB      ← Phase 0
├─ HNSW cache (LRU): 2GB         ← Phase 3
└─ Application: 512MB

Total indexing: 3GB (configurable, evicts LRU)
✅ Handles 500+ tenants safely
```

---

## 🔧 Critical Implementation Refinements

### 1. HNSW Index Persistence Strategy

**Problem:** Plan shows `load_from_file` but never `save_to_file`

**Solution: Periodic Snapshots + Dirty Tracking**

```rust
// crates/raisin-embeddings/src/hnsw_engine.rs
pub struct HnswIndexingEngine {
    base_path: PathBuf,
    index_cache: Cache<String, Arc<HnswIndex>>,

    // Track which indexes have been modified since last save
    dirty_indexes: Arc<RwLock<HashSet<String>>>,
}

impl HnswIndexingEngine {
    pub fn start_snapshot_task(&self) -> JoinHandle<()> {
        let engine = Arc::clone(self);

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));

            loop {
                interval.tick().await;

                if let Err(e) = engine.snapshot_dirty_indexes() {
                    tracing::error!("Failed to snapshot HNSW indexes: {}", e);
                }
            }
        })
    }

    fn snapshot_dirty_indexes(&self) -> Result<()> {
        let dirty = self.dirty_indexes.read().unwrap().clone();

        for key in dirty {
            if let Some(index) = self.index_cache.get(&key) {
                // Parse key: {tenant}/{repo}/{branch}
                let parts: Vec<&str> = key.split('/').collect();
                let path = self.base_path
                    .join(parts[0])
                    .join(parts[1])
                    .join(parts[2])
                    .join("hnsw.bin");

                // Save to disk
                index.save_to_file(&path)?;

                // Mark as clean
                self.dirty_indexes.write().unwrap().remove(&key);

                tracing::debug!("Saved HNSW index: {}", key);
            }
        }

        Ok(())
    }

    pub fn add_node(&self, job: &EmbeddingJob, embedding: Vec<f32>) -> Result<()> {
        // ... add to index ...

        // Mark as dirty
        let key = format!("{}/{}/{}", job.tenant_id, job.repo_id, job.branch);
        self.dirty_indexes.write().unwrap().insert(key);

        Ok(())
    }

    /// Graceful shutdown: save all dirty indexes
    pub async fn shutdown(&self) -> Result<()> {
        tracing::info!("Saving all dirty HNSW indexes before shutdown...");
        self.snapshot_dirty_indexes()?;
        tracing::info!("HNSW indexes saved successfully");
        Ok(())
    }
}
```

**Lifecycle:**
1. **On modification** - Mark index as dirty
2. **Every 60 seconds** - Background task saves all dirty indexes
3. **On graceful shutdown** - Save all remaining dirty indexes
4. **On crash** - May lose up to 60 seconds of changes (acceptable)

---

### 2. Worker Job Processing Loop (Complete Implementation)

**Missing from original plan: The actual work being done**

```rust
// crates/raisin-embeddings/src/worker.rs
impl<S, E> EmbeddingWorker<S, E>
where
    S: Storage + 'static,
    E: EmbeddingEngine + 'static,
{
    async fn process_job(
        storage: &Arc<S>,
        engine: &Arc<E>,
        job: EmbeddingJob,
    ) -> Result<()> {
        let job_id = job.job_id.clone();

        tracing::debug!(
            job_id = %job_id,
            kind = ?job.kind,
            tenant_id = %job.tenant_id,
            repo_id = %job.repo_id,
            "Processing embedding job"
        );

        let result = match job.kind {
            EmbeddingJobKind::AddNode => Self::handle_add_node(storage, engine, &job).await,
            EmbeddingJobKind::DeleteNode => Self::handle_delete_node(engine, &job).await,
            EmbeddingJobKind::BranchCreated => Self::handle_branch_created(engine, &job).await,
        };

        match result {
            Ok(()) => {
                storage.embedding_job_store().complete(&[job_id])?;
                tracing::debug!(job_id = %job_id, "Completed embedding job");
            }
            Err(e) => {
                tracing::error!(job_id = %job_id, error = %e, "Embedding job failed");
                storage.embedding_job_store().fail(&job_id, &e.to_string())?;
            }
        }

        Ok(())
    }

    async fn handle_add_node(
        storage: &Arc<S>,
        engine: &Arc<E>,
        job: &EmbeddingJob,
    ) -> Result<()> {
        // 1. Get tenant embedding config
        let config = storage
            .get_tenant_embedding_config(&job.tenant_id)?
            .ok_or_else(|| Error::NotFound("No embedding config for tenant".to_string()))?;

        if !config.enabled {
            return Ok(()); // Skip if embeddings disabled
        }

        // 2. Decrypt API key
        let encryptor = ApiKeyEncryptor::new(&storage.master_key())?;
        let api_key = config.api_key_encrypted
            .ok_or_else(|| Error::Validation("No API key configured".to_string()))
            .and_then(|enc| encryptor.decrypt(&enc))?;

        // 3. Create embedding provider
        let provider = create_provider(&config.provider, &api_key, &config.model)?;

        // 4. Fetch node at exact revision
        let node_id = job.node_id.as_ref()
            .ok_or_else(|| Error::Validation("node_id required for AddNode".to_string()))?;

        let node = storage.nodes()
            .get(
                &job.tenant_id,
                &job.repo_id,
                &job.branch,
                &job.workspace_id,
                node_id,
                Some(job.revision),
            )
            .await?
            .ok_or_else(|| Error::NotFound(format!("Node {} not found", node_id)))?;

        // 5. Get resolved node type schema (for property filtering)
        let schema = storage.node_types()
            .get_resolved(&job.tenant_id, &job.repo_id, &node.node_type)
            .await?;

        // 6. Extract embeddable content based on config
        let text = extract_embeddable_content(&node, &schema, &config)?;

        if text.is_empty() {
            tracing::warn!(node_id = %node_id, "No embeddable content found, skipping");
            return Ok(());
        }

        // 7. Generate embedding via provider API
        let embedding = provider.generate_embedding(&text).await?;

        tracing::debug!(
            node_id = %node_id,
            embedding_dims = embedding.len(),
            text_length = text.len(),
            "Generated embedding"
        );

        // 8. Store in RocksDB embeddings CF (for direct access)
        let embedding_data = EmbeddingData {
            vector: embedding.clone(),
            model: config.model.clone(),
            provider: config.provider.clone(),
            generated_at: chrono::Utc::now(),
            text_hash: hash_text(&text), // For detecting changes
        };

        storage.store_embedding(
            &job.tenant_id,
            &job.repo_id,
            &job.branch,
            &job.workspace_id,
            node_id,
            job.revision,
            &embedding_data,
        )?;

        // 9. Add to HNSW index (for KNN search)
        // This is a blocking operation, spawn on blocking thread pool
        let engine_clone = Arc::clone(engine);
        let job_clone = job.clone();

        tokio::task::spawn_blocking(move || {
            engine_clone.add_node(&job_clone, embedding)
        })
        .await
        .map_err(|e| Error::storage(format!("Blocking task failed: {}", e)))??;

        tracing::debug!(node_id = %node_id, "Added to HNSW index");

        Ok(())
    }

    async fn handle_delete_node(
        engine: &Arc<E>,
        job: &EmbeddingJob,
    ) -> Result<()> {
        let node_id = job.node_id.as_ref()
            .ok_or_else(|| Error::Validation("node_id required".to_string()))?;

        // Remove from HNSW index
        let engine_clone = Arc::clone(engine);
        let job_clone = job.clone();

        tokio::task::spawn_blocking(move || {
            engine_clone.delete_node(&job_clone)
        })
        .await??;

        tracing::debug!(node_id = %node_id, "Removed from HNSW index");

        Ok(())
    }

    async fn handle_branch_created(
        engine: &Arc<E>,
        job: &EmbeddingJob,
    ) -> Result<()> {
        // Copy HNSW index directory (same as Tantivy)
        let engine_clone = Arc::clone(engine);
        let job_clone = job.clone();

        tokio::task::spawn_blocking(move || {
            engine_clone.do_branch_created(&job_clone)
        })
        .await??;

        tracing::info!(
            branch = %job.branch,
            source_branch = ?job.source_branch,
            "Copied HNSW index for new branch"
        );

        Ok(())
    }
}

/// Extract embeddable content from node based on config
fn extract_embeddable_content(
    node: &Node,
    schema: &ResolvedNodeType,
    config: &TenantEmbeddingConfig,
) -> Result<String> {
    let mut parts = Vec::new();

    // 1. Include node name
    if config.include_name {
        parts.push(node.name.clone());
    }

    // 2. Include node path
    if config.include_path {
        parts.push(node.path.clone());
    }

    // 3. Get node type specific settings
    let node_type_config = config.node_type_settings
        .get(&node.node_type)
        .filter(|c| c.enabled);

    if let Some(nt_config) = node_type_config {
        // 4. Include configured properties (only indexed/translatable)
        for prop_name in &nt_config.properties_to_embed {
            if let Some(prop_schema) = schema.properties.get(prop_name) {
                // Only include if property is translatable (indicates text content)
                if prop_schema.is_translatable == Some(true) {
                    if let Some(PropertyValue::String(s)) = node.properties.get(prop_name) {
                        parts.push(format!("{}: {}", prop_name, s));
                    }
                }
            }
        }
    }

    Ok(parts.join("\n"))
}
```

---

### 3. Vector Storage Architecture Decision

**DECISION: Store in BOTH RocksDB + HNSW**

**RocksDB `embeddings` CF:**
- **Key:** `{tenant}\0{repo}\0{branch}\0{workspace}\0{node_id}\0{revision}`
- **Value:** `EmbeddingData` (MessagePack)
- **Purpose:**
  - Direct access to specific node's embedding
  - Revision history (all revisions preserved)
  - API exposure (optional)
  - Debugging/inspection

**HNSW Index File:**
- **Path:** `.data/embeddings/{tenant}/{repo}/{branch}/hnsw.bin`
- **Contents:** Full vectors for fast ANN search
- **Purpose:**
  - Fast KNN/similarity search (O(log n))
  - Latest revision per node only

**Data Structure:**
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingData {
    /// The actual embedding vector
    pub vector: Vec<f32>,

    /// Model used to generate (e.g., "text-embedding-3-small")
    pub model: String,

    /// Provider (OpenAI, Claude, Ollama)
    pub provider: EmbeddingProvider,

    /// When generated
    pub generated_at: DateTime<Utc>,

    /// Hash of source text (to detect if re-embedding needed)
    pub text_hash: u64,
}
```

**Storage Cost Analysis:**
```
Per embedding: 1536 dims × 4 bytes = 6KB
Per node (both stores): 6KB × 2 = 12KB

Example: 10,000 nodes
- RocksDB: 10K × 6KB = 60MB
- HNSW: 10K × 6KB = 60MB
- Total: 120MB (acceptable)

With RocksDB compression: ~80-100MB total
```

**Benefits:**
- ✅ Fast KNN search (HNSW)
- ✅ Direct retrieval (RocksDB)
- ✅ Revision awareness (RocksDB)
- ✅ Optional API exposure
- ✅ Can rebuild HNSW from RocksDB if corrupted

---

### 4. Hybrid Search Result Merging Strategy

**DECISION: Use Reciprocal Rank Fusion (RRF)**

**Implementation:**
```rust
// crates/raisin-transport-http/src/handlers/search.rs

/// Merge fulltext and vector search results using Reciprocal Rank Fusion
fn merge_search_results(
    fulltext: Vec<SearchResult>,
    vector: Vec<SearchResult>,
) -> Vec<SearchResult> {
    const K: f32 = 60.0; // RRF constant (standard value)

    let mut rrf_scores: HashMap<String, RrfScore> = HashMap::new();

    // Add fulltext results
    for (rank, result) in fulltext.iter().enumerate() {
        let score = 1.0 / (K + rank as f32 + 1.0);
        let entry = rrf_scores.entry(result.node_id.clone()).or_insert(RrfScore {
            node_id: result.node_id.clone(),
            rrf_score: 0.0,
            fulltext_rank: None,
            vector_rank: None,
            fulltext_score: None,
            vector_distance: None,
        });
        entry.rrf_score += score;
        entry.fulltext_rank = Some(rank);
        entry.fulltext_score = Some(result.score);
    }

    // Add vector results
    for (rank, result) in vector.iter().enumerate() {
        let score = 1.0 / (K + rank as f32 + 1.0);
        let entry = rrf_scores.entry(result.node_id.clone()).or_insert(RrfScore {
            node_id: result.node_id.clone(),
            rrf_score: 0.0,
            fulltext_rank: None,
            vector_rank: None,
            fulltext_score: None,
            vector_distance: None,
        });
        entry.rrf_score += score;
        entry.vector_rank = Some(rank);
        entry.vector_distance = Some(result.distance);
    }

    // Sort by RRF score (descending)
    let mut results: Vec<_> = rrf_scores.into_values().collect();
    results.sort_by(|a, b| b.rrf_score.partial_cmp(&a.rrf_score).unwrap());

    // Convert to SearchResult
    results.into_iter()
        .map(|r| SearchResult {
            node_id: r.node_id,
            score: r.rrf_score,
            search_type: match (r.fulltext_rank, r.vector_rank) {
                (Some(_), Some(_)) => SearchType::Hybrid,
                (Some(_), None) => SearchType::FullText,
                (None, Some(_)) => SearchType::Vector,
                (None, None) => unreachable!(),
            },
            metadata: Some(SearchMetadata {
                fulltext_rank: r.fulltext_rank,
                vector_rank: r.vector_rank,
                fulltext_score: r.fulltext_score,
                vector_distance: r.vector_distance,
            }),
        })
        .collect()
}

#[derive(Debug, Clone)]
struct RrfScore {
    node_id: String,
    rrf_score: f32,
    fulltext_rank: Option<usize>,
    vector_rank: Option<usize>,
    fulltext_score: Option<f32>,
    vector_distance: Option<f32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchMetadata {
    pub fulltext_rank: Option<usize>,
    pub vector_rank: Option<usize>,
    pub fulltext_score: Option<f32>,
    pub vector_distance: Option<f32>,
}
```

**Why RRF?**
1. **No normalization needed** - Works with different score scales
2. **Robust** - BM25 scores vs. cosine distances don't need alignment
3. **Industry standard** - Used by Elasticsearch, Vespa, Pinecone
4. **Simple** - No hyperparameters to tune
5. **Explainable** - Ranks based on position, not raw scores

**Example:**
```
Fulltext results:
1. doc_A (BM25: 12.5)
2. doc_B (BM25: 10.2)
3. doc_C (BM25: 8.7)

Vector results:
1. doc_B (distance: 0.15)
2. doc_D (distance: 0.22)
3. doc_A (distance: 0.28)

RRF scores (K=60):
doc_B: 1/(60+1) + 1/(60+1) = 0.0328  ← Top rank in both!
doc_A: 1/(60+1) + 1/(60+3) = 0.0322
doc_D: 0 + 1/(60+2) = 0.0161
doc_C: 1/(60+3) + 0 = 0.0159

Final ranking: doc_B > doc_A > doc_D > doc_C
```

---

### 5. Additional Phase Updates

**Phase 2: Add RocksDB embedding storage**
```rust
// crates/raisin-rocksdb/src/repositories/embeddings.rs
pub trait EmbeddingStorage: Send + Sync {
    fn store_embedding(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace_id: &str,
        node_id: &str,
        revision: u64,
        data: &EmbeddingData,
    ) -> Result<()>;

    fn get_embedding(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace_id: &str,
        node_id: &str,
        revision: Option<u64>, // None = latest
    ) -> Result<Option<EmbeddingData>>;

    fn delete_embedding(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace_id: &str,
        node_id: &str,
        revision: Option<u64>, // None = all revisions
    ) -> Result<()>;
}
```

**Phase 3: Add snapshot task to server startup**
```rust
// crates/raisin-server/src/main.rs
let hnsw_engine = Arc::new(HnswIndexingEngine::new(...)?);

// Start periodic snapshot task
let snapshot_handle = hnsw_engine.start_snapshot_task();

// On graceful shutdown
tokio::select! {
    _ = shutdown_signal() => {
        tracing::info!("Shutting down gracefully...");

        // Save all dirty HNSW indexes
        hnsw_engine.shutdown().await?;

        // Abort snapshot task
        snapshot_handle.abort();
    }
}
```

---

## Updated Task Checklist

### Phase 2: Storage & Job Infrastructure
- [ ] Add `embeddings` CF to RocksDB
- [ ] Implement `EmbeddingStorage` trait
- [ ] Add `store_embedding()`, `get_embedding()`, `delete_embedding()`
- [ ] Implement `EmbeddingJobStore` (same pattern as fulltext)
- [ ] Test revision-aware embedding storage

### Phase 3: HNSW Engine
- [ ] Implement `HnswIndexingEngine` with Moka cache
- [ ] Add dirty index tracking (`HashSet<String>`)
- [ ] Implement `start_snapshot_task()` (60s interval)
- [ ] Implement `snapshot_dirty_indexes()`
- [ ] Implement `shutdown()` for graceful termination
- [ ] Add snapshot task to server startup
- [ ] Test index persistence after crash
- [ ] Test graceful shutdown saves all dirty indexes

### Phase 5: Background Worker
- [ ] Implement complete `process_job()` logic
- [ ] Implement `handle_add_node()` with all steps (1-9)
- [ ] Implement `extract_embeddable_content()`
- [ ] Add text hash to detect changes
- [ ] Store in both RocksDB + HNSW
- [ ] Test job processing end-to-end

### Phase 1: Search API
- [ ] Implement `merge_search_results()` with RRF
- [ ] Add `SearchMetadata` to results
- [ ] Support `?include_vector=true` query param
- [ ] Test hybrid search ranking
- [ ] Verify RRF produces sensible rankings
