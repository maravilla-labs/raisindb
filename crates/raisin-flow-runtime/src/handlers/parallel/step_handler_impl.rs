// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! StepHandler trait implementation for ParallelHandler
//!
//! Lifecycle of a parallel container step:
//! 1. First execution forks one child flow per branch and records the child
//!    instance ids in `__parallel_forked_{step_id}`.
//! 2. Each child reaching a terminal state resumes the parent with a
//!    `child_completed` payload (see `notify_parent_flow`), which is
//!    collected into `__parallel_results_{step_id}`.
//! 3. Once results for all children have arrived, the configured merge
//!    strategy joins them and the flow continues.

use crate::handlers::StepHandler;
use crate::types::{ChildFlowStatus, FlowCallbacks, FlowContext, FlowNode, FlowResult, StepResult};
use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::{debug, instrument};

use super::handler::{ParallelHandler, WAIT_REASON_PARALLEL};

#[async_trait]
impl StepHandler for ParallelHandler {
    #[instrument(skip(self, context, callbacks), fields(step_id = %step.id))]
    async fn execute(
        &self,
        step: &FlowNode,
        context: &mut FlowContext,
        callbacks: &dyn FlowCallbacks,
    ) -> FlowResult<StepResult> {
        debug!("Executing parallel container: {}", step.id);

        let forked_key = format!("__parallel_forked_{}", step.id);
        let results_key = format!("__parallel_results_{}", step.id);

        // Collect an arriving child result (resume path)
        if let Some(result) = context.variables.remove("__resume_data") {
            if result
                .get("child_completed")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                let child_id = result
                    .get("child_instance_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();

                let entry = json!({
                    "branch_id": result.get("branch_id"),
                    "status": result.get("status"),
                    "output": result.get("output"),
                    "error": result.get("error"),
                });

                let mut results = context
                    .variables
                    .get(&results_key)
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                if let Some(map) = results.as_object_mut() {
                    map.insert(child_id.clone(), entry);
                }
                context.variables.insert(results_key.clone(), results);

                debug!(child_instance_id = %child_id, "Recorded parallel branch result");
            } else {
                // Not a child-flow completion - restore for other consumers
                context
                    .variables
                    .insert("__resume_data".to_string(), result);
            }
        }

        // Already forked: wait for remaining children or join
        if let Some(forked) = context.variables.get(&forked_key).cloned() {
            let child_ids: Vec<String> = forked
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            let results = context
                .variables
                .get(&results_key)
                .cloned()
                .unwrap_or_else(|| json!({}));

            let arrived = child_ids
                .iter()
                .filter(|id| results.get(id.as_str()).is_some())
                .count();

            if arrived < child_ids.len() {
                debug!(
                    arrived = arrived,
                    total = child_ids.len(),
                    "Parallel branches still running, continuing to wait"
                );
                return Ok(StepResult::Wait {
                    reason: WAIT_REASON_PARALLEL.to_string(),
                    metadata: json!({
                        "child_flow_ids": child_ids,
                        "step_id": step.id,
                        "pending_count": child_ids.len() - arrived,
                    }),
                });
            }

            // All children terminal - build statuses and join
            let child_statuses: Vec<ChildFlowStatus> = child_ids
                .iter()
                .map(|id| {
                    let r = results.get(id.as_str()).cloned().unwrap_or(Value::Null);
                    ChildFlowStatus {
                        branch_id: r
                            .get("branch_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        instance_id: id.clone(),
                        status: r
                            .get("status")
                            .and_then(|v| v.as_str())
                            .unwrap_or("failed")
                            .to_string(),
                        output: r.get("output").cloned().filter(|v| !v.is_null()),
                        error: r.get("error").and_then(|v| v.as_str()).map(String::from),
                    }
                })
                .collect();

            // Clean up bookkeeping before continuing
            context.variables.remove(&forked_key);
            context.variables.remove(&results_key);

            return self.join_with_statuses(step, context, child_ids, child_statuses);
        }

        // First execution: fork branches and record the child ids
        let result = self.fork_branches(step, context, callbacks).await?;
        if let StepResult::Wait { ref metadata, .. } = result {
            if let Some(ids) = metadata.get("child_flow_ids") {
                context.variables.insert(forked_key, ids.clone());
            }
        }
        Ok(result)
    }
}
