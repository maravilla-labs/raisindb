//! Indexing-related job handler construction
//!
//! Creates handlers for fulltext search, embedding, property indexing,
//! and compound index maintenance.

use std::sync::Arc;

use raisin_hnsw::HnswIndexingEngine;
use raisin_indexer::tantivy_engine::TantivyIndexingEngine;

use crate::jobs::{
    CompoundIndexJobHandler, EmbeddingJobHandler, FulltextJobHandler, PropertyIndexJobHandler,
};
use crate::storage::RocksDBStorage;

/// Create the fulltext indexing handler
///
/// Reuses the storage-owned `IndexLockManager` so the admin rebuild
/// path (`raisin_rocksdb::management::fulltext::rebuild_fulltext_index`)
/// can acquire the same per-(tenant,repo,branch) mutex and serialize
/// against in-flight batch indexing instead of racing the directory.
/// Also clones the storage-owned `FulltextErrorCounter` so the HTTP
/// `/fulltext/errors` endpoint sees the same in-memory map the worker
/// writes to.
pub fn create_fulltext_handler(
    storage: Arc<RocksDBStorage>,
    tantivy_engine: Arc<TantivyIndexingEngine>,
) -> Arc<FulltextJobHandler> {
    let index_lock_manager = storage.index_lock_manager().clone();
    let error_counter = storage.fulltext_error_counter().clone();
    Arc::new(FulltextJobHandler::new(
        storage,
        tantivy_engine,
        index_lock_manager,
        error_counter,
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

/// Create the spatial index build handler.
///
/// Takes only the raw database handle: the build derives everything else from the
/// node records and the local index-state records, which is what lets each cluster
/// node rebuild independently with no coordination.
pub fn create_spatial_index_handler(
    storage: &RocksDBStorage,
) -> Arc<crate::jobs::handlers::SpatialIndexJobHandler> {
    Arc::new(crate::jobs::handlers::SpatialIndexJobHandler::new(
        storage.db.clone(),
    ))
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
