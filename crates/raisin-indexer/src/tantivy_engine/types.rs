// SPDX-License-Identifier: BSL-1.1

//! Core types for the Tantivy indexing engine.

use moka::sync::Cache;
use std::path::PathBuf;
use std::sync::Arc;
use tantivy::schema::Field;
use tantivy::{Index, IndexReader};

/// Tantivy-based indexing engine implementing branch-aware, multi-language full-text search.
pub struct TantivyIndexingEngine {
    pub(crate) base_path: PathBuf,
    pub(crate) index_cache: Cache<String, Arc<CachedIndex>>,
}

/// Cached index with both Index and IndexReader for efficient searching
pub(crate) struct CachedIndex {
    pub(crate) index: Index,
    pub(crate) reader: IndexReader,
}

/// Tantivy schema field definitions
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
