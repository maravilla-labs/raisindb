//! Node creation and update event handling
//!
//! Handles node create/update events by routing to appropriate job types:
//! fulltext indexing, trigger evaluation, AI tool calls, embedding generation,
//! and asset processing.

use super::UnifiedJobEventHandler;
use raisin_error::Result;
use raisin_events::{NodeEvent, NodeEventKind};
use raisin_storage::jobs::{IndexOperation, JobContext, JobType};
use raisin_storage::{NodeRepository, Storage, StorageScope};

impl UnifiedJobEventHandler {
    /// Handle node creation/update events
    pub(crate) async fn handle_node_change(&self, node_event: &NodeEvent) -> Result<()> {
        // Check if this is a trigger-related node change that requires cache invalidation
        // This must happen BEFORE the quick-reject check
        let node_type_str = node_event.node_type.as_deref().unwrap_or("");

        if node_event.workspace_id == "functions"
            && (node_type_str == "raisin:Function" || node_type_str == "raisin:Trigger")
        {
            if let Some(registry) = &self.trigger_registry {
                tracing::info!(
                    node_type = %node_type_str,
                    node_id = %node_event.node_id,
                    "Trigger definition changed, invalidating registry"
                );
                if let Err(e) = registry
                    .invalidate(
                        &node_event.tenant_id,
                        &node_event.repository_id,
                        &node_event.branch,
                    )
                    .await
                {
                    tracing::error!(error = %e, "Failed to invalidate trigger registry");
                }
            }
        }

        let is_remote_event = Self::is_remote_event(node_event);

        tracing::debug!(
            node_id = %node_event.node_id,
            source = if is_remote_event { "replication" } else { "local" },
            "Processing node change event"
        );

        let context = Self::build_job_context(node_event);

        // Try to get node_data from event metadata (avoids DB read for index check)
        let node_from_metadata: Option<raisin_models::nodes::Node> = node_event
            .metadata
            .as_ref()
            .and_then(|m| m.get("node_data"))
            .and_then(|v| serde_json::from_value(v.clone()).ok());

        // Get index settings - uses node_data if available
        let index_settings = self
            .get_index_settings(
                &node_event.tenant_id,
                &node_event.repository_id,
                &node_event.branch,
                &node_event.workspace_id,
                &node_event.node_id,
                node_from_metadata.as_ref(),
            )
            .await;

        // Fulltext indexing runs for BOTH local and remote events
        if index_settings.fulltext {
            if let Err(e) = self
                .enqueue_fulltext_job(&node_event.node_id, IndexOperation::AddOrUpdate, &context)
                .await
            {
                tracing::error!(
                    error = %e,
                    node_id = %node_event.node_id,
                    "Failed to enqueue fulltext indexing job"
                );
            }
        } else {
            tracing::debug!(
                node_id = %node_event.node_id,
                "Skipping fulltext index for node (NodeType not indexable or Fulltext not in index_types)"
            );
        }

        // Trigger evaluation and AI jobs - only for LOCAL events
        if !is_remote_event {
            self.handle_local_node_change(node_event, &context).await;
        }

        // Embedding generation - only for LOCAL events
        if !is_remote_event {
            self.handle_embedding_generation(node_event, &context, &index_settings)
                .await?;
        } else {
            tracing::debug!(
                node_id = %node_event.node_id,
                "Skipping vector embedding for replicated event (embeddings are replicated separately)"
            );
        }

        // Asset processing - only for LOCAL events
        if !is_remote_event {
            self.handle_asset_processing(node_event, &context, node_from_metadata.as_ref())
                .await;
        }

        Ok(())
    }

    /// Handle local-only aspects of node change (triggers, AI tool calls)
    async fn handle_local_node_change(&self, node_event: &NodeEvent, context: &JobContext) {
        let event_type = match &node_event.kind {
            NodeEventKind::Created => "Created",
            NodeEventKind::Updated => "Updated",
            _ => "Unknown",
        };

        if let Err(e) = self
            .enqueue_trigger_evaluation(node_event, event_type)
            .await
        {
            tracing::error!(
                error = %e,
                node_id = %node_event.node_id,
                event_type = %event_type,
                "Failed to enqueue trigger evaluation job"
            );
        }

        // OOTB AIToolCall execution - only for Created events
        if matches!(node_event.kind, NodeEventKind::Created) {
            self.handle_ai_tool_call_events(node_event, context).await;
        }
    }

