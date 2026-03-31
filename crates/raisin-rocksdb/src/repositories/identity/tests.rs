//! Tests for identity repository.

use super::*;
use crate::open_db;
use crate::replication::OperationCapture;
use raisin_models::auth::Identity;
use rocksdb::DB;
use tempfile::TempDir;

fn setup_test() -> (TempDir, Arc<DB>, Arc<OperationCapture>) {
    let temp_dir = TempDir::new().unwrap();
    let db = Arc::new(open_db(temp_dir.path()).unwrap());
    let op_capture = Arc::new(OperationCapture::disabled(db.clone()));
    (temp_dir, db, op_capture)
}

#[tokio::test]
async fn test_identity_crud() {
    let (_temp_dir, db, op_capture) = setup_test();
    let repo = IdentityRepository::new(db, op_capture);

    // Create identity
    let identity = Identity::new(
        "id-123".to_string(),
        "tenant-1".to_string(),
        "user@example.com".to_string(),
    );

    repo.upsert("tenant-1", &identity, "test").await.unwrap();

    // Get by ID
    let retrieved = repo.get("tenant-1", "id-123").await.unwrap();
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().email, "user@example.com");

    // Get by email
    let by_email = repo
        .find_by_email("tenant-1", "user@example.com")
        .await
        .unwrap();
    assert!(by_email.is_some());
    assert_eq!(by_email.unwrap().identity_id, "id-123");

    // Delete
    repo.delete("tenant-1", "id-123", "test").await.unwrap();
    let deleted = repo.get("tenant-1", "id-123").await.unwrap();
    assert!(deleted.is_none());
}

#[tokio::test]
async fn test_email_case_insensitive() {
    let (_temp_dir, db, op_capture) = setup_test();
    let repo = IdentityRepository::new(db, op_capture);

    let identity = Identity::new(
        "id-456".to_string(),
        "tenant-1".to_string(),
        "User@Example.COM".to_string(),
    );

    repo.upsert("tenant-1", &identity, "test").await.unwrap();

    // Should find with different case
    let found = repo
        .find_by_email("tenant-1", "user@example.com")
        .await
        .unwrap();
    assert!(found.is_some());

    let found = repo
        .find_by_email("tenant-1", "USER@EXAMPLE.COM")
        .await
        .unwrap();
    assert!(found.is_some());
}
