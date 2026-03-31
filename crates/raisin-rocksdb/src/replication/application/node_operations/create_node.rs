//! CreateNode operation handler

use super::super::OperationApplicator;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_events::NodeEventKind;
use raisin_models::nodes::{properties::PropertyValue, Node};
use raisin_replication::Operation;
use rocksdb::WriteBatch;
use std::collections::HashMap;

/// Apply a CreateNode operation
#[allow(clippy::too_many_arguments)]
pub(in crate::replication::application) async fn apply_create_node(
    applicator: &OperationApplicator,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node_id: &str,
    name: &str,
    node_type: &str,
    archetype: Option<&str>,
    parent_id: Option<&str>,
    order_key: &str,
    properties: &HashMap<String, PropertyValue>,
    owner_id: Option<&str>,
    path: &str,
    op: &Operation,
) -> Result<()> {
    tracing::info!(
        op_seq = op.op_seq,
        revision = ?op.revision,
        "RECEIVED CreateNode operation from node {} with revision={:?}",
        op.cluster_node_id,
        op.revision
    );

    tracing::info!(
        "Applying CreateNode: {}/{}/{}/{} from node {}",
        tenant_id,
        repo_id,
        branch,
        node_id,
        op.cluster_node_id
    );

    // Create Node struct from operation data
    use chrono::DateTime;
    let timestamp = DateTime::from_timestamp_millis(op.timestamp_ms as i64);

    let node = Node {
        id: node_id.to_string(),
        name: name.to_string(),
        node_type: node_type.to_string(),
        archetype: archetype.map(|s| s.to_string()),
        parent: parent_id.map(|s| s.to_string()),
        order_key: order_key.to_string(),
        properties: properties.clone(),
        children: Vec::new(),
        has_children: None,
        version: 1,
        created_at: timestamp,
        updated_at: timestamp,
        path: path.to_string(),
        published_at: None,
        published_by: None,
        updated_by: Some(op.actor.clone()),
        created_by: Some(op.actor.clone()),
        translations: None,
        tenant_id: Some(tenant_id.to_string()),
        workspace: Some(workspace.to_string()),
        owner_id: owner_id.map(|s| s.to_string()),
        relations: Vec::new(),
    };

    // Extract the revision from the operation
    let revision = OperationApplicator::op_revision(op)?;

    // Create ALL indexes
    let mut batch = WriteBatch::default();

    // Get column family handles
    let cf_nodes = cf_handle(&applicator.db, cf::NODES)?;
    let cf_path = cf_handle(&applicator.db, cf::PATH_INDEX)?;
    let cf_property = cf_handle(&applicator.db, cf::PROPERTY_INDEX)?;
    let cf_reference = cf_handle(&applicator.db, cf::REFERENCE_INDEX)?;
    let cf_relation = cf_handle(&applicator.db, cf::RELATION_INDEX)?;

    // Serialize node with named fields (ensures RaisinReference has raisin:* keys)
    let node_value = rmp_serde::to_vec_named(&node)
        .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

    // 1. Store node blob with versioned key
    let node_key =
        keys::node_key_versioned(tenant_id, repo_id, branch, workspace, &node.id, &revision);
    batch.put_cf(cf_nodes, node_key, node_value);

    // 2. Index by path with versioned key
    let path_key = keys::path_index_key_versioned(
        tenant_id, repo_id, branch, workspace, &node.path, &revision,
    );
    batch.put_cf(cf_path, path_key, node.id.as_bytes());

    // 3. Add property indexes
    write_property_indexes(
        &mut batch,
        cf_property,
        tenant_id,
        repo_id,
        branch,
        workspace,
        &node,
        &revision,
    );

    // 4. Add reference indexes (forward + reverse)
    write_reference_indexes(
        &mut batch,
        cf_reference,
        tenant_id,
        repo_id,
        branch,
        workspace,
        &node,
        &revision,
    );

    // 5. Add relation indexes (forward + reverse)
    write_relation_indexes(
        &mut batch,
        cf_relation,
        tenant_id,
        repo_id,
        branch,
        workspace,
        &node,
        &revision,
    )?;

    // 6. Add ORDERED_CHILDREN index if node has a parent
    if let Some(parent) = parent_id {
        write_ordered_children_index(
            &mut batch, applicator, tenant_id, repo_id, branch, workspace, parent, order_key,
            node_id, name, &revision,
        )?;
    }

    // Atomic commit of all indexes
    applicator
        .db
        .write(batch)
        .map_err(|e| raisin_error::Error::storage(format!("Failed to apply create_node: {}", e)))?;

    tracing::info!(
        "Node created successfully: {} (HEAD will be updated by separate UpdateBranch operation)",
        node_id
    );

    // Emit NodeEvent for indexing and job processing
    super::event_helpers::emit_node_event(
        &applicator.event_bus,
        tenant_id,
        repo_id,
        branch,
        workspace,
        node_id,
        Some(node_type.to_string()),
        Some(path.to_string()),
        &revision,
        NodeEventKind::Created,
        "replication",
    );

    Ok(())
}

