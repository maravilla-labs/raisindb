// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Integration tests for one-shot scheduled invocations
//! (`JobType::ScheduledInvocation`).
//!
//! Covers the full lifecycle around the job system:
//! - a past `run_at` dispatches immediately and the handler enqueues the
//!   target function execution,
//! - a future `run_at` is persisted at registration and survives a
//!   restart (restore path re-registers it with its schedule intact),
//! - a cancellation before the fire time sticks across restarts.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use raisin_rocksdb::{
    DispatchingMonitor, JobDataStore, RocksDBStorage, RocksDBWorkerPool,
    ScheduledInvocationHandler, META_ACTOR, META_EXTERNAL_KEY, META_INPUT, META_SCHEDULED_FOR,
    META_TARGET_PATH,
};
use raisin_storage::jobs::{JobCategory, JobContext, JobId, JobStatus, JobType};

const TENANT: &str = "default";

fn invocation_context(
    target_path: &str,
    input: serde_json::Value,
    external_key: Option<&str>,
) -> JobContext {
    let mut metadata = HashMap::new();
    metadata.insert(META_TARGET_PATH.to_string(), serde_json::json!(target_path));
    metadata.insert(META_INPUT.to_string(), input);
    metadata.insert(META_ACTOR.to_string(), serde_json::json!("tester"));
    if let Some(key) = external_key {
        metadata.insert(META_EXTERNAL_KEY.to_string(), serde_json::json!(key));
    }
    metadata.insert(
        META_SCHEDULED_FOR.to_string(),
        serde_json::json!(chrono::Utc::now().to_rfc3339()),
    );
    JobContext {
        tenant_id: TENANT.to_string(),
        repo_id: "test-repo".to_string(),
        branch: "main".to_string(),
        workspace_id: "functions".to_string(),
        revision: raisin_hlc::HLC::new(0, 0),
        metadata,
    }
}

fn invocation_job_type(target_kind: &str) -> (String, JobType) {
    let invocation_id = nanoid::nanoid!();
    let job_type = JobType::ScheduledInvocation {
        invocation_id: invocation_id.clone(),
        target_kind: target_kind.to_string(),
    };
    (invocation_id, job_type)
}

/// A `run_at` in the past dispatches immediately (via the dispatching
/// monitor) and the handler routes it to a `FunctionExecution` job carrying
/// the persisted input.
#[tokio::test]
async fn past_schedule_fires_immediately_and_executes_function_target() {
    let temp_dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(RocksDBStorage::new(temp_dir.path()).unwrap());
    let registry = storage.job_registry().clone();
    let data_store = storage.job_data_store().clone();

    // Wire the auto-dispatch monitor with receivers we can observe,
    // exactly as the job-system init does.
    let (dispatcher, receivers) = RocksDBWorkerPool::create_dispatcher();
    registry
        .monitors()
        .add_monitor(Arc::new(DispatchingMonitor::new(dispatcher.clone())))
        .await;

    // Schedule an invocation 1s in the past: must dispatch immediately.
    let (invocation_id, job_type) = invocation_job_type("function");
    let context = invocation_context("/lib/hello", serde_json::json!({"n": 42}), None);
    let job_id = JobId::new();
    data_store.put(&job_id, &context).unwrap();
    registry
        .register_job_at_with_id(
            job_id.clone(),
            job_type,
            TENANT.to_string(),
            chrono::Utc::now() - chrono::Duration::seconds(1),
            Some(0),
        )
        .await
        .unwrap();

    // The invocation job must arrive on the Realtime queue promptly.
    let receiver = receivers.get(&JobCategory::Realtime).unwrap();
    let dispatched = tokio::time::timeout(Duration::from_secs(5), receiver.recv())
        .await
        .expect("invocation job must be dispatched immediately")
        .expect("dispatcher channel open");
    assert_eq!(dispatched, job_id);

    // Run the handler for the dispatched job (what a pool worker would do).
    let handler = ScheduledInvocationHandler::new(registry.clone(), data_store.clone(), dispatcher);
    let info = registry.get_job_info(&job_id).await.unwrap();
    let ctx = data_store.get(TENANT, &job_id).unwrap().unwrap();
    let result = handler.handle(&info, &ctx).await.unwrap().unwrap();
    assert_eq!(result["target_kind"], "function");
    assert_eq!(result["status"], "queued");

    // The target function execution is enqueued with the persisted input.
    let jobs = registry.list_jobs_by_tenant(TENANT).await;
    let exec_job = jobs
        .iter()
        .find(|j| matches!(j.job_type, JobType::FunctionExecution { .. }))
        .expect("FunctionExecution job registered");
    match &exec_job.job_type {
        JobType::FunctionExecution {
            function_path,
            trigger_name,
            ..
        } => {
            assert_eq!(function_path, "/lib/hello");
            assert_eq!(trigger_name.as_deref(), Some("scheduler"));
        }
        _ => unreachable!(),
    }
    let exec_ctx = data_store.get(TENANT, &exec_job.id).unwrap().unwrap();
    assert_eq!(
        exec_ctx.metadata.get("input"),
        Some(&serde_json::json!({"n": 42}))
    );
    assert_eq!(
        exec_ctx.metadata.get("invocation_id"),
        Some(&serde_json::json!(invocation_id))
    );
}

