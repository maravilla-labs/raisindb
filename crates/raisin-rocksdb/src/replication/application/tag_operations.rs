//! Tag operation handlers for replication
//!
//! This module contains operation handlers for:
//! - apply_create_tag
//! - apply_delete_tag

use super::super::OperationApplicator;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_replication::Operation;

/// Apply a tag creation operation
pub(super) async fn apply_create_tag(
    applicator: &OperationApplicator,
    tenant_id: &str,
    repo_id: &str,
    tag_name: &str,
    revision: &str,
    op: &Operation,
) -> Result<()> {
    tracing::info!(
        "📥 Applying tag creation: {}/{}/{} -> {} from node {}",
        tenant_id,
        repo_id,
        tag_name,
        revision,
        op.cluster_node_id
    );

    let key = keys::tag_key(tenant_id, repo_id, tag_name);
    let cf = cf_handle(&applicator.db, cf::TAGS)?;

    // Store the revision string as the tag value
    applicator
        .db
        .put_cf(cf, key, revision.as_bytes())
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

    tracing::info!(
        "✅ Tag created successfully: {}/{}/{}",
        tenant_id,
        repo_id,
        tag_name
    );
    Ok(())
}

/// Apply a tag deletion operation
pub(super) async fn apply_delete_tag(
    applicator: &OperationApplicator,
    tenant_id: &str,
    repo_id: &str,
    tag_name: &str,
    op: &Operation,
) -> Result<()> {
    tracing::info!(
        "📥 Applying tag delete: {}/{}/{} from node {}",
        tenant_id,
        repo_id,
        tag_name,
        op.cluster_node_id
    );

    let key = keys::tag_key(tenant_id, repo_id, tag_name);
    let cf = cf_handle(&applicator.db, cf::TAGS)?;

    applicator
        .db
        .delete_cf(cf, key)
        .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

    tracing::info!(
        "✅ Tag deleted successfully: {}/{}/{}",
        tenant_id,
        repo_id,
        tag_name
    );
    Ok(())
}
