//! Unified event handler for enqueuing indexing and embedding jobs
//!
//! This handler replaces both FullTextEventHandler and EmbeddingEventHandler
//! by enqueuing jobs to the unified JobRegistry with JobContext stored in JobDataStore.
//!
//! # Batch Aggregation
//!
//! When a `BatchIndexAggregator` is configured, fulltext indexing operations are
//! automatically batched for improved bulk import performance. Operations are
//! collected and flushed as batch jobs when thresholds are reached.

mod asset_processing;
mod delete_and_schema_handlers;
mod event_dispatch;
mod index_helpers;
mod job_helpers;
mod node_handlers;
mod replication_handlers;
mod repo_handlers;
mod trigger_helpers;

#[cfg(test)]
mod tests;

use crate::jobs::{
    dispatcher::JobDispatcher, trigger_registry::TriggerRegistry, BatchIndexAggregator,
    JobDataStore,
};
use crate::repositories::ProcessingRulesRepositoryImpl;
use crate::RocksDBStorage;
use raisin_storage::jobs::JobRegistry;
use std::sync::Arc;

/// Unified event handler that enqueues both fulltext and embedding jobs
///
/// This handler listens to:
/// - Node lifecycle events (create, update, delete)
/// - Repository events (branch creation)
///   and creates corresponding jobs in the unified job system.
///
/// # Batch Aggregation
///
/// When configured with a `BatchIndexAggregator`, fulltext indexing operations
/// are automatically batched for improved bulk import performance. This can
/// reduce indexing time from hours to minutes for large imports.
pub struct UnifiedJobEventHandler {
    storage: Arc<RocksDBStorage>,
    job_registry: Arc<JobRegistry>,
    job_data_store: Arc<JobDataStore>,
    /// Job dispatcher for routing jobs to worker queues
    dispatcher: Arc<JobDispatcher>,
    /// Optional batch aggregator for fulltext indexing
    /// When set, fulltext operations are batched for improved bulk import performance
    batch_aggregator: Option<Arc<BatchIndexAggregator>>,
    /// Cached trigger registry for quick-reject optimization
    trigger_registry: Option<Arc<TriggerRegistry<RocksDBStorage>>>,
    /// Processing rules repository for per-repo asset processing configuration
    processing_rules: ProcessingRulesRepositoryImpl,
}

impl UnifiedJobEventHandler {
    /// Creates a new UnifiedJobEventHandler without batch aggregation
    ///
    /// # Arguments
    ///
    /// * `storage` - RocksDB storage instance
    /// * `job_registry` - Job registry for tracking jobs
    /// * `job_data_store` - Data store for job contexts
    /// * `dispatcher` - Dispatcher for routing jobs to worker queues
    /// * `processing_rules` - Repository for per-repo processing rules
    pub fn new(
        storage: Arc<RocksDBStorage>,
        job_registry: Arc<JobRegistry>,
        job_data_store: Arc<JobDataStore>,
        dispatcher: Arc<JobDispatcher>,
        processing_rules: ProcessingRulesRepositoryImpl,
    ) -> Self {
        Self {
            storage,
            job_registry,
            job_data_store,
            dispatcher,
            batch_aggregator: None,
            trigger_registry: None,
            processing_rules,
        }
    }

    /// Enable batch aggregation for fulltext indexing
    ///
    /// When enabled, fulltext indexing operations are collected and flushed
    /// as batch jobs when thresholds are reached (count or time-based).
    /// This dramatically improves bulk import performance.
    ///
    /// # Arguments
    ///
    /// * `aggregator` - The batch aggregator to use for fulltext operations
    pub fn with_batch_aggregator(mut self, aggregator: Arc<BatchIndexAggregator>) -> Self {
        self.batch_aggregator = Some(aggregator);
        self
    }

    /// Enable trigger registry for quick-reject optimization
    ///
    /// When enabled, the handler uses a cached trigger registry to quickly
    /// determine if a node event could match any triggers before enqueuing
    /// TriggerEvaluation jobs. This can significantly reduce job queue pressure.
    ///
    /// # Arguments
    ///
    /// * `registry` - The trigger registry to use for quick-reject checks
    pub fn with_trigger_registry(mut self, registry: Arc<TriggerRegistry<RocksDBStorage>>) -> Self {
        self.trigger_registry = Some(registry);
        self
    }
}
