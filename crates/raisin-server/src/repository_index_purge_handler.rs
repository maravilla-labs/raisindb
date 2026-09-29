//! Removes a deleted repository's search index directories.
//!
//! The Tantivy and HNSW indexes live on disk outside RocksDB, so the storage
//! layer's repository purge cannot reach them. The HTTP and WebSocket deletes
//! remove them before answering; this handler covers a delete replicated from a
//! cluster peer, which arrives only as the repository `Deleted` event. Events
//! are handled asynchronously, so a repository recreated under the same id in
//! the meantime is left alone rather than losing its fresh indexes.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use raisin_events::{Event, EventHandler, RepositoryEventKind};
use raisin_hnsw::HnswIndexingEngine;
use raisin_indexer::TantivyIndexingEngine;
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::{RepositoryManagementRepository, Storage};

pub struct RepositoryIndexPurgeHandler {
    storage: Arc<RocksDBStorage>,
    tantivy: Option<Arc<TantivyIndexingEngine>>,
    hnsw: Option<Arc<HnswIndexingEngine>>,
}

impl RepositoryIndexPurgeHandler {
    pub fn new(
        storage: Arc<RocksDBStorage>,
        tantivy: Option<Arc<TantivyIndexingEngine>>,
        hnsw: Option<Arc<HnswIndexingEngine>>,
    ) -> Self {
        Self {
            storage,
            tantivy,
            hnsw,
        }
    }
}

impl EventHandler for RepositoryIndexPurgeHandler {
    fn name(&self) -> &str {
        "repository_index_purge"
    }

    fn handle<'a>(
        &'a self,
        event: &'a Event,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let Event::Repository(repo_event) = event else {
                return Ok(());
            };
            if !matches!(repo_event.kind, RepositoryEventKind::Deleted) {
                return Ok(());
            }
            let (tenant, repo) = (&repo_event.tenant_id, &repo_event.repository_id);
            if self
                .storage
                .repository_management()
                .repository_exists(tenant, repo)
                .await
                .unwrap_or(true)
            {
                return Ok(());
            }
            if let Some(engine) = &self.tantivy {
                if let Err(e) = engine.purge_repository(tenant, repo) {
                    tracing::warn!(%tenant, %repo, error = %e, "could not remove the deleted repository's full-text indexes");
                }
            }
            if let Some(engine) = &self.hnsw {
                if let Err(e) = engine.purge_repository(tenant, repo) {
                    tracing::warn!(%tenant, %repo, error = %e, "could not remove the deleted repository's vector indexes");
                }
            }
            Ok(())
        })
    }
}
