// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Loop step handler for iterating over collections.
//!
//! This handler provides iteration capabilities within workflows,
//! allowing steps to be executed for each item in a collection.

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::types::{FlowCallbacks, FlowContext, FlowError, FlowNode, FlowResult, StepResult};

use super::StepHandler;

/// Handler for loop steps that iterate over collections.
///
/// Loop steps support:
/// - Iterating over arrays
/// - Iterating over object keys/values
/// - While-style loops with conditions
/// - For-each with index access
pub struct LoopHandler;

impl LoopHandler {
    /// Create a new loop handler
    pub fn new() -> Self {
        Self
    }
}

impl Default for LoopHandler {
    fn default() -> Self {
        Self::new()
    }
}

/// Loop state stored in context during iteration
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct LoopState {
    /// Current iteration index (0-based)
    index: usize,
    /// Total number of items
    total: usize,
    /// Items being iterated
    items: Vec<Value>,
    /// Results collected from each iteration
    results: Vec<Value>,
    /// Variable name for current item
    item_var: String,
    /// Variable name for index (optional)
    index_var: Option<String>,
}

#[async_trait]
impl StepHandler for LoopHandler {
    async fn execute(
        &self,
        step: &FlowNode,
        context: &mut FlowContext,
        _callbacks: &dyn FlowCallbacks,
    ) -> FlowResult<StepResult> {
        // Check for existing loop state (resuming iteration)
        let loop_state_key = format!("__loop_state_{}", step.id);
        let existing_state: Option<LoopState> = context
            .variables
            .get(&loop_state_key)
            .and_then(|v| serde_json::from_value(v.clone()).ok());

        if let Some(mut state) = existing_state {
            // Continuing iteration
            return continue_loop(step, context, &mut state, &loop_state_key);
        }

        // Starting new loop
        let loop_type = step
            .get_string_property("loop_type")
            .unwrap_or_else(|| "for_each".to_string());

        match loop_type.as_str() {
            "for_each" => start_for_each_loop(step, context, &loop_state_key),
            "while" => start_while_loop(step, context, &loop_state_key),
            "times" => start_times_loop(step, context, &loop_state_key),
            _ => Err(FlowError::InvalidNodeConfiguration(format!(
                "Unknown loop type: {}",
                loop_type
            ))),
        }
    }
}

/// Start a for-each loop over a collection
fn start_for_each_loop(
    step: &FlowNode,
    context: &mut FlowContext,
    loop_state_key: &str,
) -> FlowResult<StepResult> {
    // Get the collection to iterate
    let collection_expr = step.get_string_property("collection").ok_or_else(|| {
        FlowError::MissingProperty("collection required for for_each loop".to_string())
    })?;

    // Resolve collection from context
    let items = resolve_collection(&collection_expr, context)?;

    if items.is_empty() {
        // Empty collection - skip to next step
        tracing::info!(step_id = %step.id, "Loop skipped - empty collection");

        // Set empty results
        context
            .variables
            .insert(format!("{}_results", step.id), json!([]));

        let next_node = step.next_node.clone().unwrap_or_else(|| "end".to_string());
        return Ok(StepResult::Continue {
            next_node_id: next_node,
            output: json!({"results": [], "count": 0}),
        });
    }

    // Get variable names
    let item_var = step
        .get_string_property("item_var")
        .unwrap_or_else(|| "item".to_string());
    let index_var = step.get_string_property("index_var");

    // Initialize loop state
    let state = LoopState {
        index: 0,
        total: items.len(),
        items: items.clone(),
        results: Vec::new(),
        item_var: item_var.clone(),
        index_var: index_var.clone(),
    };

    // Store loop state
    context.variables.insert(
        loop_state_key.to_string(),
        serde_json::to_value(&state).unwrap(),
    );

    // Set current item and index variables
    context.variables.insert(item_var, items[0].clone());
    if let Some(ref idx_var) = index_var {
        context.variables.insert(idx_var.clone(), json!(0));
    }
    context
        .variables
        .insert("__loop_index".to_string(), json!(0));
    context
        .variables
        .insert("__loop_total".to_string(), json!(items.len()));

    tracing::info!(
        step_id = %step.id,
        total = items.len(),
        "Starting for_each loop"
    );

    // Execute loop body (first child or specified body step)
    let body_step = step
        .get_string_property("body_step")
        .or_else(|| step.children.first().map(|c| c.id.clone()))
        .unwrap_or_else(|| step.id.clone());

    Ok(StepResult::Continue {
        next_node_id: body_step,
        output: json!({"iteration": 0, "item": items[0]}),
    })
}

