//! Tests for admin user store.

use super::*;
use raisin_models::admin_user::{AdminAccessFlags, DatabaseAdminUser};
use rocksdb::{Options, DB};
use tempfile::TempDir;

fn create_test_db() -> (TempDir, Arc<DB>) {
    let temp_dir = TempDir::new().unwrap();
    let mut opts = Options::default();
    opts.create_if_missing(true);
    opts.create_missing_column_families(true);

    let db = DB::open_cf(&opts, temp_dir.path(), vec![cf::ADMIN_USERS]).unwrap();
    (temp_dir, Arc::new(db))
}

#[test]
fn test_create_and_get_user() {
    let (_dir, db) = create_test_db();
    let store = AdminUserStore::new(db);

    let user = DatabaseAdminUser::new(
        "user1".to_string(),
        "testuser".to_string(),
        Some("test@example.com".to_string()),
        "hash".to_string(),
        "default".to_string(),
    );

    // Create user
    store.create_user(&user).unwrap();

    // Get user
    let retrieved = store.get_user("default", "testuser").unwrap();
    assert!(retrieved.is_some());
    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.username, "testuser");
    assert_eq!(retrieved.email, Some("test@example.com".to_string()));
}

#[test]
fn test_duplicate_user() {
    let (_dir, db) = create_test_db();
    let store = AdminUserStore::new(db);

    let user = DatabaseAdminUser::new(
        "user1".to_string(),
        "testuser".to_string(),
        None,
        "hash".to_string(),
        "default".to_string(),
    );

    store.create_user(&user).unwrap();
    let result = store.create_user(&user);
    assert!(result.is_err());
}

#[test]
fn test_update_user() {
    let (_dir, db) = create_test_db();
    let store = AdminUserStore::new(db);

    let mut user = DatabaseAdminUser::new(
        "user1".to_string(),
        "testuser".to_string(),
        None,
        "hash".to_string(),
        "default".to_string(),
    );

    store.create_user(&user).unwrap();

    // Update user
    user.email = Some("updated@example.com".to_string());
    user.access_flags.console_login = false;
    store.update_user(&user).unwrap();

    // Verify update
    let retrieved = store.get_user("default", "testuser").unwrap().unwrap();
    assert_eq!(retrieved.email, Some("updated@example.com".to_string()));
    assert!(!retrieved.access_flags.console_login);
}

#[test]
fn test_delete_user() {
    let (_dir, db) = create_test_db();
    let store = AdminUserStore::new(db);

    let user = DatabaseAdminUser::new(
        "user1".to_string(),
        "testuser".to_string(),
        None,
        "hash".to_string(),
        "default".to_string(),
    );

    store.create_user(&user).unwrap();
    store.delete_user("default", "testuser").unwrap();

    let retrieved = store.get_user("default", "testuser").unwrap();
    assert!(retrieved.is_none());
}

#[test]
fn test_list_users() {
    let (_dir, db) = create_test_db();
    let store = AdminUserStore::new(db);

    // Create multiple users
    for i in 1..=3 {
        let user = DatabaseAdminUser::new(
            format!("user{}", i),
            format!("user{}", i),
            None,
            "hash".to_string(),
            "default".to_string(),
        );
        store.create_user(&user).unwrap();
    }

    let users = store.list_users("default").unwrap();
    assert_eq!(users.len(), 3);
}

#[test]
fn test_tenant_isolation() {
    let (_dir, db) = create_test_db();
    let store = AdminUserStore::new(db);

    // Create users in different tenants
    let user1 = DatabaseAdminUser::new(
        "user1".to_string(),
        "admin".to_string(),
        None,
        "hash".to_string(),
        "tenant1".to_string(),
    );

    let user2 = DatabaseAdminUser::new(
        "user2".to_string(),
        "admin".to_string(),
        None,
        "hash".to_string(),
        "tenant2".to_string(),
    );

    store.create_user(&user1).unwrap();
    store.create_user(&user2).unwrap();

    // Verify isolation
    let tenant1_users = store.list_users("tenant1").unwrap();
    let tenant2_users = store.list_users("tenant2").unwrap();

    assert_eq!(tenant1_users.len(), 1);
    assert_eq!(tenant2_users.len(), 1);
    assert_eq!(tenant1_users[0].tenant_id, "tenant1");
    assert_eq!(tenant2_users[0].tenant_id, "tenant2");
}
