// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Flow resumption logic for handling external events.
//!
//! This module provides functionality to resume flows that are waiting for:
//! - Tool call results from AI agents
//! - Human task completion
//! - Scheduled delays
//! - Retry backoffs
//! - External events

#[cfg(test)]
mod tests;

use crate::runtime::execute_flow;
use crate::types::{
    FlowCallbacks, FlowError, FlowExecutionEvent, FlowInstance, FlowResult, FlowStatus, WaitType,
};
use chrono::Utc;
use serde_json::Value;
use tracing::{error, info, warn};

/// Insert a value into the instance's variables map, initializing if needed.
fn set_instance_variable(instance: &mut FlowInstance, key: &str, value: Value) {
    if let Value::Object(ref mut vars) = instance.variables {
        vars.insert(key.to_string(), value);
    } else {
        let mut vars_map = serde_json::Map::new();
        vars_map.insert(key.to_string(), value);
        instance.variables = Value::Object(vars_map);
    }
}

/// Resume a flow that was waiting for an external event.
///
/// Called when an event arrives that should resume a waiting flow (tool result,
/// human task completion, scheduled delay, retry backoff, or external event).
///
/// # Flow Resumption Process
///
/// 1. Load the flow instance from storage
/// 2. Verify it's in the `Waiting` state (idempotent for terminal/running states)
/// 3. Check if the wait has timed out — fail the flow if expired
/// 4. Process resume data based on wait type, storing in instance variables
/// 5. Clear wait info and transition to `Running`
/// 6. Save and re-execute the flow from current position
pub async fn resume_flow(
    instance_id: &str,
    resume_data: Value,
    callbacks: &dyn FlowCallbacks,
) -> FlowResult<()> {
    info!(instance_id = %instance_id, "Resuming flow execution with external data");

    // 1. Load the instance (capture version for OCC)
    let mut instance = callbacks
        .load_instance(&format!("/flows/instances/{}", instance_id))
        .await
        .map_err(|e| {
            error!(instance_id = %instance_id, error = %e, "Failed to load flow instance for resumption");
            FlowError::NodeNotFound(format!("Flow instance '{}' not found: {}", instance_id, e))
        })?;
    let expected_version = instance.version;

    // Data-carrying deliveries (function results, child-flow completions,
    // human responses) must not be swallowed by the idempotency shortcuts
    // below; null / empty-object resumes are generic wake-ups and may be.
    let has_resume_data = !resume_data.is_null()
        && resume_data
            .as_object()
            .map(|obj| !obj.is_empty())
            .unwrap_or(true);

    // 2. Check flow status for idempotency
    match instance.status {
        FlowStatus::Waiting => {} // Expected — continue
        FlowStatus::Running => {
            // E.g. a fast function finishing before the flow persists its
            // Waiting state. Surface a retryable conflict so the job
            // system redelivers.
            if has_resume_data {
                warn!(
                    instance_id = %instance_id,
                    "Flow busy while delivering resume data - requesting redelivery"
                );
                return Err(FlowError::VersionConflict);
            }
            info!(instance_id = %instance_id, "Flow is already running (idempotent)");
            return Ok(());
        }
        FlowStatus::Completed
        | FlowStatus::Failed
        | FlowStatus::Cancelled
        | FlowStatus::RolledBack => {
            info!(instance_id = %instance_id, status = ?instance.status, "Flow is in terminal state (idempotent)");
            return Ok(());
        }
        FlowStatus::Pending => {
            // Same race, even earlier: the start job hasn't transitioned the
            // instance yet (or we read a stale snapshot). A data-carrying
            // resume must be redelivered, not dropped.
            if has_resume_data {
                warn!(
                    instance_id = %instance_id,
                    "Flow still pending while delivering resume data - requesting redelivery"
                );
                return Err(FlowError::VersionConflict);
            }
            warn!(instance_id = %instance_id, "Cannot resume flow - still in pending state");
            return Err(FlowError::InvalidStateTransition {
                from: "pending".to_string(),
                to: "running".to_string(),
            });
        }
    }

    // 3. Check if the wait has timed out.
    //
    // Scheduled and Retry waits are excluded: for those, timeout_at is the
    // *wake-up* time, not a deadline - resuming at/after that time is the
    // normal path, not a failure.
    if let Some(ref wait_info) = instance.wait_info {
        if !matches!(wait_info.wait_type, WaitType::Scheduled | WaitType::Retry) {
            if let Some(timeout_at) = wait_info.timeout_at {
                if Utc::now() > timeout_at {
                    return handle_timeout_expiry(
                        instance_id,
                        &mut instance,
                        timeout_at,
                        callbacks,
                    )
                    .await;
                }
            }
        }
    }

    // 4. Process resume data based on wait_info
    process_resume_data(instance_id, &mut instance, &resume_data);

    // 5. Clear wait info and mark as running
    instance.wait_info = None;
    instance.status = FlowStatus::Running;
    info!(instance_id = %instance_id, "Transitioned flow from Waiting to Running");

    // 6. Save instance before re-executing (with OCC version check)
    callbacks.save_instance_with_version(&instance, expected_version).await.map_err(|e| {
        error!(instance_id = %instance_id, error = %e, "Failed to save instance state before resuming");
        FlowError::Other(format!("Failed to save instance before resuming: {}", e))
    })?;

    info!(instance_id = %instance_id, "Saved instance state, re-executing flow");
    execute_flow(instance_id, callbacks).await
}

