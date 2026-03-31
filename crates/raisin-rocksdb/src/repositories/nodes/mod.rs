//! Node repository implementation
//!
//! This module provides the RocksDB-backed implementation of the NodeRepository trait.
//! The implementation is split across multiple modules for maintainability:
//!
//! - `crud`: Core CRUD operations (create, read, update, delete) - now a module directory
//!   - `cascade`: Cascade deletion operations (within crud)
//! - `queries`: Query operations (filtering, searching, listing)
//! - `ordering`: Child ordering with fractional indexing
//! - `mvcc`: Time-travel and MVCC operations
//! - `publishing`: Publishing/unpublishing operations
//! - `helpers`: Utility functions
//! - `trait_impl`: NodeRepository trait delegation to internal methods

mod crud; // This is now a directory with sub-modules
pub mod helpers;
mod mvcc;
mod ordering;
mod publishing;
mod queries;
mod storage_node;
mod trait_impl;
mod validation;

// Re-export StorageNode for use within this crate
pub(crate) use storage_node::StorageNode;

// Re-export hash_property_value for use by property_index repository
pub(crate) use helpers::hash_property_value;

use raisin_error::Result;
use raisin_events::EventBus;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use raisin_storage::BranchRepository;
use rocksdb::DB;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct NodeRepositoryImpl {
    db: Arc<DB>,
    #[allow(dead_code)]
    event_bus: Arc<dyn EventBus>,
    revision_repo: Arc<crate::repositories::RevisionRepositoryImpl>,
    branch_repo: Arc<crate::repositories::BranchRepositoryImpl>,
    tag_repo: Arc<crate::repositories::TagRepositoryImpl>,
    pub(crate) node_type_repo: Arc<crate::repositories::NodeTypeRepositoryImpl>,
    workspace_repo: Arc<crate::repositories::WorkspaceRepositoryImpl>,
    /// Per-parent ordering locks to prevent concurrent modification of child order
    /// Key format: {tenant_id}/{repo_id}/{branch}/{workspace}/{parent_id}
    ordering_locks: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
    /// Operation capture for replication
    operation_capture: Arc<crate::OperationCapture>,
}

impl NodeRepositoryImpl {
    pub fn new(
        db: Arc<DB>,
        event_bus: Arc<dyn EventBus>,
        revision_repo: Arc<crate::repositories::RevisionRepositoryImpl>,
        branch_repo: Arc<crate::repositories::BranchRepositoryImpl>,
        tag_repo: Arc<crate::repositories::TagRepositoryImpl>,
        node_type_repo: Arc<crate::repositories::NodeTypeRepositoryImpl>,
        workspace_repo: Arc<crate::repositories::WorkspaceRepositoryImpl>,
        operation_capture: Arc<crate::OperationCapture>,
    ) -> Self {
        Self {
            db,
            event_bus,
            revision_repo,
            branch_repo,
            tag_repo,
            node_type_repo,
            workspace_repo,
            ordering_locks: Arc::new(Mutex::new(HashMap::new())),
            operation_capture,
        }
    }

    /// Acquire an ordering lock for a parent to prevent concurrent modifications
    ///
    /// This ensures that only one ordering operation can modify a parent's children at a time,
    /// preventing race conditions in fractional index label assignment.
    async fn acquire_ordering_lock(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        parent_id: &str,
    ) -> Arc<Mutex<()>> {
        let lock_key = format!(
            "{}/{}/{}/{}/{}",
            tenant_id, repo_id, branch, workspace, parent_id
        );

        let mut locks = self.ordering_locks.lock().await;
        locks
            .entry(lock_key)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// Resolve the HEAD revision for a branch or tag
    ///
    /// First tries to resolve as a branch. If that fails, checks if it's a tag
    /// and returns the tag's revision.
    async fn resolve_head_revision(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch_or_tag: &str,
    ) -> Result<Option<HLC>> {
        // First try as a branch
        match self
            .branch_repo
            .get_head(tenant_id, repo_id, branch_or_tag)
            .await
        {
            Ok(head) => Ok(Some(head)),
            Err(_) => {
                // If branch lookup fails, try as a tag
                use raisin_storage::TagRepository;
                match self
                    .tag_repo
                    .get_tag(tenant_id, repo_id, branch_or_tag)
                    .await?
                {
                    Some(tag) => Ok(Some(tag.revision)),
                    None => Ok(None),
                }
            }
        }
    }

    /// Add a brand new node (optimized - for testing)
    ///
    /// This is exposed for testing purposes to measure performance difference
    /// between put() and add() operations.
    #[allow(dead_code)]
    pub async fn add(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node: Node,
    ) -> Result<()> {
        self.add_impl(tenant_id, repo_id, branch, workspace, node)
            .await
    }
}
