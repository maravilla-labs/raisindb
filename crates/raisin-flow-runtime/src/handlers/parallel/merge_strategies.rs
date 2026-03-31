// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Merge strategies for joining parallel branch results

use crate::types::{ChildFlowStatus, FlowContext, FlowError, FlowNode, FlowResult, StepResult};
use serde_json::Value;
use tracing::{debug, instrument};

use super::handler::{ParallelHandler, WAIT_REASON_PARALLEL};

impl ParallelHandler {
    /// Join: Check if all branches completed and merge results
    ///
    /// Note: This method requires child_statuses to be passed in since we don't have
    /// a get_child_status method on FlowCallbacks yet. In production, this would be
    /// called by the executor after gathering child flow statuses.
    #[instrument(skip(self, step, context), fields(step_id = %step.id))]
    pub fn join_with_statuses(
        &self,
        step: &FlowNode,
        context: &mut FlowContext,
        child_flow_ids: Vec<String>,
        child_statuses: Vec<ChildFlowStatus>,
    ) -> FlowResult<StepResult> {
        debug!("Joining parallel branches for step: {}", step.id);
        debug!("Retrieved {} child flow statuses", child_statuses.len());

        // Check if all completed
        let all_completed = child_statuses
            .iter()
            .all(|s| s.status == "completed" || s.status == "failed");

        if !all_completed {
            debug!("Not all branches completed yet, continuing to wait");
            return Ok(StepResult::Wait {
                reason: WAIT_REASON_PARALLEL.to_string(),
                metadata: serde_json::json!({
                    "child_flow_ids": child_flow_ids,
                    "step_id": step.id,
                    "pending_count": child_statuses.iter().filter(|s| s.status == "running" || s.status == "waiting").count(),
                }),
            });
        }

        // Get merge strategy
        let merge_strategy = step
            .get_string("merge_strategy")
            .unwrap_or_else(|| "merge_all".to_string());

        debug!("Using merge strategy: {}", merge_strategy);

        // Merge outputs based on strategy
        match merge_strategy.as_str() {
            "merge_all" => self.merge_all_outputs(step, context, &child_statuses),
            "first_success" => self.first_success_output(step, context, &child_statuses),
            "all_success" => self.all_success_output(step, context, &child_statuses),
            _ => Err(FlowError::InvalidNodeConfiguration(format!(
                "Unknown merge strategy: {}",
                merge_strategy
            ))),
        }
    }

    /// Merge all outputs into context (regardless of success/failure)
    pub(super) fn merge_all_outputs(
        &self,
        step: &FlowNode,
        context: &mut FlowContext,
        child_statuses: &[ChildFlowStatus],
    ) -> FlowResult<StepResult> {
        let mut merged_output = serde_json::Map::new();

        for (index, child_status) in child_statuses.iter().enumerate() {
            let branch_key = format!("branch_{}", index);

            let branch_result = serde_json::json!({
                "status": child_status.status,
                "output": child_status.output,
                "error": child_status.error,
            });

            merged_output.insert(branch_key, branch_result);

            // If child has output, merge it into context
            if let Some(output) = &child_status.output {
                context.merge_output(output.clone());
            }
        }

        Ok(StepResult::Continue {
            next_node_id: step.next_node.clone().unwrap_or_else(|| "end".to_string()),
            output: Value::Object(merged_output),
        })
    }

    /// Return first successful output, fail if all failed
    pub(super) fn first_success_output(
        &self,
        step: &FlowNode,
        context: &mut FlowContext,
        child_statuses: &[ChildFlowStatus],
    ) -> FlowResult<StepResult> {
        // Find first successful child
        for child_status in child_statuses {
            if child_status.status == "completed" {
                if let Some(output) = &child_status.output {
                    context.merge_output(output.clone());
                    return Ok(StepResult::Continue {
                        next_node_id: step.next_node.clone().unwrap_or_else(|| "end".to_string()),
                        output: output.clone(),
                    });
                }
            }
        }

        // All failed
        Err(FlowError::AllChildFlowsFailed)
    }

    /// Only succeed if all branches succeeded
    pub(super) fn all_success_output(
        &self,
        step: &FlowNode,
        context: &mut FlowContext,
        child_statuses: &[ChildFlowStatus],
    ) -> FlowResult<StepResult> {
        // Check if any failed
        let failed_branches: Vec<_> = child_statuses
            .iter()
            .filter(|s| s.status == "failed")
            .collect();

        if !failed_branches.is_empty() {
            let errors: Vec<_> = failed_branches
                .iter()
                .filter_map(|s| s.error.as_ref())
                .cloned()
                .collect();

            return Err(FlowError::ParallelExecutionError(format!(
                "{} branches failed: {}",
                failed_branches.len(),
                errors.join(", ")
            )));
        }

        // All succeeded, merge outputs
        self.merge_all_outputs(step, context, child_statuses)
    }
}
