// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Step result handlers for the execution loop
//!
//! Extracted from the main execution loop for readability.
//! Handles Wait, Complete, and Error step results.

use crate::runtime::rollback_flow;
use crate::types::{
    FlowCallbacks, FlowDefinition, FlowError, FlowExecutionEvent, FlowResult, FlowStatus, WaitInfo,
    WaitType,
};
use chrono::Utc;
use serde_json::Value;
use std::time::Instant;
use tracing::{error, info, warn};

use super::helpers::{
    calculate_backoff, calculate_timeout, generate_subscription_id, get_max_retries,
    parse_wait_type, MAX_VERSION_CONFLICT_RETRIES,
};

/// Handle a Wait step result: persist state and exit the loop
pub(super) async fn handle_wait_result(
    instance_id: &str,
    instance: &mut crate::types::FlowInstance,
    step_id: &str,
    reason: &str,
    metadata: &Value,
    expected_version: i32,
    version_conflict_retries: u32,
    callbacks: &dyn FlowCallbacks,
) -> FlowResult<()> {
    info!("Step {} waiting for {}, persisting state", step_id, reason);

    // Emit FlowWaiting event
    let _ = callbacks
        .emit_event(
            instance_id,
            FlowExecutionEvent::flow_waiting(step_id, reason, reason),
        )
        .await;

    instance.status = FlowStatus::Waiting;
    instance.wait_info = Some(WaitInfo {
        subscription_id: generate_subscription_id(),
        wait_type: parse_wait_type(reason),
        target_path: metadata
            .get("target_path")
            .and_then(|v| v.as_str().map(String::from)),
        expected_event: metadata
            .get("expected_event")
            .and_then(|v| v.as_str().map(String::from)),
        timeout_at: calculate_timeout(metadata),
    });

    // Schedule a timeout check job if a timeout is configured
    if let Some(timeout_at) = instance.wait_info.as_ref().and_then(|w| w.timeout_at) {
        let timeout_payload = serde_json::json!({
            "type": "FlowInstanceExecution",
            "instance_id": instance_id,
            "execution_type": "timeout_check",
            "timeout_at": timeout_at.to_rfc3339(),
        });

        match callbacks
            .queue_job("FlowInstanceExecution", timeout_payload)
            .await
        {
            Ok(job_id) => {
                info!(
                    "Scheduled timeout check job {} for flow {} at {}",
                    job_id, instance_id, timeout_at
                );
            }
            Err(e) => {
                // Non-fatal: timeout will still be enforced on resume
                warn!(
                    "Failed to schedule timeout check job for flow {}: {}",
                    instance_id, e
                );
            }
        }
    }

    // HARD COMMIT with OCC check
    match callbacks
        .save_instance_with_version(instance, expected_version)
        .await
    {
        Ok(_) => {
            info!("Flow {} persisted in waiting state", instance_id);
            Ok(())
        }
        Err(FlowError::VersionConflict) => {
            // Another process advanced the flow - check retry limit
            if version_conflict_retries >= MAX_VERSION_CONFLICT_RETRIES {
                error!(
                    "Max version conflict retries ({}) exceeded for flow {}",
                    MAX_VERSION_CONFLICT_RETRIES, instance_id
                );
                return Err(FlowError::Other(format!(
                    "Flow {} exceeded max version conflict retries ({})",
                    instance_id, MAX_VERSION_CONFLICT_RETRIES
                )));
            }
            warn!(
                "Version conflict on flow {}, retry {}/{}",
                instance_id,
                version_conflict_retries + 1,
                MAX_VERSION_CONFLICT_RETRIES
            );
            // Add delay to reduce contention
            tokio::time::sleep(std::time::Duration::from_millis(
                50 * (version_conflict_retries + 1) as u64,
            ))
            .await;
            Box::pin(super::execution_loop::execute_flow_with_retry(
                instance_id,
                callbacks,
                version_conflict_retries + 1,
            ))
            .await
        }
        Err(e) => Err(e),
    }
}

/// Handle a Complete step result: finalize and persist the flow
pub(super) async fn handle_complete_result(
    instance_id: &str,
    instance: &mut crate::types::FlowInstance,
    step_id: &str,
    output: Value,
    expected_version: i32,
    step_duration_ms: u64,
    flow_start: &Instant,
    callbacks: &dyn FlowCallbacks,
) -> FlowResult<()> {
    info!("Flow {} completed successfully", instance_id);

    // Emit StepCompleted for the end node
    let _ = callbacks
        .emit_event(
            instance_id,
            FlowExecutionEvent::step_completed(step_id, output.clone(), step_duration_ms),
        )
        .await;

    instance.status = FlowStatus::Completed;
    instance.output = Some(output.clone());
    instance.completed_at = Some(Utc::now());

    // Calculate total duration
    let total_duration_ms = flow_start.elapsed().as_millis() as u64;
    instance.metrics.total_duration_ms = total_duration_ms;

    // Emit FlowCompleted event
    let _ = callbacks
        .emit_event(
            instance_id,
            FlowExecutionEvent::flow_completed(output, total_duration_ms),
        )
        .await;

    callbacks
        .save_instance_with_version(instance, expected_version)
        .await?;
    Ok(())
}

