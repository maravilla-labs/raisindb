// SPDX-License-Identifier: BSL-1.1

//! Index creation, caching, and management.

use moka::sync::Cache;
use raisin_error::{Error, Result};
use std::path::PathBuf;
use std::sync::Arc;
use tantivy::{Index, IndexWriter, ReloadPolicy};

use super::schema::build_schema;
use super::types::{CachedIndex, TantivyIndexingEngine};

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
            Index::open_in_dir(&index_path)
                .map_err(|e| Error::storage(format!("Failed to open index: {}", e)))?
        } else {
            Index::create_in_dir(&index_path, schema)
                .map_err(|e| Error::storage(format!("Failed to create index: {}", e)))?
        };

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .map_err(|e| Error::storage(format!("Failed to create index reader: {}", e)))?;

        let cached = Arc::new(CachedIndex { index, reader });
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
}
