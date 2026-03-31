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

//! Function execution, status updates, and result creation for AI tool calls

use raisin_error::{Error, Result};
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::{ListOptions, NodeRepository, Storage, StorageScope};
use std::collections::HashMap;

use super::types::json_to_property_value;
use super::AIToolCallExecutionHandler;
use crate::jobs::handlers::function_execution::FunctionExecutionResult;

impl<S: Storage + 'static> AIToolCallExecutionHandler<S> {
    /// Execute the function inline using the executor callback
    ///
    /// The `auth_context` parameter controls permissions:
    /// - `None`: Function runs with system context (full access)
    /// - `Some(auth)`: Function runs with user's permissions (RLS applied)
    pub(super) async fn execute_function(
        &self,
        function_path: &str,
        execution_id: &str,
        arguments: serde_json::Value,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        function_workspace: &str,
        auth_context: Option<AuthContext>,
    ) -> Result<FunctionExecutionResult> {
        // Check if executor is available
        let executor = self.executor.as_ref().ok_or_else(|| {
            Error::Validation(
                "Function executor not configured. The transport layer must provide the executor callback.".to_string()
            )
        })?;

        // Execute the function inline with auth context
        executor(
            function_path.to_string(),
            execution_id.to_string(),
            arguments,
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
            function_workspace.to_string(),
            auth_context,
            None, // No real-time log streaming for AI tool calls
        )
        .await
    }

    /// Update the status property of an AIToolCall node
    pub(super) async fn update_status(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        tool_call_path: &str,
        status: &str,
    ) -> Result<()> {
        self.storage
            .nodes()
            .update_property_by_path(
                StorageScope::new(tenant_id, repo_id, branch, workspace),
                tool_call_path,
                "status",
                PropertyValue::String(status.to_string()),
            )
            .await?;

        tracing::debug!(
            tool_call_path = %tool_call_path,
            status = %status,
            "Updated AIToolCall status"
        );

        Ok(())
    }

    /// Create an AIToolSingleCallResult child node under the AIToolCall
    ///
    /// This creates a single result node that will trigger the aggregation job.
    /// Uses NodeService callback if available for proper event publishing,
    /// otherwise falls back to direct storage creation.
    pub(super) async fn create_tool_result(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        tool_call_path: &str,
        tool_call_id: Option<&str>,
        function_name: Option<&str>,
        result: Option<serde_json::Value>,
        error: Option<String>,
        duration_ms: u64,
    ) -> Result<()> {
        // Build properties
        let mut properties = HashMap::new();

        if let Some(r) = result {
            properties.insert("result".to_string(), json_to_property_value(r)?);
        }

        if let Some(e) = error {
            properties.insert("error".to_string(), PropertyValue::String(e));
        }

        properties.insert(
            "duration_ms".to_string(),
            PropertyValue::Integer(duration_ms as i64),
        );

        if let Some(id) = tool_call_id.filter(|v| !v.is_empty()) {
            properties.insert(
                "tool_call_id".to_string(),
                PropertyValue::String(id.to_string()),
            );
        }

        if let Some(name) = function_name.filter(|v| !v.is_empty()) {
            properties.insert(
                "function_name".to_string(),
                PropertyValue::String(name.to_string()),
            );
        }

        // Use deterministic name - only one result per tool call for idempotency
        let result_name = "result".to_string();
        let result_path = format!("{}/{}", tool_call_path, result_name);

        // IDEMPOTENCY CHECK: Check if result already exists before creating
        // This handles retries where the result was already created
        let existing = self
            .storage
            .nodes()
            .get_by_path(
                StorageScope::new(tenant_id, repo_id, branch, workspace),
                &result_path,
                None,
            )
            .await?;

        if existing.is_some() {
            tracing::info!(
                tool_call_path = %tool_call_path,
                result_path = %result_path,
                "AIToolSingleCallResult already exists, skipping creation"
            );
            return Ok(());
        }

        let result_node = Node {
            id: uuid::Uuid::new_v4().to_string(),
            name: result_name,
            path: result_path.clone(),
            node_type: "raisin:AIToolSingleCallResult".to_string(),
            properties,
            created_at: Some(chrono::Utc::now()),
            ..Default::default()
        };

        // IMPORTANT: AIToolSingleCallResult MUST be created via NodeService to fire events
        // The event triggers the aggregation job which coordinates parallel tool results
        // Without events, the aggregation flow breaks completely
        let node_creator = self.node_creator.as_ref().ok_or_else(|| {
            Error::Validation(
                "NodeService callback not configured. AIToolSingleCallResult requires NodeService \
                 for proper event publishing. The transport layer must provide the node_creator callback."
                    .to_string(),
            )
        })?;

        node_creator(
            result_node,
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
            workspace.to_string(),
        )
        .await?;

        tracing::debug!(
            tool_call_path = %tool_call_path,
            result_path = %result_path,
            "Created AIToolSingleCallResult node via NodeService"
        );

        Ok(())
    }

    /// Check if the tool call already has a result child
    pub(super) async fn has_existing_result(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        tool_call_path: &str,
    ) -> Result<bool> {
        let children = self
            .storage
            .nodes()
            .list_children(
                StorageScope::new(tenant_id, repo_id, branch, workspace),
                tool_call_path,
                ListOptions::default(),
            )
            .await?;

        // Check for both new single result type and legacy aggregated result type
        Ok(children.iter().any(|child| {
            child.node_type == "raisin:AIToolSingleCallResult"
                || child.node_type == "raisin:AIToolResult"
        }))
    }
}
