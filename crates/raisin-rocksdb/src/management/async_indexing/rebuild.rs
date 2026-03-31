//! Index rebuild operations for repositories.
//!
//! Contains the main rebuild_indexes entry point and per-index-type
//! rebuild functions for path, property, reference, and child-order indexes.
//!
//! NOTE: This file intentionally exceeds 300 lines because rebuild_property_indexes
//! indexes 8+ system properties inline. Splitting each property into a separate function
//! would add complexity without improving readability.

use crate::{cf, cf_handle, keys, repositories::hash_property_value, RocksDBStorage};
use raisin_error::{Error, Result};
use raisin_storage::{IndexType, RebuildStats};
use rocksdb::WriteBatch;

use super::helpers::{
    clear_order_indexes, clear_path_indexes, clear_property_indexes, clear_reference_indexes,
    extract_references, get_current_revision, scan_nodes,
};

/// Rebuild indexes for a repository + workspace
pub async fn rebuild_indexes(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    index_type: IndexType,
) -> Result<RebuildStats> {
    let start = std::time::Instant::now();

    let mut stats = RebuildStats {
        index_type,
        items_processed: 0,
        errors: 0,
        duration_ms: 0,
        success: true,
    };

    tracing::info!(
        "Rebuilding {:?} indexes for {}/{}/{}/{}",
        index_type,
        tenant_id,
        repo_id,
        branch,
        workspace
    );

    match index_type {
        IndexType::Property => {
            rebuild_property_indexes(storage, tenant_id, repo_id, branch, workspace, &mut stats)
                .await?;
        }
        IndexType::Reference => {
            rebuild_reference_indexes(storage, tenant_id, repo_id, branch, workspace, &mut stats)
                .await?;
        }
        IndexType::ChildOrder => {
            rebuild_order_indexes(storage, tenant_id, repo_id, branch, workspace, &mut stats)
                .await?;
        }
        IndexType::All => {
            rebuild_path_indexes(storage, tenant_id, repo_id, branch, workspace, &mut stats)
                .await?;
            rebuild_property_indexes(storage, tenant_id, repo_id, branch, workspace, &mut stats)
                .await?;
            rebuild_reference_indexes(storage, tenant_id, repo_id, branch, workspace, &mut stats)
                .await?;
            rebuild_order_indexes(storage, tenant_id, repo_id, branch, workspace, &mut stats)
                .await?;
        }
        IndexType::FullText | IndexType::Vector => {
            // These index types are managed by separate systems (Tantivy/HNSW)
            // and are not handled by RocksDB's async indexing
            return Err(Error::Validation(format!(
                "Index type {:?} is not managed by RocksDB",
                index_type
            )));
        }
    }

    stats.duration_ms = start.elapsed().as_millis() as u64;
    stats.success = stats.errors == 0;

    tracing::info!(
        "Rebuild complete: {} items processed, {} errors in {}ms",
        stats.items_processed,
        stats.errors,
        stats.duration_ms
    );

    Ok(stats)
}

/// Rebuild path indexes for all nodes in a workspace
async fn rebuild_path_indexes(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    stats: &mut RebuildStats,
) -> Result<()> {
    tracing::info!("Rebuilding path indexes");

    // 1. Get all nodes
    let nodes = scan_nodes(storage, tenant_id, repo_id, branch, workspace).await?;

    // 2. Clear existing path indexes for this workspace
    clear_path_indexes(storage, tenant_id, repo_id, branch, workspace).await?;

    // 3. Rebuild from nodes using batch writes for performance
    let mut batch = WriteBatch::default();
    let cf_path = cf_handle(storage.db(), cf::PATH_INDEX)?;

    // Get current revision for this workspace
    let current_revision = get_current_revision(storage, tenant_id, repo_id, branch).await?;

    for node in nodes {
        stats.items_processed += 1;

        // Create versioned path index
        let key = keys::path_index_key_versioned(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &node.path,
            &current_revision,
        );

        batch.put_cf(cf_path, key, node.id.as_bytes());

        // Commit batch every 1000 items to avoid memory issues
        if stats.items_processed % 1000 == 0 {
            storage
                .db()
                .write(batch)
                .map_err(|e| raisin_error::Error::storage(format!("Batch write failed: {}", e)))?;
            batch = WriteBatch::default();
            tracing::debug!("Committed {} path indexes", stats.items_processed);
        }
    }

    // Commit remaining items
    if !batch.is_empty() {
        storage.db().write(batch).map_err(|e| {
            raisin_error::Error::storage(format!("Final batch write failed: {}", e))
        })?;
    }

    Ok(())
}

