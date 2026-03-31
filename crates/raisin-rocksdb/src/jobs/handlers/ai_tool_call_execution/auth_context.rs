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

use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::{NodeRepository, Storage, StorageScope};

use super::types::FUNCTIONS_WORKSPACE;
use super::AIToolCallExecutionHandler;

impl<S: Storage + 'static> AIToolCallExecutionHandler<S> {
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
    /// Path structure: /conversations/{chat-id}/{msg-id}/{tool-call-id}
    /// So we go up 2 levels to get the conversation path
    pub(super) async fn resolve_auth_context_for_tool_call(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        tool_call_path: &str,
    ) -> Option<AuthContext> {
        // Navigate up to conversation: tool-call -> assistant-msg -> conversation
        let path_parts: Vec<&str> = tool_call_path.split('/').collect();
        if path_parts.len() < 4 {
            tracing::warn!(
                tool_call_path = %tool_call_path,
                "Cannot resolve auth context: tool call path too short - using system context"
            );
            return Some(AuthContext::system());
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
                return Some(AuthContext::system());
            }
            Err(e) => {
                tracing::warn!(
                    conversation_path = %conversation_path,
                    error = %e,
                    "Failed to load conversation, using system context"
                );
                return Some(AuthContext::system());
            }
        };

        // Extract agent_ref from conversation properties
        let (agent_workspace, agent_path) =
            match self.extract_agent_ref(&conversation_path, &conversation.properties) {
                Some(result) => result,
                None => return Some(AuthContext::system()),
            };

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
                return Some(AuthContext::system());
            }
            Err(e) => {
                tracing::warn!(
                    agent_path = %agent_path,
                    error = %e,
                    "Failed to load agent, using system context"
                );
                return Some(AuthContext::system());
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
            "system" => {
                // System context: return AuthContext::system() to bypass RLS
                // NodeService requires explicit AuthContext::system() for admin operations
                // (returning None causes NodeService to DENY access for security)
                tracing::info!(
                    agent_path = %agent_path,
                    "Using system context for function execution (agent configured for system)"
                );
                Some(AuthContext::system())
            }
            _ => {
                // User context: for now, use system context as fallback
                // TODO: Implement proper user context resolution from conversation owner
                // The conversation should have created_by or owner info we can use
                tracing::info!(
                    agent_path = %agent_path,
                    "Using system context for function execution (user context not yet implemented - using system as fallback)"
                );
                Some(AuthContext::system()) // TODO: Return actual user auth context
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
