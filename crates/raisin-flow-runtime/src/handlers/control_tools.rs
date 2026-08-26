// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Control tools — the verbs an agent uses to steer the FLOW rather than to do
//! work in the world.
//!
//! An agent's own `tools` are functions: the tool loop resolves each name to a
//! path and calls `execute_function`. A control tool is different in kind. It
//! has no function behind it; the RUNTIME implements it, and calling one
//! changes what the flow does next — ends the conversation with a result, moves
//! it to another agent, or parks it on a human.
//!
//! # Why these have to be tools
//!
//! The chat step has advertised AI-initiated termination since it was written:
//! `termination.allow_ai_end` gates on a `end_session` flag that
//! [`ai_tool_loop`] reads off the AI response, which `raisin_ai::streaming`
//! only sets when a provider CHUNK carries it. No provider emits that field,
//! and nothing ever offered the model a way to ask for it — so the flag was
//! unreachable and an agent could never end a session. Same for `handoff_to`:
//! `handoff_targets` were pasted into a system prompt as prose with no verb to
//! invoke. Offering them as real tools is what makes the existing switches mean
//! something.
//!
//! # The termination contract
//!
//! Ending is not just a flag — it has to say WHAT THE ANSWER IS, or the step
//! after a chat has nothing to read. So [`END_SESSION`] takes a `result`
//! argument shaped by the step's `output_schema`, and the chat step publishes
//! it at `steps.<id>.result`. A step with no `output_schema` still gets a free
//! -form `result`, because "the agent decided it was done" and "the agent
//! produced nothing" are different outcomes.
//!
//! [`ai_tool_loop`]: super::ai_tool_loop

use serde_json::{json, Value};

/// End the conversation and return a result.
pub const END_SESSION: &str = "end_session";

/// Hand the conversation to another agent.
pub const HANDOFF: &str = "handoff_to_agent";

/// Park the flow on a human decision and resume with their answer.
pub const REQUEST_APPROVAL: &str = "request_approval";

/// Every control-tool name, for the membership test the tool loop does on each
/// call before it reaches for `execute_function`.
pub const ALL: [&str; 3] = [END_SESSION, HANDOFF, REQUEST_APPROVAL];

/// Is this tool name implemented by the runtime rather than by a function?
///
/// The check is by NAME, which means an agent whose own tool is called
/// `end_session` would be shadowed. That is deliberate and the safe direction:
/// a control tool must never be routed to `execute_function`, where an
/// unresolved name falls back to calling the name as a path.
pub fn is_control_tool(name: &str) -> bool {
    ALL.contains(&name)
}

/// Which control tools a step offers, and how they are shaped.
///
/// Default is NOTHING OFFERED. A control tool the author did not ask for must
/// not appear: an agent step in the middle of a pipeline that could end its own
/// flow is a surprise, not a feature.
#[derive(Debug, Clone, Default)]
pub struct ControlToolConfig {
    /// Offer [`END_SESSION`].
    pub allow_end: bool,

    /// JSON Schema for the `result` argument of [`END_SESSION`].
    ///
    /// `None` means a free-form result — see the module docs.
    pub result_schema: Option<Value>,

    /// Agents [`HANDOFF`] may name, as `(path, description)`. Empty means the
    /// tool is not offered: a handoff tool with no targets is a tool whose only
    /// possible call is an error.
    pub handoff_targets: Vec<(String, String)>,

    /// Offer [`REQUEST_APPROVAL`].
    pub allow_approval: bool,

    /// Who an approval request goes to when the model does not name someone.
    pub approval_assignee: Option<String>,
}

impl ControlToolConfig {
    /// Are any control tools offered at all?
    pub fn is_empty(&self) -> bool {
        !self.allow_end && !self.allow_approval && self.handoff_targets.is_empty()
    }

