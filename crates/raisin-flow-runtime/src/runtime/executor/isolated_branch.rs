// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Isolated branch execution for AI safety
//!
//! Provides step execution within a temporary branch so that
//! changes can be reviewed before merging back.

use crate::types::{
    FlowCallbacks, FlowDefinition, FlowError, FlowInstance, FlowNode, FlowResult, StepResult,
};
use tracing::{error, info, warn};

use super::step_dispatch::execute_step_inner;

/// Execute a single step based on its type
///
/// Delegates to isolated branch execution if the step requests it,
/// otherwise runs the step directly.
pub(crate) async fn execute_step(
    step: &FlowNode,
    instance: &mut FlowInstance,
    flow_def: &FlowDefinition,
    callbacks: &dyn FlowCallbacks,
) -> FlowResult<StepResult> {
    // Check if step should run in isolated branch
    let isolated_branch = step
        .properties
        .get("isolated_branch")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if isolated_branch {
        // Execute step in isolated branch for safety
        execute_step_in_isolated_branch(step, instance, flow_def, callbacks).await
    } else {
        // Normal execution
        execute_step_inner(step, instance, flow_def, callbacks).await
    }
}

/// Execute a step in an isolated branch (for AI safety)
///
/// This function:
/// 1. Creates an isolated branch from current state
/// 2. Executes the step in that branch
/// 3. On success: merges the branch back (fails on conflict)
/// 4. On failure: preserves the branch for debugging, marks step failed
async fn execute_step_in_isolated_branch(
    step: &FlowNode,
    instance: &mut FlowInstance,
    flow_def: &FlowDefinition,
    callbacks: &dyn FlowCallbacks,
) -> FlowResult<StepResult> {
    let branch_name = format!("flow-step-{}-{}", instance.id, step.id);

    // 1. Try to create isolated branch
    let original_branch = match callbacks.current_branch().await {
        Ok(branch) => Some(branch),
        Err(FlowError::NotSupported(_)) => {
            // Branch operations not supported - fall back to normal execution
            warn!(
                "Isolated branch mode requested but not supported by backend, executing normally"
            );
            return execute_step_inner(step, instance, flow_def, callbacks).await;
        }
        Err(e) => return Err(e),
    };

    // Create the isolated branch
    match callbacks
        .create_branch(&branch_name, original_branch.as_deref())
        .await
    {
        Ok(_) => {
            info!(
                "Created isolated branch '{}' for step {}",
                branch_name, step.id
            );
        }
        Err(FlowError::NotSupported(_)) => {
            // Fall back to normal execution
            warn!("Branch creation not supported, executing step normally");
            return execute_step_inner(step, instance, flow_def, callbacks).await;
        }
        Err(e) => {
            error!("Failed to create isolated branch: {}", e);
            return Err(FlowError::BranchOperationFailed(format!(
                "Failed to create isolated branch '{}': {}",
                branch_name, e
            )));
        }
    }

    // 2. Switch to isolated branch
    if let Err(e) = callbacks.switch_branch(&branch_name).await {
        error!("Failed to switch to isolated branch: {}", e);
        // Try to clean up
        let _ = callbacks.delete_branch(&branch_name).await;
        return Err(FlowError::BranchOperationFailed(format!(
            "Failed to switch to branch '{}': {}",
            branch_name, e
        )));
    }

    // 3. Execute the step
    let result = execute_step_inner(step, instance, flow_def, callbacks).await;

    // 4. Handle result
    match &result {
        Ok(StepResult::Continue { .. }) | Ok(StepResult::Complete { .. }) => {
            handle_branch_success(
                step,
                instance,
                &branch_name,
                original_branch.as_deref(),
                callbacks,
                result,
            )
            .await
        }
        Ok(StepResult::Wait { .. }) | Ok(StepResult::SameStep { .. }) => {
            // Step is pausing - keep the branch active
            info!(
                "Step {} waiting/re-executing in isolated branch '{}'",
                step.id, branch_name
            );
            // Store branch name in instance for resume
            if let serde_json::Value::Object(ref mut vars) = instance.variables {
                vars.insert(
                    "__isolated_branch".to_string(),
                    serde_json::json!(branch_name),
                );
                vars.insert(
                    "__original_branch".to_string(),
                    serde_json::json!(original_branch),
                );
            }
            result
        }
        Ok(StepResult::Error { .. }) | Err(_) => {
            // Step failed - preserve branch for debugging
            warn!(
                "Step {} failed in isolated branch '{}', preserving for debugging",
                step.id, branch_name
            );

            // Switch back to original branch
            if let Some(ref orig) = original_branch {
                let _ = callbacks.switch_branch(orig).await;
            }

            // Store failed branch info
            if let serde_json::Value::Object(ref mut vars) = instance.variables {
                vars.insert(
                    "__failed_branch".to_string(),
                    serde_json::json!(branch_name),
                );
            }

            result
        }
    }
}

/// Handle successful step execution in an isolated branch by merging back
async fn handle_branch_success(
    step: &FlowNode,
    _instance: &mut FlowInstance,
    branch_name: &str,
    original_branch: Option<&str>,
    callbacks: &dyn FlowCallbacks,
    result: FlowResult<StepResult>,
) -> FlowResult<StepResult> {
    info!(
        "Step {} succeeded in isolated branch, attempting merge",
        step.id
    );

    // Switch back to original branch first
    if let Some(orig) = original_branch {
        if let Err(e) = callbacks.switch_branch(orig).await {
            error!("Failed to switch back to original branch: {}", e);
            return Err(FlowError::BranchOperationFailed(format!(
                "Failed to switch back to '{}': {}",
                orig, e
            )));
        }
    }

    // Check for conflicts
    if let Ok(has_conflicts) = callbacks
        .has_merge_conflicts(branch_name, original_branch)
        .await
    {
        if has_conflicts {
            error!(
                "Merge conflict detected for step {} in branch '{}'",
                step.id, branch_name
            );
            // Preserve the branch for debugging
            return Err(FlowError::MergeConflict {
                branch_name: branch_name.to_string(),
                details: format!(
                    "Step {} completed but changes conflict with concurrent modifications. Branch '{}' preserved.",
                    step.id, branch_name
                ),
            });
        }
    }

    // Perform merge
    match callbacks.merge_branch(branch_name, original_branch).await {
        Ok(_) => {
            info!(
                "Successfully merged branch '{}' for step {}",
                branch_name, step.id
            );
            // Clean up the branch
            let _ = callbacks.delete_branch(branch_name).await;
        }
        Err(FlowError::MergeConflict { .. }) => {
            error!("Merge conflict during step {} merge", step.id);
            return Err(FlowError::MergeConflict {
                branch_name: branch_name.to_string(),
                details: format!(
                    "Merge conflict for step {}. Branch '{}' preserved.",
                    step.id, branch_name
                ),
            });
        }
        Err(e) => {
            error!("Failed to merge branch: {}", e);
            return Err(FlowError::BranchOperationFailed(format!(
                "Failed to merge branch '{}': {}",
                branch_name, e
            )));
        }
    }

    result
}
