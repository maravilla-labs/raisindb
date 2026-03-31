//! Indexing engine initialization.
//!
//! This module handles the initialization of Tantivy full-text search
//! and HNSW vector indexing engines.

use std::path::PathBuf;
use std::sync::Arc;

#[cfg(feature = "storage-rocksdb")]
use raisin_hnsw::HnswIndexingEngine;
#[cfg(feature = "storage-rocksdb")]
use raisin_indexer::TantivyIndexingEngine;

/// Initialize the Tantivy full-text search engine.
#[cfg(feature = "storage-rocksdb")]
pub fn init_tantivy_engine(
    index_path: PathBuf,
) -> (
    Arc<TantivyIndexingEngine>,
    Arc<raisin_indexer::TantivyManagement>,
) {
    tracing::info!("Initializing Tantivy engine (shared by API and job system)...");

    let cache_size = 512 * 1024 * 1024;
    let engine = Arc::new(
        TantivyIndexingEngine::new(index_path.clone(), cache_size)
            .expect("Failed to create indexing engine"),
    );

    let management = Arc::new(raisin_indexer::TantivyManagement::new(
        index_path,
        engine.clone(),
    ));

    tracing::info!("Tantivy search engine initialized");

    (engine, management)
}

/// Initialize the HNSW vector indexing engine.
#[cfg(feature = "storage-rocksdb")]
pub fn init_hnsw_engine(hnsw_path: PathBuf) -> Arc<HnswIndexingEngine> {
    tracing::info!("Initializing HNSW engine (shared by API and job system)...");

    let cache_size = 512 * 1024 * 1024;
    let engine = Arc::new(
        HnswIndexingEngine::new(hnsw_path, cache_size, 1536).expect("Failed to create HNSW engine"),
    );

    let _snapshot_handle = engine.start_snapshot_task();

    tracing::info!("HNSW engine initialized, snapshot task started");

    engine
}

/// Initialize HNSW management service.
#[cfg(feature = "storage-rocksdb")]
pub fn init_hnsw_management(
    hnsw_engine: Arc<HnswIndexingEngine>,
    embedding_storage: Arc<raisin_rocksdb::RocksDBEmbeddingStorage>,
    storage: &Arc<raisin_rocksdb::RocksDBStorage>,
) -> Arc<raisin_rocksdb::HnswManagement> {
    use raisin_rocksdb::HnswManagement;

    tracing::info!("Initializing HNSW management...");

    let config_repo = storage.tenant_embedding_config_repository();

    let management = Arc::new(HnswManagement::new(
        hnsw_engine,
        embedding_storage,
        config_repo,
    ));

    tracing::info!("HNSW management initialized");

    management
}

/// Initialize embedding storage for HTTP API layer.
#[cfg(feature = "storage-rocksdb")]
pub fn init_embedding_storage(
    db: Arc<rocksdb::DB>,
) -> (
    Arc<raisin_rocksdb::RocksDBEmbeddingStorage>,
    Arc<raisin_rocksdb::RocksDBEmbeddingJobStore>,
) {
    use raisin_rocksdb::{RocksDBEmbeddingJobStore, RocksDBEmbeddingStorage};

    tracing::info!("Initializing embedding storage for API...");

    let emb_storage = Arc::new(RocksDBEmbeddingStorage::new(db.clone()));
    let emb_job_store = Arc::new(RocksDBEmbeddingJobStore::new(db));

    tracing::info!("Embedding storage ready for API endpoints");

    (emb_storage, emb_job_store)
}
