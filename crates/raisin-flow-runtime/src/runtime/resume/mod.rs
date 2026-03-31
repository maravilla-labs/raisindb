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

    // 2. Check flow status for idempotency
    match instance.status {
        FlowStatus::Waiting => {} // Expected — continue
        FlowStatus::Running => {
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
            warn!(instance_id = %instance_id, "Cannot resume flow - still in pending state");
            return Err(FlowError::InvalidStateTransition {
                from: "pending".to_string(),
                to: "running".to_string(),
            });
        }
    }

    // 3. Check if the wait has timed out
    if let Some(ref wait_info) = instance.wait_info {
        if let Some(timeout_at) = wait_info.timeout_at {
            if Utc::now() > timeout_at {
                return fail_timed_out(instance_id, &mut instance, timeout_at, callbacks).await;
            }
        }
    }

    // 4. Process resume data based on wait_info
    let early_return =
        process_resume_data(instance_id, &mut instance, &resume_data, callbacks).await?;
    if early_return {
        return Ok(());
    }

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
                Some(node_id),
                0,
            ),
        )
        .await;

    callbacks.save_instance(instance).await.map_err(|e| {
        error!(instance_id = %instance_id, error = %e, "Failed to save timed-out instance");
        FlowError::Other(format!("Failed to save timed-out instance: {}", e))
    })?;

    Err(FlowError::TimeoutExceeded { duration_ms: 0 })
}

/// Process resume data by storing it in instance variables based on wait type.
///
/// Returns `true` if the caller should return early (e.g., function failure
/// already saved the instance in a terminal state).
async fn process_resume_data(
    instance_id: &str,
    instance: &mut FlowInstance,
    resume_data: &Value,
    callbacks: &dyn FlowCallbacks,
) -> FlowResult<bool> {
    if let Some(wait_info) = &instance.wait_info {
        info!(instance_id = %instance_id, wait_type = ?wait_info.wait_type, "Processing resume data");

        match wait_info.wait_type {
            WaitType::ToolCall => {
                set_instance_variable(instance, "__last_tool_result", resume_data.clone());
            }
            WaitType::HumanTask => {
                set_instance_variable(instance, "__human_response", resume_data.clone());
            }
            WaitType::Retry => {
                instance.retry_count = 0;
                info!(instance_id = %instance_id, "Resuming after retry backoff, reset retry count");
            }
            WaitType::Scheduled => {
                info!(instance_id = %instance_id, "Resuming after scheduled delay");
            }
            WaitType::FunctionCall => {
                set_instance_variable(instance, "__function_result", resume_data.clone());

                // Check if function failed — transition to Failed directly
                if let Some(false) = resume_data.get("success").and_then(|v| v.as_bool()) {
                    instance.wait_info = None;
                    instance.status = FlowStatus::Failed;
                    instance.error = resume_data
                        .get("error")
                        .and_then(|e| e.as_str())
                        .map(|s| s.to_string());

                    callbacks.save_instance(instance).await.map_err(|e| {
                        error!(instance_id = %instance_id, error = %e, "Failed to save failed instance");
                        FlowError::Other(format!("Failed to save failed instance: {}", e))
                    })?;

                    info!(instance_id = %instance_id, error = ?instance.error, "Function failed - transitioned to Failed");
                    return Ok(true); // Early return — don't re-execute
                }
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
    } else {
        warn!(instance_id = %instance_id, "Flow is waiting but has no wait_info - storing as generic resume data");
        set_instance_variable(instance, "__resume_data", resume_data.clone());
    }

    Ok(false)
}
