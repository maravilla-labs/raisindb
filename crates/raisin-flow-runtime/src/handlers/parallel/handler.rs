// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Parallel handler struct and fork logic

use crate::types::{
    CreateChildFlowRequest, FlowCallbacks, FlowContext, FlowError, FlowNode, FlowResult, StepResult,
};
use tracing::{debug, error, instrument, warn};

/// Wait reason for parallel branches
pub(super) const WAIT_REASON_PARALLEL: &str = "parallel_branches";

/// Handler for parallel execution containers
///
/// Supports fork/join patterns for executing multiple branches in parallel.
#[derive(Debug)]
pub struct ParallelHandler;

impl ParallelHandler {
    /// Create a new parallel handler
    pub fn new() -> Self {
        Self
    }

    /// Fork: Create child flows for each branch
    #[instrument(skip(self, step, context, callbacks), fields(step_id = %step.id))]
    pub(super) async fn fork_branches(
        &self,
        step: &FlowNode,
        context: &FlowContext,
        callbacks: &dyn FlowCallbacks,
    ) -> FlowResult<StepResult> {
        debug!("Forking parallel branches for step: {}", step.id);

        // Get branches from step properties
        let branches = step.get_array("branches").ok_or_else(|| {
            FlowError::MissingProperty(format!(
                "Parallel container '{}' missing required property: branches",
                step.id
            ))
        })?;

        if branches.is_empty() {
            warn!("Parallel container '{}' has no branches", step.id);
            return Ok(StepResult::Continue {
                next_node_id: step.next_node.clone().unwrap_or_else(|| "end".to_string()),
                output: serde_json::json!({}),
            });
        }

        debug!("Creating {} child flows", branches.len());

        // Create child flow for each branch
        let mut child_flow_ids = Vec::new();

        for (index, branch) in branches.iter().enumerate() {
            let branch_obj = branch.as_object().ok_or_else(|| {
                FlowError::InvalidNodeConfiguration(format!(
                    "Branch {} in parallel container '{}' is not an object",
                    index, step.id
                ))
            })?;

            // Get branch ID
            let default_branch_id = format!("branch-{}", index);
            let branch_id = branch_obj
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or(&default_branch_id);

            // Get flow definition for this branch
            let flow_definition = branch_obj
                .get("flow_definition")
                .ok_or_else(|| {
                    FlowError::MissingProperty(format!(
                        "Branch '{}' missing flow_definition",
                        branch_id
                    ))
                })?
                .clone();

            // Build input for child flow
            let child_input = if let Some(input_mapping) = branch_obj.get("input_mapping") {
                // TODO: Map parent context to child input based on mapping
                input_mapping.clone()
            } else {
                // Default: pass entire parent input
                context.input.clone()
            };

            // Create child flow request
            let request = CreateChildFlowRequest {
                branch_id: branch_id.to_string(),
                parent_instance_id: context.instance_id.clone(),
                flow_definition,
                input: child_input,
            };

            // Create the child flow via callbacks
            // Note: FlowCallbacks doesn't have create_child_flow yet, use queue_job as workaround
            match callbacks
                .queue_job(
                    "create_child_flow",
                    serde_json::to_value(&request).unwrap_or_default(),
                )
                .await
            {
                Ok(child_id) => {
                    debug!("Created child flow '{}' with ID: {}", branch_id, child_id);
                    child_flow_ids.push(child_id);
                }
                Err(e) => {
                    error!(
                        "Failed to create child flow for branch '{}': {}",
                        branch_id, e
                    );
                    return Err(FlowError::ChildFlowError(format!(
                        "Failed to create child flow for branch '{}': {}",
                        branch_id, e
                    )));
                }
            }
        }

        // Return Wait result with child flow IDs
        Ok(StepResult::Wait {
            reason: WAIT_REASON_PARALLEL.to_string(),
            metadata: serde_json::json!({
                "child_flow_ids": child_flow_ids,
                "step_id": step.id,
                "total_branches": branches.len(),
            }),
        })
    }
}

impl Default for ParallelHandler {
    fn default() -> Self {
        Self::new()
    }
}