    /// The tool definitions to append to this call, in provider format.
    pub fn definitions(&self) -> Vec<Value> {
        let mut tools = Vec::new();

        if self.allow_end {
            // An explicitly-schema'd result is described as such; otherwise the
            // model gets an open object. Never omit the property — a terminate
            // tool that takes no arguments is exactly the unreadable "it ended"
            // outcome this exists to fix.
            let result_schema = self.result_schema.clone().unwrap_or_else(|| {
                json!({
                    "type": "object",
                    "description": "What this conversation concluded. Include everything a \
                                    later step needs; nothing else survives.",
                })
            });
            tools.push(tool(
                END_SESSION,
                "Finish this conversation. Call this when the goal is met or the user is \
                 done — not to acknowledge a message. Whatever you pass as `result` is the \
                 ONLY thing later steps can read.",
                json!({
                    "type": "object",
                    "properties": {
                        "result": result_schema,
                        "reason": {
                            "type": "string",
                            "description": "Short note on why you are ending, for the audit trail.",
                        },
                    },
                    "required": ["result"],
                }),
            ));
        }

        if !self.handoff_targets.is_empty() {
            let names: Vec<Value> = self
                .handoff_targets
                .iter()
                .map(|(p, _)| Value::String(p.clone()))
                .collect();
            let roster = self
                .handoff_targets
                .iter()
                .map(|(p, d)| format!("- {}: {}", p, d))
                .collect::<Vec<_>>()
                .join("\n");
            tools.push(tool(
                HANDOFF,
                &format!(
                    "Hand this conversation to a better-suited agent. Available:\n{}",
                    roster
                ),
                json!({
                    "type": "object",
                    "properties": {
                        "agent": { "type": "string", "enum": names },
                        "reason": { "type": "string" },
                    },
                    "required": ["agent"],
                }),
            ));
        }

        if self.allow_approval {
            tools.push(tool(
                REQUEST_APPROVAL,
                "Ask a person to approve something before you continue. The conversation \
                 PAUSES until they answer, and their answer comes back to you as this \
                 tool's result. Use it when you judge you need sign-off — you do not need \
                 to have been told to.",
                json!({
                    "type": "object",
                    "properties": {
                        "title": {
                            "type": "string",
                            "description": "What you are asking them to decide, in one line.",
                        },
                        "description": {
                            "type": "string",
                            "description": "The detail they need to decide. They cannot see \
                                            this conversation.",
                        },
                        "assignee": {
                            "type": "string",
                            "description": "User or role path, e.g. /users/editor. Omit for \
                                            the step's default approver.",
                        },
                        "options": {
                            "type": "array",
                            "description": "Choices to offer. Defaults to approve / reject.",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "value": { "type": "string" },
                                    "label": { "type": "string" },
                                },
                                "required": ["value"],
                            },
                        },
                    },
                    "required": ["title"],
                }),
            ));
        }

        tools
    }
}

/// One tool definition in the provider's `{type, function}` shape.
fn tool(name: &str, description: &str, parameters: Value) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": parameters,
        },
    })
}

/// What a control-tool call asked the runtime to do.
#[derive(Debug, Clone)]
pub enum ControlAction {
    /// End with this result and optional reason.
    End { result: Value, reason: Option<String> },

    /// Continue with a different agent.
    Handoff { agent: String, reason: Option<String> },

    /// Park on a human decision.
    Approval(ApprovalRequest),
}

/// An agent's request for a human decision, as it reaches the task layer.
#[derive(Debug, Clone)]
pub struct ApprovalRequest {
    pub title: String,
    pub description: Option<String>,
    pub assignee: Option<String>,
    pub options: Vec<Value>,
}

