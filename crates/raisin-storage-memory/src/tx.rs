//! Transaction support for in-memory storage
//!
//! This module implements transactions for the in-memory storage backend
//! using a write-ahead log that is applied on commit.

use async_trait::async_trait;
use raisin_error::{Error, Result};
use raisin_models::{nodes::Node, translations::LocaleOverlay, workspace::Workspace};
use raisin_storage::{transactional::TransactionalContext, Transaction};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::RwLock;

use crate::NodeKey;

/// Operation types for the transaction log
#[derive(Debug, Clone)]
enum Operation {
    PutNode { workspace: String, node: Box<Node> },
    DeleteNode { workspace: String, node_id: String },
    PutWorkspace { _workspace: Box<Workspace> },
}

/// In-memory transaction implementation
///
/// Holds a reference to the underlying node storage so that
/// commit() can actually persist operations, and reads can
/// fall back to committed data.
pub struct InMemoryTx {
    /// Operations to be applied on commit
    operations: Arc<Mutex<Vec<Operation>>>,
    /// Read cache for consistent reads within transaction
    read_cache: Arc<Mutex<HashMap<String, Option<Vec<u8>>>>>,
    /// Path-based cache for get_node_by_path within the transaction
    path_cache: Arc<Mutex<HashMap<String, Node>>>,
    /// Whether the transaction has been completed
    completed: Arc<Mutex<bool>>,
    /// Reference to the actual node storage (for commit and fallback reads)
    nodes: Arc<RwLock<HashMap<String, Node>>>,
    /// Tenant ID (set via set_tenant_repo)
    tenant_id: Arc<Mutex<String>>,
    /// Repository ID (set via set_tenant_repo)
    repo_id: Arc<Mutex<String>>,
    /// Branch name (set via set_branch)
    branch: Arc<Mutex<String>>,
}

impl InMemoryTx {
    /// Create a new in-memory transaction with a reference to the underlying storage
    pub fn new(nodes: Arc<RwLock<HashMap<String, Node>>>) -> Self {
        Self {
            operations: Arc::new(Mutex::new(Vec::new())),
            read_cache: Arc::new(Mutex::new(HashMap::new())),
            path_cache: Arc::new(Mutex::new(HashMap::new())),
            completed: Arc::new(Mutex::new(false)),
            nodes,
            tenant_id: Arc::new(Mutex::new(String::new())),
            repo_id: Arc::new(Mutex::new(String::new())),
            branch: Arc::new(Mutex::new(String::new())),
        }
    }

    /// Check if the transaction has been completed
    fn check_not_completed(&self) -> Result<()> {
        let completed = self
            .completed
            .lock()
            .map_err(|e| Error::Backend(format!("Failed to lock completed flag: {}", e)))?;

        if *completed {
            return Err(Error::Backend("Transaction already completed".to_string()));
        }

        Ok(())
    }

    /// Mark the transaction as completed
    fn mark_completed(&self) -> Result<()> {
        let mut completed = self
            .completed
            .lock()
            .map_err(|e| Error::Backend(format!("Failed to lock completed flag: {}", e)))?;
        *completed = true;
        Ok(())
    }

    /// Get the current tenant_id, repo_id, branch context
    fn context(&self) -> Result<(String, String, String)> {
        let tenant_id = self
            .tenant_id
            .lock()
            .map_err(|e| Error::Backend(format!("Failed to lock tenant_id: {}", e)))?
            .clone();
        let repo_id = self
            .repo_id
            .lock()
            .map_err(|e| Error::Backend(format!("Failed to lock repo_id: {}", e)))?
            .clone();
        let branch = self
            .branch
            .lock()
            .map_err(|e| Error::Backend(format!("Failed to lock branch: {}", e)))?
            .clone();
        Ok((tenant_id, repo_id, branch))
    }
}

impl Clone for InMemoryTx {
    fn clone(&self) -> Self {
        Self {
            operations: self.operations.clone(),
            read_cache: self.read_cache.clone(),
            path_cache: self.path_cache.clone(),
            completed: self.completed.clone(),
            nodes: self.nodes.clone(),
            tenant_id: self.tenant_id.clone(),
            repo_id: self.repo_id.clone(),
            branch: self.branch.clone(),
        }
    }
}