/// Check whether a waiting flow's deadline has been reached and act on it.
///
/// This is the entry point for `timeout_check` jobs (scheduled wake-ups).
/// Unlike `resume_flow`, it never resumes a flow whose wait condition has
/// not been met:
///
/// - Not in `Waiting` state, or no deadline set: no-op
/// - `Scheduled` / `Retry` waits whose wake-up time arrived: resume normally
/// - Other waits (human task, function call, ...) past `timeout_at`:
///   follow the step's `timeout_edge` if configured, else fail the flow
pub async fn check_flow_timeout(
    instance_id: &str,
    callbacks: &dyn FlowCallbacks,
) -> FlowResult<()> {
    let mut instance = match callbacks
        .load_instance(&format!("/flows/instances/{}", instance_id))
        .await
    {
        Ok(i) => i,
        Err(e) => {
            // Instance may have been deleted - nothing to check
            info!(instance_id = %instance_id, error = %e, "Timeout check: instance not found, skipping");
            return Ok(());
        }
    };

    if instance.status != FlowStatus::Waiting {
        info!(
            instance_id = %instance_id,
            status = ?instance.status,
            "Timeout check: flow is not waiting, skipping"
        );
        return Ok(());
    }

    let Some(wait_info) = instance.wait_info.clone() else {
        warn!(instance_id = %instance_id, "Timeout check: waiting flow has no wait_info, skipping");
        return Ok(());
    };

    let now = Utc::now();

    match wait_info.wait_type {
        // For scheduled delays and retry backoffs, reaching the time IS the
        // resume condition.
        WaitType::Scheduled | WaitType::Retry => {
            let due = wait_info.timeout_at.map(|t| now >= t).unwrap_or(true);
            if due {
                info!(instance_id = %instance_id, wait_type = ?wait_info.wait_type, "Wake-up time reached, resuming flow");
                resume_flow(instance_id, Value::Null, callbacks).await
            } else {
                info!(instance_id = %instance_id, "Timeout check fired early, wait not yet due - skipping");
                Ok(())
            }
        }
        // For all other waits, the deadline is a timeout.
        _ => match wait_info.timeout_at {
            Some(timeout_at) if now > timeout_at => {
                handle_timeout_expiry(instance_id, &mut instance, timeout_at, callbacks).await
            }
            _ => {
                info!(instance_id = %instance_id, "Timeout check: deadline not reached, skipping");
                Ok(())
            }
        },
    }
}

/// Handle an expired wait deadline: follow the step's `timeout_edge` if one
/// is configured, otherwise fail the flow.
async fn handle_timeout_expiry(
    instance_id: &str,
    instance: &mut FlowInstance,
    timeout_at: chrono::DateTime<Utc>,
    callbacks: &dyn FlowCallbacks,
) -> FlowResult<()> {
    // Best effort: mark the inbox task (or other wait target) as expired
    if instance
        .wait_info
        .as_ref()
        .map(|w| w.wait_type == WaitType::HumanTask)
        .unwrap_or(false)
    {
        if let Some(task_path) = instance
            .wait_info
            .as_ref()
            .and_then(|w| w.target_path.clone())
        {
            let _ = callbacks
                .update_node_in_workspace(
                    crate::handlers::human_task::INBOX_WORKSPACE,
                    &task_path,
                    serde_json::json!({ "status": "expired" }),
                )
                .await;
        }
    }

    // Check for a timeout_edge on the waiting step
    let timeout_edge =
        crate::types::FlowDefinition::from_workflow_data(instance.flow_definition_snapshot.clone())
            .ok()
            .and_then(|def| {
                def.find_node(&instance.current_node_id)
                    .and_then(|node| node.get_string_property("timeout_edge"))
            });

    if let Some(timeout_target) = timeout_edge {
        info!(
            instance_id = %instance_id,
            timeout_target = %timeout_target,
            "Wait timed out, following timeout_edge"
        );

        let step_id = instance.current_node_id.clone();
        set_instance_variable(
            instance,
            "error",
            serde_json::json!({
                "error_type": "timeout",
                "message": format!("Wait timed out at step '{}' (deadline was {})", step_id, timeout_at),
                "step_id": step_id,
                "timestamp": Utc::now().to_rfc3339(),
            }),
        );

        instance.current_node_id = timeout_target;
        instance.wait_info = None;
        instance.status = FlowStatus::Running;
        instance.retry_count = 0;

        callbacks.save_instance(instance).await.map_err(|e| {
            error!(instance_id = %instance_id, error = %e, "Failed to save instance before timeout edge");
            FlowError::Other(format!("Failed to save instance before timeout edge: {}", e))
        })?;

        return execute_flow(instance_id, callbacks).await;
    }

    fail_timed_out(instance_id, instance, timeout_at, callbacks).await
}

