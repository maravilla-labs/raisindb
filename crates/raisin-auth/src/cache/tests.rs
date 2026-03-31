use super::*;
use std::time::Duration;

#[tokio::test]
async fn test_cache_basic_operations() {
    let cache = PermissionCache::new(10, Duration::from_secs(60));

    let key = CacheKey::new("session-1", "workspace-1");
    let permissions = CachedPermissions::new(
        "user-1",
        vec!["admin".to_string()],
        vec!["group1".to_string()],
        true,
        1,
    );

    // Initially not in cache
    assert!(cache.get(&key).await.is_none());

    // Set and retrieve
    cache.set(key.clone(), permissions.clone()).await;
    let cached = cache.get(&key).await;
    assert!(cached.is_some());

    let cached = cached.unwrap();
    assert_eq!(cached.user_node_id, "user-1");
    assert_eq!(cached.roles, vec!["admin".to_string()]);
    assert_eq!(cached.groups, vec!["group1".to_string()]);
    assert!(cached.is_workspace_admin);
    assert_eq!(cached.permissions_version, 1);
}

#[tokio::test]
async fn test_cache_ttl_expiration() {
    let cache = PermissionCache::new(10, Duration::from_millis(50));

    let key = CacheKey::new("session-1", "workspace-1");
    let permissions =
        CachedPermissions::new("user-1", vec!["viewer".to_string()], vec![], false, 1);

    cache.set(key.clone(), permissions).await;

    // Should be in cache immediately
    assert!(cache.get(&key).await.is_some());

    // Wait for expiration
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Should be expired and removed
    assert!(cache.get(&key).await.is_none());
}

#[tokio::test]
async fn test_get_or_resolve() {
    let cache = PermissionCache::new(10, Duration::from_secs(60));
    let key = CacheKey::new("session-1", "workspace-1");

    // First call - should resolve
    let permissions = cache
        .get_or_resolve(key.clone(), || async {
            Ok(CachedPermissions::new(
                "user-1",
                vec!["admin".to_string()],
                vec![],
                true,
                1,
            ))
        })
        .await
        .unwrap();

    assert_eq!(permissions.user_node_id, "user-1");

    // Second call - should use cache
    let cached_permissions = cache.get(&key).await.unwrap();
    assert_eq!(cached_permissions.user_node_id, "user-1");
}

#[tokio::test]
async fn test_invalidate_session() {
    let cache = PermissionCache::new(10, Duration::from_secs(60));

    let key1 = CacheKey::new("session-1", "workspace-1");
    let key2 = CacheKey::new("session-1", "workspace-2");
    let key3 = CacheKey::new("session-2", "workspace-1");

    let permissions = CachedPermissions::new("user-1", vec![], vec![], false, 1);

    cache.set(key1.clone(), permissions.clone()).await;
    cache.set(key2.clone(), permissions.clone()).await;
    cache.set(key3.clone(), permissions.clone()).await;

    assert_eq!(cache.len().await, 3);

    // Invalidate session-1
    cache.invalidate_session("session-1").await;

    assert!(cache.get(&key1).await.is_none());
    assert!(cache.get(&key2).await.is_none());
    assert!(cache.get(&key3).await.is_some());
    assert_eq!(cache.len().await, 1);
}

#[tokio::test]
async fn test_invalidate_workspace() {
    let cache = PermissionCache::new(10, Duration::from_secs(60));

    let key1 = CacheKey::new("session-1", "workspace-1");
    let key2 = CacheKey::new("session-2", "workspace-1");
    let key3 = CacheKey::new("session-1", "workspace-2");

    let permissions = CachedPermissions::new("user-1", vec![], vec![], false, 1);

    cache.set(key1.clone(), permissions.clone()).await;
    cache.set(key2.clone(), permissions.clone()).await;
    cache.set(key3.clone(), permissions.clone()).await;

    assert_eq!(cache.len().await, 3);

    cache.invalidate_workspace("workspace-1").await;

    assert!(cache.get(&key1).await.is_none());
    assert!(cache.get(&key2).await.is_none());
    assert!(cache.get(&key3).await.is_some());
    assert_eq!(cache.len().await, 1);
}

