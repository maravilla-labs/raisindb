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
use super::group_assignee::GroupAssignment;

async fn create_group_tasks(
    handler: &HumanTaskHandler,
    step: &FlowNode,
    context: &mut FlowContext,
    callbacks: &dyn FlowCallbacks,
    task_type: &TaskType,
    base_properties: &serde_json::Value,
    assignment: GroupAssignment,
) -> FlowResult<StepResult> {
    let iteration = context
        .variables
        .get("__loop_index")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let assignment_id = format!(
        "group-{}-{}-it{}",
        step.id,
        super::instance_slug(&context.instance_id),
        iteration
    );
    let task_paths: Vec<String> = assignment
        .members
        .iter()
        .map(|member| handler.generate_task_path(member, &step.id, context))
        .collect();

    let mut written: Vec<(String, serde_json::Value)> = Vec::new();
    for (member, task_path) in assignment.members.iter().zip(task_paths.iter()) {
        let mut properties = base_properties.clone();
        properties["assignee"] = serde_json::json!(member);
        properties["assignment_id"] = serde_json::json!(assignment_id);
        properties["assignment_group"] = serde_json::json!(assignment.group_path);
        properties["assignment_completion"] = serde_json::json!(assignment.completion);
        properties["assignment_required"] = serde_json::json!(assignment.required);
        properties["assignment_total"] = serde_json::json!(assignment.members.len());
        properties["assignment_task_paths"] = serde_json::json!(task_paths);
        if let Some(ref condition) = assignment.response_condition {
            properties["response_condition"] = serde_json::json!(condition);
        }

        let existing = callbacks
            .get_node_in_workspace(super::INBOX_WORKSPACE, task_path)
            .await?;
        if let Some(node) = existing {
            let props = node.get("properties").cloned().unwrap_or_default();
            let reusable = props.get("flow_instance_id").and_then(|v| v.as_str())
                == Some(context.instance_id.as_str())
                && props.get("step_id").and_then(|v| v.as_str()) == Some(step.id.as_str())
                && props.get("assignment_id").and_then(|v| v.as_str())
                    == Some(assignment_id.as_str())
                && props.get("status").and_then(|v| v.as_str()) == Some("pending");
            if !reusable {
                return Err(FlowError::InvalidNodeConfiguration(format!(
                    "Group task path '{}' is occupied by a task that does not belong to this wait",
                    task_path
                )));
            }
            written.push((task_path.clone(), props));
            continue;
        }

        match callbacks
            .create_node_in_workspace(
                super::INBOX_WORKSPACE,
                super::INBOX_TASK_NODE_TYPE,
                task_path,
                properties.clone(),
            )
            .await
        {
            Ok(_) => written.push((task_path.clone(), properties)),
            Err(cause) => {
                // A partially delivered group assignment is not allowed to sit
                // in real inboxes. Reconcile every task created in this attempt.
                for (path, mut props) in written {
                    props["status"] = serde_json::json!("cancelled");
                    props["cancellation_reason"] = serde_json::json!("group_delivery_failed");
                    let _ = callbacks
                        .update_node_in_workspace(super::INBOX_WORKSPACE, &path, props)
                        .await;
                }
                return Err(FlowError::FunctionExecution(format!(
                    "Failed to create group inbox task at '{}': {}",
                    task_path, cause
                )));
            }
        }
    }

    context.set_variable(
        format!("__human_task_paths_{}", step.id),
        serde_json::json!(task_paths),
    );

    let timeout_ms = base_properties
        .get("due_in_seconds")
        .and_then(|v| v.as_i64())
        .filter(|secs| *secs > 0)
        .map(|secs| (secs as u64) * 1000);
    let mut metadata = serde_json::json!({
        "task_path": task_paths.first(),
        "task_paths": task_paths,
        "target_path": task_paths.first(),
        "task_type": task_type.as_str(),
        "step_id": step.id,
        "assignee": assignment.group_path,
        "assignment_completion": assignment.completion,
        "assignment_required": assignment.required,
    });
    if let Some(ms) = timeout_ms {
        metadata["timeout_ms"] = serde_json::json!(ms);
    }
    Ok(StepResult::Wait {
        reason: WAIT_REASON_HUMAN_TASK.to_string(),
        metadata,
    })
}

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

        // Use the resolved assignee (may have been authored as a template like
        // "${steps.lookup_owner.user_home}" — "ask whoever owns this node").
        //
        // An UNRESOLVED template survives the mapper as its literal text, so
        // this is also where that is caught: a task addressed to the string
        // "${steps…}" reaches no inbox, and the flow would park on it forever.
        let assignee = task_properties
            .get("assignee")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty() && !value.contains("${") && !value.contains("{{"))
            .map(String::from)
            .ok_or_else(|| {
                FlowError::InvalidNodeConfiguration(format!(
                    "Human task step '{}' assignee did not resolve to a node path: {}",
                    step.id,
                    task_properties.get("assignee").unwrap_or(&serde_json::Value::Null)
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

        // A group is expanded to correlated tasks in each current member's
        // real inbox. The completion API aggregates those task nodes according
        // to any/all/quorum and resumes this same flow only when the authored
        // policy is satisfied (or every member has responded).
        if let Some(assignment) = super::group_assignee::resolve_group_assignment(
            callbacks,
            step,
            &assignee,
        )
        .await?
        {
            return create_group_tasks(
                self,
                step,
                context,
                callbacks,
                &task_type,
                &task_properties,
                assignment,
            )
            .await;
        }

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
        // May a node sitting at a task path stand in for THIS wait? Only when it
        // is this instance's still-pending task for this step. Factored out
        // because the same question is asked of two paths below, and two copies
        // of "is this task reusable" is how they drift.
        let reusable = |node: &serde_json::Value| -> bool {
            let props = node.get("properties").cloned().unwrap_or_default();
            props.get("flow_instance_id").and_then(|v| v.as_str())
                == Some(context.instance_id.as_str())
                && props.get("step_id").and_then(|v| v.as_str()) == Some(step.id.as_str())
                && props.get("status").and_then(|v| v.as_str()) == Some("pending")
        };

        let mut existing_task: Option<serde_json::Value> = None;

        // TRANSITIONAL: adopt a task this instance created under the previous
        // (prefix-based) path scheme. Without this, an instance that parked
        // before the upgrade and is redelivered after it regenerates a different
        // path, finds nothing, and opens a SECOND task — the first left pending
        // in someone's inbox forever while the flow waits on the other. Deletable
        // once no instance predating the change can still be running.
        if let Some(legacy_path) = self.legacy_task_path(&assignee, &step.id, context) {
            if legacy_path != task_path {
                if let Ok(Some(legacy)) = callbacks
                    .get_node_in_workspace(super::INBOX_WORKSPACE, &legacy_path)
                    .await
                {
                    if reusable(&legacy) {
                        debug!(
                            legacy_path = %legacy_path,
                            "Adopting this instance's pending task from the previous path scheme"
                        );
                        task_path = legacy_path;
                        existing_task = Some(legacy);
                    }
                }
            }
        }

        if existing_task.is_none() {
            if let Ok(Some(existing)) = callbacks
                .get_node_in_workspace(super::INBOX_WORKSPACE, &task_path)
                .await
            {
                if reusable(&existing) {
                    debug!(
                        task_path = %task_path,
                        "Pending task for this wait already exists - reusing (idempotent re-execution)"
                    );
                    existing_task = Some(existing);
                } else {
                    // An existing COMPLETED (or expired / cancelled /
                    // foreign-instance) task must never satisfy a new wait, so
                    // the new one goes to a fresh path. The timestamp makes this
                    // non-deterministic, which is why a path COLLISION is worse
                    // than a mis-named node: it lands here, and a redelivery
                    // then picks a different suffix and creates a duplicate.
                    // `instance_slug` hashing the whole id is what keeps genuine
                    // collisions out of this branch.
                    let unique_path =
                        format!("{}-{}", task_path, chrono::Utc::now().timestamp_millis());
                    debug!(
                        occupied_path = %task_path,
                        unique_path = %unique_path,
                        "Task path occupied by a non-reusable task node - using a fresh unique path"
                    );
                    task_path = unique_path;
                }
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
