// SPDX-License-Identifier: BSL-1.1

//! Core types for the Tantivy indexing engine.

use moka::sync::Cache;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tantivy::schema::Field;
use tantivy::{Index, IndexReader, IndexWriter};

/// Tantivy-based indexing engine implementing branch-aware, multi-language full-text search.
pub struct TantivyIndexingEngine {
    pub(crate) base_path: PathBuf,
    pub(crate) index_cache: Cache<String, Arc<CachedIndex>>,
    /// THE writer for each index directory (`tenant/repo/branch`), held for as
    /// long as the engine keeps serving that index.
    ///
    /// Two separate reasons this is one shared, long-lived writer rather than
    /// one opened per operation.
    ///
    /// **Exclusion.** Tantivy takes an EXCLUSIVE, non-blocking lock on the
    /// index directory for the lifetime of an `IndexWriter`; a second
    /// `Index::writer()` while one is alive fails immediately with `LockBusy`.
    /// Two concurrent indexing operations on the same index therefore used to
    /// make one of them fail — and a failed per-node job means that node is
    /// simply absent from search. Serializing here, in the thing that owns the
    /// directory, is what makes that impossible for EVERY caller.
    ///
    /// **Merges.** `IndexWriter::drop` calls `SegmentUpdater::kill()`, which
    /// makes every merge that writer had in flight fail with "Segment updater
    /// killed" and discards its output. A writer opened and dropped per
    /// operation therefore adds a segment on every commit and destroys the
    /// merge meant to absorb it, so segments only ever accumulate and each
    /// subsequent commit re-schedules merging over a bigger set. In production
    /// that ran away to 753 segments for 128k documents, with ~2.7 cores
    /// permanently inside `IndexMerger::write`. Keeping the writer alive is
    /// what lets those merges finish.
    ///
    /// Deliberately NOT stored on `CachedIndex`: that entry can be evicted
    /// under cache pressure while a caller still holds an `Arc` to it, which
    /// would leave two writers contending for one directory. This map outlives
    /// the cache, and `invalidate_cached_index` clears both together.
    pub(crate) writers: Mutex<HashMap<String, Arc<WriterSlot>>>,
}

/// One index's writer, plus when it was last used.
///
/// The timestamp exists only so [`TantivyIndexingEngine::with_writer`] can cap
/// the map: every live slot costs an indexing thread and holds a directory
/// lock, and a repo that forks branches would otherwise accumulate one per
/// branch it ever touched, forever.
pub(crate) struct WriterSlot {
    pub(crate) writer: Mutex<IndexWriter>,
    /// Unix milliseconds. Written under no lock; only ever read to pick
    /// eviction victims, where being a few milliseconds stale is harmless.
    pub(crate) last_used_ms: std::sync::atomic::AtomicI64,
}

impl WriterSlot {
    pub(crate) fn touch(&self) {
        self.last_used_ms.store(
            chrono::Utc::now().timestamp_millis(),
            std::sync::atomic::Ordering::Relaxed,
        );
    }
}

/// Cached index with Index, IndexReader, and the field handles resolved from the
/// index's ACTUAL on-disk schema (so an older index missing newer fields still
/// works — its `shape_types` is simply `None` until a rebuild).
pub(crate) struct CachedIndex {
    pub(crate) index: Index,
    pub(crate) reader: IndexReader,
    pub(crate) fields: SchemaFields,
}

/// Tantivy schema field definitions
#[derive(Clone)]
pub(crate) struct SchemaFields {
    pub(crate) doc_id: Field,
    pub(crate) node_id: Field,
    pub(crate) workspace_id: Field,
    pub(crate) language: Field,
    pub(crate) path: Field,
    pub(crate) node_type: Field,
    pub(crate) revision_timestamp: Field,
    pub(crate) revision_counter: Field,
    pub(crate) created_at: Field,
    pub(crate) updated_at: Field,
    pub(crate) name: Field,
    pub(crate) content: Field,
    /// `None` for indexes built before the field existed (pre-v2 on-disk schema).
    pub(crate) shape_types: Option<Field>,
    /// Per-language stemmed field pairs, keyed by ISO 639-1 code.
    ///
    /// Empty for indexes built before the pairs existed (pre-v3 on-disk
    /// schema), and missing an entry for any language added since the index was
    /// created. Both the writer and the searcher look a language up here and do
    /// nothing when it is absent, so an older index stays consistent —
    /// unstemmed on both sides — instead of writing terms nothing searches.
    pub(crate) stemmed: HashMap<String, StemmedFields>,
}

/// The `name`/`content` pair analysed with one language's stemmer.
#[derive(Clone, Copy)]
pub(crate) struct StemmedFields {
    pub(crate) name: Field,
    pub(crate) content: Field,
}

/// Batch indexing context for bulk operations
pub struct BatchIndexContext {
    pub tenant_id: String,
    pub repo_id: String,
    pub branch: String,
    pub workspace_id: String,
    pub default_language: String,
    pub supported_languages: Vec<String>,
}