#[tokio::test]
async fn test_clear_cache() {
    let cache = PermissionCache::new(10, Duration::from_secs(60));

    let key1 = CacheKey::new("session-1", "workspace-1");
    let key2 = CacheKey::new("session-2", "workspace-2");

    let permissions = CachedPermissions::new("user-1", vec![], vec![], false, 1);

    cache.set(key1.clone(), permissions.clone()).await;
    cache.set(key2.clone(), permissions.clone()).await;

    assert_eq!(cache.len().await, 2);

    cache.clear().await;

    assert_eq!(cache.len().await, 0);
    assert!(cache.is_empty().await);
}

#[tokio::test]
async fn test_lru_eviction() {
    let cache = PermissionCache::new(2, Duration::from_secs(60));

    let key1 = CacheKey::new("session-1", "workspace-1");
    let key2 = CacheKey::new("session-2", "workspace-2");
    let key3 = CacheKey::new("session-3", "workspace-3");

    let permissions = CachedPermissions::new("user-1", vec![], vec![], false, 1);

    cache.set(key1.clone(), permissions.clone()).await;
    cache.set(key2.clone(), permissions.clone()).await;

    assert_eq!(cache.len().await, 2);

    // Adding third entry should evict key1 (least recently used)
    cache.set(key3.clone(), permissions.clone()).await;

    assert_eq!(cache.len().await, 2);
    assert!(cache.get(&key1).await.is_none());
    assert!(cache.get(&key2).await.is_some());
    assert!(cache.get(&key3).await.is_some());
}

#[tokio::test]
async fn test_cache_clone() {
    let cache1 = PermissionCache::new(10, Duration::from_secs(60));
    let cache2 = cache1.clone();

    let key = CacheKey::new("session-1", "workspace-1");
    let permissions = CachedPermissions::new("user-1", vec![], vec![], false, 1);

    cache1.set(key.clone(), permissions).await;

    // Should be visible in cache2 (same underlying cache)
    assert!(cache2.get(&key).await.is_some());
}

#[tokio::test]
async fn test_permissions_version() {
    let cache = PermissionCache::new(10, Duration::from_secs(60));

    let key = CacheKey::new("session-1", "workspace-1");
    let permissions_v1 =
        CachedPermissions::new("user-1", vec!["viewer".to_string()], vec![], false, 1);
    let permissions_v2 =
        CachedPermissions::new("user-1", vec!["admin".to_string()], vec![], true, 2);

    cache.set(key.clone(), permissions_v1).await;

    let cached = cache.get(&key).await.unwrap();
    assert_eq!(cached.permissions_version, 1);
    assert_eq!(cached.roles, vec!["viewer".to_string()]);

    cache.set(key.clone(), permissions_v2).await;

    let cached = cache.get(&key).await.unwrap();
    assert_eq!(cached.permissions_version, 2);
    assert_eq!(cached.roles, vec!["admin".to_string()]);
    assert!(cached.is_workspace_admin);
}

#[test]
fn test_cache_key_equality() {
    let key1 = CacheKey::new("session-1", "workspace-1");
    let key2 = CacheKey::new("session-1", "workspace-1");
    let key3 = CacheKey::new("session-2", "workspace-1");

    assert_eq!(key1, key2);
    assert_ne!(key1, key3);
}

#[test]
fn test_permissions_is_expired() {
    let permissions = CachedPermissions::new("user-1", vec![], vec![], false, 1);

    // Not expired with 1 hour TTL
    assert!(!permissions.is_expired(Duration::from_secs(3600)));

    // Expired with 0 second TTL
    assert!(permissions.is_expired(Duration::from_secs(0)));
}

#[tokio::test]
#[should_panic(expected = "Cache capacity must be greater than zero")]
async fn test_zero_capacity_panics() {
    PermissionCache::new(0, Duration::from_secs(60));
}
