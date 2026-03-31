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
    let max_retries = step.get_u32_property("max_retries").unwrap_or(3);
    // Using warn! to ensure visibility during debugging
    tracing::warn!(
        step_id = %step.id,
        max_retries = max_retries,
        has_max_retries_property = step.properties.contains_key("max_retries"),
        all_properties = ?step.properties.keys().collect::<Vec<_>>(),
        "[DEBUG] get_max_retries called"
    );
    max_retries
}

/// Calculate exponential backoff duration
pub(crate) fn calculate_backoff(retry_count: u32) -> chrono::Duration {
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

    FlowContext {
        instance_id: instance.id.clone(),
        trigger_info,
        input: instance.input.clone(),
        step_outputs,
        variables,
        current_output: None,
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
        assert_eq!(calculate_backoff(1), chrono::Duration::seconds(10));
        assert_eq!(calculate_backoff(2), chrono::Duration::seconds(30));
        assert_eq!(calculate_backoff(3), chrono::Duration::seconds(60));
        assert_eq!(calculate_backoff(4), chrono::Duration::seconds(120));
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