/// Start a while loop with a condition
fn start_while_loop(
    step: &FlowNode,
    context: &mut FlowContext,
    loop_state_key: &str,
) -> FlowResult<StepResult> {
    // Get condition
    let condition = step.get_string_property("condition").ok_or_else(|| {
        FlowError::MissingProperty("condition required for while loop".to_string())
    })?;

    // Get max iterations (safety limit)
    let max_iterations = step.get_u32_property("max_iterations").unwrap_or(1000) as usize;

    // Initialize loop state
    let state = LoopState {
        index: 0,
        total: max_iterations,
        items: vec![], // Not used for while loops
        results: Vec::new(),
        item_var: "iteration".to_string(),
        index_var: Some("index".to_string()),
    };

    // Evaluate initial condition
    // For now, we use a simple truthy check
    // Full implementation would use REL evaluator
    let should_continue = evaluate_simple_condition(&condition, context);

    if !should_continue {
        // Condition false from start - skip loop
        let next_node = step.next_node.clone().unwrap_or_else(|| "end".to_string());
        return Ok(StepResult::Continue {
            next_node_id: next_node,
            output: json!({"results": [], "iterations": 0}),
        });
    }

    // Store state
    context.variables.insert(
        loop_state_key.to_string(),
        serde_json::to_value(&state).unwrap(),
    );
    context
        .variables
        .insert("__loop_index".to_string(), json!(0));

    tracing::info!(step_id = %step.id, "Starting while loop");

    // Execute loop body
    let body_step = step
        .get_string_property("body_step")
        .or_else(|| step.children.first().map(|c| c.id.clone()))
        .unwrap_or_else(|| step.id.clone());

    Ok(StepResult::Continue {
        next_node_id: body_step,
        output: json!({"iteration": 0}),
    })
}

/// Start a times loop (repeat N times)
fn start_times_loop(
    step: &FlowNode,
    context: &mut FlowContext,
    loop_state_key: &str,
) -> FlowResult<StepResult> {
    // Get number of times to repeat
    let times = step
        .get_u32_property("times")
        .ok_or_else(|| FlowError::MissingProperty("times required for times loop".to_string()))?
        as usize;

    if times == 0 {
        let next_node = step.next_node.clone().unwrap_or_else(|| "end".to_string());
        return Ok(StepResult::Continue {
            next_node_id: next_node,
            output: json!({"results": [], "iterations": 0}),
        });
    }

    // Initialize loop state
    let state = LoopState {
        index: 0,
        total: times,
        items: (0..times).map(|i| json!(i)).collect(),
        results: Vec::new(),
        item_var: "iteration".to_string(),
        index_var: Some("index".to_string()),
    };

    // Store state
    context.variables.insert(
        loop_state_key.to_string(),
        serde_json::to_value(&state).unwrap(),
    );
    context
        .variables
        .insert("__loop_index".to_string(), json!(0));
    context
        .variables
        .insert("__loop_total".to_string(), json!(times));

    tracing::info!(step_id = %step.id, times = times, "Starting times loop");

    // Execute loop body
    let body_step = step
        .get_string_property("body_step")
        .or_else(|| step.children.first().map(|c| c.id.clone()))
        .unwrap_or_else(|| step.id.clone());

    Ok(StepResult::Continue {
        next_node_id: body_step,
        output: json!({"iteration": 0}),
    })
}

