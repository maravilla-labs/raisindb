// SPDX-License-Identifier: BSL-1.1

//! Index creation, caching, and management.

use moka::sync::Cache;
use raisin_error::{Error, Result};
use std::path::PathBuf;
use std::sync::Arc;
use tantivy::{Index, IndexWriter, ReloadPolicy};

use super::schema::{build_schema, schema_fields, SCHEMA_VERSION};
use super::types::{CachedIndex, TantivyIndexingEngine};

/// Sidecar file (next to Tantivy's `meta.json`) recording the schema version the
/// on-disk index was built with, so version mismatches can trigger a rebuild.
const SCHEMA_VERSION_FILE: &str = "raisin_schema_version";

fn read_schema_version(index_path: &std::path::Path) -> Option<u32> {
    std::fs::read_to_string(index_path.join(SCHEMA_VERSION_FILE))
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
}

fn write_schema_version(index_path: &std::path::Path) {
    if let Err(e) = std::fs::write(
        index_path.join(SCHEMA_VERSION_FILE),
        SCHEMA_VERSION.to_string(),
    ) {
        tracing::warn!(error = %e, "Failed to write fulltext schema version sidecar");
    }
}

impl TantivyIndexingEngine {
    pub fn new(base_path: PathBuf, cache_size: usize) -> Result<Self> {
        std::fs::create_dir_all(&base_path)
            .map_err(|e| Error::storage(format!("Failed to create index base path: {}", e)))?;

        let index_cache = Cache::builder()
            .weigher(|_key: &String, _index: &Arc<CachedIndex>| -> u32 { 30 * 1024 * 1024 })
            .max_capacity(cache_size as u64)
            .eviction_listener(|key, _value, cause| {
                tracing::info!(
                    "Evicted Tantivy index from cache: {} (cause: {:?})",
                    key,
                    cause
                );
            })
            .build();

        Ok(Self {
            base_path,
            index_cache,
        })
    }

    pub(crate) fn get_or_create_index(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<Arc<CachedIndex>> {
        let cache_key = format!("{}/{}/{}", tenant_id, repo_id, branch);

        if let Some(cached) = self.index_cache.get(&cache_key) {
            tracing::debug!("Cache hit for index: {}", cache_key);
            return Ok(cached);
        }

        tracing::debug!("Cache miss for index: {}, loading from disk", cache_key);
        let index_path = self.base_path.join(tenant_id).join(repo_id).join(branch);

        std::fs::create_dir_all(&index_path)
            .map_err(|e| Error::storage(format!("Failed to create index directory: {}", e)))?;

        let (schema, _fields) = build_schema();

        let index = if index_path.join("meta.json").exists() {
            let on_disk = read_schema_version(&index_path);
            if on_disk.map(|v| v < SCHEMA_VERSION).unwrap_or(true) {
                tracing::warn!(
                    index = %cache_key,
                    on_disk_version = ?on_disk,
                    expected_version = SCHEMA_VERSION,
                    "Fulltext index schema is out of date; a rebuild is required for \
                     shape-driven indexing (element/archetype identities). \
                     Run the fulltext rebuild for this repo/branch."
                );
            }
            Index::open_in_dir(&index_path)
                .map_err(|e| Error::storage(format!("Failed to open index: {}", e)))?
        } else {
            let index = Index::create_in_dir(&index_path, schema)
                .map_err(|e| Error::storage(format!("Failed to create index: {}", e)))?;
            write_schema_version(&index_path);
            index
        };

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .map_err(|e| Error::storage(format!("Failed to create index reader: {}", e)))?;

        // Resolve field handles from the index's ACTUAL schema, not the code
        // schema, so a pre-v2 on-disk index (without `shape_types`) is written
        // and searched safely (degraded) instead of referencing a missing field.
        let fields = schema_fields(&index.schema());

        let cached = Arc::new(CachedIndex {
            index,
            reader,
            fields,
        });
        self.index_cache
            .insert(cache_key.clone(), Arc::clone(&cached));
        Ok(cached)
    }

    pub(crate) fn get_writer(index: &Index) -> Result<IndexWriter> {
        const WRITER_HEAP_SIZE: usize = 50_000_000;
        index
            .writer(WRITER_HEAP_SIZE)
            .map_err(|e| Error::storage(format!("Failed to create index writer: {}", e)))
    }

    /// Drop the cached `Index` + `IndexReader` for a given key.
    ///
    /// Called by the rebuild path before `remove_dir_all`. Moka's
    /// `invalidate` is asynchronous on a sync cache, so this isn't a
    /// hard synchronization barrier on its own — the rebuild caller
    /// must additionally hold the `IndexLockManager` lock to prevent
    /// other code from racing back in via `get_or_create_index`.
    pub fn invalidate_cached_index(&self, tenant_id: &str, repo_id: &str, branch: &str) {
        let cache_key = format!("{}/{}/{}", tenant_id, repo_id, branch);
        self.index_cache.invalidate(&cache_key);
        self.index_cache.run_pending_tasks();
    }

    /// Read-only access to the on-disk root for this engine. Needed
    /// by management/rebuild paths that have to `remove_dir_all` the
    /// directory belonging to a specific (tenant, repo, branch).
    pub fn base_path(&self) -> &std::path::Path {
        &self.base_path
    }

    /// Whether the on-disk index for this scope was built with an older schema
    /// version than the current code (or predates version tracking) and so needs
    /// a rebuild. Returns `false` for a not-yet-created index (it will be created
    /// at the current version on first use).
    pub fn is_index_stale(&self, tenant_id: &str, repo_id: &str, branch: &str) -> bool {
        let index_path = self.base_path.join(tenant_id).join(repo_id).join(branch);
        if !index_path.join("meta.json").exists() {
            return false;
        }
        read_schema_version(&index_path)
            .map(|v| v < SCHEMA_VERSION)
            .unwrap_or(true)
    }
}
