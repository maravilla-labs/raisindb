//! Workspace operations
//!
//! This module contains the implementation of workspace operations for transactions:
//! - `put_workspace`: Store workspace configuration

use raisin_error::Result;
use raisin_models::workspace::Workspace;

use crate::transaction::RocksDBTransaction;
use crate::{cf, cf_handle, keys};

/// Store workspace configuration
///
/// # Arguments
///
/// * `tx` - The transaction instance
/// * `workspace` - The workspace configuration to store
///
/// # Returns
///
/// Ok(()) on success
pub async fn put_workspace(tx: &RocksDBTransaction, workspace: &Workspace) -> Result<()> {
    // 1. Get metadata
    let (tenant_id, repo_id) = {
        let meta = tx
            .metadata
            .lock()
            .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;
        (meta.tenant_id.clone(), meta.repo_id.clone())
    };

    // 2. Serialize workspace (use to_vec_named for custom deserializer compatibility)
    let value = rmp_serde::to_vec_named(workspace)
        .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

    // 3. Lock batch and add workspace
    let mut batch = tx
        .batch
        .lock()
        .map_err(|e| raisin_error::Error::storage(format!("Lock error: {}", e)))?;

    let cf_workspaces = cf_handle(&tx.db, cf::WORKSPACES)?;
    let key = keys::workspace_key(&tenant_id, &repo_id, &workspace.name);
    batch.put_cf(cf_workspaces, key, value);

    Ok(())
}