/// Rebuild property indexes for all nodes in a workspace
async fn rebuild_property_indexes(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    stats: &mut RebuildStats,
) -> Result<()> {
    tracing::info!(
        "📝 Rebuilding property indexes for {}/{}/{}/{}",
        tenant_id,
        repo_id,
        branch,
        workspace
    );

    // 1. Get all nodes
    let nodes = scan_nodes(storage, tenant_id, repo_id, branch, workspace).await?;
    tracing::info!("📝 Found {} nodes to index", nodes.len());

    // 2. Clear existing property indexes for this workspace
    clear_property_indexes(storage, tenant_id, repo_id, branch, workspace).await?;

    // 3. Rebuild from nodes
    let mut batch = WriteBatch::default();
    let cf_prop = cf_handle(storage.db(), cf::PROPERTY_INDEX)?;
    let current_revision = get_current_revision(storage, tenant_id, repo_id, branch).await?;

    tracing::debug!("📝 Using revision: {:?}", current_revision);

    // Track index entries created
    let mut user_props_indexed = 0usize;
    let mut system_props_indexed = 0usize;
    let mut nodes_with_created_at = 0usize;
    let mut nodes_without_created_at = 0usize;

    for node in nodes {
        stats.items_processed += 1;
        let is_published = node.published_at.is_some();

        // Log first 5 nodes in detail for debugging
        if stats.items_processed <= 5 {
            tracing::debug!(
                "📝 Indexing node #{}: id={}, path={}, created_at={:?}, is_published={}",
                stats.items_processed,
                node.id,
                node.path,
                node.created_at,
                is_published
            );
        }

        // Index each user property
        for (prop_name, prop_value) in &node.properties {
            // Create a hash of the property value for indexing
            let value_hash = hash_property_value(prop_value);

            let key = keys::property_index_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                prop_name,
                &value_hash,
                &current_revision,
                &node.id,
                is_published,
            );

            batch.put_cf(cf_prop, key, b"");
            user_props_indexed += 1;
        }

        // Index system properties (required for ORDER BY queries on system fields)

        // Index: __node_type
        let node_type_key = keys::property_index_key_versioned(
            tenant_id,
            repo_id,
            branch,
            workspace,
            "__node_type",
            &node.node_type,
            &current_revision,
            &node.id,
            is_published,
        );
        batch.put_cf(cf_prop, node_type_key, node.id.as_bytes());
        system_props_indexed += 1;

        // Index: __name
        if !node.name.is_empty() {
            let name_key = keys::property_index_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                "__name",
                &node.name,
                &current_revision,
                &node.id,
                is_published,
            );
            batch.put_cf(cf_prop, name_key, node.id.as_bytes());
            system_props_indexed += 1;
        }

        // Index: __archetype
        if let Some(ref archetype) = node.archetype {
            if !archetype.is_empty() {
                let archetype_key = keys::property_index_key_versioned(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    "__archetype",
                    archetype,
                    &current_revision,
                    &node.id,
                    is_published,
                );
                batch.put_cf(cf_prop, archetype_key, node.id.as_bytes());
                system_props_indexed += 1;
            }
        }

        // Index: __created_by
        if let Some(ref created_by) = node.created_by {
            if !created_by.is_empty() {
                let created_by_key = keys::property_index_key_versioned(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    "__created_by",
                    created_by,
                    &current_revision,
                    &node.id,
                    is_published,
                );
                batch.put_cf(cf_prop, created_by_key, node.id.as_bytes());
                system_props_indexed += 1;
            }
        }

        // Index: __updated_by
        if let Some(ref updated_by) = node.updated_by {
            if !updated_by.is_empty() {
                let updated_by_key = keys::property_index_key_versioned(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    "__updated_by",
                    updated_by,
                    &current_revision,
                    &node.id,
                    is_published,
                );
                batch.put_cf(cf_prop, updated_by_key, node.id.as_bytes());
                system_props_indexed += 1;
            }
        }

        // Index: __created_at (as i64 microseconds for efficient range queries)
        if let Some(created_at) = node.created_at {
            nodes_with_created_at += 1;
            let timestamp_micros = created_at.timestamp_micros();
            let created_at_key = keys::property_index_key_versioned_timestamp(
                tenant_id,
                repo_id,
                branch,
                workspace,
                "__created_at",
                timestamp_micros,
                &current_revision,
                &node.id,
                is_published,
            );

            // Log the key for the first few nodes
            if stats.items_processed <= 5 {
                tracing::debug!(
                    "   → __created_at index: node_id={}, timestamp_micros={}, key_prefix={:?}",
                    node.id,
                    timestamp_micros,
                    String::from_utf8_lossy(
                        &created_at_key[..std::cmp::min(80, created_at_key.len())]
                    )
                );
            }

            batch.put_cf(cf_prop, created_at_key, node.id.as_bytes());
            system_props_indexed += 1;
        } else {
            nodes_without_created_at += 1;
            if stats.items_processed <= 5 {
                tracing::warn!("   ⚠️ Node {} has NO created_at timestamp!", node.id);
            }
        }

        // Index: __updated_at (as i64 microseconds for efficient range queries)
        if let Some(updated_at) = node.updated_at {
            let updated_at_key = keys::property_index_key_versioned_timestamp(
                tenant_id,
                repo_id,
                branch,
                workspace,
                "__updated_at",
                updated_at.timestamp_micros(),
                &current_revision,
                &node.id,
                is_published,
            );
            batch.put_cf(cf_prop, updated_at_key, node.id.as_bytes());
            system_props_indexed += 1;
        }

        // Commit batch every 1000 nodes to avoid memory issues
        if stats.items_processed % 1000 == 0 {
            storage
                .db()
                .write(batch)
                .map_err(|e| raisin_error::Error::storage(format!("Batch write failed: {}", e)))?;
            batch = WriteBatch::default();
            tracing::debug!("📝 Committed batch at {} nodes", stats.items_processed);
        }
    }

    tracing::info!(
        "📝 Index stats: {} user properties, {} system properties indexed",
        user_props_indexed,
        system_props_indexed
    );
    tracing::info!(
        "📝 Timestamp stats: {} nodes with created_at, {} without",
        nodes_with_created_at,
        nodes_without_created_at
    );

    // Commit remaining items
    if !batch.is_empty() {
        storage.db().write(batch).map_err(|e| {
            raisin_error::Error::storage(format!("Final batch write failed: {}", e))
        })?;
    }

    Ok(())
}

