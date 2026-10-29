//! Tests for session repository.

use super::*;
use crate::open_db;
use raisin_models::timestamp::StorageTimestamp;
use tempfile::TempDir;

fn setup_test() -> (TempDir, Arc<DB>, Arc<OperationCapture>) {
    let temp_dir = TempDir::new().unwrap();
    let db = Arc::new(open_db(temp_dir.path()).unwrap());
    let op_capture = Arc::new(OperationCapture::disabled(db.clone()));
    (temp_dir, db, op_capture)
}

fn create_test_session(session_id: &str, identity_id: &str) -> Session {
    let expires = StorageTimestamp::from_nanos(
        StorageTimestamp::now().timestamp_nanos() + 24 * 60 * 60 * 1_000_000_000,
    )
    .unwrap();

    Session::new(
        session_id.to_string(),
        "tenant-1".to_string(),
        identity_id.to_string(),
        "local".to_string(),
        format!("family-{}", session_id),
        expires,
    )
}

#[tokio::test]
async fn test_session_crud() {
    let (_temp_dir, db, op_capture) = setup_test();
    let repo = SessionRepository::new(db, op_capture);

    // Create session
    let session = create_test_session("sess-123", "id-123");
    repo.create("tenant-1", &session, "test").await.unwrap();

    // Get by ID
    let retrieved = repo.get("tenant-1", "sess-123").await.unwrap();
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().identity_id, "id-123");

    // Revoke
    repo.revoke("tenant-1", "sess-123", "test logout", "test")
        .await
        .unwrap();

    let revoked = repo.get("tenant-1", "sess-123").await.unwrap();
    assert!(revoked.is_some());
    assert!(revoked.unwrap().revoked);
}

#[tokio::test]
async fn test_list_for_identity() {
    let (_temp_dir, db, op_capture) = setup_test();
    let repo = SessionRepository::new(db, op_capture);

    // Create multiple sessions for same identity
    let session1 = create_test_session("sess-1", "id-123");
    let session2 = create_test_session("sess-2", "id-123");
    let session3 = create_test_session("sess-3", "id-456");

    repo.create("tenant-1", &session1, "test").await.unwrap();
    repo.create("tenant-1", &session2, "test").await.unwrap();
    repo.create("tenant-1", &session3, "test").await.unwrap();

    // List for id-123
    let sessions = repo.list_for_identity("tenant-1", "id-123").await.unwrap();
    assert_eq!(sessions.len(), 2);

    // List for id-456
    let sessions = repo.list_for_identity("tenant-1", "id-456").await.unwrap();
    assert_eq!(sessions.len(), 1);
}

#[tokio::test]
async fn test_revoke_all_for_identity() {
    let (_temp_dir, db, op_capture) = setup_test();
    let repo = SessionRepository::new(db, op_capture);

    // Create multiple sessions
    let session1 = create_test_session("sess-1", "id-123");
    let session2 = create_test_session("sess-2", "id-123");

    repo.create("tenant-1", &session1, "test").await.unwrap();
    repo.create("tenant-1", &session2, "test").await.unwrap();

    // Revoke all
    let count = repo
        .revoke_all_for_identity("tenant-1", "id-123", "password changed", "test")
        .await
        .unwrap();
    assert_eq!(count, 2);

    // Verify all revoked
    let sessions = repo.list_for_identity("tenant-1", "id-123").await.unwrap();
    assert!(sessions.iter().all(|s| s.revoked));
}

#[tokio::test]
async fn rotation_is_a_compare_and_swap_on_the_generation() {
    let (_temp_dir, db, op_capture) = setup_test();
    let repo = SessionRepository::new(db, op_capture);
    repo.create("tenant-1", &create_test_session("s-cas", "id-1"), "test")
        .await
        .unwrap();

    // the first refresh of generation 0 rotates, the second presents the same generation and loses
    assert!(repo
        .rotate_refresh_token_if("tenant-1", "s-cas", 0, 1, "test")
        .await
        .unwrap());
    assert!(!repo
        .rotate_refresh_token_if("tenant-1", "s-cas", 0, 1, "test")
        .await
        .unwrap());
    assert_eq!(
        repo.get("tenant-1", "s-cas")
            .await
            .unwrap()
            .unwrap()
            .token_generation,
        1
    );
    // a revoked or missing session never rotates
    assert!(!repo
        .rotate_refresh_token_if("tenant-1", "s-cas-none", 0, 1, "test")
        .await
        .unwrap());
    repo.revoke("tenant-1", "s-cas", "x", "test").await.unwrap();
    assert!(!repo
        .rotate_refresh_token_if("tenant-1", "s-cas", 1, 2, "test")
        .await
        .unwrap());
}

#[tokio::test]
async fn concurrent_refreshes_of_one_token_rotate_exactly_once() {
    let (_temp_dir, db, op_capture) = setup_test();
    let repo = Arc::new(SessionRepository::new(db, op_capture));
    repo.create("tenant-1", &create_test_session("s-conc", "id-1"), "test")
        .await
        .unwrap();

    let mut tasks = Vec::new();
    for _ in 0..16 {
        let repo = repo.clone();
        tasks.push(tokio::spawn(async move {
            repo.rotate_refresh_token_if("tenant-1", "s-conc", 0, 1, "test")
                .await
                .unwrap()
        }));
    }
    let mut winners = 0;
    for t in tasks {
        if t.await.unwrap() {
            winners += 1;
        }
    }
    assert_eq!(winners, 1, "exactly one concurrent refresh rotates");
    assert_eq!(
        repo.get("tenant-1", "s-conc")
            .await
            .unwrap()
            .unwrap()
            .token_generation,
        1
    );
}

#[tokio::test]
async fn grace_answers_are_bounded_per_generation_and_reset_by_a_rotation() {
    let (_temp_dir, db, op_capture) = setup_test();
    let repo = SessionRepository::new(db, op_capture);
    repo.create("tenant-1", &create_test_session("s-grace", "id-1"), "test")
        .await
        .unwrap();

    for _ in 0..3 {
        assert!(repo.try_consume_refresh_grace("tenant-1", "s-grace", 1, 3));
    }
    assert!(
        !repo.try_consume_refresh_grace("tenant-1", "s-grace", 1, 3),
        "a stolen old token cannot be replayed beyond the bound"
    );
    // another generation / session counts separately
    assert!(repo.try_consume_refresh_grace("tenant-1", "s-grace", 2, 3));
    assert!(repo.try_consume_refresh_grace("tenant-1", "s-grace-b", 1, 3));
    // a rotation clears the count of the session
    assert!(repo
        .rotate_refresh_token_if("tenant-1", "s-grace", 0, 1, "test")
        .await
        .unwrap());
    assert!(repo.try_consume_refresh_grace("tenant-1", "s-grace", 1, 3));
}
