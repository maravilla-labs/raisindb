// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Agent-as-assignee support for human tasks.
//!
//! The step model stays uniform: a human task is a human task regardless of
//! who the assignee is. When the assignee resolves to an AI agent
//! (`raisin:AIAgent`), the task is evaluated by the agent immediately:
//!
//! - The inbox task node is created either way (full audit trail).
//! - The agent is called with the task (title, description, options,
//!   input schema) plus the flow context, and must answer with a structured
//!   decision `{ decision | value, reasoning, confidence }`.
//! - A confident decision completes the task (`completed_by` = agent path)
//!   and the flow continues - exactly as if a human had responded.
//! - On error, unparseable output, or confidence below the step's
//!   `min_confidence` (default 0.7), the task is escalated: reassigned to
//!   `escalation_assignee` (if configured) and the flow waits for a human.
//!   Escalation in the other direction (human -> agent) is the same
//!   reassignment mechanism.

use crate::types::{FlowCallbacks, FlowContext, FlowNode, FlowResult, StepResult};
use serde_json::{json, Value};
use tracing::{debug, info, warn};

use super::{AGENT_WORKSPACE, INBOX_WORKSPACE};

/// Default minimum confidence for an agent decision to be accepted.
const DEFAULT_MIN_CONFIDENCE: f64 = 0.7;

/// If the assignee is an AI agent, evaluate the task with it.
///
/// Returns `Ok(Some(step_result))` when the agent decided (flow continues),
/// `Ok(None)` when the assignee is not an agent or the task was escalated
/// to a human (flow should wait as usual).
pub(super) async fn try_agent_assignee(
    step: &FlowNode,
    context: &mut FlowContext,
    callbacks: &dyn FlowCallbacks,
    assignee: &str,
    task_properties: &Value,
    task_path: &str,
) -> FlowResult<Option<StepResult>> {
    // Resolve the assignee in the agents workspace; non-agents simply won't
    // be found there.
    let agent_node = match callbacks
        .get_node_in_workspace(AGENT_WORKSPACE, assignee)
        .await
    {
        Ok(Some(node)) => node,
        _ => return Ok(None),
    };

    let node_type = agent_node
        .get("node_type")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if node_type != "raisin:AIAgent" {
        return Ok(None);
    }

    info!(
        step_id = %step.id,
        agent = %assignee,
        "Task assignee is an AI agent - evaluating task with agent"
    );

    let min_confidence = step
        .get_property("min_confidence")
        .and_then(|v| v.as_f64())
        .unwrap_or(DEFAULT_MIN_CONFIDENCE);

    match evaluate_with_agent(context, callbacks, assignee, task_properties).await {
        Ok(Some((response, confidence))) if confidence >= min_confidence => {
            info!(
                step_id = %step.id,
                agent = %assignee,
                confidence = confidence,
                "Agent decided the task - completing"
            );

            // Complete the inbox task (same contract as a human completion)
            let _ = callbacks
                .update_node_in_workspace(
                    INBOX_WORKSPACE,
                    task_path,
                    json!({
                        "status": "completed",
                        "response": response,
                        "completed_by": assignee,
                        "responded_at": chrono::Utc::now().to_rfc3339(),
                    }),
                )
                .await;

            // Mirror the resume contract so downstream steps see the same
            // shape as a human response (__human_response)
            context.set_variable("__human_response".to_string(), response.clone());

            let next_node_id = step.next_node.clone().unwrap_or_else(|| "end".to_string());
            return Ok(Some(StepResult::Continue {
                next_node_id,
                output: json!({
                    "response": response,
                    "completed_by": assignee,
                    "decided_by_agent": true,
                }),
            }));
        }
        Ok(Some((_, confidence))) => {
            warn!(
                step_id = %step.id,
                agent = %assignee,
                confidence = confidence,
                min_confidence = min_confidence,
                "Agent confidence below threshold - escalating to human"
            );
            escalate(
                step,
                callbacks,
                assignee,
                task_path,
                &format!(
                    "agent confidence {:.2} below threshold {:.2}",
                    confidence, min_confidence
                ),
            )
            .await;
        }
        Ok(None) => {
            warn!(
                step_id = %step.id,
                agent = %assignee,
                "Agent returned unparseable decision - escalating to human"
            );
            escalate(
                step,
                callbacks,
                assignee,
                task_path,
                "unparseable agent decision",
            )
            .await;
        }
        Err(e) => {
            warn!(
                step_id = %step.id,
                agent = %assignee,
                error = %e,
                "Agent evaluation failed - escalating to human"
            );
            escalate(
                step,
                callbacks,
                assignee,
                task_path,
                &format!("agent error: {}", e),
            )
            .await;
        }
    }

    // Escalated (or recorded failure) - the flow waits as usual
    Ok(None)
}

