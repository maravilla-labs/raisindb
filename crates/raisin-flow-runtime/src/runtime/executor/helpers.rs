// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Utility functions for flow execution
//!
//! Contains helper functions for wait types, backoff calculation,
//! context building, and context synchronization.

use crate::types::{FlowContext, FlowInstance, FlowNode, TriggerInfo, WaitType};
use chrono::Utc;
use serde_json::Value;
use std::collections::HashMap;
use uuid::Uuid;

/// Maximum number of retries for version conflicts
pub(crate) const MAX_VERSION_CONFLICT_RETRIES: u32 = 3;

/// Generate a unique subscription ID
pub(crate) fn generate_subscription_id() -> String {
    Uuid::new_v4().to_string()
}

/// Parse wait type from string
pub(crate) fn parse_wait_type(s: &str) -> WaitType {
    match s.to_lowercase().as_str() {
        "tool_call" => WaitType::ToolCall,
        "human_task" => WaitType::HumanTask,
        "scheduled" => WaitType::Scheduled,
        "event" => WaitType::Event,
        "retry" => WaitType::Retry,
        "join" => WaitType::Join,
        "function_call" => WaitType::FunctionCall,
        "chat_session" => WaitType::ChatSession,
        _ => WaitType::Event,
    }
}

/// Calculate timeout from metadata
pub(crate) fn calculate_timeout(metadata: &Value) -> Option<chrono::DateTime<Utc>> {
    metadata
        .get("timeout_ms")
        .and_then(|v| v.as_u64())
        .map(|ms| Utc::now() + chrono::Duration::milliseconds(ms as i64))
}

/// Get max retries for a step
pub(crate) fn get_max_retries(step: &FlowNode) -> u32 {
    step.get_u32_property("max_retries").unwrap_or(3)
}

/// Calculate exponential backoff duration for a retry attempt.
///
/// Honors the step's retry config (`retry_base_delay_ms` doubled per
/// attempt, capped at `retry_max_delay_ms`); without a configured base
/// delay, falls back to the legacy 10/30/60/120s ladder.
pub(crate) fn calculate_backoff(retry_count: u32, step: Option<&FlowNode>) -> chrono::Duration {
    let base_delay_ms = step.and_then(|s| s.get_u64_property("retry_base_delay_ms"));
    if let Some(base_ms) = base_delay_ms.filter(|ms| *ms > 0) {
        let max_ms = step
            .and_then(|s| s.get_u64_property("retry_max_delay_ms"))
            .filter(|ms| *ms > 0)
            .unwrap_or(u64::MAX);
        let factor = 2u64.saturating_pow(retry_count.saturating_sub(1).min(32));
        let delay_ms = base_ms.saturating_mul(factor).min(max_ms);
        return chrono::Duration::milliseconds(delay_ms.min(i64::MAX as u64) as i64);
    }
    let seconds = match retry_count {
        1 => 10,
        2 => 30,
        3 => 60,
        _ => 120,
    };
    chrono::Duration::seconds(seconds)
}

