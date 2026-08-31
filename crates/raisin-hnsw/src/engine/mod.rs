// SPDX-License-Identifier: BSL-1.1

//! HNSW indexing engine with LRU cache and persistence.
//!
//! This module provides the main engine that manages multiple HNSW indexes
//! across tenants, repositories, branches and **embedding partitions**, with
//! memory-bounded caching.
//!
//! Uses full HLC (Hybrid Logical Clock) with 16-byte encoding for revision tracking
//! to preserve both timestamp and counter components for proper distributed consistency.

mod indexing;
pub mod key;
mod lifecycle;
pub mod metrics;
mod search;

#[cfg(test)]
mod tests;

use crate::dims::{IndexSpec, IndexSpecResolver};
use crate::index::HnswIndex;
use crate::partition::PartitionId;
use crate::types::DistanceMetric;
use moka::sync::Cache;
use raisin_error::Result;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

pub use key::{index_path, meta_path, IndexKey};

/// How many mutations an index takes before the cache is asked to re-weigh it.
///
/// moka calls its weigher exactly ONCE, at insert. A freshly created index is
/// therefore pinned at ~0 bytes forever, and a loaded one is pinned at its
/// load-time size no matter how much it grows — so the 512 MB engine budget
/// bounded nothing at all and the whole eviction mechanism was inert. Moka has
/// no re-weigh API; re-`insert`ing the same `Arc` under the same key is the
/// documented way to make it run the weigher again.
///
/// Doing that on every single add would put a cache write on the per-vector
/// path, so it is amortised. 512 vectors is at most ~2 MB of drift for a
/// 1024-wide f32 index — small against a 512 MB budget, and the counter is a
/// plain integer under the lock already being taken.
const REWEIGH_EVERY_N_MUTATIONS: u32 = 512;

/// HNSW indexing engine with LRU cache.
///
/// This engine manages multiple HNSW indexes — one per
/// tenant/repo/branch/**partition** — with automatic eviction based on memory
/// usage.
///
/// # Partitions
///
/// A partition is one embedding space: `{embedder_hash}{kind}`, the same two
/// segments the `cf::EMBEDDINGS` key has always carried. Before partitioning
/// there was one index per branch, so a second embedder (image vectors, or just
/// a model change) made that index unloadable for BOTH — text search went down
/// when image search came up. See [`crate::partition`].
///
/// # Features
///
/// - **LRU Eviction**: Automatically evicts least-recently-used indexes
/// - **Dirty Tracking**: Tracks which indexes have unsaved changes
/// - **Periodic Snapshots**: Background task saves dirty indexes every 60s
/// - **Save-on-evict**: An evicted index that was dirty is written out first
/// - **Graceful Shutdown**: Ensures all dirty indexes are saved on shutdown
pub struct HnswIndexingEngine {
    /// Base directory for index files
    base_path: PathBuf,

    /// LRU cache of loaded indexes
    index_cache: Cache<IndexKey, Arc<RwLock<HnswIndex>>>,

    /// Set of dirty index keys (need to be saved)
    dirty_indexes: Arc<RwLock<HashSet<IndexKey>>>,

    /// Mutations since each index was last weighed by the cache.
    /// See [`REWEIGH_EVERY_N_MUTATIONS`].
    mutations_since_weigh: Arc<RwLock<HashMap<IndexKey, u32>>>,

    /// Width used for an index whose tenant has no embedding config.
    ///
    /// NOT "the" width. Vector width is per-partition (it is a property of the
    /// embedding model), so it is resolved per index by `spec_resolver`; this is
    /// only the answer when no configuration exists. See `crate::dims`.
    fallback_dimensions: usize,

    /// Resolves a partition's configured shape — width, metric, quantization,
    /// graph parameters. `None` when the engine was built without one (tests,
    /// and any caller that has no config store), in which case every index is
    /// created at `fallback_dimensions` with the engine's default metric.
    spec_resolver: Option<Arc<dyn IndexSpecResolver>>,

