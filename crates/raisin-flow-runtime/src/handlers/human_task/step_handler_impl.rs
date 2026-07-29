// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! StepHandler trait implementation for HumanTaskHandler

use crate::handlers::StepHandler;
use crate::types::{
    FlowCallbacks, FlowContext, FlowError, FlowNode, FlowResult, StepResult, TaskType,
};
use async_trait::async_trait;
use tracing::{debug, error, instrument};

use super::handler::{HumanTaskHandler, WAIT_REASON_HUMAN_TASK};

#[async_trait]
impl StepHandler for HumanTaskHandler {
    #[instrument(skip(self, context, callbacks), fields(step_id = %step.id))]
    async fn execute(
        &self,
        step: &FlowNode,
        context: &mut FlowContext,
        callbacks: &dyn FlowCallbacks,
    ) -> FlowResult<StepResult> {
        debug!("Executing human task step: {}", step.id);

        // Resuming after task completion? The pending flag is set by
        // process_resume_data exactly once per completion, so a later
        // human task in the same flow won't consume a stale response.
        if context
            .variables
            .remove("__human_response_pending")
            .is_some()
        {
            let response = context
                .variables
                .get("__human_response")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            debug!(
                step_id = %step.id,
                "Human task response received, continuing"
            );

            let next_node_id = step.next_node.clone().unwrap_or_else(|| "end".to_string());
            return Ok(StepResult::Continue {
                next_node_id,
                output: response,
            });
        }

        // Extract task configuration
        let task_type = self.get_task_type(step)?;

        // Build task properties (template expressions resolved against context)
        let mut task_properties = self.build_task_properties(step, context, &task_type)?;

        // Use the resolved assignee (may contain templates like
        // "/users/{{ input.approver }}")
        let assignee = task_properties
            .get("assignee")
            .and_then(|v| v.as_str())
            .map(String::from)
            .ok_or_else(|| {
                FlowError::InvalidNodeConfiguration(format!(
                    "Human task step '{}' assignee did not resolve to a string",
                    step.id
                ))
            })?;

        // Materialize the absolute due time for UI sorting/overdue display
        if let Some(due_seconds) = task_properties
            .get("due_in_seconds")
            .and_then(|v| v.as_i64())
            .filter(|secs| *secs > 0)
        {
            task_properties["due_at"] = serde_json::json!((chrono::Utc::now()
                + chrono::Duration::seconds(due_seconds))
            .to_rfc3339());
        }
        task_properties["created_at"] = serde_json::json!(chrono::Utc::now().to_rfc3339());

        // Generate the task path - deterministic per (instance, step, loop
        // iteration) so a new flow run / iteration can never alias an
        // earlier task, and a re-delivered execution of the same wait is
        // idempotent.
        let mut task_path = self.generate_task_path(&assignee, &step.id, context);
        debug!("Creating inbox task at: {}", task_path);

        // Staleness / idempotency guard for the create path. If a node
        // already exists at the task path, it may ONLY stand in for this
        // wait when it is THIS instance's still-pending task for THIS step
        // (job redelivery / version-conflict re-execution). An existing
        // COMPLETED (or expired/cancelled/foreign-instance) task must never
        // satisfy a new wait - the new task is created at a fresh unique
        // path instead.
        let mut existing_task: Option<serde_json::Value> = None;
        if let Ok(Some(existing)) = callbacks
            .get_node_in_workspace(super::INBOX_WORKSPACE, &task_path)
            .await
        {
            let props = existing.get("properties").cloned().unwrap_or_default();
            let same_instance = props.get("flow_instance_id").and_then(|v| v.as_str())
                == Some(context.instance_id.as_str());
            let same_step = props.get("step_id").and_then(|v| v.as_str()) == Some(step.id.as_str());
            let pending = props.get("status").and_then(|v| v.as_str()) == Some("pending");

            if same_instance && same_step && pending {
                debug!(
                    task_path = %task_path,
                    "Pending task for this wait already exists - reusing (idempotent re-execution)"
                );
                existing_task = Some(existing);
            } else {
                let unique_path =
                    format!("{}-{}", task_path, chrono::Utc::now().timestamp_millis());
                debug!(
                    occupied_path = %task_path,
                    unique_path = %unique_path,
                    same_instance,
                    same_step,
                    pending,
                    "Task path occupied by a non-reusable task node - using a fresh unique path"
                );
                task_path = unique_path;
            }
        }
        let reused_pending_task = existing_task.is_some();

        // Create the inbox task node in the canonical inbox workspace
        // (raisin:access_control - where user homes and their inboxes live)
        let creation_result = match existing_task {
            Some(node) => Ok(node),
            None => {
                callbacks
                    .create_node_in_workspace(
                        super::INBOX_WORKSPACE,
                        super::INBOX_TASK_NODE_TYPE,
                        &task_path,
                        task_properties.clone(),
                    )
                    .await
            }
        };

        match creation_result {
            Ok(created_node) => {
                debug!("Inbox task created successfully: {}", task_path);
                debug!("Created node: {}", created_node);

                // Agent-as-assignee: if the assignee is an AI agent, let it
                // decide now. A confident decision completes the task and
                // the flow continues; otherwise the task is escalated and
                // the flow waits for a human like any other task.
                // Skipped when an existing pending task was reused - the
                // agent already had its chance on the original delivery.
                if !reused_pending_task {
                    if let Some(agent_result) = super::agent_assignee::try_agent_assignee(
                        step,
                        context,
                        callbacks,
                        &assignee,
                        &task_properties,
                        &task_path,
                    )
                    .await?
                    {
                        return Ok(agent_result);
                    }
                }

                // A due time on the task doubles as the wait deadline so the
                // timeout machinery (timeout_edge / fail / task expiry) fires.
                let timeout_ms = task_properties
                    .get("due_in_seconds")
                    .and_then(|v| v.as_i64())
                    .filter(|secs| *secs > 0)
                    .map(|secs| (secs as u64) * 1000);

                let mut metadata = serde_json::json!({
                    "task_path": task_path,
                    // target_path lands in WaitInfo so timeout expiry can
                    // mark the inbox task as expired
                    "target_path": task_path,
                    "task_type": task_type.as_str(),
                    "step_id": step.id,
                    "assignee": assignee,
                });
                if let Some(ms) = timeout_ms {
                    metadata["timeout_ms"] = serde_json::json!(ms);
                }

                // Return Wait result
                Ok(StepResult::Wait {
                    reason: WAIT_REASON_HUMAN_TASK.to_string(),
                    metadata,
                })
            }
            Err(e) => {
                error!("Failed to create inbox task: {}", e);
                Err(FlowError::FunctionExecution(format!(
                    "Failed to create inbox task at '{}': {}",
                    task_path, e
                )))
            }
        }
    }
}
