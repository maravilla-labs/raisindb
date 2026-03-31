// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Chat step handler for interactive multi-turn conversations.
//!
//! Handles long-running chat sessions with sub-agent handoff capability.
//! Conversation history is persisted in the node tree and loaded on each
//! turn — there is no in-memory sliding window.
//!
//! # Module layout
//!
//! - [`types`]    — Configuration and session state structs
//! - [`session`]  — Session lifecycle: load/save, user message extraction,
//!                  conversation location resolution
//! - [`ai_turn`]  — AI processing: message building, tool loop, result persistence

mod ai_turn;
mod session;
mod types;

#[cfg(test)]
mod tests;

use types::{ChatConfig, ConversationType, HandoffTarget, TerminationConfig};

use super::{StepHandler, StepResult};
use crate::handlers::conversation_persistence;
use crate::types::{
    AiExecutionConfig, FlowCallbacks, FlowContext, FlowExecutionEvent, FlowNode, FlowResult,
};
use async_trait::async_trait;
use tracing::{debug, error, info, instrument, warn};

/// Wait reason for chat sessions.
const WAIT_REASON_CHAT_SESSION: &str = "chat_session";

/// Handler for chat step execution.
///
/// Creates interactive multi-turn conversation sessions with optional
/// sub-agent handoff capability.
#[derive(Debug)]
pub struct ChatStepHandler;

impl ChatStepHandler {
    pub fn new() -> Self {
        Self
    }

