// SPDX-License-Identifier: BSL-1.1

//! Background worker for processing full-text indexing jobs

use raisin_error::Result;
use raisin_storage::{
    FullTextIndexJob, FullTextJobStore, IndexingEngine, JobKind, NodeRepository, Storage,
    StorageScope,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tokio::task::JoinHandle;

/// Configuration for the indexer worker
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    /// Number of jobs to dequeue per iteration
    pub batch_size: usize,
    /// Time to wait between polling for jobs
    pub poll_interval: Duration,
    /// Maximum number of retry attempts for failed jobs
    pub max_retries: usize,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            batch_size: 10,
            poll_interval: Duration::from_secs(1),
            max_retries: 3,
        }
    }
}

/// Background worker that processes full-text indexing jobs
///
/// The worker continuously polls the job store, fetches pending jobs,
/// and executes them using the provided indexing engine.
///
/// # Type Parameters
///
/// * `S` - Storage implementation (provides job store and node access)
/// * `E` - Indexing engine implementation (typically TantivyIndexingEngine)
///
/// # Example
///
/// ```no_run
/// use raisin_indexer::worker::{IndexerWorker, WorkerConfig};
/// use raisin_indexer::{IndexCacheConfig, TantivyIndexingEngine};
/// use raisin_storage::Storage;
/// use std::sync::Arc;
/// use std::path::PathBuf;
///
/// # async fn example(storage: Arc<impl Storage + 'static>) -> raisin_error::Result<()> {
/// let cache_config = IndexCacheConfig::development();
/// let engine = Arc::new(TantivyIndexingEngine::new(
///     PathBuf::from("/data/indexes"),
///     cache_config.fulltext_cache_size
/// )?);
/// let worker = IndexerWorker::new(storage, engine, WorkerConfig::default());
///
/// let handle = worker.start();
///
/// // Later: stop the worker
/// worker.stop().await;
/// handle.await??;
/// # Ok(())
/// # }
/// ```
pub struct IndexerWorker<S, E>
where
    S: Storage,
    E: IndexingEngine,
{
    storage: Arc<S>,
    engine: Arc<E>,
    config: WorkerConfig,
    shutdown_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
}

