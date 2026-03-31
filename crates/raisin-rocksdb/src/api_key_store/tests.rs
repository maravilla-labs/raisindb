//! Tests for API key storage operations.

use super::*;
use crate::cf;
use rocksdb::{Options, DB};
use std::sync::Arc;
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
fn test_generate_token() {
    let (token, hash, prefix) = ApiKeyStore::generate_token();

    assert!(token.starts_with("raisin_"));
    assert_eq!(token.len(), 39); // "raisin_" (7) + 32 chars
    assert_eq!(hash.len(), 64); // SHA-256 hex
    assert_eq!(prefix.len(), 16);
}

#[test]
fn test_hash_token() {
    let (token, expected_hash, _) = ApiKeyStore::generate_token();
    let hash = ApiKeyStore::hash_token(&token);
    assert_eq!(hash, expected_hash);
}

#[test]
fn test_create_and_get_api_key() {
    let (_dir, db) = create_test_db();
    let store = ApiKeyStore::new(db);

    let (api_key, raw_token) = store
        .create_api_key("default", "user1", "My API Key")
        .unwrap();

    assert_eq!(api_key.name, "My API Key");
    assert_eq!(api_key.user_id, "user1");
    assert_eq!(api_key.tenant_id, "default");
    assert!(api_key.is_active);
    assert!(raw_token.starts_with("raisin_"));

    // Retrieve it
    let retrieved = store
        .get_api_key("default", "user1", &api_key.key_id)
        .unwrap();
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().name, "My API Key");
}

#[test]
fn test_list_user_api_keys() {
    let (_dir, db) = create_test_db();
    let store = ApiKeyStore::new(db);

    // Create multiple keys
    store.create_api_key("default", "user1", "Key 1").unwrap();
    store.create_api_key("default", "user1", "Key 2").unwrap();
    store.create_api_key("default", "user2", "Key 3").unwrap();

    let user1_keys = store.list_user_api_keys("default", "user1").unwrap();
    assert_eq!(user1_keys.len(), 2);

    let user2_keys = store.list_user_api_keys("default", "user2").unwrap();
    assert_eq!(user2_keys.len(), 1);
}

#[test]
fn test_revoke_api_key() {
    let (_dir, db) = create_test_db();
    let store = ApiKeyStore::new(db);

    let (api_key, _) = store.create_api_key("default", "user1", "Key").unwrap();
    assert!(api_key.is_active);

    store
        .revoke_api_key("default", "user1", &api_key.key_id)
        .unwrap();

    let retrieved = store
        .get_api_key("default", "user1", &api_key.key_id)
        .unwrap()
        .unwrap();
    assert!(!retrieved.is_active);
}

#[test]
fn test_validate_api_key() {
    let (_dir, db) = create_test_db();
    let store = ApiKeyStore::new(db);

    let (_, raw_token) = store.create_api_key("default", "user1", "Key").unwrap();

    // Valid token
    let result = store.validate_api_key(&raw_token).unwrap();
    assert!(result.is_some());
    assert!(result.unwrap().last_used_at.is_some());

    // Invalid token
    let result = store.validate_api_key("invalid_token").unwrap();
    assert!(result.is_none());
}

#[test]
fn test_validate_revoked_key() {
    let (_dir, db) = create_test_db();
    let store = ApiKeyStore::new(db);

    let (api_key, raw_token) = store.create_api_key("default", "user1", "Key").unwrap();

    // Revoke the key
    store
        .revoke_api_key("default", "user1", &api_key.key_id)
        .unwrap();

    // Should not validate
    let result = store.validate_api_key(&raw_token).unwrap();
    assert!(result.is_none());
}