    /// Default distance metric for new indexes whose partition resolves to no
    /// configuration.
    distance_metric: DistanceMetric,

    /// Observability metrics
    metrics: Arc<metrics::VectorMetrics>,
}

impl HnswIndexingEngine {
    /// Create a new HNSW indexing engine.
    ///
    /// # Arguments
    ///
    /// * `base_path` - Directory where index files will be stored
    /// * `cache_size` - Maximum cache size in bytes
    /// * `fallback_dimensions` - Width for indexes whose tenant has no embedding config.
    ///   This is NOT a global width: attach a resolver with
    ///   [`Self::with_spec_resolver`] and each index is created at its
    ///   partition's configured shape instead.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use raisin_hnsw::{HnswIndexingEngine, FALLBACK_DIMENSIONS};
    /// use std::path::PathBuf;
    ///
    /// let engine = HnswIndexingEngine::new(
    ///     PathBuf::from("./.data/hnsw"),
    ///     2 * 1024 * 1024 * 1024,  // 2GB cache
    ///     FALLBACK_DIMENSIONS,     // only used when a tenant has no config
    /// )?
    /// .with_spec_resolver(resolver);
    /// ```
    pub fn new(base_path: PathBuf, cache_size: usize, fallback_dimensions: usize) -> Result<Self> {
        Self::with_metric(
            base_path,
            cache_size,
            fallback_dimensions,
            DistanceMetric::default(),
        )
    }

    /// Create a new HNSW indexing engine with a specific distance metric.
    ///
    /// # Arguments
    ///
    /// * `base_path` - Directory where index files will be stored
    /// * `cache_size` - Maximum cache size in bytes
    /// * `fallback_dimensions` - Width for indexes whose tenant has no embedding config
    /// * `distance_metric` - Distance metric for new indexes
    pub fn with_metric(
        base_path: PathBuf,
        cache_size: usize,
        fallback_dimensions: usize,
        distance_metric: DistanceMetric,
    ) -> Result<Self> {
        let dirty_indexes: Arc<RwLock<HashSet<IndexKey>>> = Arc::new(RwLock::new(HashSet::new()));

        // The eviction listener needs both of these, and it is installed before
        // `Self` exists, so they are built first and cloned in.
        let evict_dirty = Arc::clone(&dirty_indexes);
        let evict_base = base_path.clone();

        let index_cache = Cache::builder()
            .weigher(|_key: &IndexKey, index: &Arc<RwLock<HnswIndex>>| -> u32 {
                let index_guard = index.read().unwrap();
                let size = index_guard.estimated_memory_bytes();
                (size as u64).min(u32::MAX as u64) as u32
            })
            .max_capacity(cache_size as u64)
            .eviction_listener(
                move |key: Arc<IndexKey>, value: Arc<RwLock<HnswIndex>>, cause| {
                    // An evicted index that still had unsaved vectors used to be
                    // dropped from the dirty set WITHOUT being saved
                    // (`snapshot_dirty_indexes`' `else` branch, commented "Index was
                    // evicted, remove from dirty set") — silent data loss whose
                    // likelihood rises with the number of cache entries, which is
                    // exactly what partitioning increases. So save here, while we
                    // still hold the value.
                    let is_dirty = evict_dirty.read().unwrap().contains(key.as_ref());
                    if is_dirty {
                        let path = key.index_path(&evict_base);
                        match value.read().unwrap().save_to_file(&path) {
                            Ok(()) => {
                                evict_dirty.write().unwrap().remove(key.as_ref());
                                tracing::info!(
                                    index = %key,
                                    cause = ?cause,
                                    "Saved dirty HNSW index while evicting it"
                                );
                            }
                            Err(e) => tracing::error!(
                                index = %key,
                                cause = ?cause,
                                "Evicted a DIRTY HNSW index and could not save it: {e}"
                            ),
                        }
                    } else {
                        tracing::info!(index = %key, cause = ?cause, "Evicted HNSW index");
                    }
                },
            )
            .build();

        Ok(Self {
            base_path,
            index_cache,
            dirty_indexes,
            mutations_since_weigh: Arc::new(RwLock::new(HashMap::new())),
            fallback_dimensions,
            spec_resolver: None,
            distance_metric,
            metrics: Arc::new(metrics::VectorMetrics::new()),
        })
    }

