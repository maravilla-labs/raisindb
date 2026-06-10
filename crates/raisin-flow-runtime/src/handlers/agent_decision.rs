// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! AI-routed decision step handler.
//!
//! An agent picks the next step from a declared set of branches. Used by
//! OR containers with a `router` config (the agent decides which child
//! runs, after any deterministic REL rules have had their chance) and
//! authorable directly in runtime format as `step_type: agent_decision`.
//!
//! The agent is forced into structured output
//! `{ branch, reasoning, confidence }` with `branch` constrained to the
//! declared branch ids. An invalid branch or a confidence below
//! `min_confidence` routes to `default_branch` (when set) or skips to the
//! step's successor — never to an arbitrary model-invented target.

use super::{StepHandler, StepResult};
use crate::runtime::DataMapper;
use crate::types::{
    FlowCallbacks, FlowContext, FlowError, FlowExecutionEvent, FlowNode, FlowResult,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Instant;
use tracing::{debug, instrument, warn};

/// Handler for AI-routed decisions.
#[derive(Debug)]
pub struct AgentDecisionHandler;

impl AgentDecisionHandler {
    pub fn new() -> Self {
        Self
    }
}

impl Default for AgentDecisionHandler {
    fn default() -> Self {
        Self::new()
    }
}

/// A branch the router agent can choose from.
struct Branch {
    id: String,
    description: String,
}

fn parse_branches(step: &FlowNode) -> FlowResult<Vec<Branch>> {
    let branches: Vec<Branch> = step
        .get_property("branches")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|b| {
                    let id = b.get("id")?.as_str()?.to_string();
                    let description = b
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&id)
                        .to_string();
                    Some(Branch { id, description })
                })
                .collect()
        })
        .unwrap_or_default();

    if branches.is_empty() {
        return Err(FlowError::InvalidDefinition(format!(
            "Agent decision step '{}' has no branches to route to",
            step.id
        )));
    }
    Ok(branches)
}

#[async_trait]
impl StepHandler for AgentDecisionHandler {
    #[instrument(skip(self, context, callbacks), fields(step_id = %step.id))]
    async fn execute(
        &self,
        step: &FlowNode,
        context: &mut FlowContext,
        callbacks: &dyn FlowCallbacks,
    ) -> FlowResult<StepResult> {
        debug!("Executing agent decision step: {}", step.id);
        let step_start = Instant::now();

        let _ = callbacks
            .emit_event(
                &context.instance_id,
                FlowExecutionEvent::step_started(&step.id, None, "agent_decision"),
            )
            .await;

        let agent_ref = step.get_string("agent_ref").ok_or_else(|| {
            FlowError::MissingProperty(format!(
                "Agent decision step '{}' missing required property: agent_ref",
                step.id
            ))
        })?;
        let agent_workspace = step
            .get_string_property("agent_workspace")
            .unwrap_or_else(|| "functions".to_string());

        let branches = parse_branches(step)?;
        let branch_ids: Vec<&str> = branches.iter().map(|b| b.id.as_str()).collect();

        // The skip target when nothing qualifies: explicit default_branch,
        // else the step's successor (= skip the container entirely)
        let successor = step
            .next_node
            .clone()
            .or_else(|| step.get_string_property("next_node"))
            .unwrap_or_else(|| "end".to_string());
        let default_branch = step.get_string_property("default_branch");
        let min_confidence = step
            .get_property("min_confidence")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

        // Author-provided routing instructions (templated), else a default
        // built from the flow input
        let instructions: String = match step.get_property("prompt") {
            Some(prompt_value) => match DataMapper::map(prompt_value, context)? {
                Value::String(s) => s,
                Value::Null => String::new(),
                other => other.to_string(),
            },
            None => format!(
                "Decide which branch to take next in this workflow.\nWorkflow input:\n{}",
                serde_json::to_string_pretty(&context.input).unwrap_or_default()
            ),
        };

        // Optional workflow-context injection (include_context property)
        let instructions = super::context_injection::with_context_block(
            instructions,
            context,
            super::context_injection::ContextInjection::from_step(step),
        );

        let branch_list = branches
            .iter()
            .map(|b| format!("- {}: {}", b.id, b.description))
            .collect::<Vec<_>>()
            .join("\n");
        let user_content = format!(
            "{}\n\nAvailable branches:\n{}\n\nAnswer with the branch id that should run next.",
            instructions, branch_list
        );

        let response_format = json!({
            "type": "json_schema",
            "schema": {
                "type": "object",
                "properties": {
                    "branch": { "type": "string", "enum": branch_ids },
                    "reasoning": { "type": "string" },
                    "confidence": { "type": "number", "minimum": 0.0, "maximum": 1.0 }
                },
                "required": ["branch", "confidence"]
            }
        });

        let messages = vec![json!({ "role": "user", "content": user_content })];
        let ai_response = callbacks
            .call_ai(
                &agent_workspace,
                &agent_ref,
                messages,
                Some(response_format),
            )
            .await
            .map_err(|e| FlowError::AIProvider(format!("Agent decision AI call failed: {}", e)))?;

        let content = ai_response
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let parsed: Option<Value> = serde_json::from_str(&content).ok();

        let chosen = parsed
            .as_ref()
            .and_then(|p| p.get("branch"))
            .and_then(|v| v.as_str())
            .map(String::from);
        let confidence = parsed
            .as_ref()
            .and_then(|p| p.get("confidence"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let reasoning = parsed
            .as_ref()
            .and_then(|p| p.get("reasoning"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // Validate the decision: must be a declared branch AND confident
        // enough; otherwise fall back deterministically.
        let valid_branch = chosen
            .as_deref()
            .filter(|c| branches.iter().any(|b| b.id == *c))
            .map(String::from);
        let (next_node_id, routed) = match valid_branch {
            Some(branch) if confidence >= min_confidence => (branch, true),
            other => {
                warn!(
                    "Agent decision '{}': {} (confidence {:.2}, min {:.2}) - using fallback",
                    step.id,
                    match &other {
                        Some(b) => format!("low confidence for branch '{}'", b),
                        None => "no valid branch in response".to_string(),
                    },
                    confidence,
                    min_confidence
                );
                (default_branch.unwrap_or(successor), false)
            }
        };

        let output = json!({
            "branch": if routed { Value::String(next_node_id.clone()) } else { Value::Null },
            "routed_to": next_node_id,
            "routed_by_agent": routed,
            "reasoning": reasoning,
            "confidence": confidence,
            "response": content,
            "model": ai_response.get("model"),
        });

        let _ = callbacks
            .emit_event(
                &context.instance_id,
                FlowExecutionEvent::step_completed(
                    &step.id,
                    output.clone(),
                    step_start.elapsed().as_millis() as u64,
                ),
            )
            .await;

        Ok(StepResult::Continue {
            next_node_id,
            output,
        })
    }
}
