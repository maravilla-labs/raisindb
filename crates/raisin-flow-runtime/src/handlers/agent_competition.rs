// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Competing-agents step handler.
//!
//! Every competitor agent (each potentially backed by a different LLM)
//! answers the same task. A referee agent judges the answers and either
//! accepts a winner or sends per-competitor feedback for another round
//! (bounded by `max_rounds`). The referee's declared confidence travels
//! in the step output, so downstream REL rules can gate on it — e.g.
//! route low-confidence results into a human task:
//!
//! ```yaml
//! rules:
//!   - { condition: "steps.compete.confidence < 0.7", next_step: human_review }
//! ```
//!
//! Lowered from a `competition` container (designer format) or authored
//! directly in runtime format as `step_type: competition`.

use super::{StepHandler, StepResult};
use crate::runtime::DataMapper;
use crate::types::{
    FlowCallbacks, FlowContext, FlowError, FlowExecutionEvent, FlowNode, FlowResult,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Instant;
use tracing::{debug, instrument, warn};

/// Default confidence the referee must declare to accept a winner
const DEFAULT_MIN_CONFIDENCE: f64 = 0.7;
/// Default number of refinement rounds after the initial one
const DEFAULT_MAX_ROUNDS: u32 = 1;

/// Handler for competing-agents steps.
#[derive(Debug)]
pub struct AgentCompetitionHandler;

impl AgentCompetitionHandler {
    pub fn new() -> Self {
        Self
    }
}

impl Default for AgentCompetitionHandler {
    fn default() -> Self {
        Self::new()
    }
}

struct Competitor {
    id: String,
    agent_ref: String,
    agent_workspace: String,
    prompt: Option<String>,
}

fn parse_competitors(step: &FlowNode) -> FlowResult<Vec<Competitor>> {
    let competitors: Vec<Competitor> = step
        .get_property("competitors")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|c| {
                    Some(Competitor {
                        id: c.get("id")?.as_str()?.to_string(),
                        agent_ref: c.get("agent_ref")?.as_str()?.to_string(),
                        agent_workspace: c
                            .get("agent_workspace")
                            .and_then(|v| v.as_str())
                            .unwrap_or("functions")
                            .to_string(),
                        prompt: c.get("prompt").and_then(|v| v.as_str()).map(String::from),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    if competitors.len() < 2 {
        return Err(FlowError::InvalidDefinition(format!(
            "Competition step '{}' needs at least 2 competitors with agent_ref",
            step.id
        )));
    }
    Ok(competitors)
}

#[async_trait]
impl StepHandler for AgentCompetitionHandler {
    #[instrument(skip(self, context, callbacks), fields(step_id = %step.id))]
    async fn execute(
        &self,
        step: &FlowNode,
        context: &mut FlowContext,
        callbacks: &dyn FlowCallbacks,
    ) -> FlowResult<StepResult> {
        debug!("Executing agent competition step: {}", step.id);
        let step_start = Instant::now();

        let _ = callbacks
            .emit_event(
                &context.instance_id,
                FlowExecutionEvent::step_started(&step.id, None, "competition"),
            )
            .await;

        let competitors = parse_competitors(step)?;
        let referee_ref = step.get_string("referee_agent_ref").ok_or_else(|| {
            FlowError::MissingProperty(format!(
                "Competition step '{}' missing required property: referee_agent_ref",
                step.id
            ))
        })?;
        let referee_workspace = step
            .get_string_property("referee_agent_workspace")
            .unwrap_or_else(|| "functions".to_string());
        let min_confidence = step
            .get_property("min_confidence")
            .and_then(|v| v.as_f64())
            .unwrap_or(DEFAULT_MIN_CONFIDENCE);
        let max_rounds = step
            .get_u32_property("max_rounds")
            .unwrap_or(DEFAULT_MAX_ROUNDS);

        // Shared task: container-level prompt (templated); competitors may
        // override with their own prompt
        let shared_task: String = match step.get_property("prompt") {
            Some(prompt_value) => match DataMapper::map(prompt_value, context)? {
                Value::String(s) => s,
                Value::Null => String::new(),
                other => other.to_string(),
            },
            None => format!(
                "Solve this task.\nWorkflow input:\n{}",
                serde_json::to_string_pretty(&context.input).unwrap_or_default()
            ),
        };
        // Optional workflow-context injection for the shared task
        let shared_task = super::context_injection::with_context_block(
            shared_task,
            context,
            super::context_injection::ContextInjection::from_step(step),
        );

        let referee_instructions: Option<String> = match step.get_property("referee_prompt") {
            Some(v) => match DataMapper::map(v, context)? {
                Value::String(s) => Some(s),
                Value::Null => None,
                other => Some(other.to_string()),
            },
            None => None,
        };

        let competitor_ids: Vec<&str> = competitors.iter().map(|c| c.id.as_str()).collect();
        // Per-competitor referee feedback carried into the next round
        let mut feedback: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let mut answers: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        let mut models: std::collections::HashMap<String, Value> = std::collections::HashMap::new();
        let mut rounds_run: u32 = 0;
        let mut verdict: Option<Value> = None;

        for round in 0..=max_rounds {
            rounds_run = round + 1;

            // 1. Every competitor answers (skipped if it has no fresh
            //    feedback after round 0 - its previous answer stands)
            for competitor in &competitors {
                if round > 0 && !feedback.contains_key(&competitor.id) {
                    continue;
                }
                let base_task = match &competitor.prompt {
                    Some(p) => match DataMapper::map(&Value::String(p.clone()), context)? {
                        Value::String(s) => s,
                        other => other.to_string(),
                    },
                    None => shared_task.clone(),
                };
                let task = match (round, feedback.get(&competitor.id)) {
                    (r, Some(fb)) if r > 0 => format!(
                        "{}\n\nYour previous answer:\n{}\n\nReferee feedback:\n{}\n\nPlease provide an improved answer.",
                        base_task,
                        answers.get(&competitor.id).map(String::as_str).unwrap_or(""),
                        fb
                    ),
                    _ => base_task,
                };

                let response = callbacks
                    .call_ai(
                        &competitor.agent_workspace,
                        &competitor.agent_ref,
                        vec![json!({ "role": "user", "content": task })],
                        None,
                    )
                    .await
                    .map_err(|e| {
                        FlowError::AIProvider(format!(
                            "Competitor '{}' AI call failed: {}",
                            competitor.id, e
                        ))
                    })?;
                let content = response
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if let Some(model) = response.get("model") {
                    models.insert(competitor.id.clone(), model.clone());
                }
                answers.insert(competitor.id.clone(), content);
            }
            feedback.clear();

            // 2. The referee judges
            let answers_block = competitors
                .iter()
                .map(|c| {
                    format!(
                        "### Answer from \"{}\"\n{}",
                        c.id,
                        answers.get(&c.id).map(String::as_str).unwrap_or("(none)")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n");
            let last_round = round == max_rounds;
            let referee_task = format!(
                "{}\n\nTask given to the competitors:\n{}\n\n{}\n\nJudge the answers. {}",
                referee_instructions
                    .as_deref()
                    .unwrap_or("You are the referee of competing AI answers. Pick the best one."),
                shared_task,
                answers_block,
                if last_round {
                    "This is the final round: you MUST accept a winner."
                } else {
                    "Accept a winner if one is good enough; otherwise request refinement with concrete per-competitor feedback."
                }
            );

            let referee_format = json!({
                "type": "json_schema",
                "schema": {
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["accept", "refine"] },
                        "winner": { "type": "string", "enum": competitor_ids },
                        "confidence": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                        "reasoning": { "type": "string" },
                        "feedback": {
                            "type": "object",
                            "description": "Per-competitor feedback for refinement, keyed by competitor id",
                            "additionalProperties": { "type": "string" }
                        }
                    },
                    "required": ["action", "winner", "confidence"]
                }
            });

            let referee_response = callbacks
                .call_ai(
                    &referee_workspace,
                    &referee_ref,
                    vec![json!({ "role": "user", "content": referee_task })],
                    Some(referee_format),
                )
                .await
                .map_err(|e| FlowError::AIProvider(format!("Referee AI call failed: {}", e)))?;
            let referee_content = referee_response
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let parsed: Value = serde_json::from_str(referee_content).unwrap_or(Value::Null);

            let action = parsed
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("accept");
            let confidence = parsed
                .get("confidence")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);

            let _ = callbacks
                .emit_event(
                    &context.instance_id,
                    FlowExecutionEvent::text_chunk(&format!(
                        "Referee round {}: {} (confidence {:.2})",
                        rounds_run, action, confidence
                    )),
                )
                .await;

            if action == "refine" && !last_round {
                if let Some(fb_map) = parsed.get("feedback").and_then(|v| v.as_object()) {
                    for (k, v) in fb_map {
                        if let Some(text) = v.as_str() {
                            if competitor_ids.contains(&k.as_str()) {
                                feedback.insert(k.clone(), text.to_string());
                            }
                        }
                    }
                }
                if feedback.is_empty() {
                    warn!(
                        "Competition '{}': referee requested refinement without usable feedback - accepting current verdict",
                        step.id
                    );
                    verdict = Some(parsed);
                    break;
                }
                verdict = Some(parsed);
                continue;
            }

            verdict = Some(parsed);
            break;
        }

        // Resolve the verdict (referee was forced to accept in the final round)
        let verdict = verdict.unwrap_or(Value::Null);
        let winner = verdict
            .get("winner")
            .and_then(|v| v.as_str())
            .filter(|w| competitor_ids.contains(w))
            .unwrap_or(competitor_ids[0])
            .to_string();
        let confidence = verdict
            .get("confidence")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let confident = confidence >= min_confidence;
        if !confident {
            warn!(
                "Competition '{}': final confidence {:.2} below min_confidence {:.2} - gate downstream on steps.{}.confidence",
                step.id, confidence, min_confidence, step.id
            );
        }

        let output = json!({
            "response": answers.get(&winner),
            "winner": winner,
            "confidence": confidence,
            "confident": confident,
            "reasoning": verdict.get("reasoning"),
            "rounds": rounds_run,
            "answers": answers,
            "models": models,
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

        let next_node_id = step
            .next_node
            .clone()
            .or_else(|| step.get_string_property("next_node"))
            .unwrap_or_else(|| "end".to_string());

        Ok(StepResult::Continue {
            next_node_id,
            output,
        })
    }
}