/// Rebuild reference indexes for all nodes in a workspace
async fn rebuild_reference_indexes(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    stats: &mut RebuildStats,
) -> Result<()> {
    tracing::info!("Rebuilding reference indexes");

    // 1. Get all nodes
    let nodes = scan_nodes(storage, tenant_id, repo_id, branch, workspace).await?;

    // 2. Clear existing reference indexes for this workspace
    clear_reference_indexes(storage, tenant_id, repo_id, branch, workspace).await?;

    // 3. Rebuild from nodes
    let mut batch = WriteBatch::default();
    let cf_ref = cf_handle(storage.db(), cf::REFERENCE_INDEX)?;
    let current_revision = get_current_revision(storage, tenant_id, repo_id, branch).await?;

    for node in nodes {
        stats.items_processed += 1;
        let is_published = node.published_at.is_some();

        // Extract and index all references from properties
        let references = extract_references(&node.properties);

        for (prop_path, reference) in references {
            // Forward index: source -> target
            let fwd_key = keys::reference_forward_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                &node.id,
                &prop_path,
                &current_revision,
                is_published,
            );

            let ref_json = rmp_serde::to_vec(&reference).unwrap_or_default();
            batch.put_cf(cf_ref, fwd_key, ref_json);

            // Reverse index: target -> source
            let rev_key = keys::reference_reverse_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                &reference.workspace,
                &reference.path,
                &node.id,
                &prop_path,
                &current_revision,
                is_published,
            );

            batch.put_cf(cf_ref, rev_key, b"");
        }

        // Commit batch every 500 nodes
        if stats.items_processed % 500 == 0 {
            storage
                .db()
                .write(batch)
                .map_err(|e| raisin_error::Error::storage(format!("Batch write failed: {}", e)))?;
            batch = WriteBatch::default();
        }
    }

    // Commit remaining items
    if !batch.is_empty() {
        storage.db().write(batch).map_err(|e| {
            raisin_error::Error::storage(format!("Final batch write failed: {}", e))
        })?;
    }

    Ok(())
}

/// Rebuild child order indexes for all nodes in a workspace
async fn rebuild_order_indexes(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    stats: &mut RebuildStats,
) -> Result<()> {
    tracing::info!("Rebuilding child order indexes");

    // 1. Get all nodes
    let nodes = scan_nodes(storage, tenant_id, repo_id, branch, workspace).await?;

    // 2. Clear existing order indexes for this workspace
    clear_order_indexes(storage, tenant_id, repo_id, branch, workspace).await?;

    // 3. Rebuild from parent.children arrays
    let mut batch = WriteBatch::default();
    let cf_order = cf_handle(storage.db(), cf::ORDER_INDEX)?;

    for node in nodes {
        if !node.children.is_empty() {
            stats.items_processed += 1;

            // Store the ordered list of children for this parent
            let children_json = rmp_serde::to_vec(&node.children).unwrap_or_default();

            let key = keys::KeyBuilder::new()
                .push(tenant_id)
                .push(repo_id)
                .push(branch)
                .push(workspace)
                .push("order")
                .push(&node.id)
                .build();

            batch.put_cf(cf_order, key, children_json);

            // Commit batch every 1000 items
            if stats.items_processed % 1000 == 0 {
                storage.db().write(batch).map_err(|e| {
                    raisin_error::Error::storage(format!("Batch write failed: {}", e))
                })?;
                batch = WriteBatch::default();
            }
        }
    }

    // Commit remaining items
    if !batch.is_empty() {
        storage.db().write(batch).map_err(|e| {
            raisin_error::Error::storage(format!("Final batch write failed: {}", e))
        })?;
    }

    Ok(())
}