impl Transaction for InMemoryTx {
    async fn commit(&self) -> Result<()> {
        self.check_not_completed()?;

        let ops = self
            .operations
            .lock()
            .map_err(|e| Error::Backend(format!("Failed to lock operations: {}", e)))?
            .clone();

        let (tenant_id, repo_id, branch) = self.context()?;

        // Apply operations to the actual storage
        let mut map = self.nodes.write().await;
        for op in &ops {
            match op {
                Operation::PutNode { workspace, node } => {
                    let key =
                        NodeKey::new(&tenant_id, &repo_id, &branch, workspace, &node.id).to_path();
                    map.insert(key, node.as_ref().clone());
                }
                Operation::DeleteNode { workspace, node_id } => {
                    let key =
                        NodeKey::new(&tenant_id, &repo_id, &branch, workspace, node_id).to_path();
                    map.remove(&key);
                }
                Operation::PutWorkspace { .. } => {
                    // Workspace metadata is not stored in the nodes map
                }
            }
        }

        self.mark_completed()?;
        log::debug!(
            "InMemory transaction committed with {} operations",
            ops.len()
        );
        Ok(())
    }

    async fn rollback(&self) -> Result<()> {
        self.check_not_completed()?;

        // Simply discard all operations
        self.operations
            .lock()
            .map_err(|e| Error::Backend(format!("Failed to lock operations: {}", e)))?
            .clear();

        self.mark_completed()?;
        log::debug!("InMemory transaction rolled back");
        Ok(())
    }
}

#[async_trait]
impl TransactionalContext for InMemoryTx {
    async fn put_node(&self, workspace: &str, node: &Node) -> Result<()> {
        self.check_not_completed()?;

        // CRITICAL: Auto-derive parent NAME from path before saving
        // This ensures node.parent always contains the parent's NAME, not PATH
        let mut node = node.clone();
        node.parent = Node::extract_parent_name_from_path(&node.path);

        let mut ops = self
            .operations
            .lock()
            .map_err(|e| Error::Backend(format!("Failed to lock operations: {}", e)))?;

        ops.push(Operation::PutNode {
            workspace: workspace.to_string(),
            node: Box::new(node.clone()),
        });

        // Update read cache (by node ID)
        let key = format!("{}:nodes:{}", workspace, node.id);
        let value = rmp_serde::to_vec(&node)
            .map_err(|e| Error::Backend(format!("Failed to serialize node: {}", e)))?;

        self.read_cache
            .lock()
            .map_err(|e| Error::Backend(format!("Failed to lock cache: {}", e)))?
            .insert(key, Some(value));

        // Update path cache (by workspace:path)
        let path_key = format!("{}:{}", workspace, node.path);
        self.path_cache
            .lock()
            .map_err(|e| Error::Backend(format!("Failed to lock path cache: {}", e)))?
            .insert(path_key, node);

        Ok(())
    }

    async fn add_node(&self, workspace: &str, node: &Node) -> Result<()> {
        // For in-memory storage, just delegate to put_node
        // No performance benefit from a separate add since lookups are O(1) HashMap
        self.put_node(workspace, node).await
    }

    async fn upsert_node(&self, workspace: &str, node: &Node) -> Result<()> {
        // In-memory backend does not differentiate path-based upserts.
        self.put_node(workspace, node).await
    }

    async fn delete_node(&self, workspace: &str, node_id: &str) -> Result<()> {
        self.check_not_completed()?;

        let mut ops = self
            .operations
            .lock()
            .map_err(|e| Error::Backend(format!("Failed to lock operations: {}", e)))?;

        ops.push(Operation::DeleteNode {
            workspace: workspace.to_string(),
            node_id: node_id.to_string(),
        });

        // Update read cache
        let key = format!("{}:nodes:{}", workspace, node_id);
        self.read_cache
            .lock()
            .map_err(|e| Error::Backend(format!("Failed to lock cache: {}", e)))?
            .insert(key, None);

        Ok(())
    }

