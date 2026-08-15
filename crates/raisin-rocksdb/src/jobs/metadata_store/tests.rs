//! Tests for job metadata store

use super::*;
use chrono::Utc;
use raisin_storage::jobs::{JobContext, JobId, JobStatus, JobType};
use std::collections::HashMap;
use std::sync::Arc;

fn create_test_entry(id: &str) -> PersistedJobEntry {
    PersistedJobEntry {
        id: id.to_string(),
        job_type: JobType::FulltextIndex {
            node_id: "test-node".to_string(),
            operation: raisin_storage::jobs::IndexOperation::AddOrUpdate,
        },
        status: JobStatus::Scheduled,
        tenant: "test-tenant".to_string(),
        started_at: Utc::now(),
        completed_at: None,
        error: None,
        progress: None,
        result: None,
        retry_count: 0,
        max_retries: 3,
        last_heartbeat: None,
        timeout_seconds: 300,
        next_retry_at: None,
        executing_since: None,
    }
}

fn create_test_context() -> JobContext {
    JobContext {
        tenant_id: "test-tenant".to_string(),
        repo_id: "test-repo".to_string(),
        branch: "main".to_string(),
        workspace_id: "test-workspace".to_string(),
        revision: raisin_hlc::HLC::new(42, 0),
        metadata: HashMap::new(),
    }
}

#[test]
fn test_put_and_get() {
    let temp_dir = tempfile::tempdir().unwrap();
    let db = crate::open_db(temp_dir.path()).unwrap();
    let store = JobMetadataStore::new(Arc::new(db));

    let job_id = JobId::new();
    let entry = create_test_entry(&job_id.0);
    let context = create_test_context();

    // Store metadata + context atomically
    store.put_with_context(&job_id, &entry, &context).unwrap();

    // Retrieve metadata
    let retrieved = store.get("test-tenant", &job_id).unwrap();
    assert!(retrieved.is_some());
    let retrieved_entry = retrieved.unwrap();
    assert_eq!(retrieved_entry.id, entry.id);
    assert_eq!(retrieved_entry.retry_count, 0);
    assert_eq!(retrieved_entry.max_retries, 3);
}

#[test]
fn test_list_by_status() {
    let temp_dir = tempfile::tempdir().unwrap();
    let db = crate::open_db(temp_dir.path()).unwrap();
    let store = JobMetadataStore::new(Arc::new(db));

    // Create jobs with different statuses
    let job1_id = JobId::new();
    let mut entry1 = create_test_entry(&job1_id.0);
    entry1.status = JobStatus::Scheduled;

    let job2_id = JobId::new();
    let mut entry2 = create_test_entry(&job2_id.0);
    entry2.status = JobStatus::Running;

    let job3_id = JobId::new();
    let mut entry3 = create_test_entry(&job3_id.0);
    entry3.status = JobStatus::Completed;

    let context = create_test_context();

    store.put_with_context(&job1_id, &entry1, &context).unwrap();
    store.put_with_context(&job2_id, &entry2, &context).unwrap();
    store.put_with_context(&job3_id, &entry3, &context).unwrap();

    // List only pending jobs (Scheduled + Running)
    let pending = store
        .list_by_status(&[JobStatus::Scheduled, JobStatus::Running])
        .unwrap();

    assert_eq!(pending.len(), 2);
}

#[test]
fn test_cleanup_old_jobs() {
    let temp_dir = tempfile::tempdir().unwrap();
    let db = crate::open_db(temp_dir.path()).unwrap();
    let store = JobMetadataStore::new(Arc::new(db));

    // Create an old completed job
    let job_id = JobId::new();
    let mut entry = create_test_entry(&job_id.0);
    entry.status = JobStatus::Completed;
    entry.completed_at = Some(Utc::now() - chrono::Duration::hours(48)); // 2 days ago

    let context = create_test_context();
    store.put_with_context(&job_id, &entry, &context).unwrap();

    // Cleanup jobs older than 24 hours
    let cutoff = Utc::now() - chrono::Duration::hours(24);
    let deleted = store.cleanup_old_jobs(cutoff).unwrap();

    assert_eq!(deleted, 1);

    // Verify job is gone
    let retrieved = store.get("test-tenant", &job_id).unwrap();
    assert!(retrieved.is_none());
}

#[test]
fn test_list_for_tenant_isolation() {
    let temp_dir = tempfile::tempdir().unwrap();
    let db = crate::open_db(temp_dir.path()).unwrap();
    let store = JobMetadataStore::new(Arc::new(db));

    // Create one job in tenant-a and one in tenant-b.
    let mut ctx_a = create_test_context();
    ctx_a.tenant_id = "tenant-a".to_string();
    let mut ctx_b = create_test_context();
    ctx_b.tenant_id = "tenant-b".to_string();

    let job_a = JobId::new();
    let mut entry_a = create_test_entry(&job_a.0);
    entry_a.tenant = "tenant-a".to_string();
    let job_b = JobId::new();
    let mut entry_b = create_test_entry(&job_b.0);
    entry_b.tenant = "tenant-b".to_string();

    store.put_with_context(&job_a, &entry_a, &ctx_a).unwrap();
    store.put_with_context(&job_b, &entry_b, &ctx_b).unwrap();

    // Each tenant only sees its own job.
    let only_a = store.list_for_tenant("tenant-a").unwrap();
    assert_eq!(only_a.len(), 1);
    assert_eq!(only_a[0].1.tenant, "tenant-a");

    let only_b = store.list_for_tenant("tenant-b").unwrap();
    assert_eq!(only_b.len(), 1);
    assert_eq!(only_b[0].1.tenant, "tenant-b");

    // Empty tenant returns nothing.
    let none = store.list_for_tenant("tenant-c").unwrap();
    assert!(none.is_empty());
}

#[test]
fn history_page_uses_newest_first_write_index() {
    let temp_dir = tempfile::tempdir().unwrap();
    let db = crate::open_db(temp_dir.path()).unwrap();
    let store = JobMetadataStore::new(Arc::new(db));
    let context = create_test_context();

    let first_id = JobId::new();
    let mut first = create_test_entry(&first_id.0);
    first.started_at = Utc::now() - chrono::Duration::seconds(10);
    store.put_with_context(&first_id, &first, &context).unwrap();

    let second_id = JobId::new();
    let mut second = create_test_entry(&second_id.0);
    second.started_at = Utc::now();
    store
        .put_with_context(&second_id, &second, &context)
        .unwrap();

    let (page_one, cursor) = store.list_history_page("test-tenant", 1, None).unwrap();
    assert_eq!(page_one[0].0, second_id);
    let (page_two, _) = store
        .list_history_page("test-tenant", 1, cursor.as_deref())
        .unwrap();
    assert_eq!(page_two[0].0, first_id);
}

#[test]
fn history_backfill_is_bounded_and_resumable() {
    let temp_dir = tempfile::tempdir().unwrap();
    let db = crate::open_db(temp_dir.path()).unwrap();
    let store = JobMetadataStore::new(Arc::new(db));
    let context = create_test_context();

    for _ in 0..2 {
        let id = JobId::new();
        let entry = create_test_entry(&id.0);
        store.put_with_context(&id, &entry, &context).unwrap();
    }

    let (first_indexed, cursor) = store
        .backfill_history_index_page("test-tenant", 1, None)
        .unwrap();
    assert_eq!(first_indexed, 1);
    let (second_indexed, next_cursor) = store
        .backfill_history_index_page("test-tenant", 1, cursor.as_deref())
        .unwrap();
    assert_eq!(second_indexed, 1);
    assert!(next_cursor.is_some());
}
