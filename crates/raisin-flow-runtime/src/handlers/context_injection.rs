// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Workflow-context injection for AI steps.
//!
//! Every AI construct (agent step, AI container, router, competition)
//! supports an `include_context` property that appends the workflow
//! context as a fenced JSON block to the agent's prompt - the same
//! pattern the agent-as-assignee handler uses. Templates remain the
//! precise tool ("{{ steps.reserve.total }}"); `include_context` is the
//! broad one ("here is everything, figure it out").
//!
//! Accepted values:
//! - `"none"` / `false` / absent — prompt only (default)
//! - `"input"` — append the flow input
//! - `"full"` / `true` — append input + step outputs + trigger info +
//!   flow variables (`FlowContext::to_json()`)

use crate::types::{FlowContext, FlowNode};
use serde_json::Value;

/// How much workflow context to append to the agent prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextInjection {
    None,
    Input,
    Full,
}

impl ContextInjection {
    /// Read the `include_context` property from a step.
    pub fn from_step(step: &FlowNode) -> Self {
        match step.get_property("include_context") {
            Some(Value::Bool(true)) => ContextInjection::Full,
            Some(Value::Bool(false)) | None => ContextInjection::None,
            Some(Value::String(s)) => match s.as_str() {
                "full" => ContextInjection::Full,
                "input" => ContextInjection::Input,
                _ => ContextInjection::None,
            },
            Some(_) => ContextInjection::None,
        }
    }

    /// Parse from a raw JSON value (e.g. nested config objects).
    pub fn from_value(value: Option<&Value>) -> Self {
        match value {
            Some(Value::Bool(true)) => ContextInjection::Full,
            Some(Value::String(s)) => match s.as_str() {
                "full" => ContextInjection::Full,
                "input" => ContextInjection::Input,
                _ => ContextInjection::None,
            },
            _ => ContextInjection::None,
        }
    }
}

/// Append the requested workflow-context block to a prompt. Returns the
/// prompt unchanged for `ContextInjection::None`.
pub fn with_context_block(
    prompt: String,
    context: &FlowContext,
    injection: ContextInjection,
) -> String {
    let payload = match injection {
        ContextInjection::None => return prompt,
        ContextInjection::Input => context.input.clone(),
        ContextInjection::Full => context.to_json(),
    };
    format!(
        "{}\n\n# Workflow context\n```json\n{}\n```",
        prompt,
        serde_json::to_string_pretty(&payload).unwrap_or_default()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    fn step_with_include(value: Value) -> FlowNode {
        let mut properties = HashMap::new();
        properties.insert("include_context".to_string(), value);
        FlowNode {
            id: "s".to_string(),
            step_type: crate::types::StepType::AgentStep,
            properties,
            children: Vec::new(),
            next_node: None,
        }
    }

    #[test]
    fn test_parse_modes() {
        assert_eq!(
            ContextInjection::from_step(&step_with_include(json!(true))),
            ContextInjection::Full
        );
        assert_eq!(
            ContextInjection::from_step(&step_with_include(json!("input"))),
            ContextInjection::Input
        );
        assert_eq!(
            ContextInjection::from_step(&step_with_include(json!("none"))),
            ContextInjection::None
        );
        assert_eq!(
            ContextInjection::from_step(&step_with_include(json!(false))),
            ContextInjection::None
        );
    }

    #[test]
    fn test_block_contains_input_and_steps() {
        let mut context = FlowContext::new("i-1".to_string(), json!({"sku": "A-1"}));
        context
            .step_outputs
            .insert("reserve".to_string(), json!({"total": 100}));

        let full = with_context_block("Do it.".to_string(), &context, ContextInjection::Full);
        assert!(full.starts_with("Do it."));
        assert!(full.contains("# Workflow context"));
        assert!(full.contains("\"sku\": \"A-1\""));
        assert!(full.contains("\"total\": 100"));

        let input_only =
            with_context_block("Do it.".to_string(), &context, ContextInjection::Input);
        assert!(input_only.contains("\"sku\""));
        assert!(!input_only.contains("\"total\""));

        let none = with_context_block("Do it.".to_string(), &context, ContextInjection::None);
        assert_eq!(none, "Do it.");
    }
}