    /// Attach the per-partition index-shape resolver.
    ///
    /// Without this the engine creates every index at `fallback_dimensions` with
    /// the engine's default metric and F32 storage, which is correct only for
    /// tenants on a 1536-wide model. Call it before wrapping the engine in an
    /// `Arc` and sharing it.
    pub fn with_spec_resolver(mut self, resolver: Arc<dyn IndexSpecResolver>) -> Self {
        self.spec_resolver = Some(resolver);
        self
    }

    /// Resolve the shape of one index.
    ///
    /// Consulted on a cache MISS only (see `get_or_load_index`), so this is an
    /// index-load-frequency read, not a per-vector one.
    pub(crate) fn resolve_spec(&self, key: &IndexKey) -> IndexSpec {
        self.spec_resolver
            .as_ref()
            .and_then(|r| r.spec_for(&key.tenant_id, &key.repo_id, &key.branch, &key.partition))
            .unwrap_or_else(|| {
                IndexSpec::new(self.fallback_dimensions).with_metric(self.distance_metric)
            })
    }

    /// The partition a caller with no embedder identity of its own should read.
    ///
    /// The SQL query path is the case: it has a query vector and a tenant, and
    /// needs to know which index that vector belongs in. Resolved off the same
    /// config row the write path uses, so read and write cannot land in
    /// different partitions. `None` when the tenant has no embedding config —
    /// which is also "there is nothing to search".
    pub fn default_text_partition(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Option<PartitionId> {
        self.spec_resolver
            .as_ref()
            .and_then(|r| r.default_text_partition(tenant_id, repo_id, branch))
    }

    /// Get the default distance metric for this engine.
    pub fn distance_metric(&self) -> DistanceMetric {
        self.distance_metric
    }

    /// The directory index files live under.
    pub fn base_path(&self) -> &Path {
        &self.base_path
    }

    /// The `.hnsw` path for one partition — THE path builder, exposed so that
    /// nothing else has to reimplement it.
    pub fn index_path_for(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        partition: &PartitionId,
    ) -> PathBuf {
        IndexKey::new(tenant_id, repo_id, branch, partition).index_path(&self.base_path)
    }

    /// Get a snapshot of vector search metrics.
    pub fn metrics(&self) -> metrics::VectorMetricsSnapshot {
        self.metrics.snapshot()
    }

    /// Every partition of this branch: on disk UNION resident in the cache.
    ///
    /// Both halves are load-bearing.
    ///
    /// * **Disk**, so a partition that exists but has never been loaded is
    ///   reported — `SHOW VECTOR INDEX HEALTH` and the replication transfer
    ///   listing both need that, and an operator cannot rebuild a partition they
    ///   cannot see.
    /// * **Cache**, because snapshots lag by up to 60s
    ///   (`lifecycle.rs`). A disk-only answer would omit a partition whose first
    ///   vector was added seconds ago — and the node-delete sweep uses this list
    ///   to decide which partitions to remove a deleted node from. Missing one
    ///   there leaves a deleted node's vector in the graph, winning ANN slots
    ///   that belong to live content, with nothing to report it. That is the
    ///   same class of residue the spec-blind delete used to leave, so it is
    ///   closed here rather than at each caller.
    pub fn list_partitions(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<Vec<PartitionId>> {
        let dir = self.base_path.join(tenant_id).join(repo_id).join(branch);
        let mut found: HashSet<PartitionId> = list_partitions_in(&dir).into_iter().collect();

        for (key, _) in self.index_cache.iter() {
            if key.tenant_id == tenant_id && key.repo_id == repo_id && key.branch == branch {
                found.insert(key.partition.clone());
            }
        }

        let mut out: Vec<PartitionId> = found.into_iter().collect();
        out.sort();
        Ok(out)
    }

    /// Get or load an HNSW index for one partition.
    ///
    /// If the index is in cache, returns it immediately.
    /// Otherwise, loads from disk or creates a new one.
    fn get_or_load_index(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        partition: &PartitionId,
    ) -> Result<Arc<RwLock<HnswIndex>>> {
        let key = IndexKey::new(tenant_id, repo_id, branch, partition);

        // Check cache first
        if let Some(index) = self.index_cache.get(&key) {
            self.metrics.record_cache_hit();
            return Ok(index);
        }

        self.metrics.record_cache_miss();

        // The shape is the PARTITION's, not the process's. Resolved here — on
        // the miss path — so a missing index is born at the width its own
        // embedding jobs will produce, instead of at a startup constant that
        // rejects every one of them.
        //
        // Miss-path only, deliberately: this is one small point-read per index
        // LOAD, not per vector, so it needs no cache of its own. The consequence
        // is that an index already resident keeps the shape it was loaded at
        // until it is evicted, restarted or explicitly recreated — changing the
        // config under a live index is what `REBUILD VECTOR INDEX` is for, and
        // it purges the cache entry.
        let spec = self.resolve_spec(&key);

        // One-time layout migration, before the existence check below. See the
        // function's own doc for why this is a rename and not a rebuild.
        migrate_legacy_layout(&self.base_path, &key);

        // Load from disk or create new
        let path = key.index_path(&self.base_path);
        let index = if path.exists() {
            let loaded = HnswIndex::view_from_file(&path)?;
            let on_disk = loaded.dimensions();
            if on_disk == spec.dimensions {
                loaded
            } else if loaded.len() == 0 {
                // Nothing to lose: an empty index at the wrong width is just a stale
                // artifact of a previous config (or of the old startup constant). Adopt
                // the configured width silently rather than making the tenant run a
                // rebuild over zero rows.
                tracing::warn!(
                    index = %key,
                    on_disk_dimensions = on_disk,
                    configured_dimensions = spec.dimensions,
                    "HNSW index is empty and was built at a different width; recreating it at the configured width"
                );
                self.dirty_indexes.write().unwrap().insert(key.clone());
                HnswIndex::with_params(spec.dimensions, spec.metric, spec.params)
            } else {
                // Loud, and it names the fix. The alternative is what this code used to
                // do: hand back an index of the wrong width, whose every `add` fails
                // with a bare "dimension mismatch" from deep inside a background job and
                // whose every search returns nothing.
                return Err(raisin_error::Error::storage(format!(
                    "HNSW index '{}' was built at {} dimensions but the tenant's embedding \
                     configuration is {} dimensions, and the index already holds {} vectors. \
                     Run REBUILD VECTOR INDEX to recreate it at the configured width \
                     (the vectors themselves are safe in the embeddings column family).",
                    key,
                    on_disk,
                    spec.dimensions,
                    loaded.len()
                )));
            }
        } else {
            HnswIndex::with_params(spec.dimensions, spec.metric, spec.params)
        };

        let index_arc = Arc::new(RwLock::new(index));

        // Insert into cache
        self.index_cache.insert(key, Arc::clone(&index_arc));

        Ok(index_arc)
    }

    /// Mark an index dirty and, periodically, make the cache re-weigh it.
    ///
    /// One place, called by every mutation, because a mutation that marks dirty
    /// without bumping the weigh counter is an index that grows invisibly to the
    /// memory budget — the exact defect this counter exists to fix.
    pub(crate) fn mark_mutated(&self, key: &IndexKey, index: &Arc<RwLock<HnswIndex>>) {
        self.dirty_indexes.write().unwrap().insert(key.clone());

        let due = {
            let mut counts = self.mutations_since_weigh.write().unwrap();
            let n = counts.entry(key.clone()).or_insert(0);
            *n += 1;
            if *n >= REWEIGH_EVERY_N_MUTATIONS {
                *n = 0;
                true
            } else {
                false
            }
        };

        if due {
            // Re-inserting the same Arc under the same key is how moka is made
            // to run the weigher again; it has no re-weigh API.
            self.index_cache.insert(key.clone(), Arc::clone(index));
            tracing::debug!(
                index = %key,
                bytes = index.read().unwrap().estimated_memory_bytes(),
                "Re-weighed HNSW index in the memory-bounded cache"
            );
        }
    }

    /// Total bytes the cache currently believes it is holding.
    ///
    /// This is the number the 512 MB budget is enforced against. Before the
    /// re-weigh fix it was frozen at the sum of load-time sizes (and zero for
    /// every index created rather than loaded), so it is exposed here to make
    /// the budget observable rather than notional.
    pub fn cached_bytes(&self) -> u64 {
        self.index_cache.run_pending_tasks();
        self.index_cache.weighted_size()
    }

    /// How many indexes are resident.
    pub fn cached_index_count(&self) -> u64 {
        self.index_cache.run_pending_tasks();
        self.index_cache.entry_count()
    }

    /// Get index statistics for one partition.
    ///
    /// Note: Returns stats for the entire partition (all workspaces combined).
    pub fn stats(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        partition: &PartitionId,
    ) -> Result<IndexStats> {
        let index_arc = self.get_or_load_index(tenant_id, repo_id, branch, partition)?;

        // A BOUNDED wait, not `read().unwrap()`.
        //
        // This is the diagnostic an operator reaches for when vector search has
        // gone quiet, and it used to queue behind the same write guard that had
        // gone quiet — so `SHOW VECTOR INDEX HEALTH` hung exactly when it was
        // needed, and the one tool that could have named the stuck partition
        // became another symptom of it. Reporting the partition as unavailable
        // is strictly more informative than not reporting at all: the caller
        // renders an `Err` as a `status: error` row and keeps going, so the
        // other partitions still answer.
        let index = read_guard_bounded(&index_arc).ok_or_else(|| {
            raisin_error::Error::storage(format!(
                "vector index for partition {partition} is busy: its write guard has been \
                 held for over {}s, so every search on this partition is blocked. A mutation \
                 that does not return is the signature of an index-level stall.",
                STATS_GUARD_WAIT.as_secs(),
            ))
        })?;

        Ok(IndexStats {
            count: index.len(),
            // The INDEX's width, not the engine's fallback. `SHOW VECTOR INDEX HEALTH`
            // renders this, and reporting the constant here is what made a correctly
            // configured 768-wide index report 1536 — the one number an operator would
            // check to confirm their config took effect.
            dimensions: index.dimensions(),
            memory_bytes: index.estimated_memory_bytes(),
            quantization: index.quantization(),
            distance_metric: index.distance_metric(),
        })
    }
}

/// Every partition token with a `.hnsw` file in `dir`.
///
/// Public because the replication transfer listing needs exactly this and a
/// second copy of it would be a second definition of "what counts as an index
/// file" — the shape that has already cost this module one data-loss bug.
pub fn list_partitions_in(dir: &Path) -> Vec<PartitionId> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        // `.hnsw.meta` also ends with nothing useful here; strip the exact
        // suffix and require the remainder to be a valid token, which rejects
        // `x.hnsw.meta` (its stem would be `x.hnsw`, and dots are invalid).
        let Some(stem) = name.strip_suffix(".hnsw") else {
            continue;
        };
        if let Some(p) = PartitionId::parse(stem) {
            out.push(p);
        }
    }
    out.sort();
    out
}

