//! Write operations for the property index
//!
//! Handles indexing, unindexing, and publish status updates.

use crate::repositories::nodes::hash_property_value;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_models::nodes::properties::PropertyValue;
use rocksdb::DB;
use std::collections::HashMap;
use std::sync::Arc;

pub(super) async fn index_properties(
    db: &Arc<DB>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    properties: &HashMap<String, PropertyValue>,
    is_published: bool,
) -> Result<()> {
    let cf = cf_handle(db, cf::PROPERTY_INDEX)?;

    for (prop_name, prop_value) in properties {
        let value_hash = hash_property_value(prop_value);
        let key = keys::property_index_key(
            tenant_id,
            repo_id,
            branch,
            workspace,
            prop_name,
            &value_hash,
            node_id,
            is_published,
        );

        db.put_cf(cf, key, b"")
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
    }

    Ok(())
}

pub(super) async fn unindex_properties(
    db: &Arc<DB>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
) -> Result<()> {
    // Scan and delete all property indexes for this node
    let prefix_draft = keys::KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push("prop")
        .build_prefix();

    let prefix_pub = keys::KeyBuilder::new()
        .push(tenant_id)
        .push(repo_id)
        .push(branch)
        .push(workspace)
        .push("prop_pub")
        .build_prefix();

    let cf = cf_handle(db, cf::PROPERTY_INDEX)?;

    for prefix in [prefix_draft, prefix_pub] {
        let prefix_clone = prefix.clone();
        let iter = db.prefix_iterator_cf(cf, prefix);

        for item in iter {
            let (key, _) = item.map_err(|e| raisin_error::Error::storage(e.to_string()))?;

            // Verify key actually starts with our prefix
            if !key.starts_with(&prefix_clone) {
                break;
            }

            let key_str = String::from_utf8_lossy(&key);

            if key_str.ends_with(&format!("\0{}", node_id)) {
                db.delete_cf(cf, key)
                    .map_err(|e| raisin_error::Error::storage(e.to_string()))?;
            }
        }
    }

    Ok(())
}

pub(super) async fn update_publish_status(
    db: &Arc<DB>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    properties: &HashMap<String, PropertyValue>,
    is_published: bool,
) -> Result<()> {
    // Remove old indexes
    unindex_properties(db, tenant_id, repo_id, branch, workspace, node_id).await?;

    // Add new indexes with correct publish status
    index_properties(
        db,
        tenant_id,
        repo_id,
        branch,
        workspace,
        node_id,
        properties,
        is_published,
    )
    .await?;

    Ok(())
}
