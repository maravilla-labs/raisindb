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

#[test]
fn test_has_users_is_strictly_tenant_scoped() {
    // Regression: `prefix_iterator_cf` seeks but does not filter, so
    // `iter.next()` returns whatever key is lexically next in the CF —
    // even one belonging to a different tenant. Without the bounds
    // check, has_users() returned true for empty tenants whose id sorts
    // before any tenant that has users.
    let (_dir, db) = create_test_db();
    let store = AdminUserStore::new(db);

    // Seed a user in a tenant whose id sorts AFTER our query target.
    let later = DatabaseAdminUser::new(
        "u-later".to_string(),
        "admin".to_string(),
        None,
        "hash".to_string(),
        "z-later-tenant".to_string(),
    );
    store.create_user(&later).unwrap();

    // Tenant with no users at all — must return false, regardless of
    // what other tenants exist in the CF.
    assert!(
        !store.has_users("a-empty-tenant").unwrap(),
        "has_users() leaked across tenants: it should return false for an empty tenant even when a later-sorting tenant has users",
    );

    // And the populated tenant must still report true.
    assert!(store.has_users("z-later-tenant").unwrap());
}

// ---------------------------------------------------------------------------
// Replication apply path
//
// `apply_update_user` / `apply_delete_user` delegate to the two methods below
// instead of re-deriving this store's key layout and encoding, so what needs
// proving is that a record written the replica way is readable the normal way.
// ---------------------------------------------------------------------------

fn sample_user(username: &str) -> DatabaseAdminUser {
    DatabaseAdminUser::new(
        format!("id-{username}"),
        username.to_string(),
        Some(format!("{username}@example.com")),
        "hash".to_string(),
        "default".to_string(),
    )
}

/// A user replicated from another node must be readable through the normal
/// lookup — same key, same encoding.
#[test]
fn a_replicated_user_is_readable_by_the_normal_lookup() {
    let (_dir, db) = create_test_db();
    let store = AdminUserStore::new(db);

    store.put_replicated(&sample_user("alice")).unwrap();

    let loaded = store
        .get_user("default", "alice")
        .unwrap()
        .expect("the replicated user must resolve");
    assert_eq!(loaded.user_id, "id-alice");
    assert_eq!(loaded.email.as_deref(), Some("alice@example.com"));
}

/// `capture_user_operation` emits `UpdateUser` for creates too, so the apply
/// path must not require the user to already exist — `update_user` does, which
/// is why it cannot be reused here.
#[test]
fn a_replicated_create_does_not_require_a_pre_existing_user() {
    let (_dir, db) = create_test_db();
    let store = AdminUserStore::new(db);

    assert!(
        store.update_user(&sample_user("bob")).is_err(),
        "update_user requires an existing user — the reason put_replicated exists"
    );
    store
        .put_replicated(&sample_user("bob"))
        .expect("the apply path must accept a user it has never seen");
    assert!(store.get_user("default", "bob").unwrap().is_some());
}

/// A replicated delete for a user this node never had must be a no-op, not an
/// error: returning `Err` would make the operation redeliver forever.
#[test]
fn a_replicated_delete_is_idempotent() {
    let (_dir, db) = create_test_db();
    let store = AdminUserStore::new(db);

    store
        .delete_replicated("default", "never-existed")
        .expect("deleting an absent user must not error");

    store.put_replicated(&sample_user("carol")).unwrap();
    store.delete_replicated("default", "carol").unwrap();
    assert!(store.get_user("default", "carol").unwrap().is_none());
    store
        .delete_replicated("default", "carol")
        .expect("a redelivered delete must stay a no-op");
}
