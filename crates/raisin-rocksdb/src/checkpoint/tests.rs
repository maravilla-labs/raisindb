//! Tests for checkpoint management.

use super::*;
use rocksdb::DB;
use std::sync::Arc;
use tempfile::TempDir;

#[tokio::test]
async fn test_checkpoint_creation() {
    use rocksdb::Options;

    let temp_db = TempDir::new().unwrap();
    let temp_checkpoints = TempDir::new().unwrap();

    let mut opts = Options::default();
    opts.create_if_missing(true);

    let db = Arc::new(DB::open(&opts, temp_db.path()).unwrap());

    let manager = CheckpointManager::new(db.clone(), temp_checkpoints.path().to_path_buf());

    let metadata = manager.create_checkpoint("test-snapshot").await.unwrap();

    assert_eq!(metadata.snapshot_id, "test-snapshot");
    assert!(metadata.checkpoint_path.exists());
}

#[tokio::test]
async fn test_checkpoint_cleanup() {
    use rocksdb::Options;

    let temp_db = TempDir::new().unwrap();
    let temp_checkpoints = TempDir::new().unwrap();

    let mut opts = Options::default();
    opts.create_if_missing(true);

    let db = Arc::new(DB::open(&opts, temp_db.path()).unwrap());

    let manager = CheckpointManager::new(db.clone(), temp_checkpoints.path().to_path_buf());

    // Create 3 checkpoints
    manager.create_checkpoint("snap1").await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    manager.create_checkpoint("snap2").await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    manager.create_checkpoint("snap3").await.unwrap();

    // Keep latest 2
    let removed = manager.cleanup_old_checkpoints(2).await.unwrap();
    assert_eq!(removed, 1);
}