/// Write property indexes (user properties + system properties) to a batch
fn write_property_indexes(
    batch: &mut WriteBatch,
    cf_property: &rocksdb::ColumnFamily,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node: &Node,
    revision: &raisin_hlc::HLC,
) {
    let is_published = node.published_at.is_some();

    for (prop_name, prop_value) in &node.properties {
        let value_hash = crate::repositories::hash_property_value(prop_value);
        let prop_key = keys::property_index_key_versioned(
            tenant_id,
            repo_id,
            branch,
            workspace,
            prop_name,
            &value_hash,
            revision,
            &node.id,
            is_published,
        );
        batch.put_cf(cf_property, prop_key, node.id.as_bytes());
    }

    // System property: __node_type
    let node_type_key = keys::property_index_key_versioned(
        tenant_id,
        repo_id,
        branch,
        workspace,
        "__node_type",
        &node.node_type,
        revision,
        &node.id,
        is_published,
    );
    batch.put_cf(cf_property, node_type_key, node.id.as_bytes());

    // System property: __name
    if !node.name.is_empty() {
        let name_key = keys::property_index_key_versioned(
            tenant_id,
            repo_id,
            branch,
            workspace,
            "__name",
            &node.name,
            revision,
            &node.id,
            is_published,
        );
        batch.put_cf(cf_property, name_key, node.id.as_bytes());
    }

    // System property: __archetype
    if let Some(ref archetype) = node.archetype {
        if !archetype.is_empty() {
            let archetype_key = keys::property_index_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                "__archetype",
                archetype,
                revision,
                &node.id,
                is_published,
            );
            batch.put_cf(cf_property, archetype_key, node.id.as_bytes());
        }
    }
}

/// Write forward and reverse reference indexes to a batch
fn write_reference_indexes(
    batch: &mut WriteBatch,
    cf_reference: &rocksdb::ColumnFamily,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node: &Node,
    revision: &raisin_hlc::HLC,
) {
    let is_published = node.published_at.is_some();

    for (prop_path, prop_value) in &node.properties {
        if let PropertyValue::Reference(ref_data) = prop_value {
            // Forward reference
            let fwd_key = keys::reference_forward_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                &node.id,
                prop_path,
                revision,
                is_published,
            );
            batch.put_cf(cf_reference, fwd_key, ref_data.id.as_bytes());

            // Reverse reference
            let rev_key = keys::reference_reverse_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                &ref_data.workspace,
                &ref_data.path,
                &node.id,
                prop_path,
                revision,
                is_published,
            );
            batch.put_cf(cf_reference, rev_key, node.id.as_bytes());
        }
    }
}

/// Write forward and reverse relation indexes to a batch
fn write_relation_indexes(
    batch: &mut WriteBatch,
    cf_relation: &rocksdb::ColumnFamily,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    node: &Node,
    revision: &raisin_hlc::HLC,
) -> Result<()> {
    for relation in &node.relations {
        let relation_bytes = rmp_serde::to_vec(&relation).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to serialize relation: {}", e))
        })?;

        // Forward relation index
        let fwd_key = keys::relation_forward_key_versioned(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &node.id,
            &relation.relation_type,
            revision,
            &relation.target,
        );
        batch.put_cf(cf_relation, fwd_key, &relation_bytes);

        // Reverse relation index
        let rev_key = keys::relation_reverse_key_versioned(
            tenant_id,
            repo_id,
            branch,
            &relation.workspace,
            &relation.target,
            &relation.relation_type,
            revision,
            &node.id,
        );
        batch.put_cf(cf_relation, rev_key, &relation_bytes);
    }
    Ok(())
}

/// Write ORDERED_CHILDREN index and update the metadata cache
#[allow(clippy::too_many_arguments)]
fn write_ordered_children_index(
    batch: &mut WriteBatch,
    applicator: &OperationApplicator,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    workspace: &str,
    parent: &str,
    order_key: &str,
    node_id: &str,
    name: &str,
    revision: &raisin_hlc::HLC,
) -> Result<()> {
    let cf_ordered = cf_handle(&applicator.db, cf::ORDERED_CHILDREN)?;

    let parent_key = if parent.is_empty() || parent == "/" {
        "/"
    } else {
        parent
    };

    let ordered_key = keys::ordered_child_key_versioned(
        tenant_id, repo_id, branch, workspace, parent_key, order_key, revision, node_id,
    );

    batch.put_cf(cf_ordered, ordered_key, name.as_bytes());

    tracing::debug!(
        "Adding to ORDERED_CHILDREN index: parent={}, order_key={}, child={}",
        parent_key,
        order_key,
        node_id
    );

    // Update metadata cache for last child optimization
    let metadata_key =
        keys::last_child_metadata_key(tenant_id, repo_id, branch, workspace, parent_key);

    let should_update =
        if let Ok(Some(cached_value)) = applicator.db.get_cf(cf_ordered, &metadata_key) {
            let cached_label = String::from_utf8_lossy(&cached_value);
            order_key > cached_label.as_ref()
        } else {
            true
        };

    if should_update {
        batch.put_cf(cf_ordered, metadata_key, order_key.as_bytes());

        tracing::debug!(
            "Updating last_child metadata cache: parent={}, order_key={}",
            parent_key,
            order_key
        );
    }

    Ok(())
}
