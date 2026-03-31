//! Transaction metadata setters
//!
//! This module contains all metadata setter functions for transactions:
//! - `set_branch`: Set the branch for this transaction
//! - `set_actor`: Set the actor (user) performing the transaction
//! - `set_message`: Set the commit message
//! - `set_tenant_repo`: Set tenant and repository IDs
//! - `set_is_manual_version`: Mark as manual version creation
//! - `set_manual_version_node_id`: Set the node ID for manual versioning
//! - `set_is_system`: Mark as system transaction
//! - `set_auth_context`: Set authentication context for RLS
//! - `get_auth_context`: Get current authentication context

use raisin_error::Result;
use raisin_models::auth::AuthContext;
use std::sync::Arc;

use crate::transaction::RocksDBTransaction;

/// Set the branch for this transaction
///
/// All operations will be performed on this branch.
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `branch` - The branch name
///
/// # Returns
///
/// Ok(()) on success
pub fn set_branch(tx: &RocksDBTransaction, branch: &str) -> Result<()> {
    let mut metadata = tx
        .metadata
        .lock()
        .map_err(|e| raisin_error::Error::storage(format!("Failed to lock metadata: {}", e)))?;
    metadata.branch = Some(Arc::new(branch.to_string()));
    Ok(())
}

/// Set the actor (user) performing this transaction
///
/// Used for commit metadata and audit logging.
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `actor` - The actor identifier
///
/// # Returns
///
/// Ok(()) on success
pub fn set_actor(tx: &RocksDBTransaction, actor: &str) -> Result<()> {
    let mut metadata = tx
        .metadata
        .lock()
        .map_err(|e| raisin_error::Error::storage(format!("Failed to lock metadata: {}", e)))?;
    metadata.actor = Some(Arc::new(actor.to_string()));
    Ok(())
}

/// Set the commit message for this transaction
///
/// Describes the changes made in this transaction.
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `message` - The commit message
///
/// # Returns
///
/// Ok(()) on success
pub fn set_message(tx: &RocksDBTransaction, message: &str) -> Result<()> {
    let mut metadata = tx
        .metadata
        .lock()
        .map_err(|e| raisin_error::Error::storage(format!("Failed to lock metadata: {}", e)))?;
    metadata.message = Some(Arc::new(message.to_string()));
    Ok(())
}

/// Get the current commit message (if set)
///
/// Returns the message that will be used for this transaction's commit.
///
/// # Arguments
///
/// * `tx` - The transaction instance
///
/// # Returns
///
/// Ok(Some(message)) if set, Ok(None) if not set
pub fn get_message(tx: &RocksDBTransaction) -> Result<Option<String>> {
    let metadata = tx
        .metadata
        .lock()
        .map_err(|e| raisin_error::Error::storage(format!("Failed to lock metadata: {}", e)))?;
    Ok(metadata.message.as_ref().map(|s| s.to_string()))
}

/// Get the current actor (if set)
///
/// Returns the actor (user) performing this transaction.
///
/// # Arguments
///
/// * `tx` - The transaction instance
///
/// # Returns
///
/// Ok(Some(actor)) if set, Ok(None) if not set
pub fn get_actor(tx: &RocksDBTransaction) -> Result<Option<String>> {
    let metadata = tx
        .metadata
        .lock()
        .map_err(|e| raisin_error::Error::storage(format!("Failed to lock metadata: {}", e)))?;
    Ok(metadata.actor.as_ref().map(|s| s.to_string()))
}

/// Set tenant and repository IDs for this transaction
///
/// All operations will be scoped to this tenant and repository.
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `tenant_id` - The tenant ID
/// * `repo_id` - The repository ID
///
/// # Returns
///
/// Ok(()) on success
pub fn set_tenant_repo(tx: &RocksDBTransaction, tenant_id: &str, repo_id: &str) -> Result<()> {
    let mut metadata = tx
        .metadata
        .lock()
        .map_err(|e| raisin_error::Error::storage(format!("Failed to lock metadata: {}", e)))?;
    metadata.tenant_id = Arc::new(tenant_id.to_string());
    metadata.repo_id = Arc::new(repo_id.to_string());
    Ok(())
}

/// Mark this transaction as manual version creation
///
/// Used for explicit versioning operations.
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `is_manual` - True if this is a manual version
///
/// # Returns
///
/// Ok(()) on success
pub fn set_is_manual_version(tx: &RocksDBTransaction, is_manual: bool) -> Result<()> {
    let mut metadata = tx
        .metadata
        .lock()
        .map_err(|e| raisin_error::Error::storage(format!("Failed to lock metadata: {}", e)))?;
    metadata.is_manual_version = is_manual;
    Ok(())
}

/// Set the node ID for manual versioning
///
/// Identifies the node being manually versioned.
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `node_id` - The node ID
///
/// # Returns
///
/// Ok(()) on success
pub fn set_manual_version_node_id(tx: &RocksDBTransaction, node_id: &str) -> Result<()> {
    let mut metadata = tx
        .metadata
        .lock()
        .map_err(|e| raisin_error::Error::storage(format!("Failed to lock metadata: {}", e)))?;
    metadata.manual_version_node_id = Some(Arc::new(node_id.to_string()));
    Ok(())
}

/// Mark this transaction as a system transaction
///
/// System transactions are created by background jobs, migrations, etc.
/// and may have different validation or auditing rules.
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `is_system` - True if this is a system transaction
///
/// # Returns
///
/// Ok(()) on success
pub fn set_is_system(tx: &RocksDBTransaction, is_system: bool) -> Result<()> {
    let mut metadata = tx
        .metadata
        .lock()
        .map_err(|e| raisin_error::Error::storage(format!("Failed to lock metadata: {}", e)))?;
    metadata.is_system = is_system;
    Ok(())
}

/// Set the authentication context for this transaction
///
/// When set, RLS (row-level security) and field-level security will be
/// enforced for all operations in this transaction.
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `auth_context` - The authentication context containing user identity and permissions
///
/// # Returns
///
/// Ok(()) on success
pub fn set_auth_context(tx: &RocksDBTransaction, auth_context: AuthContext) -> Result<()> {
    let mut metadata = tx
        .metadata
        .lock()
        .map_err(|e| raisin_error::Error::storage(format!("Failed to lock metadata: {}", e)))?;
    metadata.auth_context = Some(Arc::new(auth_context));
    Ok(())
}

/// Get the current authentication context (if set)
///
/// Returns the authentication context for this transaction, used for
/// RLS and field-level security enforcement.
///
/// # Arguments
///
/// * `tx` - The transaction instance
///
/// # Returns
///
/// Ok(Some(auth_context)) if set, Ok(None) if not set
pub fn get_auth_context(tx: &RocksDBTransaction) -> Result<Option<Arc<AuthContext>>> {
    let metadata = tx
        .metadata
        .lock()
        .map_err(|e| raisin_error::Error::storage(format!("Failed to lock metadata: {}", e)))?;
    Ok(metadata.auth_context.clone())
}