/// Call the agent with the task content and parse the structured decision.
///
/// Returns `Ok(Some((response, confidence)))` on a parsed decision,
/// `Ok(None)` when the output couldn't be parsed.
async fn evaluate_with_agent(
    context: &FlowContext,
    callbacks: &dyn FlowCallbacks,
    agent_ref: &str,
    task_properties: &Value,
) -> FlowResult<Option<(Value, f64)>> {
    let title = task_properties
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("Task");
    let description = task_properties
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let task_type = task_properties
        .get("task_type")
        .and_then(|v| v.as_str())
        .unwrap_or("action");
    let options = task_properties.get("options").cloned();
    let input_schema = task_properties.get("input_schema").cloned();

    // Build the decision schema: approval-style tasks decide between the
    // configured options; input tasks produce a value.
    let mut decision_props = serde_json::Map::new();
    decision_props.insert(
        "reasoning".to_string(),
        json!({"type": "string", "description": "Why this decision was made"}),
    );
    decision_props.insert(
        "confidence".to_string(),
        json!({"type": "number", "minimum": 0, "maximum": 1,
               "description": "How confident you are in this decision (0-1)"}),
    );

    let mut required = vec!["reasoning", "confidence"];
    if let Some(opts) = options.as_ref().and_then(|o| o.as_array()) {
        let values: Vec<String> = opts
            .iter()
            .filter_map(|o| o.get("value").and_then(|v| v.as_str()).map(String::from))
            .collect();
        decision_props.insert(
            "decision".to_string(),
            json!({"type": "string", "enum": values}),
        );
        required.push("decision");
    } else if let Some(schema) = input_schema.as_ref() {
        decision_props.insert("value".to_string(), schema.clone());
        required.push("value");
    } else {
        decision_props.insert(
            "decision".to_string(),
            json!({"type": "string", "description": "Your decision or acknowledgement"}),
        );
        required.push("decision");
    }

    let response_format = json!({
        "type": "json_schema",
        "json_schema": {
            "name": "task_decision",
            "schema": {
                "type": "object",
                "properties": decision_props,
                "required": required,
            },
            "strict": true,
        }
    });

    let mut prompt = format!(
        "You are the assignee of the following {} task in a workflow. \
         Evaluate it and respond with your structured decision.\n\n\
         # Task: {}\n",
        task_type, title
    );
    if !description.is_empty() {
        prompt.push_str(&format!("\n{}\n", description));
    }
    if let Some(opts) = options.as_ref() {
        prompt.push_str(&format!(
            "\n# Options\n{}\n",
            serde_json::to_string_pretty(opts).unwrap_or_default()
        ));
    }
    prompt.push_str(&format!(
        "\n# Workflow context\n```json\n{}\n```\n\n\
         Respond ONLY with the structured decision. Set `confidence` honestly: \
         use a low value if information is missing or the decision is unclear.",
        serde_json::to_string_pretty(&context.to_json()).unwrap_or_default()
    ));

    let messages = vec![json!({"role": "user", "content": prompt})];

    let ai_response = callbacks
        .call_ai(AGENT_WORKSPACE, agent_ref, messages, Some(response_format))
        .await?;

    let content = ai_response
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let Ok(parsed) = serde_json::from_str::<Value>(content) else {
        debug!(content = %content, "Agent decision is not valid JSON");
        return Ok(None);
    };

    let confidence = parsed
        .get("confidence")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    // The "response" stored on the task mirrors what a human would submit:
    // approval -> { action, comment }, input -> the value, generic -> decision
    let response = if let Some(decision) = parsed.get("decision") {
        json!({
            "action": decision,
            "comment": parsed.get("reasoning"),
            "confidence": confidence,
        })
    } else if let Some(value) = parsed.get("value") {
        json!({
            "value": value,
            "comment": parsed.get("reasoning"),
            "confidence": confidence,
        })
    } else {
        return Ok(None);
    };

    Ok(Some((response, confidence)))
}

/// Reassign the task to the escalation assignee (or record the failed agent
/// attempt) so a human can pick it up.
async fn escalate(
    step: &FlowNode,
    callbacks: &dyn FlowCallbacks,
    agent_ref: &str,
    task_path: &str,
    reason: &str,
) {
    let mut updates = serde_json::Map::new();
    updates.insert(
        "escalated_from".to_string(),
        Value::String(agent_ref.to_string()),
    );
    updates.insert(
        "escalation_reason".to_string(),
        Value::String(reason.to_string()),
    );
    updates.insert(
        "escalated_at".to_string(),
        Value::String(chrono::Utc::now().to_rfc3339()),
    );

    if let Some(human) = step.get_string("escalation_assignee") {
        updates.insert("assignee".to_string(), Value::String(human.clone()));
        info!(
            task_path = %task_path,
            escalation_assignee = %human,
            reason = %reason,
            "Escalating agent task to human assignee"
        );
    } else {
        warn!(
            task_path = %task_path,
            reason = %reason,
            "Agent task failed with no escalation_assignee configured - task stays assigned to the agent and must be completed via the inbox API"
        );
    }

    let _ = callbacks
        .update_node_in_workspace(INBOX_WORKSPACE, task_path, Value::Object(updates))
        .await;
}