    /// Handle AI tool call and result aggregation events
    async fn handle_ai_tool_call_events(&self, node_event: &NodeEvent, _context: &JobContext) {
        let node_type = node_event
            .node_type
            .clone()
            .or_else(|| {
                node_event
                    .metadata
                    .as_ref()
                    .and_then(|m| m.get("node_type"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_default();

        if node_type == "raisin:AIToolCall" {
            let tool_call_path = node_event.path.clone().unwrap_or_default();
            if !tool_call_path.is_empty() {
                if let Err(e) = self
                    .enqueue_ai_tool_call_execution(node_event, &tool_call_path)
                    .await
                {
                    tracing::error!(
                        error = %e,
                        node_id = %node_event.node_id,
                        tool_call_path = %tool_call_path,
                        "Failed to enqueue AIToolCall execution job"
                    );
                }
            }
        }

        if node_type == "raisin:AIToolSingleCallResult" {
            let result_path = node_event.path.clone().unwrap_or_default();
            if !result_path.is_empty() {
                if let Err(e) = self
                    .enqueue_ai_tool_result_aggregation(node_event, &result_path)
                    .await
                {
                    tracing::error!(
                        error = %e,
                        node_id = %node_event.node_id,
                        result_path = %result_path,
                        "Failed to enqueue AIToolResult aggregation job"
                    );
                }
            }
        }
    }

    /// Handle embedding generation for node changes
    async fn handle_embedding_generation(
        &self,
        node_event: &NodeEvent,
        context: &JobContext,
        index_settings: &super::index_helpers::IndexSettings,
    ) -> Result<()> {
        if self.embeddings_enabled(&node_event.tenant_id).await? && index_settings.vector {
            if let Err(e) = self
                .enqueue_job(
                    JobType::EmbeddingGenerate {
                        node_id: node_event.node_id.clone(),
                    },
                    context,
                )
                .await
            {
                tracing::error!(
                    error = %e,
                    node_id = %node_event.node_id,
                    "Failed to enqueue embedding generation job"
                );
            }
        } else {
            tracing::debug!(
                node_id = %node_event.node_id,
                "Skipping vector embedding for node (tenant disabled or NodeType not indexable for Vector)"
            );
        }
        Ok(())
    }

    /// Handle asset processing for node changes
    async fn handle_asset_processing(
        &self,
        node_event: &NodeEvent,
        context: &JobContext,
        node_from_metadata: Option<&raisin_models::nodes::Node>,
    ) {
        if let Some(node) = node_from_metadata {
            self.enqueue_asset_processing_if_needed(node_event, context, node)
                .await;
        } else {
            let node_type = node_event.node_type.as_deref().unwrap_or("");
            if node_type == "raisin:Asset" {
                self.fetch_and_process_asset(node_event, context).await;
            }
        }
    }

    /// Enqueue asset processing if the node qualifies
    async fn enqueue_asset_processing_if_needed(
        &self,
        node_event: &NodeEvent,
        context: &JobContext,
        node: &raisin_models::nodes::Node,
    ) {
        if !self.should_process_asset(node) {
            return;
        }

        let options = self.get_asset_processing_options(node, context).await;
        let mime_type = self.extract_mime_type(node);

        if options.extract_pdf_text
            || options.generate_image_embedding
            || options.generate_image_caption
        {
            tracing::info!(
                node_id = %node_event.node_id,
                mime_type = ?mime_type,
                content_hash = ?options.content_hash,
                pdf = %options.extract_pdf_text,
                clip = %options.generate_image_embedding,
                blip = %options.generate_image_caption,
                "Enqueuing AssetProcessing job"
            );

            if let Err(e) = self
                .enqueue_job(
                    JobType::AssetProcessing {
                        node_id: node_event.node_id.clone(),
                        options,
                    },
                    context,
                )
                .await
            {
                tracing::error!(
                    error = %e,
                    node_id = %node_event.node_id,
                    "Failed to enqueue asset processing job"
                );
            }
        }
    }

    /// Fetch a node from storage and process it as an asset if applicable
    async fn fetch_and_process_asset(&self, node_event: &NodeEvent, context: &JobContext) {
        if let Ok(Some(node)) = self
            .storage
            .nodes()
            .get(
                StorageScope::new(
                    &node_event.tenant_id,
                    &node_event.repository_id,
                    &node_event.branch,
                    &node_event.workspace_id,
                ),
                &node_event.node_id,
                None,
            )
            .await
        {
            self.enqueue_asset_processing_if_needed(node_event, context, &node)
                .await;
        }
    }
}
