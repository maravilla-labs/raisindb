//! Legacy node operations: CreateNode, DeleteNode, SetProperty, RenameNode, MoveNode, SetArchetype

use crate::{cf, cf_handle, keys, repositories::hash_property_value};
use raisin_error::Result;
use raisin_models::nodes::Node;
use raisin_replication::Operation;
use raisin_storage::BranchRepository;
use std::collections::HashMap;

use super::{node_workspace, OperationApplicator};

impl OperationApplicator {
    /// Apply a CreateNode operation
    ///
    /// Creates the node with all necessary indexes.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::replication::application) async fn apply_create_node(
        &self,
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
        properties: &HashMap<String, raisin_models::nodes::properties::PropertyValue>,
        owner_id: Option<&str>,
        path: &str,
        op: &Operation,
    ) -> Result<()> {
        tracing::info!(
            op_seq = op.op_seq,
            revision = ?op.revision,
            "📥 RECEIVED CreateNode operation from node {} with revision={:?}",
            op.cluster_node_id,
            op.revision
        );

        tracing::info!(
            "📥 Applying CreateNode: {}/{}/{}/{} from node {}",
            tenant_id,
            repo_id,
            branch,
            node_id,
            op.cluster_node_id
        );

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

        let revision = Self::op_revision(op)?;

        let mut batch = rocksdb::WriteBatch::default();
        let cf_nodes = cf_handle(&self.db, cf::NODES)?;
        let cf_path = cf_handle(&self.db, cf::PATH_INDEX)?;
        let cf_property = cf_handle(&self.db, cf::PROPERTY_INDEX)?;
        let cf_reference = cf_handle(&self.db, cf::REFERENCE_INDEX)?;
        let cf_relation = cf_handle(&self.db, cf::RELATION_INDEX)?;

        let node_value = rmp_serde::to_vec_named(&node)
            .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

        // 1. Store node blob with versioned key
        let node_key =
            keys::node_key_versioned(tenant_id, repo_id, branch, workspace, &node.id, &revision);
        batch.put_cf(cf_nodes, node_key, node_value);

        // 2. Index by path
        let path_key = keys::path_index_key_versioned(
            tenant_id, repo_id, branch, workspace, &node.path, &revision,
        );
        batch.put_cf(cf_path, path_key, node.id.as_bytes());

        // 3. Add property indexes
        let is_published = node.published_at.is_some();
        for (prop_name, prop_value) in &node.properties {
            let value_hash = hash_property_value(prop_value);
            let prop_key = keys::property_index_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                prop_name,
                &value_hash,
                &revision,
                &node.id,
                is_published,
            );
            batch.put_cf(cf_property, prop_key, node.id.as_bytes());
        }

        // 4. Add system property indexes
        let node_type_key = keys::property_index_key_versioned(
            tenant_id,
            repo_id,
            branch,
            workspace,
            "__node_type",
            &node.node_type,
            &revision,
            &node.id,
            is_published,
        );
        batch.put_cf(cf_property, node_type_key, node.id.as_bytes());

        if !node.name.is_empty() {
            let name_key = keys::property_index_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                "__name",
                &node.name,
                &revision,
                &node.id,
                is_published,
            );
            batch.put_cf(cf_property, name_key, node.id.as_bytes());
        }

        if let Some(ref archetype) = node.archetype {
            if !archetype.is_empty() {
                let archetype_key = keys::property_index_key_versioned(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    "__archetype",
                    archetype,
                    &revision,
                    &node.id,
                    is_published,
                );
                batch.put_cf(cf_property, archetype_key, node.id.as_bytes());
            }
        }

        // 5. Add reference indexes
        for (prop_path, prop_value) in &node.properties {
            if let raisin_models::nodes::properties::PropertyValue::Reference(ref_data) = prop_value
            {
                let fwd_key = keys::reference_forward_key_versioned(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &node.id,
                    prop_path,
                    &revision,
                    is_published,
                );
                batch.put_cf(cf_reference, fwd_key, ref_data.id.as_bytes());

                let rev_key = keys::reference_reverse_key_versioned(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &ref_data.workspace,
                    &ref_data.path,
                    &node.id,
                    prop_path,
                    &revision,
                    is_published,
                );
                batch.put_cf(cf_reference, rev_key, node.id.as_bytes());
            }
        }

        // 6. Add relation indexes
        for relation in &node.relations {
            let relation_bytes = rmp_serde::to_vec(&relation).map_err(|e| {
                raisin_error::Error::storage(format!("Failed to serialize relation: {}", e))
            })?;

            let fwd_key = keys::relation_forward_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                &node.id,
                &relation.relation_type,
                &revision,
                &relation.target,
            );
            batch.put_cf(cf_relation, fwd_key, &relation_bytes);

            let rev_key = keys::relation_reverse_key_versioned(
                tenant_id,
                repo_id,
                branch,
                &relation.workspace,
                &relation.target,
                &relation.relation_type,
                &revision,
                &node.id,
            );
            batch.put_cf(cf_relation, rev_key, &relation_bytes);
        }

        // 7. Add ORDERED_CHILDREN index
        if let Some(parent) = parent_id {
            let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;
            let parent_key = if parent.is_empty() || parent == "/" {
                "/"
            } else {
                parent
            };

            let ordered_key = keys::ordered_child_key_versioned(
                tenant_id, repo_id, branch, workspace, parent_key, order_key, &revision, node_id,
            );
            batch.put_cf(cf_ordered, ordered_key, name.as_bytes());

            tracing::debug!(
                "✅ Adding to ORDERED_CHILDREN index: parent={}, order_key={}, child={}",
                parent_key,
                order_key,
                node_id
            );

            // Update metadata cache
            let metadata_key =
                keys::last_child_metadata_key(tenant_id, repo_id, branch, workspace, parent_key);
            let should_update =
                if let Ok(Some(cached_value)) = self.db.get_cf(cf_ordered, &metadata_key) {
                    let cached_label = String::from_utf8_lossy(&cached_value);
                    order_key > cached_label.as_ref()
                } else {
                    true
                };

            if should_update {
                batch.put_cf(cf_ordered, metadata_key, order_key.as_bytes());
            }
        }

        // Atomic commit
        self.db.write(batch).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to apply create_node: {}", e))
        })?;

        tracing::info!(
            "✅ Node created successfully: {} (HEAD will be updated by separate UpdateBranch operation)",
            node_id
        );

        super::super::node_operations::emit_node_event(
            &self.event_bus,
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            Some(node_type.to_string()),
            Some(path.to_string()),
            &revision,
            raisin_events::NodeEventKind::Created,
            "replication",
        );

        Ok(())
    }

    /// Apply a DeleteNode operation
    pub(in crate::replication::application) async fn apply_delete_node(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        node_id: &str,
        op: &Operation,
    ) -> Result<()> {
        let revision = Self::op_revision(op)?;
        tracing::info!(
            "📥 Applying DeleteNode: {}/{}/{}/{} from node {} at revision {}",
            tenant_id,
            repo_id,
            branch,
            node_id,
            op.cluster_node_id,
            revision
        );

        let node_snapshot = match self.load_latest_node(tenant_id, repo_id, branch, node_id)? {
            Some(node) => node,
            None => {
                tracing::warn!(
                    "DeleteNode skipped: node {} not found when applying replication delete",
                    node_id
                );
                return Ok(());
            }
        };

        let workspace = node_workspace(&node_snapshot);
        let parent_id = self.resolve_parent_id_for_snapshot(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &node_snapshot,
        )?;

        self.apply_replicated_delete(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &node_snapshot,
            parent_id.as_deref(),
            &revision,
        )?;

        self.branch_repo
            .update_head(tenant_id, repo_id, branch, revision)
            .await?;

        tracing::info!(
            "✅ Node deleted successfully: {} (branch HEAD updated to revision {})",
            node_id,
            revision
        );

        super::super::node_operations::emit_node_event(
            &self.event_bus,
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            Some(node_snapshot.node_type.clone()),
            Some(node_snapshot.path.clone()),
            &revision,
            raisin_events::NodeEventKind::Deleted,
            "replication",
        );

        Ok(())
    }

    /// Apply a SetProperty operation
    ///
    /// Reads the latest node, updates the property, and writes a new revision.
    pub(in crate::replication::application) async fn apply_set_property(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        node_id: &str,
        property_name: &str,
        value: &raisin_models::nodes::properties::PropertyValue,
        op: &Operation,
    ) -> Result<()> {
        tracing::debug!(
            "📥 Applying SetProperty: {} -> {:?} on node {}",
            property_name,
            value,
            node_id
        );

        let new_revision = Self::op_revision(op)?;
        let prefix = keys::node_key_prefix(tenant_id, repo_id, branch, "default", node_id);
        let cf_nodes = cf_handle(&self.db, cf::NODES)?;

        let mut iter = self.db.iterator_cf(
            cf_nodes,
            rocksdb::IteratorMode::From(&prefix, rocksdb::Direction::Forward),
        );

        let mut latest_node: Option<Node> = None;
        while let Some(Ok((key, value))) = iter.next() {
            if !key.starts_with(&prefix) {
                break;
            }
            if let Ok(node) = rmp_serde::from_slice::<Node>(&value) {
                latest_node = Some(node);
                break;
            }
        }

        let mut node = match latest_node {
            Some(n) => n,
            None => {
                tracing::warn!(
                    "Cannot apply SetProperty: node {} not found in database",
                    node_id
                );
                return Ok(());
            }
        };

        node.properties
            .insert(property_name.to_string(), value.clone());

        use chrono::DateTime;
        let timestamp = DateTime::from_timestamp_millis(op.timestamp_ms as i64);
        node.updated_at = timestamp;
        node.updated_by = Some(op.actor.clone());
        node.version += 1;

        let key = keys::node_key_versioned(
            tenant_id,
            repo_id,
            branch,
            "default",
            node_id,
            &new_revision,
        );
        let value = rmp_serde::to_vec_named(&node)
            .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

        self.db
            .put_cf(cf_nodes, key, value)
            .map_err(|e| raisin_error::Error::storage(e.to_string()))?;

        self.branch_repo
            .update_head(tenant_id, repo_id, branch, new_revision)
            .await?;

        tracing::info!(
            "✅ SetProperty applied: property={}, node={} (branch HEAD updated to revision {})",
            property_name,
            node_id,
            new_revision
        );

        let workspace = node.workspace.as_deref().unwrap_or("default");
        super::super::node_operations::emit_node_event(
            &self.event_bus,
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_id,
            Some(node.node_type.clone()),
            Some(node.path.clone()),
            &new_revision,
            raisin_events::NodeEventKind::Updated,
            "replication",
        );

        Ok(())
    }

    /// Apply a RenameNode operation
    pub(in crate::replication::application) async fn apply_rename_node(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _branch: &str,
        node_id: &str,
        new_name: &str,
        _op: &Operation,
    ) -> Result<()> {
        tracing::info!("📥 Applying RenameNode: {} -> {}", node_id, new_name);
        // Simplified implementation
        Ok(())
    }

    /// Apply a SetArchetype operation
    pub(in crate::replication::application) async fn apply_set_archetype(
        &self,
        _tenant_id: &str,
        _repo_id: &str,
        _branch: &str,
        node_id: &str,
        new_archetype: Option<&str>,
        _op: &Operation,
    ) -> Result<()> {
        tracing::info!(
            "📥 Applying SetArchetype: {} -> {:?}",
            node_id,
            new_archetype
        );
        // Simplified implementation
        Ok(())
    }
}