/// Move a pre-partition index into its partitioned home. Rename, never rebuild.
///
/// Before partitioning an index lived at `<base>/<t>/<r>/<branch>.hnsw`. Because
/// `TenantEmbeddingConfig` is per-tenant single-model, **every existing index
/// has exactly one possible partition** — the tenant's resolved embedder, kind
/// `Text` — so the migration is a two-file rename with zero vectors re-encoded.
/// It is idempotent (a target that already exists wins) and it is a no-op for
/// anyone who never had a legacy index.
///
/// Deliberately NOT routed through `migration.rs`: that is a *format* migration
/// triggered by a missing sidecar, and feeding a v2 usearch file into it is
/// precisely the `hnsw_transfer` data-loss bug — it bincode-deserialises a
/// usearch file and the index is gone. The sidecar moves WITH the graph file
/// here, so the loader never sees a bare `.hnsw`.
///
/// Failures are logged, not returned: a rename that could not happen leaves the
/// legacy file exactly where it was, and the caller then creates an empty index
/// — recoverable with `REBUILD VECTOR INDEX`, which is strictly better than
/// refusing to open the branch at all.
pub(crate) fn migrate_legacy_layout(base: &Path, key: &IndexKey) {
    let new_index = key.index_path(base);
    if new_index.exists() {
        return;
    }
    let old_index = key.legacy_index_path(base);
    if !old_index.exists() {
        return;
    }

    let old_meta = crate::persistence::meta_path_for(&old_index);
    let new_meta = key.meta_path(base);

    if let Some(parent) = new_index.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            tracing::error!(
                index = %key,
                "Could not create the partition directory for the legacy HNSW index: {e}"
            );
            return;
        }
    }

    // Sidecar FIRST. If the process dies between the two renames, the graph
    // file is still at the legacy path and this runs again from the top; the
    // other order would leave a bare `.hnsw` at the new path, which is the
    // shape that routes into the bincode migration and loses the index.
    if old_meta.exists() {
        if let Err(e) = std::fs::rename(&old_meta, &new_meta) {
            tracing::error!(
                index = %key,
                "Could not move the legacy HNSW metadata sidecar: {e}"
            );
            return;
        }
    }
    if let Err(e) = std::fs::rename(&old_index, &new_index) {
        tracing::error!(index = %key, "Could not move the legacy HNSW index: {e}");
        // Put the sidecar back so the pair stays together for the next attempt.
        let _ = std::fs::rename(&new_meta, &old_meta);
        return;
    }

    tracing::info!(
        index = %key,
        from = %old_index.display(),
        to = %new_index.display(),
        "Moved a pre-partition HNSW index into its partition (rename only, no vectors re-encoded)"
    );
}

