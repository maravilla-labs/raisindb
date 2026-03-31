use raisin_error::Result;
use raisin_models::tree::TreeEntry;
use raisin_storage::scope::RepoScope;
use raisin_storage::TreeRepository;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone, Default)]
pub struct InMemoryTreeRepo {
    trees: Arc<RwLock<HashMap<Vec<u8>, Vec<TreeEntry>>>>,
    commit_trees: Arc<RwLock<HashMap<String, [u8; 32]>>>,
}

impl InMemoryTreeRepo {
    pub fn new() -> Self {
        Self {
            trees: Arc::new(RwLock::new(HashMap::new())),
            commit_trees: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    fn commit_tree_key(tenant_id: &str, repo_id: &str, revision: u64) -> String {
        format!("{}/{}/{}", tenant_id, repo_id, revision)
    }

    /// For use by InMemoryTx during commit
    pub async fn store_commit_tree_internal(
        &self,
        tenant_id: &str,
        repo_id: &str,
        revision: u64,
        root_tree_id: [u8; 32],
    ) -> Result<()> {
        let key = Self::commit_tree_key(tenant_id, repo_id, revision);
        let mut commit_trees = self.commit_trees.write().await;
        commit_trees.insert(key, root_tree_id);
        Ok(())
    }
}

impl TreeRepository for InMemoryTreeRepo {
    async fn build_leaf(&self, _scope: RepoScope<'_>, entries: &[TreeEntry]) -> Result<[u8; 32]> {
        use blake3::Hasher;

        // Sort entries for deterministic hashing
        let mut sorted_entries = entries.to_vec();
        sorted_entries.sort_by(|a, b| a.entry_key.cmp(&b.entry_key));

        // Compute hash (using MessagePack for consistent hashing with RocksDB)
        let bytes = rmp_serde::to_vec(&sorted_entries).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to serialize tree: {}", e))
        })?;
        let hash = Hasher::new().update(&bytes).finalize();
        let tree_id: [u8; 32] = *hash.as_bytes();

        // Store
        let mut trees = self.trees.write().await;
        trees.insert(tree_id.to_vec(), sorted_entries);

        Ok(tree_id)
    }

    async fn iter_tree(
        &self,
        _scope: RepoScope<'_>,
        tree_id: &[u8; 32],
        _start_after: Option<&str>,
        _limit: usize,
    ) -> Result<Vec<TreeEntry>> {
        let trees = self.trees.read().await;
        Ok(trees.get(&tree_id.to_vec()).cloned().unwrap_or_default())
    }

    async fn get_tree_entry(
        &self,
        _scope: RepoScope<'_>,
        tree_id: &[u8; 32],
        entry_key: &str,
    ) -> Result<Option<TreeEntry>> {
        let trees = self.trees.read().await;
        if let Some(entries) = trees.get(&tree_id.to_vec()) {
            Ok(entries.iter().find(|e| e.entry_key == entry_key).cloned())
        } else {
            Ok(None)
        }
    }

    async fn get_root_tree_id(
        &self,
        scope: RepoScope<'_>,
        revision: &raisin_hlc::HLC,
    ) -> Result<Option<[u8; 32]>> {
        // For memory storage, use timestamp_ms as the revision key
        let key = Self::commit_tree_key(scope.tenant_id, scope.repo_id, revision.timestamp_ms);
        let commit_trees = self.commit_trees.read().await;
        Ok(commit_trees.get(&key).copied())
    }
}
