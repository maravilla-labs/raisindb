//! CRDT operations: replicated revision, upsert/delete snapshots

use crate::{cf, cf_handle, fractional_index, keys, repositories::hash_property_value};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_replication::{
    operation::{ReplicatedNodeChange, ReplicatedNodeChangeKind},
    Operation,
};
use raisin_storage::BranchRepository;
use rocksdb::WriteBatch;
use std::collections::HashMap;

use super::super::index_writers::write_all_node_indexes;
use super::{is_tombstone, OperationApplicator, TOMBSTONE};

impl OperationApplicator {
    /// Apply a replicated revision (batch of node changes with branch head update)
    pub(in crate::replication::application) async fn apply_replicated_revision(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        branch_head: &HLC,
        node_changes: &[ReplicatedNodeChange],
        op: &Operation,
    ) -> Result<()> {
        let revision = Self::op_revision(op)?;

        for change in node_changes {
            let workspace = change.node.workspace.as_deref().unwrap_or("default");
            match change.kind {
                ReplicatedNodeChangeKind::Upsert => self.apply_replicated_upsert(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &change.node,
                    change.parent_id.as_deref(),
                    &revision,
                    &change.cf_order_key,
                )?,
                ReplicatedNodeChangeKind::Delete => self.apply_replicated_delete(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    &change.node,
                    change.parent_id.as_deref(),
                    &revision,
                )?,
            }
        }

        self.branch_repo
            .update_head(tenant_id, repo_id, branch, *branch_head)
            .await?;

        Ok(())
    }

