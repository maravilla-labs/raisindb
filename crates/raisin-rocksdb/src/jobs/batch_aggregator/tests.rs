//! Tests for batch aggregator.

use super::core::*;
use crate::jobs::dispatcher::JobDispatcher;
use raisin_hlc::HLC;
use raisin_storage::jobs::{IndexOperation, JobContext, JobRegistry, JobType};
use std::collections::HashMap;
use std::sync::Arc;

fn create_test_context(tenant: &str, repo: &str, branch: &str) -> JobContext {
    JobContext {
        tenant_id: tenant.to_string(),
        repo_id: repo.to_string(),
        branch: branch.to_string(),
        workspace_id: "default".to_string(),
        revision: HLC::new(1, 0),
        metadata: HashMap::new(),
    }
}

fn create_test_dispatcher() -> Arc<JobDispatcher> {
    let (dispatcher, _receivers) = JobDispatcher::new();
    Arc::new(dispatcher)
}

#[tokio::test]
async fn test_queue_single_operation() {
    let job_registry = Arc::new(JobRegistry::new());
    let temp_dir = tempfile::TempDir::new().unwrap();
    let db = Arc::new(crate::open_db(temp_dir.path()).unwrap());
    let job_data_store = Arc::new(crate::jobs::JobDataStore::new(db));
    let dispatcher = create_test_dispatcher();

    let aggregator = BatchIndexAggregator::new(
        BatchAggregatorConfig::default(),
        job_registry.clone(),
        job_data_store,
        dispatcher,
    );

    let context = create_test_context("tenant1", "repo1", "main");
    aggregator
        .queue("node1", IndexOperation::AddOrUpdate, &context)
        .await
        .unwrap();

    let counts = aggregator.pending_counts().await;
    assert_eq!(counts.get("tenant1/repo1/main"), Some(&1));
}

#[tokio::test]
async fn test_auto_flush_on_threshold() {
    let job_registry = Arc::new(JobRegistry::new());
    let temp_dir = tempfile::TempDir::new().unwrap();
    let db = Arc::new(crate::open_db(temp_dir.path()).unwrap());
    let job_data_store = Arc::new(crate::jobs::JobDataStore::new(db));
    let dispatcher = create_test_dispatcher();

    let config = BatchAggregatorConfig {
        max_batch_size: 3, // Small threshold for testing
        ..Default::default()
    };

    let aggregator =
        BatchIndexAggregator::new(config, job_registry.clone(), job_data_store, dispatcher);

    let context = create_test_context("tenant1", "repo1", "main");

    // Queue 2 operations - should not flush
    aggregator
        .queue("node1", IndexOperation::AddOrUpdate, &context)
        .await
        .unwrap();
    aggregator
        .queue("node2", IndexOperation::AddOrUpdate, &context)
        .await
        .unwrap();

    let counts = aggregator.pending_counts().await;
    assert_eq!(counts.get("tenant1/repo1/main"), Some(&2));

    // Queue 3rd operation - should trigger flush
    aggregator
        .queue("node3", IndexOperation::AddOrUpdate, &context)
        .await
        .unwrap();

    let counts = aggregator.pending_counts().await;
    assert_eq!(counts.get("tenant1/repo1/main"), None); // Flushed

    // Verify job was created
    let jobs = job_registry.list_jobs().await;
    assert_eq!(jobs.len(), 1);
    match &jobs[0].job_type {
        JobType::FulltextBatchIndex { operation_count } => {
            assert_eq!(*operation_count, 3);
        }
        _ => panic!("Expected FulltextBatchIndex job"),
    }
}

#[tokio::test]
async fn test_separate_batches_per_index() {
    let job_registry = Arc::new(JobRegistry::new());
    let temp_dir = tempfile::TempDir::new().unwrap();
    let db = Arc::new(crate::open_db(temp_dir.path()).unwrap());
    let job_data_store = Arc::new(crate::jobs::JobDataStore::new(db));
    let dispatcher = create_test_dispatcher();

    let aggregator = BatchIndexAggregator::new(
        BatchAggregatorConfig::default(),
        job_registry.clone(),
        job_data_store,
        dispatcher,
    );

    let context1 = create_test_context("tenant1", "repo1", "main");
    let context2 = create_test_context("tenant1", "repo1", "develop");

    aggregator
        .queue("node1", IndexOperation::AddOrUpdate, &context1)
        .await
        .unwrap();
    aggregator
        .queue("node2", IndexOperation::AddOrUpdate, &context2)
        .await
        .unwrap();

    let counts = aggregator.pending_counts().await;
    assert_eq!(counts.get("tenant1/repo1/main"), Some(&1));
    assert_eq!(counts.get("tenant1/repo1/develop"), Some(&1));
}
