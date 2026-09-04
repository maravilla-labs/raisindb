//! Node deletion and schema change handling
//!
//! Handles node delete events (fulltext deletion, embedding deletion,
//! cleanup jobs, trigger evaluation) and schema change events.

use super::UnifiedJobEventHandler;
use raisin_error::Result;
use raisin_events::NodeEvent;
use raisin_storage::jobs::{IndexOperation, JobContext, JobType};
use std::collections::HashMap;

impl UnifiedJobEventHandler {
    /// Handle node deletion events
    pub(crate) async fn handle_node_delete(&self, node_event: &NodeEvent) -> Result<()> {
        let is_remote_event = Self::is_remote_event(node_event);

        let context = Self::build_job_context(node_event);

        // Always enqueue fulltext deletion job
        if let Err(e) = self
            .enqueue_fulltext_job(&node_event.node_id, IndexOperation::Delete, &context)
            .await
        {
            tracing::error!(
                error = %e,
                node_id = %node_event.node_id,
                "Failed to enqueue fulltext deletion job"
            );
        }

        // Check if embeddings are enabled for this tenant
        if self.embeddings_enabled(&node_event.tenant_id).await? {
            if let Err(e) = self
                .enqueue_job(
                    JobType::EmbeddingDelete {
                        node_id: node_event.node_id.clone(),
                    },
                    &context,
                )
                .await
            {
                tracing::error!(
                    error = %e,
                    node_id = %node_event.node_id,
                    "Failed to enqueue embedding deletion job"
                );
            }
        }

        // Enqueue node delete cleanup job to tombstone global relation indexes
        if let Err(e) = self
            .enqueue_job(
                JobType::NodeDeleteCleanup {
                    node_id: node_event.node_id.clone(),
                    workspace: node_event.workspace_id.clone(),
                },
                &context,
            )
            .await
        {
            tracing::error!(
                error = %e,
                node_id = %node_event.node_id,
                "Failed to enqueue node delete cleanup job"
            );
        }

        // Virtual-mount writeback capture for delete events - LOCAL only, for
        // the same reason the create/update arm is: a replicated delete must not
        // make every replica report the same removal to the provider.
        if !is_remote_event {
            self.capture_virtual_delete(node_event).await;
        }

        // Trigger evaluation for delete events - only for LOCAL events
        if !is_remote_event {
            if let Err(e) = self.enqueue_trigger_evaluation(node_event, "Deleted").await {
                tracing::error!(
                    error = %e,
                    node_id = %node_event.node_id,
                    event_type = "Deleted",
                    "Failed to enqueue trigger evaluation job"
                );
            }
        }

        Ok(())
    }

    /// Handle schema change events (NodeType, Archetype, ElementType)
    pub(crate) async fn handle_schema_change(
        &self,
        schema_event: &raisin_events::SchemaEvent,
    ) -> Result<()> {
        tracing::info!(
            schema_id = %schema_event.schema_id,
            schema_type = %schema_event.schema_type,
            kind = ?schema_event.kind,
            tenant_id = %schema_event.tenant_id,
            repo_id = %schema_event.repository_id,
            branch = %schema_event.branch,
            "Schema change event received"
        );

        // TODO: Future enhancements:
        // - Rebuild fulltext indexes if NodeType.indexable or index_types changed
        // - Invalidate cached NodeType/Archetype/ElementType schemas
        // - Queue property index rebuilding if NodeType properties changed
        // - Trigger webhook notifications (for local events only)

        Ok(())
    }

    /// Check if an event originated from replication (remote)
    pub(super) fn is_remote_event(node_event: &NodeEvent) -> bool {
        node_event
            .metadata
            .as_ref()
            .and_then(|m| m.get("source"))
            .and_then(|v| v.as_str())
            .map(|s| s == "replication")
            .unwrap_or(false)
    }

    /// Is this write engine bookkeeping — operational metadata, not content?
    ///
    /// Set by `TransactionalContext::set_bookkeeping`, whose contract is that
    /// such a commit stays durable, versioned, replicated and observable, but
    /// skips the CONTENT fan-out that is meaningless for a state blob and
    /// ruinous at bookkeeping write rates.
    ///
    /// Reindexing is that fan-out. A virtual mount evicting its content cache
    /// and refetching it moves `file` / `content_hash` / `__content_cached_at`
    /// and nothing a vector is built from — yet each of those writes minted a
    /// revision, which enqueued an embedding job, which re-derived the same
    /// text, hashed it, found it identical and reused the vectors it already
    /// had. Measured on one tenant: 2,233 of 2,339 embedding jobs in half an
    /// hour were that no-op, and every one still rewrote its rows.
    ///
    /// Content writes never carry the marker, so a real edit is unaffected.
    pub(super) fn is_bookkeeping_event(node_event: &NodeEvent) -> bool {
        node_event
            .metadata
            .as_ref()
            .and_then(|m| m.get("bookkeeping"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    /// Build a JobContext from a NodeEvent
    pub(super) fn build_job_context(node_event: &NodeEvent) -> JobContext {
        JobContext {
            tenant_id: node_event.tenant_id.clone(),
            repo_id: node_event.repository_id.clone(),
            branch: node_event.branch.clone(),
            workspace_id: node_event.workspace_id.clone(),
            revision: node_event.revision,
            metadata: HashMap::new(),
        }
    }
}
