// SPDX-License-Identifier: BSL-1.1

//! Index creation, caching, and management.

use moka::sync::Cache;
use raisin_error::{Error, Result};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tantivy::{Index, IndexWriter, ReloadPolicy};

use super::language::register_analyzers;
use super::schema::{build_schema, schema_fields, SCHEMA_VERSION};
use super::types::{CachedIndex, TantivyIndexingEngine, WriterSlot};

/// Sidecar file (next to Tantivy's `meta.json`) recording the schema version the
/// on-disk index was built with, so version mismatches can trigger a rebuild.
const SCHEMA_VERSION_FILE: &str = "raisin_schema_version";

/// Weight charged to the index cache per entry, against the byte-denominated
/// `cache_size` the engine is constructed with.
///
/// A cache entry is a set of handles — Tantivy mmaps the segment files — so the
/// previous 30 MB-per-entry figure measured nothing while capping a 512 MB
/// cache at seventeen indexes, against twenty-three in production. Evicting an
/// entry costs a re-open on the next query, so routine eviction is pure waste.
/// 1 MB per handle makes the existing knob a pressure valve instead.
const CACHE_ENTRY_WEIGHT: u32 = 1024 * 1024;

/// Upper bound on live writers, each of which costs one indexing thread and an
/// open directory lock. Past this, [`TantivyIndexingEngine::with_writer`] drops
/// the least recently used idle ones. Comfortably above the number of indexes
/// a single deployment serves; the cap exists for the pathological case of a
/// repo that forks branches without bound.
const MAX_LIVE_WRITERS: usize = 64;

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
            .weigher(|_key: &String, _index: &Arc<CachedIndex>| -> u32 { CACHE_ENTRY_WEIGHT })
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
            writers: Mutex::new(HashMap::new()),
        })
    }

    /// The canonical key for one index directory. Used for the moka cache AND
    /// for the writer slot, so the two can never disagree about what "one
    /// index" means.
    pub(crate) fn index_key(tenant_id: &str, repo_id: &str, branch: &str) -> String {
        format!("{}/{}/{}", tenant_id, repo_id, branch)
    }

    pub(crate) fn get_or_create_index(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
    ) -> Result<Arc<CachedIndex>> {
        let cache_key = Self::index_key(tenant_id, repo_id, branch);

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
                     shape-driven indexing (element/archetype identities) and for \
                     language analysis (stemming, CJK segmentation). Until it is \
                     rebuilt this index keeps its old analyzer on both the write and \
                     the query side — consistent, but unstemmed and dropping CJK. \
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

        // The ONE registration site. Analyzers are looked up by name, lazily, by
        // both the writer and the query parser, so they must be in place before
        // either exists — and `get_or_create_index` is the only code that ever
        // constructs or opens an `Index`. Registering from the write paths (as
        // this used to, once in `indexing_impl` and again in `batch`) left a
        // search-before-first-write on a freshly opened index with no analyzer,
        // and gave the same rule two copies to drift apart.
        register_analyzers(&index)?;

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

    /// Construct the one long-lived writer for an index directory.
    ///
    /// One indexing thread, deliberately. `Index::writer(budget)` divides the
    /// budget across up to eight threads and each thread flushes its OWN
    /// segment, so a multi-threaded writer multiplies the segment count of
    /// every commit — the opposite of what this index needs, since merging is
    /// the bottleneck here and indexing is not. It also keeps the thread count
    /// sane now that a writer lives for as long as the engine serves the index.
    fn new_writer(index: &Index) -> Result<IndexWriter> {
        // Tantivy's per-thread floor is `MEMORY_BUDGET_NUM_BYTES_MIN` (15 MB);
        // below it, writer construction is an `InvalidArgument`. The arena
        // fills as documents arrive and is released at each commit, so this is
        // a ceiling rather than a resident cost.
        const WRITER_HEAP_SIZE: usize = 50_000_000;
        index
            .writer_with_num_threads(1, WRITER_HEAP_SIZE)
            .map_err(|e| Error::storage(format!("Failed to create index writer: {}", e)))
    }

    /// The ONE writer lifecycle for this engine: take the index's writer, run
    /// `f`, commit — and leave the writer open.
    ///
    /// Every write path (single node, delete, batch) goes through here. That is
    /// deliberate:
    ///
    /// * **Mutual exclusion.** Tantivy's directory lock is exclusive and
    ///   *non-blocking*: a second `Index::writer()` while one is alive returns
    ///   `LockBusy` rather than waiting. Two concurrent indexing jobs on the
    ///   same (tenant, repo, branch) therefore used to make one of them fail
    ///   outright — and a failed single-node job leaves that node missing from
    ///   search until someone rebuilds. Queueing on the writer turns a lost
    ///   document into a few milliseconds of waiting.
    /// * **The writer is NOT dropped afterwards.** `IndexWriter::drop` kills the
    ///   segment updater, so a writer that dies at the end of every operation
    ///   cancels the merge its own commit just scheduled. See `WriterSlot` for
    ///   what that cost in production. This is the whole reason the writer is
    ///   owned by the engine rather than by this function.
    /// * **One commit, always.** Returning early on error rolls back rather
    ///   than leaving pending operations for the next caller to commit.
    pub(crate) fn with_writer<T>(
        &self,
        index_key: &str,
        index: &Index,
        f: impl FnOnce(&mut IndexWriter) -> Result<T>,
    ) -> Result<T> {
        let slot = self.writer_slot(index_key, index)?;
        slot.touch();

        // A panic inside a previous writer must not permanently wedge this
        // index: recover the guard rather than propagating the poison.
        let mut writer = slot
            .writer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let outcome = f(&mut writer).and_then(|value| {
            writer
                .commit()
                .map_err(|e| Error::storage(format!("Failed to commit index: {}", e)))?;
            Ok(value)
        });

        if outcome.is_err() {
            // Best-effort: leave no half-applied operations behind for the next
            // caller on this writer.
            let _ = writer.rollback();
        }

        outcome
    }

    /// Get this index's writer slot, opening the writer on first use.
    fn writer_slot(&self, index_key: &str, index: &Index) -> Result<Arc<WriterSlot>> {
        let mut writers = self
            .writers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        if let Some(slot) = writers.get(index_key) {
            return Ok(Arc::clone(slot));
        }

        let slot = Arc::new(WriterSlot {
            writer: Mutex::new(Self::new_writer(index)?),
            last_used_ms: std::sync::atomic::AtomicI64::new(chrono::Utc::now().timestamp_millis()),
        });
        writers.insert(index_key.to_string(), Arc::clone(&slot));

        Self::prune_writers(&mut writers, index_key);

        Ok(slot)
    }

    /// Drop the least recently used idle writers once the map exceeds
    /// `MAX_LIVE_WRITERS`.
    ///
    /// Only entries nobody else holds an `Arc` to are eligible: removing one
    /// that is still in use would leave its writer alive — and its directory
    /// lock held — while the next caller tried to open a second one, turning a
    /// bookkeeping decision into a `LockBusy`. Dropping the last `Arc` here
    /// does kill whatever merges that writer had in flight, which is why the
    /// cap is a backstop for unbounded branch growth and not a routine size.
    fn prune_writers(writers: &mut HashMap<String, Arc<WriterSlot>>, keep: &str) {
        if writers.len() <= MAX_LIVE_WRITERS {
            return;
        }

        let mut idle: Vec<(String, i64)> = writers
            .iter()
            .filter(|(key, slot)| key.as_str() != keep && Arc::strong_count(slot) == 1)
            .map(|(key, slot)| {
                (
                    key.clone(),
                    slot.last_used_ms.load(std::sync::atomic::Ordering::Relaxed),
                )
            })
            .collect();
        idle.sort_by_key(|(_, last_used)| *last_used);

        for (key, _) in idle {
            if writers.len() <= MAX_LIVE_WRITERS {
                break;
            }
            writers.remove(&key);
            tracing::debug!(index = %key, "Closed idle Tantivy writer to stay under the cap");
        }
    }

    /// Merge every segment of one index into a single segment.
    ///
    /// Goes through the shared writer rather than opening its own: Tantivy's
    /// writer lock is exclusive, so a second writer on a live index fails with
    /// `DirectoryLockBusy`, and a throwaway writer would have to
    /// `wait_merging_threads()` to keep its own merge from being killed on drop
    /// — precisely the trap this design removes.
    ///
    /// Holding the writer for the duration means indexing on this index waits
    /// rather than racing the merge. Returns `(segments_before, segments_after)`.
    pub fn optimize(&self, tenant_id: &str, repo_id: &str, branch: &str) -> Result<(usize, usize)> {
        let cached = self.get_or_create_index(tenant_id, repo_id, branch)?;

        let segment_ids = cached
            .index
            .searchable_segment_ids()
            .map_err(|e| Error::storage(format!("Failed to list segments: {}", e)))?;
        let before = segment_ids.len();
        if before <= 1 {
            return Ok((before, before));
        }

        let index_key = Self::index_key(tenant_id, repo_id, branch);
        self.with_writer(&index_key, &cached.index, |writer| {
            writer
                .merge(&segment_ids)
                .wait()
                .map_err(|e| Error::storage(format!("Merge failed: {}", e)))?;
            Ok(())
        })?;

        let after = cached
            .index
            .searchable_segment_ids()
            .map_err(|e| Error::storage(format!("Failed to list segments: {}", e)))?
            .len();

        tracing::info!(
            tenant_id,
            repo_id,
            branch,
            segments_before = before,
            segments_after = after,
            "Optimized fulltext index"
        );

        Ok((before, after))
    }

    /// Drop the cached `Index` + `IndexReader` for a given key.
    ///
    /// Called by the rebuild path before `remove_dir_all`. Moka's
    /// `invalidate` is asynchronous on a sync cache, so this isn't a
    /// hard synchronization barrier on its own — the rebuild caller
    /// must additionally hold the `IndexLockManager` lock to prevent
    /// other code from racing back in via `get_or_create_index`.
    pub fn invalidate_cached_index(&self, tenant_id: &str, repo_id: &str, branch: &str) {
        let cache_key = Self::index_key(tenant_id, repo_id, branch);
        self.index_cache.invalidate(&cache_key);
        self.index_cache.run_pending_tasks();

        // Close the writer too. Every caller of this is about to delete or
        // replace the directory, and a writer left open would hold the
        // directory lock and go on writing merge output into a tree that is
        // being removed underneath it.
        let removed = self
            .writers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&cache_key);
        if let Some(slot) = removed {
            if Arc::strong_count(&slot) > 1 {
                tracing::warn!(
                    index = %cache_key,
                    "Closed a Tantivy writer that another operation still holds; \
                     it will release the directory lock when that finishes"
                );
            }
        }
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
