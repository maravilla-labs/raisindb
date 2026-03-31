use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

/// Key to identify a unique Tantivy index directory
/// Represents (tenant_id, repo_id, branch_name)
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct IndexKey {
    pub tenant_id: String,
    pub repo_id: String,
    pub branch_name: String,
}

impl IndexKey {
    pub fn new(tenant_id: &str, repo_id: &str, branch_name: &str) -> Self {
        Self {
            tenant_id: tenant_id.to_string(),
            repo_id: repo_id.to_string(),
            branch_name: branch_name.to_string(),
        }
    }
}

impl Hash for IndexKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.tenant_id.hash(state);
        self.repo_id.hash(state);
        self.branch_name.hash(state);
    }
}

/// Manages locks for Tantivy index directories to prevent concurrent writes
///
/// This ensures that only one worker can perform indexing operations on a specific
/// (tenant, repo, branch) combination at a time, preventing Tantivy's LockBusy errors.
#[derive(Clone)]
pub struct IndexLockManager {
    locks: Arc<RwLock<HashMap<IndexKey, Arc<Mutex<()>>>>>,
}

impl IndexLockManager {
    /// Creates a new IndexLockManager
    pub fn new() -> Self {
        Self {
            locks: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Gets the lock for the specified index key
    ///
    /// Returns an Arc<Mutex<()>> that can be locked by the caller.
    /// If multiple workers try to lock the same mutex, they will wait in queue.
    pub async fn get_lock(&self, key: &IndexKey) -> Arc<Mutex<()>> {
        // Get or create the lock for this index
        let mut locks = self.locks.write().await;
        locks
            .entry(key.clone())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// Returns the number of unique indexes being tracked
    /// Useful for monitoring and testing
    #[allow(dead_code)]
    pub async fn lock_count(&self) -> usize {
        let locks = self.locks.read().await;
        locks.len()
    }
}

impl Default for IndexLockManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::timeout;

    #[tokio::test]
    async fn test_same_key_blocks() {
        let manager = IndexLockManager::new();
        let key = IndexKey::new("tenant1", "repo1", "main");

        // Acquire first lock
        let lock = manager.get_lock(&key).await;
        let _guard1 = lock.lock().await;

        // Try to acquire same lock with timeout - should timeout
        let key2 = key.clone();
        let manager2 = manager.clone();
        let lock2 = manager2.get_lock(&key2).await;
        let result = timeout(Duration::from_millis(100), lock2.lock()).await;

        assert!(
            result.is_err(),
            "Second lock should timeout while first is held"
        );
    }

    #[tokio::test]
    async fn test_different_keys_dont_block() {
        let manager = IndexLockManager::new();
        let key1 = IndexKey::new("tenant1", "repo1", "main");
        let key2 = IndexKey::new("tenant1", "repo2", "main");

        // Acquire first lock
        let lock1 = manager.get_lock(&key1).await;
        let _guard1 = lock1.lock().await;

        // Acquire second lock with different key - should succeed immediately
        let lock2 = manager.get_lock(&key2).await;
        let result = timeout(Duration::from_millis(100), lock2.lock()).await;

        assert!(result.is_ok(), "Different keys should not block each other");
    }

    #[tokio::test]
    async fn test_lock_released_after_drop() {
        let manager = IndexLockManager::new();
        let key = IndexKey::new("tenant1", "repo1", "main");

        // Acquire and release first lock
        {
            let lock = manager.get_lock(&key).await;
            let _guard1 = lock.lock().await;
        } // guard1 dropped here

        // Acquire same lock again - should succeed immediately
        let lock = manager.get_lock(&key).await;
        let result = timeout(Duration::from_millis(100), lock.lock()).await;

        assert!(result.is_ok(), "Lock should be released after guard drops");
    }

    #[tokio::test]
    async fn test_index_key_equality() {
        let key1 = IndexKey::new("tenant1", "repo1", "main");
        let key2 = IndexKey::new("tenant1", "repo1", "main");
        let key3 = IndexKey::new("tenant1", "repo1", "dev");

        assert_eq!(key1, key2);
        assert_ne!(key1, key3);
    }
}
