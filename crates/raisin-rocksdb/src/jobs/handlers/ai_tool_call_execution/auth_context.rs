// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Auth context resolution for AI tool call execution
//!
//! Resolves the appropriate auth context based on the agent's
//! `execution_context` setting, navigating from tool call through
//! the conversation to the referenced agent node.

use raisin_models::auth::{agent_identity, AuthContext};
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::{NodeRepository, Storage, StorageScope};

use super::types::FUNCTIONS_WORKSPACE;
use super::AIToolCallExecutionHandler;

/// A system context carrying the provenance of whatever initiated this run.
///
/// `agent` records the *initiator*, and is set independently of `user_id`: an
/// AI agent runs with no human behind it, and that absence is precisely why the
/// agent must be named — otherwise the write is attributable to nothing at all.
/// It grants nothing (see `AuthContext::agent`), so adding it cannot widen the
/// permissions of any of the branches below.
fn system_initiated_by(marker: Option<String>) -> Option<AuthContext> {
    let ctx = AuthContext::system();
    Some(match marker {
        Some(marker) => ctx.with_agent(marker),
        None => ctx,
    })
}

/// A node property read as a list of ids, tolerating `/roles/x` path forms the
/// way the user resolver does.
fn string_array(
    properties: &std::collections::HashMap<String, PropertyValue>,
    key: &str,
) -> Vec<String> {
    match properties.get(key) {
        Some(PropertyValue::Array(values)) => values
            .iter()
            .filter_map(|value| match value {
                PropertyValue::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

impl<S: Storage + 'static> AIToolCallExecutionHandler<S> {
    /// The SAME resolver a signed-in user goes through — group expansion, role
    /// inheritance, permission set. An agent's rights must never be computed by
    /// a second implementation.
    fn permission_service(&self) -> raisin_core::PermissionService<S> {
        raisin_core::PermissionService::new(self.storage.clone())
    }

    /// Resolve the auth context for a tool call based on the agent's execution_context setting
    ///
    /// This method:
    /// 1. Navigates from tool call path up to the conversation
    /// 2. Loads the conversation to get the agent_ref
    /// 3. Loads the agent to get execution_context setting
    /// 4. Returns appropriate auth context:
    ///    - "system": Returns None (system context, bypasses RLS)
    ///    - "user" (default): Returns system context for now (TODO: implement user context)
    ///
    /// Whatever it returns carries an `agent` marker naming the agent that is
    /// running (`agent:<agent-node-path>`), composed with `origin` when this run
    /// was caused by something else — a trigger, a flow — giving
    /// `agent:/agents/bot@trigger:/triggers/on-order-created`. See
    /// `raisin_models::auth::agent_identity` for the vocabulary.
    ///
    /// Path structure: /conversations/{chat-id}/{msg-id}/{tool-call-id}
    /// So we go up 2 levels to get the conversation path
    pub(super) async fn resolve_auth_context_for_tool_call(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        tool_call_path: &str,
        origin: Option<&str>,
    ) -> Option<AuthContext> {
        // Before the agent is identified the origin is all the provenance there
        // is; recording it alone still beats an anonymous write.
        let origin_only = || origin.map(|o| o.to_string());
        // Navigate up to conversation: tool-call -> assistant-msg -> conversation
        let path_parts: Vec<&str> = tool_call_path.split('/').collect();
        if path_parts.len() < 4 {
            tracing::warn!(
                tool_call_path = %tool_call_path,
                "Cannot resolve auth context: tool call path too short - using system context"
            );
            return system_initiated_by(origin_only());
        }

        // Go up 2 levels to get conversation path
        let conversation_path = path_parts[..path_parts.len() - 2].join("/");

        // Load conversation to get agent_ref
        let conversation = match self
            .storage
            .nodes()
            .get_by_path(
                StorageScope::new(tenant_id, repo_id, branch, workspace),
                &conversation_path,
                None,
            )
            .await
        {
            Ok(Some(c)) => c,
            Ok(None) => {
                tracing::warn!(
                    conversation_path = %conversation_path,
                    "Conversation not found, using system context"
                );
                return system_initiated_by(origin_only());
            }
            Err(e) => {
                tracing::warn!(
                    conversation_path = %conversation_path,
                    error = %e,
                    "Failed to load conversation, using system context"
                );
                return system_initiated_by(origin_only());
            }
        };

        // Extract agent_ref from conversation properties
        let (agent_workspace, agent_path) =
            match self.extract_agent_ref(&conversation_path, &conversation.properties) {
                Some(result) => result,
                None => return system_initiated_by(origin_only()),
            };

        // The agent is identified from here on, so every remaining exit records
        // it — including the failure paths, where knowing which agent tried is
        // exactly what makes the audit trail useful.
        let agent_marker = agent_identity::with_origin(agent_identity::agent(&agent_path), origin);

        // Load agent to get execution_context setting
        let agent = match self
            .storage
            .nodes()
            .get_by_path(
                StorageScope::new(tenant_id, repo_id, branch, &agent_workspace),
                &agent_path,
                None,
            )
            .await
        {
            Ok(Some(a)) => a,
            Ok(None) => {
                tracing::warn!(
                    agent_path = %agent_path,
                    "Agent not found, using system context"
                );
                return system_initiated_by(Some(agent_marker));
            }
            Err(e) => {
                tracing::warn!(
                    agent_path = %agent_path,
                    error = %e,
                    "Failed to load agent, using system context"
                );
                return system_initiated_by(Some(agent_marker));
            }
        };

        // Get execution_context from agent properties (default: "user")
        let execution_context = agent
            .properties
            .get("execution_context")
            .and_then(|v| match v {
                PropertyValue::String(s) => Some(s.as_str()),
                _ => None,
            })
            .unwrap_or("user");

        tracing::info!(
            agent_path = %agent_path,
            execution_context = %execution_context,
            "Resolved agent execution_context"
        );

        match execution_context {
            // The agent's OWN rights. An agent with roles or groups is a
            // principal like any other: it executes under exactly what it was
            // granted, RLS included, so "what may this bot touch" is answered
            // by the access model rather than by hoping the prompt behaves.
            //
            // Opt-in by configuration AND by content: an agent set to `agent`
            // with nothing granted would be able to do NOTHING, which is a
            // silent, confusing failure — so an empty grant list falls back to
            // the previous behaviour and says so in the log.
            "agent" => {
                let roles = string_array(&agent.properties, "roles");
                let groups = string_array(&agent.properties, "groups");
                if roles.is_empty() && groups.is_empty() {
                    tracing::warn!(
                        agent_path = %agent_path,
                        "Agent is set to run under its own permissions but has no roles or \
                         groups; falling back to system context"
                    );
                    return system_initiated_by(Some(agent_marker));
                }

                match self
                    .permission_service()
                    .resolve_for_principal_node(tenant_id, repo_id, branch, &agent)
                    .await
                {
                    Ok(resolved) => {
                        tracing::info!(
                            agent_path = %agent_path,
                            roles = ?resolved.effective_roles,
                            permissions = resolved.permissions.len(),
                            "Agent executing under its own permissions"
                        );
                        // `user_id` is the agent's own marker, so a REL
                        // condition (`node.created_by == auth.user_id`) and the
                        // authorship stamp agree on who this is.
                        Some(
                            AuthContext::for_user(&agent_marker)
                                .with_permissions(resolved)
                                .with_agent(agent_marker.clone()),
                        )
                    }
                    Err(error) => {
                        // FAIL CLOSED. An agent whose rights cannot be resolved
                        // must not silently inherit system privileges — that is
                        // the one outcome nobody would notice.
                        tracing::error!(
                            agent_path = %agent_path,
                            error = %error,
                            "Failed to resolve the agent's permissions; refusing the tool call"
                        );
                        None
                    }
                }
            }
            "system" => {
                // System context: return AuthContext::system() to bypass RLS
                // NodeService requires explicit AuthContext::system() for admin operations
                // (returning None causes NodeService to DENY access for security)
                tracing::info!(
                    agent_path = %agent_path,
                    "Using system context for function execution (agent configured for system)"
                );
                system_initiated_by(Some(agent_marker))
            }
            _ => {
                // User context: for now, use system context as fallback
                // TODO: Implement proper user context resolution from conversation owner
                // The conversation should have created_by or owner info we can use
                tracing::info!(
                    agent_path = %agent_path,
                    "Using system context for function execution (user context not yet implemented - using system as fallback)"
                );
                // NOTE: the agent marker is provenance only and does NOT resolve
                // the TODO below — that is about which *human's* permissions
                // apply, which is an authorization question, not an attribution
                // one.
                system_initiated_by(Some(agent_marker)) // TODO: Return actual user auth context
            }
        }
    }

    /// Extract agent_ref workspace and path from conversation properties
    fn extract_agent_ref(
        &self,
        conversation_path: &str,
        properties: &std::collections::HashMap<String, PropertyValue>,
    ) -> Option<(String, String)> {
        match properties.get("agent_ref") {
            Some(PropertyValue::Reference(r)) => {
                let ws = if r.workspace.is_empty() {
                    FUNCTIONS_WORKSPACE.to_string()
                } else {
                    r.workspace.clone()
                };
                Some((ws, r.path.clone()))
            }
            Some(PropertyValue::Object(obj)) => {
                let ws = obj
                    .get("raisin:workspace")
                    .and_then(|v| match v {
                        PropertyValue::String(s) => Some(s.clone()),
                        _ => None,
                    })
                    .unwrap_or_else(|| FUNCTIONS_WORKSPACE.to_string());
                let path = obj.get("raisin:path").and_then(|v| match v {
                    PropertyValue::String(s) => Some(s.clone()),
                    _ => None,
                });
                match path {
                    Some(p) => Some((ws, p)),
                    None => {
                        tracing::warn!(
                            conversation_path = %conversation_path,
                            "agent_ref missing raisin:path, using system context"
                        );
                        None
                    }
                }
            }
            _ => {
                tracing::warn!(
                    conversation_path = %conversation_path,
                    "No agent_ref in conversation, using system context"
                );
                None
            }
        }
    }
}