/// Transition a timed-out flow to Failed state.
async fn fail_timed_out(
    instance_id: &str,
    instance: &mut FlowInstance,
    timeout_at: chrono::DateTime<Utc>,
    callbacks: &dyn FlowCallbacks,
) -> FlowResult<()> {
    warn!(
        instance_id = %instance_id,
        timeout_at = %timeout_at,
        "Flow wait has timed out, transitioning to Failed"
    );

    let node_id = instance.current_node_id.clone();
    instance.wait_info = None;
    instance.status = FlowStatus::Failed;
    instance.error = Some(format!(
        "Wait timed out at step '{}' (deadline was {})",
        node_id, timeout_at
    ));
    instance.completed_at = Some(Utc::now());

    let _ = callbacks
        .emit_event(
            instance_id,
            FlowExecutionEvent::flow_failed(
                format!("Wait timed out at step '{}'", node_id),
                Some(node_id.clone()),
                0,
            ),
        )
        .await;

    callbacks.save_instance(instance).await.map_err(|e| {
        error!(instance_id = %instance_id, error = %e, "Failed to save timed-out instance");
        FlowError::Other(format!("Failed to save timed-out instance: {}", e))
    })?;

    // A timed-out CHILD must still tell its parent, exactly as any other
    // terminal state does. Without this a parent parked on a join (a
    // parallel fan-out, a sub-flow) waits forever the moment one child's
    // wait expires: the child is Failed, the parent never learns, and no
    // error surfaces anywhere. Same payload shape as the ordinary failure
    // path in `result_handlers::fail_flow`.
    let error_message = instance
        .error
        .clone()
        .unwrap_or_else(|| format!("Wait timed out at step '{}'", node_id));
    crate::runtime::executor::result_handlers::notify_parent_flow(
        instance,
        "failed",
        None,
        Some(error_message),
        callbacks,
    )
    .await;

    Err(FlowError::TimeoutExceeded { duration_ms: 0 })
}

/// Process resume data by storing it in instance variables based on wait type.
fn process_resume_data(instance_id: &str, instance: &mut FlowInstance, resume_data: &Value) {
    if let Some(wait_info) = &instance.wait_info {
        info!(instance_id = %instance_id, wait_type = ?wait_info.wait_type, "Processing resume data");
        let is_retry = matches!(wait_info.wait_type, WaitType::Retry);

        match wait_info.wait_type {
            WaitType::ToolCall => {
                set_instance_variable(instance, "__last_tool_result", resume_data.clone());
            }
            WaitType::HumanTask => {
                set_instance_variable(instance, "__human_response", resume_data.clone());
                // One-shot flag consumed by the waiting human task step
                set_instance_variable(instance, "__human_response_pending", Value::Bool(true));
            }
            WaitType::Retry => {
                // Keep retry_count: it tracks attempts for the current step
                // and is checked against max_retries on the next failure.
                // It is reset when the step eventually succeeds.
                info!(
                    instance_id = %instance_id,
                    retry_count = instance.retry_count,
                    "Resuming after retry backoff"
                );
            }
            WaitType::Scheduled => {
                info!(instance_id = %instance_id, "Resuming after scheduled delay");
                // One-shot flag consumed by the waiting wait step so it
                // continues instead of starting a new wait
                set_instance_variable(instance, "__wait_elapsed_pending", Value::Bool(true));
            }
            WaitType::FunctionCall => {
                // Store the result (success or failure) and re-execute the
                // step. On failure, FunctionStepHandler converts the cached
                // result into StepResult::Error, which routes through the
                // full error machinery: retry -> error_edge /
                // continue_on_fail -> saga rollback. Failing the flow
                // directly here would bypass all of that.
                set_instance_variable(instance, "__function_result", resume_data.clone());
            }
            WaitType::ChatSession => {
                let message = resume_data
                    .get("message")
                    .or_else(|| resume_data.get("content"))
                    .cloned()
                    .unwrap_or_else(|| resume_data.clone());
                set_instance_variable(instance, "__chat_user_message", message);
            }
            WaitType::Event | WaitType::Join => {
                set_instance_variable(instance, "__resume_data", resume_data.clone());
            }
        }
        // Every wait except a retry backoff re-enters the SAME node to consume
        // what it waited for — a function step parks and comes back for its
        // result, a chat step comes back for each user message. That re-entry
        // is not a new visit, and must not count as one: `visits.<step>` is
        // the author's loop bound, and doubling it for every parked step made
        // `visits.draft < 3` allow one real pass. A retry is different — a
        // failed attempt burns a visit on purpose (see `bump_visit_count`).
        if !is_retry {
            set_instance_variable(
                instance,
                crate::runtime::executor::RESUME_REENTRY_KEY,
                Value::Bool(true),
            );
        }
    } else {
        warn!(instance_id = %instance_id, "Flow is waiting but has no wait_info - storing as generic resume data");
        set_instance_variable(instance, "__resume_data", resume_data.clone());
        set_instance_variable(
            instance,
            crate::runtime::executor::RESUME_REENTRY_KEY,
            Value::Bool(true),
        );
    }
}
