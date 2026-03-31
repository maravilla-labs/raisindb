//! Transaction API for atomic multi-node operations
//!
//! Provides user-facing transaction control with explicit commit/rollback.
//! Commits create immutable repository revisions.

mod commit;

use raisin_models::auth::AuthContext;
use raisin_models::nodes::Node;
use raisin_storage::Storage;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Transaction builder for atomic multi-node operations.
///
/// Accumulates operations in memory, then commits them all at once
/// creating a single repository revision.
///
/// # Example
/// ```rust,ignore
/// let mut tx = workspace.nodes().transaction();
/// tx.create(node1);
/// tx.update(node2_id, props);
/// tx.delete(node3_id);
/// tx.commit("Bulk update", "user-123").await?;
/// ```
pub struct Transaction<S: Storage> {
    pub(crate) storage: Arc<S>,
    pub(crate) tenant_id: String,
    pub(crate) repo_id: String,
    pub(crate) branch: String,
    pub(crate) workspace_id: String,
    pub(crate) operations: Vec<TxOperation>,
    /// Auth context for RLS enforcement during commit
    pub(crate) auth_context: Option<AuthContext>,
}

/// Operations that can be performed in a transaction
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TxOperation {
    Create {
        node: Box<Node>,
    },
    Update {
        node_id: String,
        properties: serde_json::Value,
    },
    Delete {
        node_id: String,
    },
    Move {
        node_id: String,
        new_path: String,
    },
    Rename {
        node_id: String,
        new_name: String,
    },
    Copy {
        source_path: String,
        target_parent: String,
        new_name: Option<String>,
    },
    CopyTree {
        source_path: String,
        target_parent: String,
        new_name: Option<String>,
    },
}

impl<S: Storage> Transaction<S> {
    /// Create a new transaction
    pub fn new(
        storage: Arc<S>,
        tenant_id: String,
        repo_id: String,
        branch: String,
        workspace_id: String,
    ) -> Self {
        Self {
            storage,
            tenant_id,
            repo_id,
            branch,
            workspace_id,
            operations: Vec::new(),
            auth_context: None,
        }
    }

    /// Set auth context for RLS enforcement during commit
    pub fn with_auth_context(mut self, auth: AuthContext) -> Self {
        self.auth_context = Some(auth);
        self
    }

    /// Add a create operation
    pub fn create(&mut self, node: Node) -> &mut Self {
        self.operations.push(TxOperation::Create {
            node: Box::new(node),
        });
        self
    }

    /// Add an update operation
    pub fn update(&mut self, node_id: String, properties: serde_json::Value) -> &mut Self {
        self.operations.push(TxOperation::Update {
            node_id,
            properties,
        });
        self
    }

    /// Add a delete operation
    pub fn delete(&mut self, node_id: String) -> &mut Self {
        self.operations.push(TxOperation::Delete { node_id });
        self
    }

    /// Add a move operation
    pub fn move_node(&mut self, node_id: String, new_path: String) -> &mut Self {
        self.operations
            .push(TxOperation::Move { node_id, new_path });
        self
    }

    /// Add a rename operation
    pub fn rename(&mut self, node_id: String, new_name: String) -> &mut Self {
        self.operations
            .push(TxOperation::Rename { node_id, new_name });
        self
    }

    /// Add a copy operation
    pub fn copy(
        &mut self,
        source_path: String,
        target_parent: String,
        new_name: Option<String>,
    ) -> &mut Self {
        self.operations.push(TxOperation::Copy {
            source_path,
            target_parent,
            new_name,
        });
        self
    }

    /// Add a copy tree operation (recursive copy)
    pub fn copy_tree(
        &mut self,
        source_path: String,
        target_parent: String,
        new_name: Option<String>,
    ) -> &mut Self {
        self.operations.push(TxOperation::CopyTree {
            source_path,
            target_parent,
            new_name,
        });
        self
    }

    /// Get the number of pending operations
    pub fn len(&self) -> usize {
        self.operations.len()
    }

    /// Check if there are no pending operations
    pub fn is_empty(&self) -> bool {
        self.operations.is_empty()
    }

    /// Rollback (discard) all pending operations
    pub fn rollback(self) {
        tracing::info!(
            "Transaction rolled back: {} operations discarded",
            self.operations.len()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_models::nodes::Node;
    use raisin_storage_memory::InMemoryStorage;

    #[tokio::test]
    async fn test_transaction_rollback_discards_operations() {
        let storage = Arc::new(InMemoryStorage::default());
        let mut tx = Transaction::new(
            storage.clone(),
            "test-tenant".into(),
            "test-repo".into(),
            "main".into(),
            "test-ws".into(),
        );

        tx.create(Node::default());
        tx.delete("node-123".into());
        tx.move_node("node-456".into(), "/new/parent".into());

        assert_eq!(tx.len(), 3);

        tx.rollback();
    }

    #[tokio::test]
    async fn test_empty_transaction_commit_fails() {
        let storage = Arc::new(InMemoryStorage::default());
        let tx = Transaction::new(
            storage,
            "test-tenant".into(),
            "test-repo".into(),
            "main".into(),
            "test-ws".into(),
        );

        let result = tx.commit("Empty commit", "user-123").await;
        assert!(result.is_err());
        assert!(matches!(result, Err(raisin_error::Error::Validation(_))));
    }
}