/// How long a read-only diagnostic will wait for the index guard before it
/// reports the partition as busy rather than blocking with it.
const STATS_GUARD_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// Acquire a read guard, or give up.
///
/// `std::sync::RwLock` has no timed acquire, so this polls `try_read`. A
/// poisoned lock returns `None` too — a panic inside a mutation leaves the
/// index unusable, and a diagnostic that panics in sympathy tells the operator
/// nothing they could not already see.
fn read_guard_bounded<T>(lock: &RwLock<T>) -> Option<std::sync::RwLockReadGuard<'_, T>> {
    let deadline = std::time::Instant::now() + STATS_GUARD_WAIT;
    loop {
        match lock.try_read() {
            Ok(guard) => return Some(guard),
            Err(std::sync::TryLockError::Poisoned(_)) => return None,
            Err(std::sync::TryLockError::WouldBlock) => {
                if std::time::Instant::now() >= deadline {
                    return None;
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
        }
    }
}

/// Index statistics.
#[derive(Debug, Clone)]
pub struct IndexStats {
    /// Number of vectors in the index
    pub count: usize,

    /// Vector dimensions
    pub dimensions: usize,

    /// Estimated memory usage in bytes
    pub memory_bytes: usize,

    /// Scalar kind vectors are stored at. `Int8` is a quarter the payload of
    /// `F32`; reporting it is how an operator confirms a quantization setting
    /// actually took.
    pub quantization: crate::types::QuantizationType,

    /// The metric the graph was BUILT with — not the one a query asked for.
    pub distance_metric: DistanceMetric,
}
