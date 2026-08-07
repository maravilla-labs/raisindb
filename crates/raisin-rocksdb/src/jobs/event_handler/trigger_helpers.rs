//! Trigger evaluation helpers for the event handler
//!
//! Provides methods for enqueuing trigger evaluation, AI tool call execution,
//! and AI tool result aggregation jobs.

use super::UnifiedJobEventHandler;
use crate::jobs::ORIGIN_AGENT_KEY;
use raisin_error::Result;
use raisin_events::NodeEvent;
use raisin_storage::jobs::{JobContext, JobType};
use std::collections::HashMap;

/// Carry the initiating principal of the write that produced `node_event` into
/// the job it is about to enqueue.
///
/// Without this the chain breaks at the node → event → job hop: the commit
/// stamped `agent` on the event (see `transaction/commit/events.rs`) but the job
/// context was rebuilt from scratch, so anything the job went on to write was
/// attributable to nothing. Copying it lets the job compose its own identity on
/// top (`agent:/agents/bot@trigger:/triggers/t`).
fn carry_origin_agent(node_event: &NodeEvent, metadata: &mut HashMap<String, serde_json::Value>) {
    if let Some(agent) = node_event
        .metadata
        .as_ref()
        .and_then(|m| m.get("agent"))
        .and_then(|v| v.as_str())
    {
        metadata.insert(ORIGIN_AGENT_KEY.to_string(), serde_json::json!(agent));
    }
}

impl UnifiedJobEventHandler {
    /// Enqueue trigger evaluation job for a node event
    ///
    /// This queues a TriggerEvaluation job which will find matching triggers
    /// and enqueue FunctionExecution jobs for each match.
    ///
    /// Note: Trigger evaluation only runs for LOCAL events, not replicated events.
    /// This prevents duplicate function executions in cluster scenarios.
    pub(crate) async fn enqueue_trigger_evaluation(
        &self,
        node_event: &NodeEvent,
        event_type: &str,
    ) -> Result<()> {
        // Engine-bookkeeping writes (mount state blobs, counters) never fire
        // triggers: the payload is operational metadata, and at bookkeeping
        // write rates a full trigger walk per commit is the cost that burned
        // three cores on an idle mount. Content writes never carry the marker.
        if node_event
            .metadata
            .as_ref()
            .and_then(|m| m.get("bookkeeping"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            tracing::trace!(
                node_id = %node_event.node_id,
                "Skipping TriggerEvaluation: bookkeeping write"
            );
            return Ok(());
        }

        // Get node_type from the event field first, fallback to metadata
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
            .unwrap_or_else(|| "unknown".to_string());

        // Quick-reject check using cached registry.
        //
        // MUST be scope-aware: the cached snapshot is loaded from a single
        // (tenant, repo, branch). Checking an event from another scope against
        // it silently drops valid trigger events (e.g. the agent-continuation
        // aggregated_result event), permanently hanging the conversation.
        if let Some(registry) = &self.trigger_registry {
            if !registry.could_have_matches_for_scope(
                &node_event.tenant_id,
                &node_event.repository_id,
                &node_event.branch,
                &node_event.workspace_id,
                &node_type,
            ) {
                tracing::trace!(
                    workspace = %node_event.workspace_id,
                    node_type = %node_type,
                    "Skipping TriggerEvaluation: no triggers could match"
                );
                return Ok(());
            }
        }

        // Get node_path from NodeEvent.path field (not from metadata)
        let node_path = node_event.path.clone().unwrap_or_else(|| "/".to_string());

        let mut metadata = HashMap::new();
        metadata.insert("node_path".to_string(), serde_json::json!(node_path));
        carry_origin_agent(node_event, &mut metadata);

        // Include node_data if available from event metadata
        if let Some(meta) = &node_event.metadata {
            if let Some(node_data) = meta.get("node_data") {
                metadata.insert("node_data".to_string(), node_data.clone());
            }
        }

        let context = JobContext {
            tenant_id: node_event.tenant_id.clone(),
            repo_id: node_event.repository_id.clone(),
            branch: node_event.branch.clone(),
            workspace_id: node_event.workspace_id.clone(),
            revision: node_event.revision,
            metadata,
        };

        let job_type = JobType::TriggerEvaluation {
            event_type: event_type.to_string(),
            node_id: node_event.node_id.clone(),
            node_type,
        };

        // Use common idempotent enqueue method
        self.enqueue_job(job_type, &context).await
    }

    /// Enqueue AIToolCall execution job for a tool call node
    ///
    /// This queues an AIToolCallExecution job which will execute the tool
    /// function inline and create the AIToolResult node.
    pub(crate) async fn enqueue_ai_tool_call_execution(
        &self,
        node_event: &NodeEvent,
        tool_call_path: &str,
    ) -> Result<()> {
        // The AIToolCall node was written by whatever drove the conversation (a
        // trigger's function, a flow step). Carrying its marker here is what
        // makes the agent run traceable back to that initiator.
        let mut metadata = HashMap::new();
        carry_origin_agent(node_event, &mut metadata);

        let context = JobContext {
            tenant_id: node_event.tenant_id.clone(),
            repo_id: node_event.repository_id.clone(),
            branch: node_event.branch.clone(),
            workspace_id: node_event.workspace_id.clone(),
            revision: node_event.revision,
            metadata,
        };

        let job_type = JobType::AIToolCallExecution {
            tool_call_path: tool_call_path.to_string(),
            tool_call_workspace: node_event.workspace_id.clone(),
        };

        // Use common idempotent enqueue method
        self.enqueue_job(job_type, &context).await
    }

    /// Enqueue AIToolResult aggregation job for a single result node
    ///
    /// This queues an AIToolResultAggregation job which will:
    /// 1. Find the aggregator node
    /// 2. Atomically increment completed_count
    /// 3. If all tools complete, call LLM and create appropriate nodes
    pub(crate) async fn enqueue_ai_tool_result_aggregation(
        &self,
        node_event: &NodeEvent,
        result_path: &str,
    ) -> Result<()> {
        let context = JobContext {
            tenant_id: node_event.tenant_id.clone(),
            repo_id: node_event.repository_id.clone(),
            branch: node_event.branch.clone(),
            workspace_id: node_event.workspace_id.clone(),
            revision: node_event.revision,
            metadata: HashMap::new(),
        };

        let job_type = JobType::AIToolResultAggregation {
            single_result_path: result_path.to_string(),
            workspace: node_event.workspace_id.clone(),
        };

        // Use common idempotent enqueue method
        self.enqueue_job(job_type, &context).await
    }
}
