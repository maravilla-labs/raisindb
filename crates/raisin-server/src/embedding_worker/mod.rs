//! Background worker for processing embedding generation jobs.
//!
//! This worker follows the same pattern as IndexerWorker,
//! continuously polling for jobs and processing them asynchronously.

mod config;
mod helpers;
mod job_handlers;

pub use config::WorkerConfig;

use raisin_embeddings::models::{EmbeddingJob, EmbeddingJobKind};
use raisin_embeddings::{EmbeddingJobStore, EmbeddingStorage};
use raisin_error::Result;
use raisin_hnsw::HnswIndexingEngine;
use raisin_rocksdb::RocksDBStorage;
use std::sync::Arc;
use tokio::sync::watch;
use tokio::task::JoinHandle;

/// Background worker that processes embedding generation jobs
///
/// Uses RocksDBStorage concretely (not generic) because we need access to
/// tenant_ai_config_repository() which is specific to RocksDB.
/// This is consistent with multi-tenant architecture where each job has a tenant_id
/// and we look up that tenant's configuration when processing.
pub struct EmbeddingWorker<E, J>
where
    E: EmbeddingStorage,
    J: EmbeddingJobStore,
{
    storage: Arc<RocksDBStorage>,
    embedding_storage: Arc<E>,
    job_store: Arc<J>,
    hnsw_engine: Arc<HnswIndexingEngine>,
    master_key: [u8; 32],
    config: WorkerConfig,
    shutdown_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
}

impl<E, J> EmbeddingWorker<E, J>
where
    E: EmbeddingStorage + 'static,
    J: EmbeddingJobStore + 'static,
{
    pub fn new(
        storage: Arc<RocksDBStorage>,
        embedding_storage: Arc<E>,
        job_store: Arc<J>,
        hnsw_engine: Arc<HnswIndexingEngine>,
        master_key: [u8; 32],
        config: WorkerConfig,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            storage,
            embedding_storage,
            job_store,
            hnsw_engine,
            master_key,
            config,
            shutdown_tx,
            shutdown_rx,
        }
    }

    pub fn start(&self) -> JoinHandle<Result<()>> {
        let storage = Arc::clone(&self.storage);
        let embedding_storage = Arc::clone(&self.embedding_storage);
        let job_store = Arc::clone(&self.job_store);
        let hnsw_engine = Arc::clone(&self.hnsw_engine);
        let master_key = self.master_key;
        let config = self.config.clone();
        let mut shutdown_rx = self.shutdown_rx.clone();

        tokio::spawn(async move {
            Self::run_loop(
                storage,
                embedding_storage,
                job_store,
                hnsw_engine,
                master_key,
                config,
                &mut shutdown_rx,
            )
            .await
        })
    }

    pub async fn stop(&self) {
        let _ = self.shutdown_tx.send(true);
    }

    async fn run_loop(
        storage: Arc<RocksDBStorage>,
        embedding_storage: Arc<E>,
        job_store: Arc<J>,
        hnsw_engine: Arc<HnswIndexingEngine>,
        master_key: [u8; 32],
        config: WorkerConfig,
        shutdown_rx: &mut watch::Receiver<bool>,
    ) -> Result<()> {
        tracing::info!(
            batch_size = config.batch_size,
            poll_interval_ms = config.poll_interval.as_millis(),
            "Embedding worker started"
        );

        loop {
            if *shutdown_rx.borrow() {
                tracing::info!("Embedding worker shutting down");
                break;
            }

            let jobs = match job_store.dequeue(config.batch_size) {
                Ok(jobs) => jobs,
                Err(e) => {
                    tracing::error!(error = %e, "Failed to dequeue embedding jobs");
                    tokio::time::sleep(config.poll_interval).await;
                    continue;
                }
            };

            if jobs.is_empty() {
                tokio::time::sleep(config.poll_interval).await;
                continue;
            }

            tracing::debug!(count = jobs.len(), "Processing embedding jobs");

            for job in jobs {
                if *shutdown_rx.borrow() {
                    tracing::info!("Shutdown requested, stopping job processing");
                    break;
                }

                Self::process_job(
                    &storage,
                    &embedding_storage,
                    &job_store,
                    &hnsw_engine,
                    master_key,
                    job,
                )
                .await;
            }
        }

        tracing::info!("Embedding worker stopped");
        Ok(())
    }

    async fn process_job(
        storage: &Arc<RocksDBStorage>,
        embedding_storage: &Arc<E>,
        job_store: &Arc<J>,
        hnsw_engine: &Arc<HnswIndexingEngine>,
        master_key: [u8; 32],
        job: EmbeddingJob,
    ) {
        let job_id = job.job_id.clone();

        tracing::debug!(
            job_id = %job_id,
            kind = ?job.kind,
            tenant_id = %job.tenant_id,
            repo_id = %job.repo_id,
            "Processing embedding job"
        );

        let result = match job.kind {
            EmbeddingJobKind::AddNode => {
                job_handlers::handle_add_node(
                    storage,
                    embedding_storage,
                    hnsw_engine,
                    master_key,
                    &job,
                )
                .await
            }
            EmbeddingJobKind::DeleteNode => {
                job_handlers::handle_delete_node(hnsw_engine, &job).await
            }
            EmbeddingJobKind::BranchCreated => {
                job_handlers::handle_branch_created(hnsw_engine, &job).await
            }
        };

        match result {
            Ok(()) => {
                if let Err(e) = job_store.complete(&[job_id.clone()]) {
                    tracing::error!(job_id = %job_id, error = %e, "Failed to mark job as complete");
                } else {
                    tracing::debug!(job_id = %job_id, "Completed embedding job");
                }
            }
            Err(e) => {
                tracing::error!(job_id = %job_id, error = %e, "Embedding job failed");
                if let Err(mark_err) = job_store.fail(&job_id, &e.to_string()) {
                    tracing::error!(
                        job_id = %job_id,
                        error = %mark_err,
                        "Failed to mark job as failed"
                    );
                }
            }
        }
    }
}
