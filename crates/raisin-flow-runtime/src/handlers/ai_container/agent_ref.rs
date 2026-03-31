// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Agent reference resolution for AI containers
//!
//! Handles resolving agent references from various formats:
//! - `$auto`: Derives agent from conversation properties
//! - String path: Direct path to the agent node
//! - Reference object: Structured reference with workspace and path

use super::types::AgentReference;
use crate::types::{FlowCallbacks, FlowContext, FlowError, FlowNode, FlowResult};
use tracing::debug;

use super::AiContainerHandler;

impl AiContainerHandler {
    /// Resolve $auto agent_ref by looking up the conversation's agent_ref property
    ///
    /// When agent_ref is "$auto", this method:
    /// 1. Gets the message node path from flow context (trigger_info.node_path or input.event.node_path)
    /// 2. Derives the conversation path (parent of message)
    /// 3. Loads the conversation node
    /// 4. Returns the agent_ref from conversation properties (as a Reference object)
    ///
    /// The agent_ref in conversation is stored as a Reference object:
    /// ```json
    /// {
    ///   "raisin:ref": "node-id",
    ///   "raisin:workspace": "functions",
    ///   "raisin:path": "/agents/my-agent"
    /// }
    /// ```
    pub(super) async fn resolve_auto_agent_ref(
        &self,
        context: &FlowContext,
        callbacks: &dyn FlowCallbacks,
    ) -> FlowResult<AgentReference> {
        // Get message path from trigger info or flow input
        let message_path = context
            .trigger_info
            .as_ref()
            .and_then(|t| t.node_path.clone())
            .or_else(|| {
                context
                    .input
                    .get("event")
                    .and_then(|e| e.get("node_path"))
                    .and_then(|p| p.as_str())
                    .map(String::from)
            })
            .ok_or_else(|| {
                FlowError::MissingProperty(
                    "Cannot resolve $auto agent_ref: message path not found in context".to_string(),
                )
            })?;

        debug!("Resolving agent_ref from message path: {}", message_path);

        // Derive conversation path (parent of message)
        let conversation_path = message_path
            .rsplit_once('/')
            .map(|(parent, _)| if parent.is_empty() { "/" } else { parent })
            .ok_or_else(|| {
                FlowError::MissingProperty(format!(
                    "Cannot resolve $auto agent_ref: invalid message path: {}",
                    message_path
                ))
            })?
            .to_string();

        debug!("Derived conversation path: {}", conversation_path);

        // Get the workspace from flow input (where the conversation lives)
        let conversation_workspace = context
            .input
            .get("workspace")
            .and_then(|w| w.as_str())
            .unwrap_or("ai")
            .to_string();

        debug!("Conversation workspace: {}", conversation_workspace);

        // Load conversation node from the correct workspace
        // Note: We need to pass workspace info to get_node, but current callback doesn't support it
        // For now, we'll extract the node data from flow_input which contains the full node
        let conversation_node = context
            .input
            .get("node")
            .and_then(|n| n.get("properties"))
            .and_then(|p| p.get("agent_ref"))
            .cloned();

        // If not in flow_input.node, try loading via callback (for non-trigger contexts)
        let agent_ref_value = if let Some(ref_val) = conversation_node {
            ref_val
        } else {
            // Fallback: try to get from callbacks (may not work if workspace is wrong)
            let conv_node = callbacks
                .get_node(&conversation_path)
                .await
                .map_err(|e| FlowError::Other(format!("Failed to load conversation: {}", e)))?
                .ok_or_else(|| {
                    FlowError::NodeNotFound(format!(
                        "Conversation not found at path: {}",
                        conversation_path
                    ))
                })?;

            conv_node
                .get("properties")
                .and_then(|p| p.get("agent_ref"))
                .cloned()
                .ok_or_else(|| {
                    FlowError::MissingProperty(format!(
                        "Conversation at {} missing agent_ref property",
                        conversation_path
                    ))
                })?
        };

        // Parse agent_ref as a Reference object
        // Reference format: { "raisin:ref": "id", "raisin:workspace": "ws", "raisin:path": "/path" }
        let agent_ref = if agent_ref_value.is_object() {
            let ref_obj = agent_ref_value.as_object().unwrap();
            AgentReference {
                id: ref_obj
                    .get("raisin:ref")
                    .and_then(|v| v.as_str())
                    .map(String::from),
                workspace: ref_obj
                    .get("raisin:workspace")
                    .and_then(|v| v.as_str())
                    .unwrap_or("functions")
                    .to_string(),
                path: ref_obj
                    .get("raisin:path")
                    .and_then(|v| v.as_str())
                    .or_else(|| ref_obj.get("raisin:ref").and_then(|v| v.as_str()))
                    .ok_or_else(|| {
                        FlowError::MissingProperty(
                            "agent_ref Reference missing raisin:path or raisin:ref".to_string(),
                        )
                    })?
                    .to_string(),
            }
        } else if agent_ref_value.is_string() {
            // Legacy support: plain string path (assumes functions workspace)
            AgentReference {
                id: None,
                workspace: "functions".to_string(),
                path: agent_ref_value.as_str().unwrap().to_string(),
            }
        } else {
            return Err(FlowError::MissingProperty(
                "agent_ref must be a Reference object or string".to_string(),
            ));
        };

        debug!(
            "Resolved agent_ref: workspace={}, path={}",
            agent_ref.workspace, agent_ref.path
        );

        Ok(agent_ref)
    }

    /// Parse a direct agent_ref string into AgentReference
    ///
    /// Handles both simple path strings and Reference object format in step properties.
    pub(super) fn parse_agent_ref(
        &self,
        agent_ref_str: &str,
        step: &FlowNode,
    ) -> FlowResult<AgentReference> {
        // First, check if there's a structured agent_ref object in properties
        if let Some(ref_obj) = step.get_property("agent_ref").and_then(|v| v.as_object()) {
            // Check if it's a Reference object (has raisin:ref or raisin:path)
            if ref_obj.contains_key("raisin:ref") || ref_obj.contains_key("raisin:path") {
                return Ok(AgentReference {
                    id: ref_obj
                        .get("raisin:ref")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                    workspace: ref_obj
                        .get("raisin:workspace")
                        .and_then(|v| v.as_str())
                        .unwrap_or("functions")
                        .to_string(),
                    path: ref_obj
                        .get("raisin:path")
                        .and_then(|v| v.as_str())
                        .or_else(|| ref_obj.get("raisin:ref").and_then(|v| v.as_str()))
                        .unwrap_or(agent_ref_str)
                        .to_string(),
                });
            }
        }

        // Simple string path - assume functions workspace (where agents are typically stored)
        Ok(AgentReference {
            id: None,
            workspace: "functions".to_string(),
            path: agent_ref_str.to_string(),
        })
    }
}
