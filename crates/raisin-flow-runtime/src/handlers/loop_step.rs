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
    /// No iteration ceiling: run while the condition holds.
    ///
    /// `#[serde(default)]` so loop state persisted before this field existed
    /// still deserialises, as bounded — the safe reading.
    #[serde(default)]
    unbounded: bool,
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

    // Resolve collection from context (shared with the parallel
    // container's `for_each`, so both accept the same expression forms)
    let mut items = crate::handlers::collection::resolve_collection(&collection_expr, context)?;

    // Optional safety cap on the number of iterations
    if let Some(max) = step.get_u32_property("max_iterations") {
        items.truncate(max as usize);
    }

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
        // A for_each is bounded by its collection by definition.
        unbounded: false,
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

    // A `while` loop terminates on its CONDITION; a count is a safety net, not
    // semantics. `unbounded: true` removes the net deliberately — which is only
    // reasonable because the executor bounds a runaway itself
    // (`MAX_STEPS_PER_EXECUTION`), so a mis-written condition costs a failed flow
    // rather than a wedged worker. Absent, the ceiling stays at 1000: silently
    // dropping a safety limit is not something an author should get by omission.
    let unbounded = step
        .properties
        .get("unbounded")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let max_iterations = if unbounded {
        usize::MAX
    } else {
        step.get_u32_property("max_iterations").unwrap_or(1000) as usize
    };

    // Initialize loop state
    let state = LoopState {
        index: 0,
        total: max_iterations,
        items: vec![], // Not used for while loops
        results: Vec::new(),
        item_var: "iteration".to_string(),
        index_var: Some("index".to_string()),
        unbounded,
    };

    // REL, the same evaluator decision steps and the `until` clause use.
    //
    // This used to call a placeholder truthy check that looked up the condition
    // string as a VARIABLE NAME and, failing that, returned `true` for any
    // non-empty string. So every real expression — `steps.critic.passed == false`,
    // anything with an operator in it — evaluated to `true` unconditionally, and
    // a `while` loop silently became "run max_iterations times".
    let should_continue = evaluate_rel_condition(&condition, context)?;

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
        unbounded: false,
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

    // Early-exit: the optional 'until' REL condition is evaluated after
    // each completed iteration, with the just-finished iteration's loop
    // variables and step outputs visible. When true the loop finishes
    // with the results collected so far.
    let until_satisfied = match step.get_string_property("until") {
        Some(condition) => evaluate_rel_condition(&condition, context)?,
        None => false,
    };

    // A `while` loop re-tests its CONDITION after every iteration — that is what
    // makes it a while loop. It was previously evaluated once, at loop start, and
    // never again: `continue_loop` only ever looked at `until` and the index, so
    // the loop ran `max_iterations` times regardless of the condition going
    // false. `state.total` is still the safety ceiling.
    let is_while = step.get_string_property("loop_type").as_deref() == Some("while");
    let while_exhausted = match (is_while, step.get_string_property("condition")) {
        (true, Some(condition)) => !evaluate_rel_condition(&condition, context)?,
        _ => false,
    };

    // Move to next iteration
    state.index += 1;

    // Check if loop is complete. An unbounded `while` has no index ceiling —
    // its condition (re-tested above) and `until` are the only ways out, with the
    // executor's own step budget as the backstop against a condition that never
    // goes false.
    let ceiling_reached = !state.unbounded && state.index >= state.total;
    if until_satisfied || while_exhausted || ceiling_reached {
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

/// Evaluate a raisin-rel condition against the flow context (same
/// namespaces as decision steps: input.*, steps.*, trigger.*, variables)
fn evaluate_rel_condition(condition: &str, context: &FlowContext) -> FlowResult<bool> {
    let eval_ctx = raisin_rel::EvalContext::from_json(context.to_json()).map_err(|e| {
        FlowError::ConditionEvaluation(format!("Invalid evaluation context: {}", e))
    })?;

    // Safety: raisin_rel::eval is a sandboxed expression evaluator (no
    // side effects, no host access) - the same engine decision steps use.
    let result = raisin_rel::eval(condition, &eval_ctx).map_err(|e| {
        FlowError::ConditionEvaluation(format!("Loop 'until' evaluation failed: {}", e))
    })?;

    Ok(match result {
        raisin_rel::Value::Boolean(b) => b,
        raisin_rel::Value::Null => false,
        raisin_rel::Value::Integer(n) => n != 0,
        raisin_rel::Value::Float(f) => f != 0.0,
        raisin_rel::Value::String(s) => !s.is_empty(),
        raisin_rel::Value::Array(arr) => !arr.is_empty(),
        raisin_rel::Value::Object(obj) => !obj.is_empty(),
    })
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
        let result = crate::handlers::collection::resolve_collection("$items", &context).unwrap();
        assert_eq!(result.len(), 5);
    }

    /// An unbounded `while` has no index ceiling: `state.total` is not what ends
    /// it. Before this, `max_iterations` (default 1000) was always the real
    /// terminator, so "run while the condition holds" could not be expressed.
    #[test]
    fn test_unbounded_while_state_has_no_ceiling() {
        let step = FlowNode {
            id: "spin".to_string(),
            step_type: crate::types::StepType::Loop,
            properties: [
                ("loop_type".to_string(), json!("while")),
                ("condition".to_string(), json!("true")),
                ("unbounded".to_string(), json!(true)),
            ]
            .into_iter()
            .collect(),
            children: Vec::new(),
            next_node: Some("end".to_string()),
        };
        let mut context = create_test_context();
        let result = start_while_loop(&step, &mut context, "__loop_spin").unwrap();
        assert!(matches!(result, StepResult::Continue { .. }));

        let state: LoopState =
            serde_json::from_value(context.variables.get("__loop_spin").unwrap().clone()).unwrap();
        assert!(state.unbounded, "the flag must survive into the loop state");
        assert_eq!(state.total, usize::MAX, "no ceiling");

        // And the bounded default is unchanged when the flag is absent.
        let mut bounded_step = step.clone();
        bounded_step.properties.remove("unbounded");
        let mut context = create_test_context();
        start_while_loop(&bounded_step, &mut context, "__loop_spin").unwrap();
        let state: LoopState =
            serde_json::from_value(context.variables.get("__loop_spin").unwrap().clone()).unwrap();
        assert!(!state.unbounded);
        assert_eq!(state.total, 1000, "the safety ceiling stays by default");
    }

    /// Loop state persisted before `unbounded` existed must deserialise as
    /// BOUNDED - the safe reading.
    #[test]
    fn test_legacy_loop_state_reads_as_bounded() {
        let legacy = json!({
            "index": 2, "total": 10, "items": [], "results": [],
            "item_var": "iteration", "index_var": "index"
        });
        let state: LoopState = serde_json::from_value(legacy).unwrap();
        assert!(!state.unbounded);
    }

    /// The `while` condition is REL, not a variable-name truthy check.
    ///
    /// The placeholder this replaces looked the condition string up as a
    /// VARIABLE and, failing that, returned `true` for any non-empty string —
    /// so every real expression evaluated to `true` and a `while` loop silently
    /// meant "run max_iterations times". These are the exact shapes that used to
    /// be indistinguishable.
    #[test]
    fn test_while_condition_is_evaluated_as_rel() {
        let context = create_test_context();

        // A real expression that is FALSE must evaluate false, not "non-empty
        // string, therefore true".
        assert!(
            !evaluate_rel_condition("1 == 2", &context).unwrap(),
            "a false expression must not read as true"
        );
        assert!(evaluate_rel_condition("1 == 1", &context).unwrap());

        // A variable still works, now through the expression language.
        assert!(evaluate_rel_condition("continue_flag", &context).unwrap());
    }
}
