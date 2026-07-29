// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Parallel handler struct and fork logic

use crate::types::{
    CreateChildFlowRequest, FlowCallbacks, FlowContext, FlowError, FlowNode, FlowResult, StepResult,
};
use serde_json::Value;
use tracing::{debug, error, instrument, warn};

/// Wait reason for parallel branches
pub(super) const WAIT_REASON_PARALLEL: &str = "parallel_branches";

/// Default cap on how many branches a `for_each` fan-out may create.
///
/// A fan-out is driven by runtime data, so an unbounded collection would
/// otherwise turn one step into an unbounded number of flow instances.
/// Override per step with `max_branches`.
pub(super) const DEFAULT_MAX_BRANCHES: usize = 500;

/// One branch to fork, after static branches / `for_each` items have been
/// normalized into the same shape.
struct PlannedBranch {
    /// Stable identifier, echoed back on the child's completion so the join
    /// can correlate a result with the item that produced it
    branch_id: String,
    /// Deployed flow to run, when the branch references one by path
    flow_path: Option<String>,
    /// Inline flow definition, when the branch carries its own graph
    flow_definition: Option<Value>,
    /// Resolved input for the child flow
    input: Value,
}

/// Handler for parallel execution containers
///
/// Supports fork/join patterns for executing multiple branches in parallel.
///
/// Two ways to say what the branches ARE:
///
/// - **Static** - a `branches` array authored in the flow, one entry per
///   branch, each with an inline `flow_definition`.
/// - **Dynamic fan-out** - `for_each` names a collection expression resolved
///   at run time (same forms as the loop step's `collection`) and `branch`
///   is a template instantiated once per item. This is what makes
///   "one child per row, then join" expressible, including when each child
///   parks on a human task:
///
///   ```yaml
///   - id: collect-approvals
///     type: parallel
///     properties:
///       for_each: "${steps.plan.items}"
///       max_branches: 200            # optional, defaults to 500
///       branch:
///         id: "item-${index}"        # optional, defaults to "{step_id}-{index}"
///         flow_path: "/flows/approve-item"
///         input_mapping:
///           item: "${item}"
///           index: "${index}"
///       merge_strategy: all_success
///   ```
///
///   Inside `branch`, `item` and `index` are bound to the current element
///   and its position. A branch may use `flow_path` (a deployed flow) or
///   `flow_definition` (an inline graph).
///
/// The join is the same in both cases: every child must reach a terminal
/// state before the configured `merge_strategy` runs, so a fan-out whose
/// children each wait for a person is a real join, not a poll.
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

        let planned = if step.get_property("for_each").is_some() {
            self.plan_fan_out_branches(step, context)?
        } else {
            self.plan_static_branches(step, context)?
        };

        if planned.is_empty() {
            warn!("Parallel container '{}' has no branches", step.id);
            return Ok(StepResult::Continue {
                next_node_id: step.next_node.clone().unwrap_or_else(|| "end".to_string()),
                output: serde_json::json!({}),
            });
        }

        debug!("Creating {} child flows", planned.len());

        // Create child flow for each branch
        let mut child_flow_ids = Vec::new();
        let mut branch_ids = Vec::new();

        for branch in &planned {
            let (job_type, payload) = self.child_flow_job(step, context, branch)?;

            match callbacks.queue_job(job_type, payload).await {
                Ok(child_id) => {
                    debug!(
                        "Created child flow '{}' with ID: {}",
                        branch.branch_id, child_id
                    );
                    child_flow_ids.push(child_id);
                    branch_ids.push(branch.branch_id.clone());
                }
                Err(e) => {
                    error!(
                        "Failed to create child flow for branch '{}': {}",
                        branch.branch_id, e
                    );
                    return Err(FlowError::ChildFlowError(format!(
                        "Failed to create child flow for branch '{}': {}",
                        branch.branch_id, e
                    )));
                }
            }
        }

        // Return Wait result with child flow IDs
        Ok(StepResult::Wait {
            reason: WAIT_REASON_PARALLEL.to_string(),
            metadata: serde_json::json!({
                "child_flow_ids": child_flow_ids,
                "branch_ids": branch_ids,
                "step_id": step.id,
                "total_branches": planned.len(),
            }),
        })
    }

    /// Normalize a statically authored `branches` array.
    fn plan_static_branches(
        &self,
        step: &FlowNode,
        context: &FlowContext,
    ) -> FlowResult<Vec<PlannedBranch>> {
        let branches = step.get_array("branches").ok_or_else(|| {
            FlowError::MissingProperty(format!(
                "Parallel container '{}' missing required property: branches (or for_each)",
                step.id
            ))
        })?;

        let mut planned = Vec::with_capacity(branches.len());
        for (index, branch) in branches.iter().enumerate() {
            let branch_obj = branch.as_object().ok_or_else(|| {
                FlowError::InvalidNodeConfiguration(format!(
                    "Branch {} in parallel container '{}' is not an object",
                    index, step.id
                ))
            })?;

            let branch_id = branch_obj
                .get("id")
                .and_then(|v| v.as_str())
                .map(String::from)
                .unwrap_or_else(|| format!("branch-{}", index));

            let flow_path = branch_obj
                .get("flow_path")
                .and_then(|v| v.as_str())
                .map(String::from);
            let flow_definition = branch_obj.get("flow_definition").cloned();

            if flow_path.is_none() && flow_definition.is_none() {
                return Err(FlowError::MissingProperty(format!(
                    "Branch '{}' missing flow_definition (or flow_path)",
                    branch_id
                )));
            }

            // Build input for child flow: resolve the mapping's template
            // expressions against the parent context (e.g.
            // { "item": "${input.orders[0]}", "run": "${steps.prep.run_id}" })
            let input = match branch_obj.get("input_mapping") {
                Some(mapping) => crate::runtime::DataMapper::map(mapping, context)?,
                // Default: pass entire parent input
                None => context.input.clone(),
            };

            planned.push(PlannedBranch {
                branch_id,
                flow_path,
                flow_definition,
                input,
            });
        }

        Ok(planned)
    }

    /// Instantiate the `branch` template once per item in `for_each`.
    fn plan_fan_out_branches(
        &self,
        step: &FlowNode,
        context: &FlowContext,
    ) -> FlowResult<Vec<PlannedBranch>> {
        let for_each = step.get_string("for_each").ok_or_else(|| {
            FlowError::InvalidNodeConfiguration(format!(
                "Parallel container '{}' property 'for_each' must be a collection expression string",
                step.id
            ))
        })?;

        let mut items = crate::handlers::collection::resolve_collection(&for_each, context)?;

        let max_branches = step
            .get_u32_property("max_branches")
            .map(|v| v as usize)
            .unwrap_or(DEFAULT_MAX_BRANCHES);
        if items.len() > max_branches {
            // Truncating silently would look like a completed fan-out that
            // simply had fewer items, so this is loud.
            warn!(
                step_id = %step.id,
                total = items.len(),
                max_branches,
                "for_each collection exceeds max_branches - truncating fan-out"
            );
            items.truncate(max_branches);
        }

        let template = step.get_object("branch").ok_or_else(|| {
            FlowError::MissingProperty(format!(
                "Parallel container '{}' uses for_each but is missing the 'branch' template",
                step.id
            ))
        })?;

        let mut planned = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            // Per-item context: `item` and `index` are visible to every
            // template expression in the branch template. Cloning keeps the
            // parent context untouched, so one item's bindings can never
            // leak into the next.
            let mut item_context = context.clone();
            item_context
                .variables
                .insert("item".to_string(), item.clone());
            item_context
                .variables
                .insert("index".to_string(), Value::from(index));

            let branch_id = match template.get("id") {
                Some(id_expr) => crate::runtime::DataMapper::map(id_expr, &item_context)?
                    .as_str()
                    .map(String::from)
                    .ok_or_else(|| {
                        FlowError::InvalidNodeConfiguration(format!(
                            "Parallel container '{}' branch id did not resolve to a string",
                            step.id
                        ))
                    })?,
                None => format!("{}-{}", step.id, index),
            };

            let flow_path = match template.get("flow_path") {
                Some(path_expr) => Some(
                    crate::runtime::DataMapper::map(path_expr, &item_context)?
                        .as_str()
                        .map(String::from)
                        .ok_or_else(|| {
                            FlowError::InvalidNodeConfiguration(format!(
                                "Parallel container '{}' branch flow_path did not resolve to a string",
                                step.id
                            ))
                        })?,
                ),
                None => None,
            };
            let flow_definition = template.get("flow_definition").cloned();

            if flow_path.is_none() && flow_definition.is_none() {
                return Err(FlowError::MissingProperty(format!(
                    "Parallel container '{}' branch template needs flow_path or flow_definition",
                    step.id
                )));
            }

            // Default input is the item itself - the common case is "run this
            // flow for this row" and spelling out an input_mapping for it is
            // noise.
            let input = match template.get("input_mapping") {
                Some(mapping) => crate::runtime::DataMapper::map(mapping, &item_context)?,
                None => item.clone(),
            };

            planned.push(PlannedBranch {
                branch_id,
                flow_path,
                flow_definition,
                input,
            });
        }

        Ok(planned)
    }

    /// Build the job type + payload that creates one child flow.
    ///
    /// A branch referencing a deployed flow goes through `flow_execution`
    /// (which loads and version-pins the stored definition); an inline
    /// branch goes through `create_child_flow`. Both carry `branch_id` so
    /// the child echoes it back on completion and the join can correlate.
    fn child_flow_job(
        &self,
        step: &FlowNode,
        context: &FlowContext,
        branch: &PlannedBranch,
    ) -> FlowResult<(&'static str, Value)> {
        if let Some(flow_path) = &branch.flow_path {
            return Ok((
                "flow_execution",
                serde_json::json!({
                    "flow_path": flow_path,
                    "input": branch.input,
                    "parent_instance_id": context.instance_id,
                    "parent_step_id": step.id,
                    "branch_id": branch.branch_id,
                }),
            ));
        }

        let request = CreateChildFlowRequest {
            branch_id: branch.branch_id.clone(),
            parent_instance_id: context.instance_id.clone(),
            parent_step_id: Some(step.id.clone()),
            flow_definition: branch.flow_definition.clone().unwrap_or(Value::Null),
            input: branch.input.clone(),
        };

        let payload = serde_json::to_value(&request).map_err(|e| {
            FlowError::InvalidNodeConfiguration(format!(
                "Failed to serialize child flow request for branch '{}': {}",
                branch.branch_id, e
            ))
        })?;

        Ok(("create_child_flow", payload))
    }
}

impl Default for ParallelHandler {
    fn default() -> Self {
        Self::new()
    }
}
