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
///
/// The engine gets a per-partition SHAPE resolver, not a width. Width, distance
/// metric and scalar quantization are all properties of the tenant's embedding
/// configuration (768 for `nomic-embed-text`, 1024 for `bge-m3`, 1536 for
/// OpenAI's small models), and they are already stored in
/// `TenantEmbeddingConfig` — which the `EmbeddingGenerate` job handler and
/// `REBUILD VECTOR INDEX` both read. Passing a constant here instead is what made a
/// correctly configured tenant look broken: every generated vector reached
/// `cf::EMBEDDINGS` and was then refused by an index built at somebody else's width,
/// so vector queries returned nothing while the embedding count climbed.
///
/// The 512 MB cache budget is shared by every tenant AND, now, by every
/// partition — a branch holds one index per embedding space. It only became a
/// real bound with the re-weigh fix in `raisin-hnsw`: moka calls its weigher
/// once at insert and never again, so a created index was pinned at ~0 bytes
/// forever and a loaded one at its load-time size, and this number bounded
/// nothing.
#[cfg(feature = "storage-rocksdb")]
pub fn init_hnsw_engine(
    hnsw_path: PathBuf,
    storage: &Arc<raisin_rocksdb::RocksDBStorage>,
) -> Arc<HnswIndexingEngine> {
    tracing::info!("Initializing HNSW engine (shared by API and job system)...");

    let cache_size = 512 * 1024 * 1024;
    let spec_resolver = Arc::new(raisin_rocksdb::TenantEmbeddingSpecResolver::new(
        storage.db().clone(),
    ));
    let engine = Arc::new(
        HnswIndexingEngine::new(hnsw_path, cache_size, raisin_hnsw::FALLBACK_DIMENSIONS)
            .expect("Failed to create HNSW engine")
            .with_spec_resolver(spec_resolver),
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