/// Handle an Error step result: retry, error edges, continue-on-fail, or fail
///
/// Returns `Ok(true)` if the caller should return from the loop,
/// `Ok(false)` if the loop should continue (error edge / continue-on-fail).
pub(super) async fn handle_error_result(
    instance_id: &str,
    instance: &mut crate::types::FlowInstance,
    current_step: &crate::types::FlowNode,
    flow_def: &FlowDefinition,
    error: FlowError,
    expected_version: i32,
    step_duration_ms: u64,
    flow_start: &Instant,
    callbacks: &dyn FlowCallbacks,
) -> FlowResult<bool> {
    error!("Step {} failed with error: {}", current_step.id, error);

    // Emit StepFailed event
    let _ = callbacks
        .emit_event(
            instance_id,
            FlowExecutionEvent::step_failed(&current_step.id, error.to_string(), step_duration_ms),
        )
        .await;

    // Check for error_edge - if set, follow it instead of normal error handling
    let error_edge = current_step
        .properties
        .get("error_edge")
        .and_then(|v| v.as_str())
        .map(String::from);

    if let Some(error_target) = error_edge {
        info!(
            "Step {} has error_edge, navigating to error handler: {}",
            current_step.id, error_target
        );

        // Populate $.error context for error handler node
        if let Value::Object(ref mut vars) = instance.variables {
            vars.insert(
                "error".to_string(),
                serde_json::json!({
                    "error_type": "step_error",
                    "message": error.to_string(),
                    "step_id": current_step.id,
                    "timestamp": Utc::now().to_rfc3339(),
                }),
            );
        }

        instance.current_node_id = error_target;
        instance.retry_count = 0; // Reset retry count for error path
        return Ok(false); // Continue loop - follow error edge
    }

    // Check for continue_on_fail - if set, continue to next step
    let continue_on_fail = current_step
        .properties
        .get("continue_on_fail")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if continue_on_fail {
        info!(
            "Step {} has continue_on_fail=true, continuing despite error",
            current_step.id
        );

        // Populate $.error context for downstream steps
        if let Value::Object(ref mut vars) = instance.variables {
            vars.insert(
                "error".to_string(),
                serde_json::json!({
                    "error_type": "step_error",
                    "message": error.to_string(),
                    "step_id": current_step.id,
                    "timestamp": Utc::now().to_rfc3339(),
                    "continued": true,
                }),
            );
        }

        // Get next node and continue
        let next_node_id = flow_def
            .next_node_id(&current_step.id)
            .unwrap_or_else(|| "end".to_string());
        instance.current_node_id = next_node_id;
        instance.retry_count = 0;
        return Ok(false); // Continue loop
    }

    // Standard retry logic
    let max_retries = get_max_retries(current_step);

    if instance.retry_count < max_retries {
        instance.retry_count += 1;
        instance.metrics.retry_count += 1;
        let backoff = calculate_backoff(instance.retry_count);

        warn!(
            "Retrying flow {} (attempt {}/{}) after {:?}",
            instance_id, instance.retry_count, max_retries, backoff
        );

        instance.status = FlowStatus::Waiting;
        instance.wait_info = Some(WaitInfo {
            subscription_id: generate_subscription_id(),
            wait_type: WaitType::Retry,
            target_path: None,
            expected_event: None,
            timeout_at: Some(Utc::now() + backoff),
        });

        callbacks
            .save_instance_with_version(instance, expected_version)
            .await?;
        // TODO: Schedule retry job
        return Ok(true); // Return from loop
    }

    // Max retries exceeded - initiate rollback
    error!(
        "Flow {} failed after {} retries, initiating rollback",
        instance_id, instance.retry_count
    );

    instance.status = FlowStatus::Failed;
    instance.error = Some(error.to_string());
    instance.completed_at = Some(Utc::now());

    // Execute saga compensation
    rollback_flow(instance, callbacks).await?;

    // Calculate total duration and emit FlowFailed event
    let total_duration_ms = flow_start.elapsed().as_millis() as u64;
    let _ = callbacks
        .emit_event(
            instance_id,
            FlowExecutionEvent::flow_failed(
                error.to_string(),
                Some(current_step.id.clone()),
                total_duration_ms,
            ),
        )
        .await;

    callbacks
        .save_instance_with_version(instance, expected_version)
        .await?;
    Err(error)
}