/// Build a FlowContext from a FlowInstance
///
/// Converts the instance state into a context that handlers can use.
pub(crate) fn build_context_from_instance(instance: &FlowInstance) -> FlowContext {
    // Extract step outputs from instance variables if they exist
    let step_outputs: HashMap<String, Value> = instance
        .variables
        .as_object()
        .and_then(|obj| obj.get("step_outputs"))
        .and_then(|v| v.as_object())
        .map(|obj| obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();

    // Extract flow variables (excluding step_outputs)
    let mut variables: HashMap<String, Value> = instance
        .variables
        .as_object()
        .map(|obj| {
            obj.iter()
                .filter(|(k, _)| k.as_str() != "step_outputs")
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        })
        .unwrap_or_default();

    // If variables is empty, initialize as empty HashMap
    if variables.is_empty() {
        variables = HashMap::new();
    }

    // Extract trigger info from instance variables if stored there
    let trigger_info = instance
        .variables
        .as_object()
        .and_then(|obj| obj.get("__trigger_info"))
        .and_then(|v| serde_json::from_value::<TriggerInfo>(v.clone()).ok());

    // The previous step's output (recorded by the execution loop) - used by
    // loop steps to collect per-iteration results via context.current_output
    let current_output = instance
        .variables
        .as_object()
        .and_then(|obj| obj.get("__last_output"))
        .filter(|v| !v.is_null())
        .cloned();

    FlowContext {
        instance_id: instance.id.clone(),
        trigger_info,
        input: instance.input.clone(),
        step_outputs,
        variables,
        current_output,
        error: None,
        context_stack: Vec::new(),
        compensation_stack: instance.compensation_stack.clone(),
    }
}

/// Sync FlowContext changes back to FlowInstance
///
/// Updates the instance with any changes made to the context during step execution.
pub(crate) fn sync_context_to_instance(context: &FlowContext, instance: &mut FlowInstance) {
    use serde_json::Map;

    // Build variables object
    let mut vars_map = Map::new();

    // Add all flow variables
    for (key, value) in &context.variables {
        vars_map.insert(key.clone(), value.clone());
    }

    // Add step outputs under "step_outputs" namespace
    if !context.step_outputs.is_empty() {
        let step_outputs_obj: Map<String, Value> = context
            .step_outputs
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        vars_map.insert("step_outputs".to_string(), Value::Object(step_outputs_obj));
    }

    instance.variables = Value::Object(vars_map);

    // Sync compensation stack back
    instance.compensation_stack = context.compensation_stack.clone();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_backoff() {
        assert_eq!(calculate_backoff(1, None), chrono::Duration::seconds(10));
        assert_eq!(calculate_backoff(2, None), chrono::Duration::seconds(30));
        assert_eq!(calculate_backoff(3, None), chrono::Duration::seconds(60));
        assert_eq!(calculate_backoff(4, None), chrono::Duration::seconds(120));
    }

    #[test]
    fn test_calculate_backoff_uses_step_retry_config() {
        let mut step = FlowNode {
            id: "s1".to_string(),
            step_type: crate::types::StepType::FunctionStep,
            properties: HashMap::new(),
            children: Vec::new(),
            next_node: None,
        };
        step.properties.insert(
            "retry_base_delay_ms".to_string(),
            serde_json::json!(1000u64),
        );
        step.properties
            .insert("retry_max_delay_ms".to_string(), serde_json::json!(5000u64));

        // Exponential: 1000, 2000, 4000, then capped at 5000
        assert_eq!(
            calculate_backoff(1, Some(&step)),
            chrono::Duration::milliseconds(1000)
        );
        assert_eq!(
            calculate_backoff(2, Some(&step)),
            chrono::Duration::milliseconds(2000)
        );
        assert_eq!(
            calculate_backoff(3, Some(&step)),
            chrono::Duration::milliseconds(4000)
        );
        assert_eq!(
            calculate_backoff(4, Some(&step)),
            chrono::Duration::milliseconds(5000)
        );

        // Zero base delay falls back to the legacy ladder
        step.properties
            .insert("retry_base_delay_ms".to_string(), serde_json::json!(0u64));
        assert_eq!(
            calculate_backoff(1, Some(&step)),
            chrono::Duration::seconds(10)
        );
    }

    #[test]
    fn test_generate_subscription_id() {
        let id1 = generate_subscription_id();
        let id2 = generate_subscription_id();
        assert_ne!(id1, id2);
        assert_eq!(id1.len(), 36); // UUID v4 format
    }

    #[test]
    fn test_parse_wait_type() {
        assert_eq!(parse_wait_type("tool_call"), WaitType::ToolCall);
        assert_eq!(parse_wait_type("human_task"), WaitType::HumanTask);
        assert_eq!(parse_wait_type("retry"), WaitType::Retry);
        assert_eq!(parse_wait_type("unknown"), WaitType::Event);
    }
}