/// A future `run_at` is persisted at registration time and restored after a
/// restart with its schedule, variant fields, and context intact.
#[tokio::test]
async fn future_schedule_survives_restart_and_restore() {
    let temp_dir = tempfile::tempdir().unwrap();
    let run_at = chrono::Utc::now() + chrono::Duration::hours(1);
    let job_id = JobId::new();
    let (invocation_id, job_type) = invocation_job_type("function");

    // "First server run": schedule and stop.
    {
        let storage = RocksDBStorage::new(temp_dir.path()).unwrap();
        let context = invocation_context(
            "/lib/nightly-report",
            serde_json::json!({"day": "2026-07-12"}),
            Some("nightly-2026-07-12"),
        );
        storage.job_data_store().put(&job_id, &context).unwrap();
        storage
            .job_registry()
            .register_job_at_with_id(
                job_id.clone(),
                job_type.clone(),
                TENANT.to_string(),
                run_at,
                Some(0),
            )
            .await
            .unwrap();

        // The delayed job must already be in the persistent metadata store
        // (persist-at-registration), or a restart would silently drop it.
        let persisted = storage
            .job_metadata_store()
            .get(TENANT, &job_id)
            .unwrap()
            .expect("delayed job persisted at registration");
        assert_eq!(persisted.status, JobStatus::Scheduled);
        assert_eq!(persisted.next_retry_at, Some(run_at));
        assert_eq!(persisted.max_retries, 0);
    }

    // "Restart": reopen the same data dir and restore pending jobs.
    let storage = RocksDBStorage::new(temp_dir.path()).unwrap();
    let stats = storage.restore_pending_jobs().await.unwrap();
    assert!(stats.restored >= 1, "invocation restored: {:?}", stats);

    let info = storage.job_registry().get_job_info(&job_id).await.unwrap();
    assert!(matches!(info.status, JobStatus::Scheduled));
    assert_eq!(
        info.next_retry_at,
        Some(run_at),
        "future fire time honored after restart"
    );
    assert_eq!(info.max_retries, 0);
    // The JobType round-trips through its display string form.
    match &info.job_type {
        JobType::ScheduledInvocation {
            invocation_id: restored_id,
            target_kind,
        } => {
            assert_eq!(restored_id, &invocation_id);
            assert_eq!(target_kind, "function");
        }
        other => panic!("expected ScheduledInvocation, got {:?}", other),
    }

    // The invocation payload survives alongside the job.
    let ctx = storage
        .job_data_store()
        .get(TENANT, &job_id)
        .unwrap()
        .expect("job context restored");
    assert_eq!(
        ctx.metadata.get(META_TARGET_PATH),
        Some(&serde_json::json!("/lib/nightly-report"))
    );
    assert_eq!(
        ctx.metadata.get(META_INPUT),
        Some(&serde_json::json!({"day": "2026-07-12"}))
    );
    assert_eq!(
        ctx.metadata.get(META_EXTERNAL_KEY),
        Some(&serde_json::json!("nightly-2026-07-12"))
    );
}

/// Cancelling a pending invocation persists the cancellation: after a
/// restart the job is NOT restored to the pending set, so it never fires.
#[tokio::test]
async fn cancel_before_fire_sticks_across_restart() {
    let temp_dir = tempfile::tempdir().unwrap();
    let job_id = JobId::new();
    let (_invocation_id, job_type) = invocation_job_type("function");

    {
        let storage = RocksDBStorage::new(temp_dir.path()).unwrap();
        let context = invocation_context("/lib/never-runs", serde_json::json!({}), None);
        storage.job_data_store().put(&job_id, &context).unwrap();
        storage
            .job_registry()
            .register_job_at_with_id(
                job_id.clone(),
                job_type,
                TENANT.to_string(),
                chrono::Utc::now() + chrono::Duration::hours(1),
                Some(0),
            )
            .await
            .unwrap();

        storage.job_registry().cancel_job(&job_id).await.unwrap();
        let info = storage.job_registry().get_job_info(&job_id).await.unwrap();
        assert!(matches!(info.status, JobStatus::Cancelled));

        // Cancellation reached the persistent store.
        let persisted = storage
            .job_metadata_store()
            .get(TENANT, &job_id)
            .unwrap()
            .expect("cancelled job still persisted");
        assert_eq!(persisted.status, JobStatus::Cancelled);
    }

    // After a restart the cancelled invocation is not restored as pending.
    let storage = RocksDBStorage::new(temp_dir.path()).unwrap();
    storage.restore_pending_jobs().await.unwrap();
    assert!(
        storage.job_registry().get_job_info(&job_id).await.is_err(),
        "cancelled invocation must not be restored to the pending set"
    );
}

/// The `JobDataStore` context written by the transport layers uses the same
/// tenant-scoped key the restore path reads (guards against key drift).
#[tokio::test]
async fn job_context_round_trips_through_data_store() {
    let temp_dir = tempfile::tempdir().unwrap();
    let db = Arc::new(raisin_rocksdb::open_db(temp_dir.path()).unwrap());
    let data_store = JobDataStore::new(db);

    let job_id = JobId::new();
    let context = invocation_context("/flows/nightly", serde_json::json!({"x": 1}), Some("k-1"));
    data_store.put(&job_id, &context).unwrap();

    let loaded = data_store.get(TENANT, &job_id).unwrap().unwrap();
    assert_eq!(loaded.repo_id, "test-repo");
    assert_eq!(
        loaded.metadata.get(META_TARGET_PATH),
        Some(&serde_json::json!("/flows/nightly"))
    );
    assert_eq!(
        loaded.metadata.get(META_EXTERNAL_KEY),
        Some(&serde_json::json!("k-1"))
    );
}