/// Continue an existing loop iteration
fn continue_loop(
    step: &FlowNode,
    context: &mut FlowContext,
    state: &mut LoopState,
    loop_state_key: &str,
) -> FlowResult<StepResult> {
    // Collect result from previous iteration
    if let Some(output) = &context.current_output {
        state.results.push(output.clone());
    }

    // Move to next iteration
    state.index += 1;

    // Check if loop is complete
    if state.index >= state.total {
        // Loop complete - clean up and continue
        context.variables.remove(loop_state_key);
        context.variables.remove(&state.item_var);
        if let Some(ref idx_var) = state.index_var {
            context.variables.remove(idx_var);
        }
        context.variables.remove("__loop_index");
        context.variables.remove("__loop_total");

        // Store results
        context
            .variables
            .insert(format!("{}_results", step.id), json!(state.results));

        tracing::info!(
            step_id = %step.id,
            iterations = state.index,
            "Loop completed"
        );

        let next_node = step.next_node.clone().unwrap_or_else(|| "end".to_string());
        return Ok(StepResult::Continue {
            next_node_id: next_node,
            output: json!({
                "results": state.results,
                "count": state.results.len()
            }),
        });
    }

    // Update loop variables for next iteration
    if !state.items.is_empty() {
        context
            .variables
            .insert(state.item_var.clone(), state.items[state.index].clone());
    }
    if let Some(ref idx_var) = state.index_var {
        context
            .variables
            .insert(idx_var.clone(), json!(state.index));
    }
    context
        .variables
        .insert("__loop_index".to_string(), json!(state.index));

    // Update stored state
    context.variables.insert(
        loop_state_key.to_string(),
        serde_json::to_value(&state).unwrap(),
    );

    tracing::debug!(
        step_id = %step.id,
        iteration = state.index,
        total = state.total,
        "Continuing loop iteration"
    );

    // Execute loop body again
    let body_step = step
        .get_string_property("body_step")
        .or_else(|| step.children.first().map(|c| c.id.clone()))
        .unwrap_or_else(|| step.id.clone());

    Ok(StepResult::Continue {
        next_node_id: body_step,
        output: json!({
            "iteration": state.index,
            "item": state.items.get(state.index).cloned().unwrap_or(Value::Null)
        }),
    })
}

/// Resolve a collection expression to a Vec<Value>
fn resolve_collection(expr: &str, context: &FlowContext) -> FlowResult<Vec<Value>> {
    // Handle variable reference
    let var_name = expr.trim_start_matches("$.").trim_start_matches('$');

    if let Some(value) = context.variables.get(var_name) {
        match value {
            Value::Array(arr) => Ok(arr.clone()),
            Value::Object(obj) => {
                // Iterate over object as key-value pairs
                Ok(obj
                    .iter()
                    .map(|(k, v)| json!({"key": k, "value": v}))
                    .collect())
            }
            _ => Err(FlowError::InvalidNodeConfiguration(format!(
                "Collection '{}' is not iterable",
                expr
            ))),
        }
    } else if let Some(value) = context.input.get(var_name) {
        match value {
            Value::Array(arr) => Ok(arr.clone()),
            _ => Err(FlowError::InvalidNodeConfiguration(format!(
                "Collection '{}' is not an array",
                expr
            ))),
        }
    } else {
        // Try to parse as JSON array literal
        serde_json::from_str(expr).map_err(|_| {
            FlowError::InvalidNodeConfiguration(format!("Could not resolve collection: {}", expr))
        })
    }
}

/// Simple condition evaluator (placeholder for REL integration)
fn evaluate_simple_condition(condition: &str, context: &FlowContext) -> bool {
    // Very simple truthy check - full implementation would use REL
    if let Some(value) = context.variables.get(condition.trim_start_matches('$')) {
        match value {
            Value::Bool(b) => *b,
            Value::Null => false,
            Value::Number(n) => n.as_f64().map(|f| f != 0.0).unwrap_or(false),
            Value::String(s) => !s.is_empty(),
            Value::Array(a) => !a.is_empty(),
            Value::Object(o) => !o.is_empty(),
        }
    } else {
        // Default to true for non-empty conditions
        !condition.trim().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn create_test_context() -> FlowContext {
        let mut variables = HashMap::new();
        variables.insert("items".to_string(), json!([1, 2, 3, 4, 5]));
        variables.insert("continue_flag".to_string(), json!(true));

        FlowContext {
            instance_id: "test-instance".to_string(),
            trigger_info: None,
            input: json!({}),
            step_outputs: HashMap::new(),
            variables,
            current_output: None,
            error: None,
            context_stack: Vec::new(),
            compensation_stack: Vec::new(),
        }
    }

    #[test]
    fn test_resolve_collection_array() {
        let context = create_test_context();
        let result = resolve_collection("$items", &context).unwrap();
        assert_eq!(result.len(), 5);
    }

    #[test]
    fn test_evaluate_simple_condition() {
        let context = create_test_context();
        assert!(evaluate_simple_condition("$continue_flag", &context));
    }
}