    /// Extract chat configuration from step properties.
    fn get_chat_config(&self, step: &FlowNode) -> ChatConfig {
        let agent_ref = step.get_string("agent_ref");
        let agent_workspace = step.get_string("agent_workspace");
        let system_prompt = step.get_string("system_prompt");
        let max_turns = step
            .properties
            .get("max_turns")
            .and_then(|v| v.as_u64())
            .unwrap_or(50) as u32;
        let session_timeout_ms = step
            .properties
            .get("session_timeout_ms")
            .and_then(|v| v.as_u64());

        let handoff_targets = step
            .get_array("handoff_targets")
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| {
                        let obj = v.as_object()?;
                        Some(HandoffTarget {
                            agent_ref: obj.get("agent_ref")?.as_str()?.to_string(),
                            description: obj
                                .get("description")
                                .and_then(|v| v.as_str())
                                .map(String::from),
                            condition: obj
                                .get("condition")
                                .and_then(|v| v.as_str())
                                .map(String::from),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let termination = step
            .properties
            .get("termination")
            .map(|v| TerminationConfig {
                allow_user_end: v
                    .get("allow_user_end")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true),
                allow_ai_end: v
                    .get("allow_ai_end")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true),
                end_keywords: v
                    .get("end_keywords")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
            })
            .unwrap_or_default();

        let execution = AiExecutionConfig {
            max_retries: step
                .properties
                .get("max_retries")
                .and_then(|v| v.as_u64())
                .unwrap_or(2) as u32,
            retry_delay_ms: step
                .properties
                .get("retry_delay_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(1000),
            timeout_ms: step.properties.get("timeout_ms").and_then(|v| v.as_u64()),
            thinking_enabled: step
                .properties
                .get("thinking_enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        };

        let conversation_type = step
            .get_string("conversation_type")
            .or_else(|| step.get_string("conversation_format"))
            .map(|s| match s.as_str() {
                "inbox" | "flow_chat" => ConversationType::FlowChat,
                "ai_chat" | "" => ConversationType::AiChat,
                "direct_message" => ConversationType::DirectMessage,
                other => {
                    warn!(
                        "Unknown conversation_type '{}', falling back to AiChat",
                        other
                    );
                    ConversationType::AiChat
                }
            })
            .unwrap_or_default();

        ChatConfig {
            agent_ref,
            agent_workspace,
            system_prompt,
            max_turns,
            session_timeout_ms,
            handoff_targets,
            termination,
            execution,
            conversation_type,
        }
    }

    /// Check if a message triggers session termination.
    fn should_terminate(&self, message: &str, config: &TerminationConfig) -> bool {
        if !config.allow_user_end {
            return false;
        }
        let message_lower = message.to_lowercase();
        config
            .end_keywords
            .iter()
            .any(|kw| message_lower.contains(&kw.to_lowercase()))
    }
}

impl Default for ChatStepHandler {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// StepHandler implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl StepHandler for ChatStepHandler {
    #[instrument(skip(self, context, callbacks), fields(step_id = %step.id))]
    async fn execute(
        &self,
        step: &FlowNode,
        context: &mut FlowContext,
        callbacks: &dyn FlowCallbacks,
    ) -> FlowResult<StepResult> {
        let start = std::time::Instant::now();

        let config = session::resolve_config(self.get_chat_config(step), context);
        debug!(
            "Executing chat step: {} with agent: {:?}",
            step.id, config.agent_ref
        );

        // Emit StepStarted event
        let _ = callbacks
            .emit_event(
                &context.instance_id,
                FlowExecutionEvent::step_started(&step.id, None, "chat"),
            )
            .await;

        let session_key = format!("__chat_session_{}", step.id);
        let (is_new, mut session) = session::load_or_init_session(context, &session_key, &config);

        let (conv_ws, conv_prefix) = session::resolve_conversation_location(context);
        debug!(
            "ChatStep conversation location: workspace={}, prefix='{}'",
            conv_ws, conv_prefix
        );

        // Create conversation node on first execution
        if is_new {
            let conv_id = format!("{}-{}", context.instance_id, step.id);
            let conv_path = format!("{}/conversations/{}", conv_prefix, conv_id);
            match conversation_persistence::ensure_conversation(
                callbacks,
                &context.instance_id,
                &conv_path,
                &conv_ws,
                config.conversation_type,
                config.agent_ref.as_deref(),
                &[],
                None,
            )
            .await
            {
                Ok(ref path) => {
                    debug!("Created conversation node at: {}", path);
                    session.conversation_path = Some(path.clone());
                }
                Err(e) => error!("Failed to create conversation node: {}", e),
            }
        }

        let is_resumed = session.turn_count > 0;

        // Handle user message
        if let Some(message) = session::take_user_message(context) {
            let source = if is_new {
                "context.input (first turn)"
            } else {
                "__chat_user_message (resume)"
            };
            debug!("User message found (source: {}): {}", source, message);

            if is_resumed {
                let _ = callbacks
                    .emit_event(
                        &context.instance_id,
                        FlowExecutionEvent::flow_resumed(&step.id, 0),
                    )
                    .await;
            }

            if self.should_terminate(&message, &config.termination) {
                session.is_complete = true;
                session.completion_reason = Some("user_terminated".to_string());
                session::update_stats_and_save(
                    callbacks,
                    context,
                    &session_key,
                    &session,
                    &conv_ws,
                )
                .await;
                info!("Chat session terminated by user keyword");
                return Ok(session::make_continue(step, &session));
            }

            session.turn_count += 1;

            if let Some(ref conv_path) = session.conversation_path {
                if let Err(e) = conversation_persistence::persist_user_message(
                    callbacks,
                    &context.instance_id,
                    conv_path,
                    &conv_ws,
                    &message,
                    None,
                    None,
                )
                .await
                {
                    error!("Failed to persist user message: {}", e);
                }
            }
        } else {
            debug!("No user message found for step '{}'", step.id);
        }

        // Check turn limit
        if session.turn_count >= config.max_turns {
            session.is_complete = true;
            session.completion_reason = Some("max_turns_reached".to_string());
            session::update_stats_and_save(callbacks, context, &session_key, &session, &conv_ws)
                .await;
            warn!("Chat session reached max turns: {}", config.max_turns);
            return Ok(session::make_continue(step, &session));
        }

        // Process with AI
        if let Some(ref agent_path) = config.agent_ref {
            debug!(
                "Starting AI turn: agent='{}', turn={}, conv_path={:?}",
                agent_path, session.turn_count, session.conversation_path
            );
            if let Err(e) = ai_turn::process_ai_turn(
                callbacks,
                context,
                &config,
                &mut session,
                agent_path,
                &conv_ws,
                &step.id,
            )
            .await
            {
                let error_msg = e.to_string();
                warn!(
                    "ChatStep AI turn failed for step '{}': {}",
                    step.id, error_msg
                );

                // Emit a log event (not StepFailed) since we're recovering and the
                // session will continue in Wait state for the user to retry.
                let _ = callbacks
                    .emit_event(
                        &context.instance_id,
                        FlowExecutionEvent::log("error", &error_msg, Some(step.id.clone())),
                    )
                    .await;
            }
        }

        // If session is complete, finish the step
        if session.is_complete {
            session::update_stats_and_save(callbacks, context, &session_key, &session, &conv_ws)
                .await;
            return Ok(session::make_continue(step, &session));
        }

        // Save session state and wait for next user message
        context.variables.insert(
            session_key,
            serde_json::to_value(&session).unwrap_or_default(),
        );

        let elapsed = start.elapsed().as_millis() as u64;
        let _ = callbacks
            .emit_event(
                &context.instance_id,
                FlowExecutionEvent::step_completed(
                    &step.id,
                    serde_json::json!({
                        "status": "waiting",
                        "turn_count": session.turn_count,
                    }),
                    elapsed,
                ),
            )
            .await;

        Ok(StepResult::Wait {
            reason: WAIT_REASON_CHAT_SESSION.to_string(),
            metadata: serde_json::json!({
                "session_id": session.session_id,
                "step_id": step.id,
                "turn_count": session.turn_count,
                "current_agent": session.current_agent,
                "timeout_ms": config.session_timeout_ms,
                "conversation_path": session.conversation_path,
                "conversation_type": config.conversation_type,
            }),
        })
    }
}
