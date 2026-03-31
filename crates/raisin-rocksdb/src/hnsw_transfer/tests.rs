//! Tests for HNSW index transfer.

use super::*;
use tempfile::TempDir;

#[tokio::test]
async fn test_collect_nonexistent_index() {
    let temp_dir = TempDir::new().unwrap();
    let manager = HnswIndexManager::new(temp_dir.path().to_path_buf());

    let metadata = manager
        .collect_index_metadata("tenant1", "repo1", "main")
        .await
        .unwrap();

    assert!(metadata.is_none());
}

#[tokio::test]
async fn test_collect_existing_index() {
    let temp_dir = TempDir::new().unwrap();

    // Create dummy index file
    let index_path = temp_dir
        .path()
        .join("tenant1")
        .join("repo1")
        .join("main.hnsw");
    tokio::fs::create_dir_all(index_path.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&index_path, b"test index data")
        .await
        .unwrap();

    let manager = HnswIndexManager::new(temp_dir.path().to_path_buf());
    let metadata = manager
        .collect_index_metadata("tenant1", "repo1", "main")
        .await
        .unwrap();

    assert!(metadata.is_some());
    let metadata = metadata.unwrap();
    assert_eq!(metadata.size_bytes, 15);
}

#[tokio::test]
async fn test_list_all_indexes() {
    let temp_dir = TempDir::new().unwrap();

    // Create some dummy index files
    let index1 = temp_dir
        .path()
        .join("tenant1")
        .join("repo1")
        .join("main.hnsw");
    let index2 = temp_dir
        .path()
        .join("tenant1")
        .join("repo1")
        .join("develop.hnsw");

    tokio::fs::create_dir_all(index1.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&index1, b"test1").await.unwrap();
    tokio::fs::write(&index2, b"test2").await.unwrap();

    let manager = HnswIndexManager::new(temp_dir.path().to_path_buf());
    let indexes = manager.list_all_indexes().await.unwrap();

    assert_eq!(indexes.len(), 2);
    assert!(indexes.contains(&(
        "tenant1".to_string(),
        "repo1".to_string(),
        "main".to_string()
    )));
    assert!(indexes.contains(&(
        "tenant1".to_string(),
        "repo1".to_string(),
        "develop".to_string()
    )));
}

#[tokio::test]
async fn test_receive_and_ingest() {
    let temp_base = TempDir::new().unwrap();
    let temp_staging = TempDir::new().unwrap();

    let receiver = HnswIndexReceiver::new(
        temp_base.path().to_path_buf(),
        temp_staging.path().to_path_buf(),
    );

    let test_data = b"test hnsw index data".to_vec();

    // Calculate expected CRC32
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(&test_data);
    let expected_crc32 = hasher.finalize();

    // Receive index
    let staging_path = receiver
        .receive_index(
            "tenant1",
            "repo1",
            "main",
            test_data.clone(),
            expected_crc32,
        )
        .await
        .unwrap();

    assert!(staging_path.exists());

    // Ingest index
    receiver
        .ingest_index(&staging_path, "tenant1", "repo1", "main")
        .await
        .unwrap();

    // Verify final file
    let final_path = temp_base
        .path()
        .join("tenant1")
        .join("repo1")
        .join("main.hnsw");
    assert!(final_path.exists());

    let final_data = tokio::fs::read(&final_path).await.unwrap();
    assert_eq!(final_data, test_data);
}

#[tokio::test]
async fn test_checksum_mismatch() {
    let temp_base = TempDir::new().unwrap();
    let temp_staging = TempDir::new().unwrap();

    let receiver = HnswIndexReceiver::new(
        temp_base.path().to_path_buf(),
        temp_staging.path().to_path_buf(),
    );

    let test_data = b"test data".to_vec();
    let wrong_crc32 = 0xDEADBEEF;

    let result = receiver
        .receive_index("tenant1", "repo1", "main", test_data, wrong_crc32)
        .await;

    assert!(result.is_err());
}
