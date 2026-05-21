//! Tests for batch aggregator.

use super::core::*;
use crate::jobs::dispatcher::JobDispatcher;
use raisin_hlc::HLC;
use raisin_storage::jobs::{IndexOperation, JobContext, JobRegistry, JobType};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

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

// ---------------------------------------------------------------------------
// v0.1.29 regression tests for the three bugs fixed in this revision:
// A — aggregator AND-gate (single ops never flushed)
// C — restart durability of pending ops
// ---------------------------------------------------------------------------

/// Bug A regression: with the new idle-flush clause, a single
/// `queue()` on an otherwise idle key MUST land in `flush_expired`'s
/// candidate set after `idle_flush_interval`, even though
/// `min_flush_size` (100) is nowhere near satisfied.
#[tokio::test]
async fn idle_flush_picks_up_single_op() {
    let job_registry = Arc::new(JobRegistry::new());
    let temp_dir = tempfile::TempDir::new().unwrap();
    let db = Arc::new(crate::open_db(temp_dir.path()).unwrap());
    let job_data_store = Arc::new(crate::jobs::JobDataStore::new(db));
    let dispatcher = create_test_dispatcher();

    let config = BatchAggregatorConfig {
        idle_flush_interval: Duration::from_millis(80),
        ..Default::default()
    };

    let aggregator =
        BatchIndexAggregator::new(config, job_registry.clone(), job_data_store, dispatcher);

    let context = create_test_context("tenant1", "repo1", "main");
    aggregator
        .queue("node1", IndexOperation::AddOrUpdate, &context)
        .await
        .unwrap();

    // Pre-idle: nothing should flush yet.
    let jobs_before = aggregator.flush_expired().await.unwrap();
    assert!(
        jobs_before.is_empty(),
        "Idle window has not elapsed; expected no flush, got {} jobs",
        jobs_before.len()
    );

    tokio::time::sleep(Duration::from_millis(150)).await;

    // Post-idle: the single op must now flush.
    let jobs_after = aggregator.flush_expired().await.unwrap();
    assert_eq!(jobs_after.len(), 1, "Expected idle clause to fire exactly once");

    let counts = aggregator.pending_counts().await;
    assert!(counts.is_empty(), "Pending map should be drained after flush");
}

/// Bug A regression (the part that matters for bulk imports):
/// continuous `queue()` activity within `idle_flush_interval` MUST
/// NOT trigger a flush — otherwise the amortization the aggregator
/// exists for would disappear. Idle window resets on every push.
#[tokio::test]
async fn idle_flush_does_not_fire_during_burst() {
    let job_registry = Arc::new(JobRegistry::new());
    let temp_dir = tempfile::TempDir::new().unwrap();
    let db = Arc::new(crate::open_db(temp_dir.path()).unwrap());
    let job_data_store = Arc::new(crate::jobs::JobDataStore::new(db));
    let dispatcher = create_test_dispatcher();

    let config = BatchAggregatorConfig {
        idle_flush_interval: Duration::from_millis(200),
        ..Default::default()
    };

    let aggregator = Arc::new(BatchIndexAggregator::new(
        config,
        job_registry.clone(),
        job_data_store,
        dispatcher,
    ));

    let context = create_test_context("tenant1", "repo1", "main");

    // Push for 500 ms with 50 ms gaps — well under the 200 ms idle
    // window, so the idle clause should never trigger.
    for i in 0..10 {
        aggregator
            .queue(&format!("node{}", i), IndexOperation::AddOrUpdate, &context)
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        let _ = aggregator.flush_expired().await.unwrap();
    }

    let counts = aggregator.pending_counts().await;
    assert_eq!(
        counts.get("tenant1/repo1/main"),
        Some(&10),
        "Burst should accumulate; idle clause must not interrupt"
    );
}

/// Bug C regression: when wired with `with_persistence`, a `queue()`
/// must be recoverable from RocksDB after dropping the in-memory
/// state. A fresh aggregator pointed at the same DB should rebuild
/// the in-memory map via `replay_pending`.
#[tokio::test]
async fn replay_pending_restores_in_memory_state() {
    let job_registry = Arc::new(JobRegistry::new());
    let temp_dir = tempfile::TempDir::new().unwrap();
    let db = Arc::new(crate::open_db(temp_dir.path()).unwrap());
    let job_data_store = Arc::new(crate::jobs::JobDataStore::new(db.clone()));
    let dispatcher = create_test_dispatcher();

    let aggregator_a = Arc::new(
        BatchIndexAggregator::new(
            BatchAggregatorConfig::default(),
            job_registry.clone(),
            job_data_store.clone(),
            dispatcher.clone(),
        )
        .with_persistence(db.clone()),
    );

    let context = create_test_context("tenant1", "repo1", "main");
    aggregator_a
        .queue("node-survivor", IndexOperation::AddOrUpdate, &context)
        .await
        .unwrap();

    // Aggregator B simulates a fresh process pointed at the same DB.
    let aggregator_b = Arc::new(
        BatchIndexAggregator::new(
            BatchAggregatorConfig::default(),
            job_registry,
            job_data_store,
            dispatcher,
        )
        .with_persistence(db),
    );

    let restored = aggregator_b.replay_pending().await.unwrap();
    assert_eq!(restored, 1, "Expected exactly one persisted op replayed");

    let counts = aggregator_b.pending_counts().await;
    assert_eq!(counts.get("tenant1/repo1/main"), Some(&1));
}

/// Bug C regression: after a successful `flush()`, the
/// `pending_batch_ops` CF must be empty — otherwise a restart would
/// replay already-completed ops and waste indexing I/O.
#[tokio::test]
async fn flush_clears_persistent_records() {
    let job_registry = Arc::new(JobRegistry::new());
    let temp_dir = tempfile::TempDir::new().unwrap();
    let db = Arc::new(crate::open_db(temp_dir.path()).unwrap());
    let job_data_store = Arc::new(crate::jobs::JobDataStore::new(db.clone()));
    let dispatcher = create_test_dispatcher();

    let aggregator = Arc::new(
        BatchIndexAggregator::new(
            BatchAggregatorConfig {
                max_batch_size: 2,
                ..Default::default()
            },
            job_registry.clone(),
            job_data_store.clone(),
            dispatcher.clone(),
        )
        .with_persistence(db.clone()),
    );

    let context = create_test_context("tenant1", "repo1", "main");
    aggregator
        .queue("a", IndexOperation::AddOrUpdate, &context)
        .await
        .unwrap();
    aggregator
        .queue("b", IndexOperation::AddOrUpdate, &context)
        .await
        .unwrap();
    // Threshold hit; queue() called flush() inline.

    // A second aggregator should observe no leftover persisted ops.
    let aggregator_b = Arc::new(
        BatchIndexAggregator::new(
            BatchAggregatorConfig::default(),
            job_registry,
            job_data_store,
            dispatcher,
        )
        .with_persistence(db),
    );
    let restored = aggregator_b.replay_pending().await.unwrap();
    assert_eq!(
        restored, 0,
        "Flushed ops must not remain in the pending CF (would cause duplicate replay on restart)"
    );
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
