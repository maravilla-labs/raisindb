//! Integration tests for the RocksDB-based full-text job store

use raisin_hlc::HLC;
use raisin_rocksdb::RocksDBStorage;
use raisin_storage::{FullTextIndexJob, FullTextJobStore, JobKind, Storage};
use tempfile::TempDir;

fn create_test_job(job_id: &str, kind: JobKind) -> FullTextIndexJob {
    FullTextIndexJob {
        job_id: job_id.to_string(),
        kind,
        tenant_id: "test-tenant".to_string(),
        repo_id: "test-repo".to_string(),
        workspace_id: "test-workspace".to_string(),
        branch: "main".to_string(),
        revision: HLC::new(1, 0),
        node_id: Some("node-123".to_string()),
        source_branch: None,
        default_language: "en".to_string(),
        supported_languages: vec!["en".to_string()],
        properties_to_index: None,
    }
}

#[test]
fn test_enqueue_and_dequeue() {
    let temp_dir = TempDir::new().unwrap();
    let storage = RocksDBStorage::new(temp_dir.path()).unwrap();
    let job_store = storage.fulltext_job_store();

    // Enqueue some jobs
    let job1 = create_test_job("job-1", JobKind::AddNode);
    let job2 = create_test_job("job-2", JobKind::DeleteNode);
    let job3 = create_test_job("job-3", JobKind::BranchCreated);

    job_store.enqueue(&job1).unwrap();
    job_store.enqueue(&job2).unwrap();
    job_store.enqueue(&job3).unwrap();

    // Dequeue jobs (FIFO order)
    let jobs = job_store.dequeue(2).unwrap();
    assert_eq!(jobs.len(), 2);
    assert_eq!(jobs[0].job_id, "job-1");
    assert_eq!(jobs[1].job_id, "job-2");

    // Dequeue remaining job
    let jobs = job_store.dequeue(10).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].job_id, "job-3");

    // No more jobs
    let jobs = job_store.dequeue(10).unwrap();
    assert_eq!(jobs.len(), 0);
}

#[test]
fn test_complete_jobs() {
    let temp_dir = TempDir::new().unwrap();
    let storage = RocksDBStorage::new(temp_dir.path()).unwrap();
    let job_store = storage.fulltext_job_store();

    // Enqueue and dequeue a job
    let job = create_test_job("job-1", JobKind::AddNode);
    job_store.enqueue(&job).unwrap();

    let jobs = job_store.dequeue(1).unwrap();
    assert_eq!(jobs.len(), 1);

    // Complete the job
    job_store.complete(&["job-1".to_string()]).unwrap();

    // Verify job is gone (no more processing jobs)
    let jobs = job_store.dequeue(10).unwrap();
    assert_eq!(jobs.len(), 0);
}

#[test]
fn test_fail_job() {
    let temp_dir = TempDir::new().unwrap();
    let storage = RocksDBStorage::new(temp_dir.path()).unwrap();
    let job_store = storage.fulltext_job_store();

    // Enqueue and dequeue a job
    let job = create_test_job("job-1", JobKind::AddNode);
    job_store.enqueue(&job).unwrap();

    let jobs = job_store.dequeue(1).unwrap();
    assert_eq!(jobs.len(), 1);

    // Mark job as failed
    job_store.fail("job-1", "Test error").unwrap();

    // Verify job moved to failed state (no more processing jobs)
    let jobs = job_store.dequeue(10).unwrap();
    assert_eq!(jobs.len(), 0);
}

#[test]
fn test_multiple_job_types() {
    let temp_dir = TempDir::new().unwrap();
    let storage = RocksDBStorage::new(temp_dir.path()).unwrap();
    let job_store = storage.fulltext_job_store();

    // Enqueue different job types
    let job1 = create_test_job("job-1", JobKind::AddNode);
    let job2 = create_test_job("job-2", JobKind::DeleteNode);
    let job3 = create_test_job("job-3", JobKind::BranchCreated);

    job_store.enqueue(&job1).unwrap();
    job_store.enqueue(&job2).unwrap();
    job_store.enqueue(&job3).unwrap();

    // Dequeue all jobs
    let jobs = job_store.dequeue(10).unwrap();
    assert_eq!(jobs.len(), 3);

    // Verify job types are preserved
    assert!(matches!(jobs[0].kind, JobKind::AddNode));
    assert!(matches!(jobs[1].kind, JobKind::DeleteNode));
    assert!(matches!(jobs[2].kind, JobKind::BranchCreated));
}

#[test]
fn test_empty_queue_dequeue() {
    let temp_dir = TempDir::new().unwrap();
    let storage = RocksDBStorage::new(temp_dir.path()).unwrap();
    let job_store = storage.fulltext_job_store();

    // Dequeue from empty queue
    let jobs = job_store.dequeue(10).unwrap();
    assert_eq!(jobs.len(), 0);
}

#[test]
fn test_complete_empty_list() {
    let temp_dir = TempDir::new().unwrap();
    let storage = RocksDBStorage::new(temp_dir.path()).unwrap();
    let job_store = storage.fulltext_job_store();

    // Complete with empty list should succeed
    job_store.complete(&[]).unwrap();
}

#[test]
fn test_persistence() {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path().to_path_buf();

    // Create storage and enqueue jobs
    {
        let storage = RocksDBStorage::new(&path).unwrap();
        let job_store = storage.fulltext_job_store();

        let job1 = create_test_job("job-1", JobKind::AddNode);
        let job2 = create_test_job("job-2", JobKind::DeleteNode);

        job_store.enqueue(&job1).unwrap();
        job_store.enqueue(&job2).unwrap();
    }

    // Reopen storage and verify jobs are still there
    {
        let storage = RocksDBStorage::new(&path).unwrap();
        let job_store = storage.fulltext_job_store();

        let jobs = job_store.dequeue(10).unwrap();
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0].job_id, "job-1");
        assert_eq!(jobs[1].job_id, "job-2");
    }
}