/// Interpret a control-tool call. Returns `None` for a name this module does
/// not own, and for a call whose required arguments are missing — a malformed
/// control call is reported to the model as a tool error rather than silently
/// ending a conversation or parking a flow.
pub fn interpret(name: &str, args: &Value) -> Option<ControlAction> {
    let reason = args
        .get("reason")
        .and_then(|v| v.as_str())
        .map(String::from);

    match name {
        END_SESSION => Some(ControlAction::End {
            // A model that calls the tool without the required `result` still
            // MEANT to end. Null is honest about there being no payload; a
            // refusal here would leave the session running with the agent
            // believing it had finished.
            result: args.get("result").cloned().unwrap_or(Value::Null),
            reason,
        }),
        HANDOFF => args
            .get("agent")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|agent| ControlAction::Handoff {
                agent: agent.to_string(),
                reason,
            }),
        REQUEST_APPROVAL => args
            .get("title")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|title| {
                ControlAction::Approval(ApprovalRequest {
                    title: title.to_string(),
                    description: args
                        .get("description")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    assignee: args
                        .get("assignee")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .map(String::from),
                    options: args
                        .get("options")
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default(),
                })
            }),
        _ => None,
    }
}

/// The error a malformed control call is reported back to the model as.
pub fn malformed(name: &str) -> Value {
    json!({
        "error": format!(
            "Malformed call to `{}` — a required argument was missing. Check the tool's \
             schema and try again.",
            name
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offers_nothing_by_default() {
        let cfg = ControlToolConfig::default();
        assert!(cfg.is_empty());
        assert!(cfg.definitions().is_empty());
    }

    #[test]
    fn end_tool_always_carries_a_result_property() {
        let cfg = ControlToolConfig {
            allow_end: true,
            ..Default::default()
        };
        let defs = cfg.definitions();
        assert_eq!(defs.len(), 1);
        let params = &defs[0]["function"]["parameters"];
        assert!(params["properties"]["result"].is_object());
        assert_eq!(params["required"][0], "result");
    }

    #[test]
    fn end_tool_uses_the_steps_output_schema_for_result() {
        let schema = json!({"type": "object", "properties": {"verdict": {"type": "string"}}});
        let cfg = ControlToolConfig {
            allow_end: true,
            result_schema: Some(schema.clone()),
            ..Default::default()
        };
        assert_eq!(cfg.definitions()[0]["function"]["parameters"]["properties"]["result"], schema);
    }

    #[test]
    fn handoff_is_not_offered_without_targets() {
        let cfg = ControlToolConfig {
            allow_end: true,
            ..Default::default()
        };
        assert!(!cfg
            .definitions()
            .iter()
            .any(|d| d["function"]["name"] == HANDOFF));
    }

    #[test]
    fn handoff_targets_become_an_enum() {
        let cfg = ControlToolConfig {
            handoff_targets: vec![("/agents/legal".into(), "Contracts".into())],
            ..Default::default()
        };
        let defs = cfg.definitions();
        assert_eq!(defs[0]["function"]["parameters"]["properties"]["agent"]["enum"][0], "/agents/legal");
    }

    #[test]
    fn end_without_a_result_still_ends() {
        // The session must not stay open just because the model skipped the
        // required argument — it believes it has finished.
        match interpret(END_SESSION, &json!({})) {
            Some(ControlAction::End { result, .. }) => assert!(result.is_null()),
            other => panic!("expected End, got {:?}", other),
        }
    }

    #[test]
    fn handoff_without_an_agent_is_malformed() {
        assert!(interpret(HANDOFF, &json!({"reason": "not sure"})).is_none());
        assert!(interpret(HANDOFF, &json!({"agent": ""})).is_none());
    }

    #[test]
    fn approval_without_a_title_is_malformed() {
        assert!(interpret(REQUEST_APPROVAL, &json!({"description": "x"})).is_none());
    }

    #[test]
    fn approval_carries_assignee_and_options_through() {
        let args = json!({
            "title": "Publish?",
            "assignee": "/users/editor",
            "options": [{"value": "yes"}],
        });
        match interpret(REQUEST_APPROVAL, &args) {
            Some(ControlAction::Approval(req)) => {
                assert_eq!(req.assignee.as_deref(), Some("/users/editor"));
                assert_eq!(req.options.len(), 1);
            }
            other => panic!("expected Approval, got {:?}", other),
        }
    }

    #[test]
    fn a_function_named_like_a_control_tool_is_shadowed() {
        assert!(is_control_tool(END_SESSION));
        assert!(!is_control_tool("send_email"));
    }
}
