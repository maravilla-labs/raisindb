//! Tests for Tantivy index transfer.

use super::*;
use tempfile::TempDir;

#[tokio::test]
async fn test_collect_empty_index() {
    let temp_dir = TempDir::new().unwrap();
    let manager = TantivyIndexManager::new(temp_dir.path().to_path_buf());

    let metadata = manager
        .collect_index_metadata("tenant1", "repo1", "main")
        .await
        .unwrap();

    assert_eq!(metadata.files.len(), 0);
    assert_eq!(metadata.total_size_bytes, 0);
}

#[tokio::test]
async fn test_list_all_indexes() {
    let temp_dir = TempDir::new().unwrap();

    // Create some dummy index directories
    let index_dir = temp_dir.path().join("tenant1").join("repo1").join("main");
    tokio::fs::create_dir_all(&index_dir).await.unwrap();

    let manager = TantivyIndexManager::new(temp_dir.path().to_path_buf());
    let indexes = manager.list_all_indexes().await.unwrap();

    assert_eq!(indexes.len(), 1);
    assert_eq!(
        indexes[0],
        (
            "tenant1".to_string(),
            "repo1".to_string(),
            "main".to_string()
        )
    );
}

#[tokio::test]
async fn test_prepare_and_abort_receive() {
    let temp_base = TempDir::new().unwrap();
    let temp_staging = TempDir::new().unwrap();

    let receiver = TantivyIndexReceiver::new(
        temp_base.path().to_path_buf(),
        temp_staging.path().to_path_buf(),
    );

    let staging_path = receiver
        .prepare_receive("tenant1", "repo1", "main")
        .await
        .unwrap();

    assert!(staging_path.exists());

    receiver
        .abort_receive("tenant1", "repo1", "main")
        .await
        .unwrap();

    assert!(!staging_path.exists());
}