    /// Apply a single replicated node upsert
    pub(in crate::replication::application) fn apply_replicated_upsert(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node: &Node,
        parent_id: Option<&str>,
        revision: &HLC,
        cf_order_key: &str,
    ) -> Result<()> {
        let mut normalized_node = node.clone();
        normalized_node.has_children = None;

        // Determine CF order key to use
        let cf_key_to_use = if !cf_order_key.is_empty() {
            cf_order_key.to_string()
        } else if let Some(pid) = parent_id {
            tracing::warn!(
                node_id = %normalized_node.id,
                "⚠️ REPLICATION BUG: cf_order_key is empty - falling back to local generation"
            );
            self.allocate_order_label(tenant_id, repo_id, branch, workspace, pid)?
        } else {
            String::new()
        };

        tracing::debug!(
            node_id = %normalized_node.id,
            cf_key = %cf_key_to_use,
            "📥 Applying CF order key from replication"
        );

        let mut batch = WriteBatch::default();
        let cf_nodes = cf_handle(&self.db, cf::NODES)?;
        let cf_path = cf_handle(&self.db, cf::PATH_INDEX)?;
        let cf_property = cf_handle(&self.db, cf::PROPERTY_INDEX)?;
        let cf_reference = cf_handle(&self.db, cf::REFERENCE_INDEX)?;
        let cf_relation = cf_handle(&self.db, cf::RELATION_INDEX)?;
        let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;

        let node_value = rmp_serde::to_vec_named(&normalized_node)
            .map_err(|e| raisin_error::Error::storage(format!("Serialization error: {}", e)))?;

        let node_key = keys::node_key_versioned(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &normalized_node.id,
            revision,
        );
        batch.put_cf(cf_nodes, node_key, node_value);

        let path_key = keys::path_index_key_versioned(
            tenant_id,
            repo_id,
            branch,
            workspace,
            &normalized_node.path,
            revision,
        );
        batch.put_cf(cf_path, path_key, normalized_node.id.as_bytes());

        // Use index writer helpers to write all indexes
        write_all_node_indexes(
            &mut batch,
            cf_property,
            cf_reference,
            cf_relation,
            tenant_id,
            repo_id,
            branch,
            workspace,
            &normalized_node,
            revision,
        )?;

        if let Some(pid) = parent_id {
            if cf_key_to_use.is_empty() {
                tracing::warn!(
                    node_id = %normalized_node.id,
                    parent_id = %pid,
                    "⚠️ Skipping ORDERED_CHILDREN update due to missing cf_order_key"
                );
            } else {
                let ordered_key = keys::ordered_child_key_versioned(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    pid,
                    &cf_key_to_use,
                    revision,
                    &normalized_node.id,
                );
                batch.put_cf(cf_ordered, ordered_key, normalized_node.name.as_bytes());

                let metadata_key =
                    keys::last_child_metadata_key(tenant_id, repo_id, branch, workspace, pid);
                let should_update = match self.db.get_cf(cf_ordered, &metadata_key) {
                    Ok(Some(existing)) => {
                        let existing_label = String::from_utf8_lossy(&existing);
                        cf_key_to_use.as_str() > existing_label.as_ref()
                    }
                    _ => true,
                };

                if should_update {
                    batch.put_cf(cf_ordered, metadata_key, cf_key_to_use.as_bytes());
                }
            }
        }

        self.db.write(batch).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to apply replicated upsert: {}", e))
        })?;

        super::super::node_operations::emit_node_event(
            &self.event_bus,
            tenant_id,
            repo_id,
            branch,
            workspace,
            &normalized_node.id,
            Some(normalized_node.node_type.clone()),
            Some(normalized_node.path.clone()),
            revision,
            raisin_events::NodeEventKind::Updated,
            "replication",
        );

        Ok(())
    }

    /// Apply a single replicated node delete
    pub(in crate::replication::application) fn apply_replicated_delete(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        node: &Node,
        parent_id: Option<&str>,
        revision: &HLC,
    ) -> Result<()> {
        let mut batch = WriteBatch::default();
        let cf_nodes = cf_handle(&self.db, cf::NODES)?;
        let cf_path = cf_handle(&self.db, cf::PATH_INDEX)?;
        let cf_property = cf_handle(&self.db, cf::PROPERTY_INDEX)?;
        let cf_reference = cf_handle(&self.db, cf::REFERENCE_INDEX)?;
        let cf_relation = cf_handle(&self.db, cf::RELATION_INDEX)?;
        let cf_ordered = cf_handle(&self.db, cf::ORDERED_CHILDREN)?;
        let cf_translation = cf_handle(&self.db, cf::TRANSLATION_DATA)?;

        let node_key =
            keys::node_key_versioned(tenant_id, repo_id, branch, workspace, &node.id, revision);
        batch.put_cf(cf_nodes, node_key, TOMBSTONE);

        let path_key = keys::path_index_key_versioned(
            tenant_id, repo_id, branch, workspace, &node.path, revision,
        );
        batch.put_cf(cf_path, path_key, TOMBSTONE);

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
                revision,
                &node.id,
                is_published,
            );
            batch.put_cf(cf_property, prop_key, TOMBSTONE);
        }

        let mut tombstone_field = |field: &str, value: &str| {
            if value.is_empty() {
                return;
            }
            let key = keys::property_index_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                field,
                value,
                revision,
                &node.id,
                is_published,
            );
            batch.put_cf(cf_property, key, TOMBSTONE);
        };

        tombstone_field("__node_type", &node.node_type);
        tombstone_field("__name", &node.name);
        if let Some(ref archetype) = node.archetype {
            tombstone_field("__archetype", archetype);
        }
        if let Some(ref created_by) = node.created_by {
            tombstone_field("__created_by", created_by);
        }
        if let Some(ref updated_by) = node.updated_by {
            tombstone_field("__updated_by", updated_by);
        }
        // Write timestamp tombstones using microsecond precision
        if let Some(created_at) = node.created_at {
            let key = keys::property_index_key_versioned_timestamp(
                tenant_id,
                repo_id,
                branch,
                workspace,
                "__created_at",
                created_at.timestamp_micros(),
                revision,
                &node.id,
                is_published,
            );
            batch.put_cf(cf_property, key, TOMBSTONE);
        }
        if let Some(updated_at) = node.updated_at {
            let key = keys::property_index_key_versioned_timestamp(
                tenant_id,
                repo_id,
                branch,
                workspace,
                "__updated_at",
                updated_at.timestamp_micros(),
                revision,
                &node.id,
                is_published,
            );
            batch.put_cf(cf_property, key, TOMBSTONE);
        }

        for (prop_path, prop_value) in &node.properties {
            if let PropertyValue::Reference(ref_data) = prop_value {
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
                batch.put_cf(cf_reference, fwd_key, TOMBSTONE);

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
                batch.put_cf(cf_reference, rev_key, TOMBSTONE);
            }
        }

        let outgoing_relations =
            self.collect_outgoing_relations(tenant_id, repo_id, branch, workspace, &node.id)?;
        for relation in outgoing_relations {
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
            batch.put_cf(cf_relation, fwd_key, TOMBSTONE);

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
            batch.put_cf(cf_relation, rev_key, TOMBSTONE);
        }

        let incoming_relations =
            self.collect_incoming_relations(tenant_id, repo_id, branch, workspace, &node.id)?;
        for (source_node_id, relation_type, source_workspace) in incoming_relations {
            let fwd_key = keys::relation_forward_key_versioned(
                tenant_id,
                repo_id,
                branch,
                &source_workspace,
                &source_node_id,
                &relation_type,
                revision,
                &node.id,
            );
            batch.put_cf(cf_relation, fwd_key, TOMBSTONE);

            let rev_key = keys::relation_reverse_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                &node.id,
                &relation_type,
                revision,
                &source_node_id,
            );
            batch.put_cf(cf_relation, rev_key, TOMBSTONE);
        }

        if let Some(pid) = parent_id {
            let ordered_key = keys::ordered_child_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                pid,
                &node.order_key,
                revision,
                &node.id,
            );
            batch.put_cf(cf_ordered, ordered_key, TOMBSTONE);
        }

        let translation_locales =
            self.list_translation_locales(tenant_id, repo_id, branch, workspace, &node.id)?;
        for locale in translation_locales {
            let mut translation_key = format!(
                "{}\0{}\0{}\0{}\0translations\0{}\0{}\0",
                tenant_id, repo_id, branch, workspace, node.id, locale
            )
            .into_bytes();
            translation_key.extend_from_slice(&keys::encode_descending_revision(revision));
            batch.put_cf(cf_translation, translation_key, TOMBSTONE);
        }

        self.db.write(batch).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to apply replicated delete: {}", e))
        })?;

        super::super::node_operations::emit_node_event(
            &self.event_bus,
            tenant_id,
            repo_id,
            branch,
            workspace,
            &node.id,
            Some(node.node_type.clone()),
            Some(node.path.clone()),
            revision,
            raisin_events::NodeEventKind::Deleted,
            "replication",
        );

        Ok(())
    }

    /// Apply a node snapshot upsert (decomposed from ApplyRevision for CRDT commutativity)
    ///
    /// Uses Last-Write-Wins (LWW) semantics via the revision HLC.
    pub(in crate::replication::application) async fn apply_upsert_node_snapshot(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        node: &Node,
        parent_id: Option<&str>,
        revision: &HLC,
        cf_order_key: &str,
        _op: &Operation,
    ) -> Result<()> {
        let workspace = node.workspace.as_deref().unwrap_or("default");

        self.apply_replicated_upsert(
            tenant_id,
            repo_id,
            branch,
            workspace,
            node,
            parent_id,
            revision,
            cf_order_key,
        )?;

        tracing::debug!(
            node_id = %node.id,
            revision = ?revision,
            "Applied UpsertNodeSnapshot with LWW semantics"
        );

        Ok(())
    }

    /// Apply a node snapshot delete (decomposed from ApplyRevision for CRDT commutativity)
    ///
    /// Uses Delete-Wins semantics - deletions always take precedence.
    pub(in crate::replication::application) async fn apply_delete_node_snapshot(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        node_id: &str,
        revision: &HLC,
        _op: &Operation,
    ) -> Result<()> {
        let node = match self.load_latest_node(tenant_id, repo_id, branch, node_id)? {
            Some(n) => n,
            None => {
                tracing::debug!(
                    node_id = %node_id,
                    revision = ?revision,
                    "Node not found for DeleteNodeSnapshot - treating as already deleted"
                );
                return Ok(());
            }
        };

        let workspace = node.workspace.as_deref().unwrap_or("default");

        // We use None for parent_id - delete logic handles this gracefully
        let _parent_id: Option<&str> = None;

        self.apply_replicated_delete(tenant_id, repo_id, branch, workspace, &node, None, revision)?;

        tracing::debug!(
            node_id = %node_id,
            revision = ?revision,
            "Applied DeleteNodeSnapshot with Delete-Wins semantics"
        );

        Ok(())
    }
}
