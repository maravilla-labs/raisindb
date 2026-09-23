// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! The flow's AI agent step: the agent runs as a durable AgentRun.
//!
//! The step does not call a model itself. On first entry it asks the AI
//! package's start function ([`FLOW_AGENT_RUN_FUNCTION`]) to open a
//! conversation with the step's prompt and create an AgentRun for the agent —
//! the same run every conversation uses, with the agent's own tools, its
//! budgets, stop / steer / approve, leases and resume. The run carries a
//! WAITER naming this flow instance, and the step parks (`agent_run` wait).
//!
//! When the run ends, core hands its result to the waiter through a
//! `FlowInstanceExecution` resume job (durably owed until enqueued, on any
//! node), and the step re-enters with it: the run's final answer becomes the
//! step's `response`, parsed into `structured_output` when a
//! `response_format` was asked for. A run that failed or was stopped fails the
//! step through the ordinary error machinery (retry, error edge, rollback).
//!
//! Use cases: classify email, extract entities, summarize — including agents
//! that need their own tools to do so.

use super::{StepHandler, StepResult};
use crate::runtime::DataMapper;
use crate::types::{
    FlowCallbacks, FlowContext, FlowError, FlowExecutionEvent, FlowNode, FlowResult,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::{debug, instrument, warn};

/// The AI package function that starts a flow step's AgentRun.
pub const FLOW_AGENT_RUN_FUNCTION: &str = "/lib/raisin/ai/flow-agent-run";

/// Wait reason (and `WaitType`) of a step waiting for its run.
pub const AGENT_RUN_WAIT: &str = "agent_run";

/// Variable the resume stores the run's result under.
pub const AGENT_RUN_RESULT_VAR: &str = "__agent_run_result";

/// The `expected_event` of the wait: which run the step is waiting for.
pub fn expected_event(run_id: &str) -> String {
    format!("{AGENT_RUN_WAIT}:{run_id}")
}

/// Default model-call budget of a step's run (the old tool-loop cap was 5).
const MAX_MODEL_CALLS_DEFAULT: u32 = 8;

/// Handler for AI agent steps.
#[derive(Debug, Default)]
pub struct AgentStepHandler;

impl AgentStepHandler {
    /// A handler.
    pub fn new() -> Self {
        Self
    }
}

fn agent_ref_of(step: &FlowNode, context: &FlowContext) -> FlowResult<String> {
    let raw = step.get_property("agent_ref").cloned().ok_or_else(|| {
        FlowError::MissingProperty(format!(
            "Agent step '{}' missing required property: agent_ref",
            step.id
        ))
    })?;
    let resolved = DataMapper::map(&raw, context)?;
    resolved
        .as_str()
        .map(String::from)
        .or_else(|| {
            resolved.as_object().and_then(|obj| {
                obj.get("raisin:path")
                    .or_else(|| obj.get("raisin:ref"))
                    .and_then(Value::as_str)
                    .map(String::from)
            })
        })
        .ok_or_else(|| {
            FlowError::InvalidDefinition(format!(
                "Agent step '{}' resolved agent_ref to an invalid value",
                step.id
            ))
        })
}

/// The user message: an explicit `prompt` (or `message`), template-resolved;
/// otherwise the triggering content from the flow input.
fn prompt_of(step: &FlowNode, context: &FlowContext) -> FlowResult<String> {
    let text = match step
        .get_property("prompt")
        .or_else(|| step.get_property("message"))
    {
        Some(v) => match DataMapper::map(v, context)? {
            Value::String(s) => s,
            Value::Null => String::new(),
            other => other.to_string(),
        },
        // A trigger-started flow carries the changed node as `input.node`.
        None => context
            .input
            .get("node")
            .or_else(|| context.input.get("event").and_then(|e| e.get("node_data")))
            .and_then(|n| n.get("properties"))
            .and_then(|p| p.get("content"))
            .and_then(Value::as_str)
            .or_else(|| context.input.get("message").and_then(Value::as_str))
            .or_else(|| context.input.get("input").and_then(Value::as_str))
            .unwrap_or("")
            .to_string(),
    };
    Ok(super::context_injection::with_context_block(
        text,
        context,
        super::context_injection::ContextInjection::from_step(step),
    ))
}

/// How many times this step has been entered (loops create a new run per visit).
fn visit_of(step: &FlowNode, context: &FlowContext) -> u64 {
    context
        .variables
        .get(crate::runtime::executor::VISITS_KEY)
        .and_then(|v| v.get(&step.id))
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

/// The step's output from its run's result.
fn output_of(step: &FlowNode, result: &Value) -> Result<Value, FlowError> {
    let status = result
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("failed");
    let run_id = result.get("agent_run_id").cloned().unwrap_or(Value::Null);
    let outcome = result.get("outcome").cloned().unwrap_or(Value::Null);
    let message = outcome
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if status != "completed" {
        let code = outcome
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or(status);
        return Err(FlowError::AIProvider(format!(
            "agent run {} {status} ({code}){}",
            run_id.as_str().unwrap_or("?"),
            if message.is_empty() {
                String::new()
            } else {
                format!(": {message}")
            }
        )));
    }
    let mut output = json!({
        "response": message,
        "agent_run_id": run_id,
        "outcome": outcome.get("kind").cloned().unwrap_or(Value::Null),
        "usage": result.get("usage").cloned().unwrap_or(Value::Null),
    });
    if step.get_property("response_format").is_some() && !message.is_empty() {
        match serde_json::from_str::<Value>(message.trim()) {
            Ok(parsed) => output["structured_output"] = parsed,
            Err(e) => {
                warn!(step_id = %step.id, "response_format set but the answer is not JSON: {e}")
            }
        }
    }
    Ok(output)
}

#[async_trait]
impl StepHandler for AgentStepHandler {
    #[instrument(skip(self, context, callbacks), fields(step_id = %step.id))]
    async fn execute(
        &self,
        step: &FlowNode,
        context: &mut FlowContext,
        callbacks: &dyn FlowCallbacks,
    ) -> FlowResult<StepResult> {
        // Re-entry with the run's result.
        if let Some(result) = context.variables.remove(AGENT_RUN_RESULT_VAR) {
            let output = match output_of(step, &result) {
                Ok(output) => output,
                Err(error) => return Ok(StepResult::Error { error }),
            };
            if let Some(text) = output["response"].as_str().filter(|t| !t.is_empty()) {
                let _ = callbacks
                    .emit_event(&context.instance_id, FlowExecutionEvent::text_chunk(text))
                    .await;
            }
            let _ = callbacks
                .emit_event(
                    &context.instance_id,
                    FlowExecutionEvent::step_completed(&step.id, output.clone(), 0),
                )
                .await;
            let next_node_id = step
                .next_node
                .clone()
                .or_else(|| step.get_string_property("next_node"))
                .unwrap_or_else(|| "end".to_string());
            return Ok(StepResult::Continue {
                next_node_id,
                output,
            });
        }

        let _ = callbacks
            .emit_event(
                &context.instance_id,
                FlowExecutionEvent::step_started(&step.id, None, "agent_step"),
            )
            .await;
        let agent_ref = agent_ref_of(step, context)?;
        let agent_workspace = step
            .get_string_property("agent_workspace")
            .unwrap_or_else(|| "functions".to_string());
        let max_model_calls = step
            .get_u32_property("max_model_calls")
            .or_else(|| step.get_u32_property("max_tool_iterations").map(|n| n + 1))
            .unwrap_or(MAX_MODEL_CALLS_DEFAULT);
        let request = super::function_step::with_flow_context(
            json!({
                "agent_ref": agent_ref,
                "agent_workspace": agent_workspace,
                "prompt": prompt_of(step, context)?,
                "response_format": step.get_property("response_format").cloned(),
                "skills": super::ai_tool_loop::step_skills(step),
                "max_model_calls": max_model_calls,
                "visit": visit_of(step, context),
            }),
            &context.instance_id,
            &step.id,
        );
        debug!(agent = %agent_ref, "Starting the agent step's run");
        let started = callbacks
            .execute_function(FLOW_AGENT_RUN_FUNCTION, request)
            .await
            .map_err(|e| FlowError::AIProvider(format!("could not start the agent run: {e}")))?;
        let run_id = started
            .get("run_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                FlowError::AIProvider(format!("the agent run did not start: {started}"))
            })?
            .to_string();

        let mut metadata = json!({
            "run_id": run_id,
            "expected_event": expected_event(&run_id),
            "target_path": started.get("chat_path").cloned().unwrap_or(Value::Null),
            "step_id": step.id,
        });
        if let Some(ms) = step
            .get_property("timeout_ms")
            .and_then(Value::as_u64)
            .filter(|ms| *ms > 0)
        {
            metadata["timeout_ms"] = json!(ms);
        }
        Ok(StepResult::Wait {
            reason: AGENT_RUN_WAIT.to_string(),
            metadata,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::StepType;
    use std::collections::HashMap;

    fn step(props: Value) -> FlowNode {
        let properties: HashMap<String, Value> = serde_json::from_value(props).unwrap();
        FlowNode {
            id: "decide".to_string(),
            step_type: StepType::AgentStep,
            properties,
            children: vec![],
            next_node: Some("end".to_string()),
        }
    }

    fn done(message: &str) -> Value {
        json!({ "agent_run_id": "r1", "status": "completed",
                "outcome": { "kind": "succeeded", "message": message }, "usage": {} })
    }

    #[test]
    fn a_completed_run_is_the_step_output() {
        let out = output_of(&step(json!({ "agent_ref": "/a" })), &done("Positive")).unwrap();
        assert_eq!(out["response"], "Positive");
        assert_eq!(out["agent_run_id"], "r1");
        assert_eq!(out["outcome"], "succeeded");
        assert!(out.get("structured_output").is_none());
    }

    #[test]
    fn a_response_format_parses_the_answer() {
        let s = step(json!({ "agent_ref": "/a", "response_format": { "type": "json_object" } }));
        let out = output_of(&s, &done("{\"label\":\"spam\"}")).unwrap();
        assert_eq!(out["structured_output"]["label"], "spam");
    }

    #[test]
    fn a_failed_or_stopped_run_fails_the_step() {
        let failed = json!({ "agent_run_id": "r1", "status": "stopped",
                             "outcome": { "kind": "stopped", "message": "user stop" } });
        let err = output_of(&step(json!({ "agent_ref": "/a" })), &failed).unwrap_err();
        assert!(err.to_string().contains("r1 stopped"), "{err}");
    }
}