    async fn get_node_by_path(&self, workspace: &str, path: &str) -> Result<Option<Node>> {
        self.check_not_completed()?;

        // First check the path cache (nodes written in this transaction)
        let path_key = format!("{}:{}", workspace, path);
        {
            let cache = self
                .path_cache
                .lock()
                .map_err(|e| Error::Backend(format!("Failed to lock path cache: {}", e)))?;
            if let Some(node) = cache.get(&path_key) {
                return Ok(Some(node.clone()));
            }
        }

        // Fall back to the committed storage
        let (tenant_id, repo_id, branch) = self.context()?;
        if tenant_id.is_empty() {
            return Ok(None);
        }
        let workspace_prefix = NodeKey::workspace_prefix(&tenant_id, &repo_id, &branch, workspace);
        let map = self.nodes.read().await;
        let found = map
            .iter()
            .find(|(k, n)| k.starts_with(&workspace_prefix) && n.path == path)
            .map(|(_, n)| n.clone());
        Ok(found)
    }

    async fn store_translation(
        &self,
        _workspace: &str,
        _node_id: &str,
        _locale: &str,
        _overlay: LocaleOverlay,
    ) -> Result<()> {
        Ok(())
    }

    async fn get_translation(
        &self,
        _workspace: &str,
        _node_id: &str,
        _locale: &str,
    ) -> Result<Option<LocaleOverlay>> {
        Ok(None)
    }

    async fn list_translations_for_node(
        &self,
        _workspace: &str,
        _node_id: &str,
    ) -> Result<Vec<String>> {
        Ok(Vec::new())
    }

    async fn list_children(&self, _workspace: &str, _parent_path: &str) -> Result<Vec<Node>> {
        Ok(Vec::new())
    }

    async fn reorder_child_before(
        &self,
        _workspace: &str,
        _parent_path: &str,
        _child_name: &str,
        _before_child_name: &str,
    ) -> Result<()> {
        // Ordering support is not implemented for the in-memory backend.
        Ok(())
    }

    async fn reorder_child_after(
        &self,
        _workspace: &str,
        _parent_path: &str,
        _child_name: &str,
        _after_child_name: &str,
    ) -> Result<()> {
        // Ordering support is not implemented for the in-memory backend.
        Ok(())
    }

    async fn copy_node_tree(
        &self,
        _workspace: &str,
        _source_path: &str,
        _target_parent: &str,
        _new_name: Option<&str>,
        _actor: &str,
    ) -> Result<Node> {
        Err(Error::Backend(
            "copy_node_tree is not supported in the in-memory storage backend".to_string(),
        ))
    }

    async fn delete_path_index(&self, workspace: &str, path: &str) -> Result<()> {
        self.check_not_completed()?;

        // In-memory implementation: no-op (mainly for testing, RocksDB is production)
        let _ = (workspace, path);
        Ok(())
    }

    async fn get_node(&self, workspace: &str, node_id: &str) -> Result<Option<Node>> {
        self.check_not_completed()?;

        // First check the read cache
        let key = format!("{}:nodes:{}", workspace, node_id);
        {
            let cache = self
                .read_cache
                .lock()
                .map_err(|e| Error::Backend(format!("Failed to lock cache: {}", e)))?;

            if let Some(cached) = cache.get(&key) {
                match cached {
                    Some(bytes) => {
                        let node: Node = rmp_serde::from_slice(bytes).map_err(|e| {
                            Error::Backend(format!("Failed to deserialize node: {}", e))
                        })?;
                        return Ok(Some(node));
                    }
                    None => return Ok(None),
                }
            }
        }

        // Fall back to the committed storage
        let (tenant_id, repo_id, branch) = self.context()?;
        if tenant_id.is_empty() {
            return Ok(None);
        }
        let node_key = NodeKey::new(&tenant_id, &repo_id, &branch, workspace, node_id).to_path();
        let map = self.nodes.read().await;
        Ok(map.get(&node_key).cloned())
    }

    async fn put_workspace(&self, workspace: &Workspace) -> Result<()> {
        self.check_not_completed()?;

        let mut ops = self
            .operations
            .lock()
            .map_err(|e| Error::Backend(format!("Failed to lock operations: {}", e)))?;

        ops.push(Operation::PutWorkspace {
            _workspace: Box::new(workspace.clone()),
        });

        Ok(())
    }

