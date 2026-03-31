//! Indexing-related job handler construction
//!
//! Creates handlers for fulltext search, embedding, property indexing,
//! and compound index maintenance.

use std::sync::Arc;

use raisin_hnsw::HnswIndexingEngine;
use raisin_indexer::tantivy_engine::TantivyIndexingEngine;

use crate::jobs::{
    CompoundIndexJobHandler, EmbeddingJobHandler, FulltextJobHandler, IndexLockManager,
    PropertyIndexJobHandler,
};
use crate::storage::RocksDBStorage;

/// Create the fulltext indexing handler
pub fn create_fulltext_handler(
    storage: Arc<RocksDBStorage>,
    tantivy_engine: Arc<TantivyIndexingEngine>,
) -> Arc<FulltextJobHandler> {
    let index_lock_manager = Arc::new(IndexLockManager::new());
    Arc::new(FulltextJobHandler::new(
        storage,
        tantivy_engine,
        index_lock_manager,
    ))
}

/// Create the embedding indexing handler
pub fn create_embedding_handler(
    storage: Arc<RocksDBStorage>,
    hnsw_engine: Arc<HnswIndexingEngine>,
    master_key: [u8; 32],
) -> Arc<EmbeddingJobHandler> {
    Arc::new(EmbeddingJobHandler::new(storage, hnsw_engine, master_key))
}

/// Create the property index handler
pub fn create_property_index_handler(storage: &RocksDBStorage) -> Arc<PropertyIndexJobHandler> {
    Arc::new(PropertyIndexJobHandler::new(Arc::new(
        storage.lazy_index_manager.clone(),
    )))
}

/// Create the compound index handler
pub fn create_compound_index_handler(storage: &RocksDBStorage) -> Arc<CompoundIndexJobHandler> {
    let revision_repo = Arc::new(crate::repositories::RevisionRepositoryImpl::new(
        storage.db.clone(),
        storage.config.cluster_node_id.clone().unwrap_or_default(),
    ));
    let branch_repo = Arc::new(crate::repositories::BranchRepositoryImpl::new(
        storage.db.clone(),
    ));
    Arc::new(CompoundIndexJobHandler::new(
        storage.db.clone(),
        revision_repo,
        branch_repo,
    ))
}