impl<S, E> IndexerWorker<S, E>
where
    S: Storage + 'static,
    E: IndexingEngine + 'static,
{
    /// Creates a new IndexerWorker
    ///
    /// # Arguments
    ///
    /// * `storage` - Storage implementation for accessing nodes and job store
    /// * `engine` - Indexing engine implementation for executing index operations
    /// * `config` - Worker configuration (batch size, poll interval, retries)
    pub fn new(storage: Arc<S>, engine: Arc<E>, config: WorkerConfig) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        Self {
            storage,
            engine,
            config,
            shutdown_tx,
            shutdown_rx,
        }
    }

    /// Starts the worker in a background task
    ///
    /// Returns a JoinHandle that completes when the worker stops.
    /// The worker will continue processing jobs until `stop()` is called.
    ///
    /// # Returns
    ///
    /// A `JoinHandle` that resolves to `Result<()>` when the worker terminates
    pub fn start(&self) -> JoinHandle<Result<()>> {
        let storage = Arc::clone(&self.storage);
        let engine = Arc::clone(&self.engine);
        let config = self.config.clone();
        let mut shutdown_rx = self.shutdown_rx.clone();

        tokio::spawn(async move { Self::run_loop(storage, engine, config, &mut shutdown_rx).await })
    }

    /// Signals the worker to stop gracefully
    ///
    /// The worker will complete its current job batch and then terminate.
    /// Use the JoinHandle returned by `start()` to await termination.
    pub async fn stop(&self) {
        let _ = self.shutdown_tx.send(true);
    }

    /// Main worker loop
    ///
    /// Continuously polls for jobs, processes them, and handles shutdown signals.
    async fn run_loop(
        storage: Arc<S>,
        engine: Arc<E>,
        config: WorkerConfig,
        shutdown_rx: &mut watch::Receiver<bool>,
    ) -> Result<()> {
        tracing::info!(
            batch_size = config.batch_size,
            poll_interval_ms = config.poll_interval.as_millis(),
            "Full-text indexer worker started"
        );

        loop {
            // Check for shutdown signal
            if *shutdown_rx.borrow() {
                tracing::info!("Full-text indexer worker shutting down");
                break;
            }

            // Dequeue jobs
            let jobs = match storage.fulltext_job_store().dequeue(config.batch_size) {
                Ok(jobs) => jobs,
                Err(e) => {
                    tracing::error!(error = %e, "Failed to dequeue jobs");
                    tokio::time::sleep(config.poll_interval).await;
                    continue;
                }
            };

            if jobs.is_empty() {
                // No jobs available, wait before polling again
                tokio::time::sleep(config.poll_interval).await;
                continue;
            }

            tracing::debug!(count = jobs.len(), "Processing indexing jobs");

            // Process each job
            for job in jobs {
                if *shutdown_rx.borrow() {
                    tracing::info!("Shutdown requested, stopping job processing");
                    break;
                }

                Self::process_job(&storage, &engine, job).await;
            }
        }

        tracing::info!("Full-text indexer worker stopped");
        Ok(())
    }

    /// Processes a single indexing job
    ///
    /// Executes the appropriate handler based on job kind and marks the job
    /// as complete or failed in the job store.
    async fn process_job(storage: &Arc<S>, engine: &Arc<E>, job: FullTextIndexJob) {
        let job_id = job.job_id.clone();

        tracing::debug!(
            job_id = %job_id,
            kind = ?job.kind,
            tenant_id = %job.tenant_id,
            repo_id = %job.repo_id,
            branch = %job.branch,
            workspace_id = %job.workspace_id,
            revision = %job.revision,
            "Processing job"
        );

        let result = match job.kind {
            JobKind::AddNode => Self::handle_add_node(storage, engine, &job).await,
            JobKind::DeleteNode => Self::handle_delete_node(engine, &job).await,
            JobKind::BranchCreated => Self::handle_branch_created(engine, &job).await,
        };

        match result {
            Ok(()) => {
                if let Err(e) = storage
                    .fulltext_job_store()
                    .complete(std::slice::from_ref(&job_id))
                {
                    tracing::error!(job_id = %job_id, error = %e, "Failed to mark job as complete");
                } else {
                    tracing::debug!(job_id = %job_id, "Completed job");
                }
            }
            Err(e) => {
                tracing::error!(job_id = %job_id, error = %e, "Job failed");
                if let Err(mark_err) = storage.fulltext_job_store().fail(&job_id, &e.to_string()) {
                    tracing::error!(
                        job_id = %job_id,
                        error = %mark_err,
                        "Failed to mark job as failed"
                    );
                }
            }
        }
    }

    /// Handles AddNode job: fetches node from storage and indexes it
    ///
    /// Retrieves the node at the exact revision specified in the job and
    /// passes it to the indexing engine for indexing.
    async fn handle_add_node(
        storage: &Arc<S>,
        engine: &Arc<E>,
        job: &FullTextIndexJob,
    ) -> Result<()> {
        let node_id = job.node_id.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation("node_id is required for AddNode operation".to_string())
        })?;

        tracing::trace!(
            node_id = %node_id,
            revision = %job.revision,
            "Fetching node for indexing"
        );

        // Fetch node from storage at exact revision
        let scope = StorageScope::new(&job.tenant_id, &job.repo_id, &job.branch, &job.workspace_id);
        let node = storage
            .nodes()
            .get(scope, node_id, Some(&job.revision))
            .await?
            .ok_or_else(|| {
                raisin_error::Error::NotFound(format!(
                    "Node {} not found at revision {}",
                    node_id, job.revision
                ))
            })?;

        // Index the node (Tantivy is sync, so use spawn_blocking)
        let engine = Arc::clone(engine);
        let job = job.clone();
        let node_clone = node.clone();

        tokio::task::spawn_blocking(move || engine.do_index_node(&job, &node_clone))
            .await
            .map_err(|e| raisin_error::Error::storage(format!("Blocking task failed: {}", e)))??;

        tracing::trace!(node_id = %node_id, "Node indexed successfully");
        Ok(())
    }

    /// Handles DeleteNode job: removes node from index
    ///
    /// Removes the specified node from the full-text index without needing
    /// to fetch it from storage.
    async fn handle_delete_node(engine: &Arc<E>, job: &FullTextIndexJob) -> Result<()> {
        let node_id = job.node_id.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation(
                "node_id is required for DeleteNode operation".to_string(),
            )
        })?;

        tracing::trace!(node_id = %node_id, "Deleting node from index");

        let engine = Arc::clone(engine);
        let job = job.clone();

        tokio::task::spawn_blocking(move || engine.do_delete_node(&job))
            .await
            .map_err(|e| raisin_error::Error::storage(format!("Blocking task failed: {}", e)))??;

        tracing::trace!(node_id = %node_id, "Node deleted from index");
        Ok(())
    }

    /// Handles BranchCreated job: copies index from source branch
    ///
    /// When a new branch is created, this copies the full-text index from
    /// the source branch to ensure the new branch has searchable content.
    async fn handle_branch_created(engine: &Arc<E>, job: &FullTextIndexJob) -> Result<()> {
        let source_branch = job.source_branch.as_ref().ok_or_else(|| {
            raisin_error::Error::Validation(
                "source_branch is required for BranchCreated operation".to_string(),
            )
        })?;

        tracing::trace!(
            branch = %job.branch,
            source_branch = %source_branch,
            "Copying index for new branch"
        );

        let engine = Arc::clone(engine);
        let job_clone = job.clone();
        let branch = job.branch.clone();
        let source_branch_clone = source_branch.clone();

        tokio::task::spawn_blocking(move || engine.do_branch_created(&job_clone))
            .await
            .map_err(|e| raisin_error::Error::storage(format!("Blocking task failed: {}", e)))??;

        tracing::trace!(
            branch = %branch,
            source_branch = %source_branch_clone,
            "Index copied for new branch"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_worker_config_default() {
        let config = WorkerConfig::default();
        assert_eq!(config.batch_size, 10);
        assert_eq!(config.poll_interval, Duration::from_secs(1));
        assert_eq!(config.max_retries, 3);
    }

    #[test]
    fn test_worker_config_custom() {
        let config = WorkerConfig {
            batch_size: 50,
            poll_interval: Duration::from_millis(500),
            max_retries: 5,
        };
        assert_eq!(config.batch_size, 50);
        assert_eq!(config.poll_interval, Duration::from_millis(500));
        assert_eq!(config.max_retries, 5);
    }
}
