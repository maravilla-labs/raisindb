//! Event emission and snapshot job enqueueing

use super::super::RocksDBTransaction;
use crate::transaction::change_types::{ChangedNodesMap, ChangedTranslationsMap};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_storage::StorageScope;

impl RocksDBTransaction {
    /// PHASE 5.5: Enqueue async snapshot creation job
    pub(in crate::transaction) async fn enqueue_snapshot_job(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch_name: &str,
        new_revision: &HLC,
        changed_nodes: &ChangedNodesMap,
        changed_translations: &ChangedTranslationsMap,
    ) -> Result<()> {
        if changed_nodes.is_empty() && changed_translations.is_empty() {
            return Ok(());
        }

        // Prepare node change info for job metadata
        let node_changes: Vec<crate::jobs::NodeChangeInfo> = changed_nodes
            .iter()
            .map(|(node_id, change)| crate::jobs::NodeChangeInfo {
                node_id: node_id.clone(),
                workspace: change.workspace.clone(),
            })
            .collect();

        // Prepare translation change info for job metadata
        let translation_changes: Vec<crate::jobs::TranslationChangeInfo> = changed_translations
            .iter()
            .map(
                |((node_id, locale), change)| crate::jobs::TranslationChangeInfo {
                    node_id: node_id.clone(),
                    locale: locale.clone(),
                    workspace: change.workspace.clone(),
                },
            )
            .collect();

        // Create job context with metadata
        let mut metadata = std::collections::HashMap::new();
        metadata.insert(
            "changed_nodes".to_string(),
            serde_json::to_value(&node_changes).map_err(|e| {
                raisin_error::Error::storage(format!("Failed to serialize node changes: {}", e))
            })?,
        );
        metadata.insert(
            "changed_translations".to_string(),
            serde_json::to_value(&translation_changes).map_err(|e| {
                raisin_error::Error::storage(format!(
                    "Failed to serialize translation changes: {}",
                    e
                ))
            })?,
        );

        let job_context = raisin_storage::jobs::JobContext {
            tenant_id: tenant_id.to_string(),
            repo_id: repo_id.to_string(),
            branch: branch_name.to_string(),
            workspace_id: "<snapshot>".to_string(),
            revision: *new_revision,
            metadata,
        };

        // Register job in job registry
        let job_id = self
            .job_registry
            .register_job(
                raisin_storage::jobs::JobType::TreeSnapshot {
                    revision: *new_revision,
                },
                Some(tenant_id.to_string()),
                None,
                None,
                None,
            )
            .await?;

        // Store job context for worker processing
        self.job_data_store.put(&job_id, &job_context)?;

        tracing::debug!(
            job_id = %job_id,
            revision = %new_revision,
            changed_nodes = changed_nodes.len(),
            changed_translations = changed_translations.len(),
            "Enqueued async tree snapshot creation job"
        );

        Ok(())
    }

    /// PHASE 6: Emit NodeEvent for each changed node
    pub(in crate::transaction) async fn emit_node_events(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch_name: &str,
        changed_nodes: &ChangedNodesMap,
    ) {
        use raisin_storage::NodeRepository;

        for (node_id, change) in changed_nodes.iter() {
            let workspace = &change.workspace;
            let revision = &change.revision;
            let operation = &change.operation;
            let stored_path = &change.path;
            let stored_node_type = &change.node_type;
            use raisin_events::{NodeEvent, NodeEventKind};
            use raisin_models::tree::ChangeOperation;

            // Map operation to proper event kind
            let event_kind = match operation {
                ChangeOperation::Added => NodeEventKind::Created,
                ChangeOperation::Modified => NodeEventKind::Updated,
                ChangeOperation::Deleted => NodeEventKind::Deleted,
                ChangeOperation::Reordered => NodeEventKind::Reordered,
            };

            tracing::debug!(
                "Emitting NodeEvent: kind={:?}, node_id={}, workspace={}, revision={}, stored_path={:?}, stored_node_type={:?}",
                event_kind,
                node_id,
                workspace,
                revision,
                stored_path,
                stored_node_type
            );

            // For delete events, use stored path and node_type
            // For create/update events, fetch the node to get the latest data
            let (node_path, node_node_type, node_data) =
                if matches!(event_kind, NodeEventKind::Deleted) {
                    (stored_path.clone(), stored_node_type.clone(), None)
                } else {
                    match self
                        .node_repo
                        .get(
                            StorageScope::new(tenant_id, repo_id, branch_name, workspace),
                            node_id,
                            Some(revision),
                        )
                        .await
                    {
                        Ok(Some(node)) => {
                            let path = Some(node.path.clone());
                            let node_type = Some(node.node_type.clone());
                            let node_json = serde_json::to_value(&node).ok();
                            (path, node_type, node_json)
                        }
                        Ok(None) => {
                            tracing::warn!(
                            "Node not found after commit: node_id={}, workspace={}, revision={}",
                            node_id,
                            workspace,
                            revision
                        );
                            (stored_path.clone(), stored_node_type.clone(), None)
                        }
                        Err(e) => {
                            tracing::error!(
                                "Failed to fetch node for event: node_id={}, error={}",
                                node_id,
                                e
                            );
                            (stored_path.clone(), stored_node_type.clone(), None)
                        }
                    }
                };

            // Add metadata to indicate this is a local event (not from replication)
            let mut metadata = std::collections::HashMap::new();
            metadata.insert(
                "source".to_string(),
                serde_json::Value::String("local".to_string()),
            );
            if let Some(data) = node_data {
                metadata.insert("node_data".to_string(), data);
            }

            let event = raisin_events::NodeEvent {
                tenant_id: tenant_id.to_string(),
                repository_id: repo_id.to_string(),
                branch: branch_name.to_string(),
                workspace_id: workspace.clone(),
                node_id: node_id.clone(),
                node_type: node_node_type,
                revision: *revision,
                kind: event_kind,
                path: node_path,
                metadata: Some(metadata),
            };

            self.event_bus.publish(raisin_events::Event::Node(event));
        }
    }
}
