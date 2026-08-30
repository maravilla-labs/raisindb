// SPDX-License-Identifier: BSL-1.1

//! Core types for the Tantivy indexing engine.

use moka::sync::Cache;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tantivy::schema::Field;
use tantivy::{Index, IndexReader};

/// Tantivy-based indexing engine implementing branch-aware, multi-language full-text search.
pub struct TantivyIndexingEngine {
    pub(crate) base_path: PathBuf,
    pub(crate) index_cache: Cache<String, Arc<CachedIndex>>,
    /// One writer slot per index directory (`tenant/repo/branch`).
    ///
    /// Tantivy takes an EXCLUSIVE, non-blocking lock on the index directory for
    /// the lifetime of an `IndexWriter`; a second `Index::writer()` while one is
    /// alive fails immediately with `LockBusy`. Every write path here opens a
    /// short-lived writer, so two concurrent indexing operations on the same
    /// (tenant, repo, branch) used to make one of them fail — and a failed
    /// per-node indexing job means that node is simply absent from search.
    ///
    /// Serializing here, in the engine, is what makes that impossible for EVERY
    /// caller. Callers must not be trusted to bring their own lock: the guarantee
    /// belongs to the thing that owns the directory.
    pub(crate) writer_slots: Mutex<HashMap<String, Arc<Mutex<()>>>>,
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