    /// Set the branch for this transaction
    fn set_branch(&self, branch: &str) -> Result<()> {
        let mut b = self
            .branch
            .lock()
            .map_err(|e| Error::Backend(format!("Failed to lock branch: {}", e)))?;
        *b = branch.to_string();
        Ok(())
    }

    /// Set the actor (user ID) for this transaction (no-op for in-memory storage)
    fn set_actor(&self, _actor: &str) -> Result<()> {
        Ok(()) // In-memory storage doesn't track actor metadata
    }

    /// Set the commit message for this transaction (no-op for in-memory storage)
    fn set_message(&self, _message: &str) -> Result<()> {
        Ok(()) // In-memory storage doesn't track commit messages
    }

    /// Get the current commit message (always None for in-memory storage)
    fn get_message(&self) -> Result<Option<String>> {
        Ok(None) // In-memory storage doesn't track commit messages
    }

    /// Get the current actor (always None for in-memory storage)
    fn get_actor(&self) -> Result<Option<String>> {
        Ok(None) // In-memory storage doesn't track actor metadata
    }

    /// Set tenant and repository IDs for this transaction
    fn set_tenant_repo(&self, tenant_id: &str, repo_id: &str) -> Result<()> {
        {
            let mut t = self
                .tenant_id
                .lock()
                .map_err(|e| Error::Backend(format!("Failed to lock tenant_id: {}", e)))?;
            *t = tenant_id.to_string();
        }
        {
            let mut r = self
                .repo_id
                .lock()
                .map_err(|e| Error::Backend(format!("Failed to lock repo_id: {}", e)))?;
            *r = repo_id.to_string();
        }
        Ok(())
    }

    /// Set whether this commit is a manual version (no-op for in-memory storage)
    fn set_is_manual_version(&self, _is_manual: bool) -> Result<()> {
        Ok(()) // In-memory storage doesn't track manual version metadata
    }

    /// Set the node ID this manual version applies to (no-op for in-memory storage)
    fn set_manual_version_node_id(&self, _node_id: &str) -> Result<()> {
        Ok(()) // In-memory storage doesn't track manual version metadata
    }

    /// Set whether this is a system commit (no-op for in-memory storage)
    fn set_is_system(&self, _is_system: bool) -> Result<()> {
        Ok(()) // In-memory storage doesn't track system commit metadata
    }

    /// Add a relationship from source node to target node (not implemented for in-memory storage)
    async fn add_relation(
        &self,
        _source_workspace: &str,
        _source_node_id: &str,
        _source_node_type: &str,
        _relation: raisin_models::nodes::RelationRef,
    ) -> Result<()> {
        Ok(())
    }

    /// Remove a specific relationship between two nodes (not implemented for in-memory storage)
    async fn remove_relation(
        &self,
        _source_workspace: &str,
        _source_node_id: &str,
        _target_workspace: &str,
        _target_node_id: &str,
    ) -> Result<bool> {
        Ok(false)
    }

    /// Scan all nodes in a workspace (collects all into memory)
    async fn scan_nodes(&self, _workspace: &str) -> Result<Vec<Node>> {
        self.check_not_completed()?;
        Ok(Vec::new())
    }

    async fn move_node_tree(
        &self,
        _workspace: &str,
        _node_id: &str,
        _new_path: &str,
    ) -> Result<()> {
        Ok(())
    }

    async fn commit(&self) -> Result<()> {
        Transaction::commit(self).await
    }

    async fn rollback(&self) -> Result<()> {
        Transaction::rollback(self).await
    }

    // Deep node operations

    async fn add_deep_node(&self, workspace: &str, node: &Node, _actor: &str) -> Result<()> {
        self.add_node(workspace, node).await
    }

    async fn upsert_deep_node(&self, workspace: &str, node: &Node, _actor: &str) -> Result<()> {
        self.upsert_node(workspace, node).await
    }

    // Auth context methods (no-op for in-memory storage)

    fn set_auth_context(&self, _auth_context: raisin_models::auth::AuthContext) -> Result<()> {
        Ok(())
    }

    fn get_auth_context(&self) -> Result<Option<std::sync::Arc<raisin_models::auth::AuthContext>>> {
        Ok(None)
    }

    fn set_validate_schema(&self, _enabled: bool) -> Result<()> {
        Ok(())
    }

    fn validate_schema(&self) -> bool {
        true
    }
}
