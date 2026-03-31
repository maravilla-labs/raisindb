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

//! Aggregator node management for multi-tool coordination
//!
//! When multiple AI tool calls execute in parallel, an aggregator node
//! tracks expected vs completed results to coordinate the final response.

use raisin_error::Result;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::{ListOptions, NodeRepository, Storage, StorageScope};

use super::AIToolCallExecutionHandler;

impl<S: Storage + 'static> AIToolCallExecutionHandler<S> {
    /// Ensure an AIToolResultAggregator node exists for multi-tool coordination
    ///
    /// This is called by each AIToolCallExecution job, but only the first one
    /// to run will actually create the aggregator. Others will skip via
    /// idempotent check (node already exists).
    ///
    /// The aggregator tracks:
    /// - expected_count: Number of AIToolCall sibling nodes
    /// - completed_count: Number of results received (starts at 0)
    pub(super) async fn ensure_aggregator_exists(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        tool_call_path: &str,
    ) -> Result<()> {
        // Navigate to assistant message path (parent of tool-call)
        // tool_call_path is like "/conversations/chat-123/assistant-msg/tool-call-xxx"
        let assistant_msg_path = tool_call_path
            .rsplit_once('/')
            .map(|(parent, _)| parent)
            .unwrap_or("/");

        // Check if aggregator already exists
        let aggregator_path = format!("{}/tool_aggregator", assistant_msg_path);
        let aggregator_exists = self
            .storage
            .nodes()
            .get_by_path(
                StorageScope::new(tenant_id, repo_id, branch, workspace),
                &aggregator_path,
                None,
            )
            .await?
            .is_some();

        if aggregator_exists {
            tracing::debug!(
                aggregator_path = %aggregator_path,
                "Aggregator already exists, skipping creation"
            );
            return Ok(());
        }

        // Count total tool calls (siblings under assistant message)
        let siblings = self
            .storage
            .nodes()
            .list_children(
                StorageScope::new(tenant_id, repo_id, branch, workspace),
                assistant_msg_path,
                ListOptions::default(),
            )
            .await?;

        let tool_call_count = siblings
            .iter()
            .filter(|n| n.node_type == "raisin:AIToolCall")
            .count();

        // Only create aggregator if there are multiple tool calls
        // (single tool call doesn't need aggregation)
        if tool_call_count <= 1 {
            tracing::debug!(
                tool_call_count = tool_call_count,
                "Single tool call, skipping aggregator creation"
            );
            return Ok(());
        }

        tracing::info!(
            aggregator_path = %aggregator_path,
            tool_call_count = tool_call_count,
            "Creating AIToolResultAggregator node"
        );

        // Create aggregator node (via NodeService for proper events)
        let mut properties = std::collections::HashMap::new();
        properties.insert(
            "expected_count".to_string(),
            PropertyValue::Integer(tool_call_count as i64),
        );
        properties.insert("completed_count".to_string(), PropertyValue::Integer(0));

        let aggregator_node = Node {
            id: uuid::Uuid::new_v4().to_string(),
            name: "tool_aggregator".to_string(),
            path: aggregator_path.clone(),
            node_type: "raisin:AIToolResultAggregator".to_string(),
            properties,
            created_at: Some(chrono::Utc::now()),
            ..Default::default()
        };

        if let Some(ref node_creator) = self.node_creator {
            self.create_aggregator_via_node_service(
                node_creator,
                aggregator_node,
                tenant_id,
                repo_id,
                branch,
                workspace,
                &aggregator_path,
            )
            .await;
        } else {
            self.create_aggregator_via_direct_storage(
                aggregator_node,
                tenant_id,
                repo_id,
                branch,
                workspace,
                &aggregator_path,
            )
            .await;
        }

        Ok(())
    }

    /// Create aggregator via NodeService callback for proper event publishing
    async fn create_aggregator_via_node_service(
        &self,
        node_creator: &super::types::NodeCreatorCallback,
        aggregator_node: Node,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        aggregator_path: &str,
    ) {
        // Note: This may fail if another handler already created the aggregator
        // That's fine - it's idempotent
        let result = node_creator(
            aggregator_node,
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
            workspace.to_string(),
        )
        .await;

        match result {
            Ok(_) => {
                tracing::debug!(
                    aggregator_path = %aggregator_path,
                    "Created AIToolResultAggregator node"
                );
            }
            Err(e) => {
                // Check if it's a "node already exists" error (race condition)
                let err_str = e.to_string();
                if err_str.contains("already exists") || err_str.contains("duplicate") {
                    tracing::debug!(
                        aggregator_path = %aggregator_path,
                        "Aggregator already created by another handler"
                    );
                } else {
                    tracing::warn!(
                        error = %e,
                        aggregator_path = %aggregator_path,
                        "Failed to create aggregator (non-critical)"
                    );
                }
            }
        }
    }

    /// Fall back to direct storage creation for aggregator
    ///
    /// The aggregator doesn't strictly need events - it's a coordination node.
    async fn create_aggregator_via_direct_storage(
        &self,
        aggregator_node: Node,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        aggregator_path: &str,
    ) {
        tracing::warn!(
            aggregator_path = %aggregator_path,
            "NodeService callback not configured, creating aggregator via direct storage"
        );

        use raisin_storage::CreateNodeOptions;

        let result = self
            .storage
            .nodes()
            .create(
                StorageScope::new(tenant_id, repo_id, branch, workspace),
                aggregator_node,
                CreateNodeOptions::default(),
            )
            .await;

        match result {
            Ok(_) => {
                tracing::debug!(
                    aggregator_path = %aggregator_path,
                    "Created AIToolResultAggregator node via direct storage"
                );
            }
            Err(e) => {
                let err_str = e.to_string();
                if err_str.contains("already exists") || err_str.contains("duplicate") {
                    tracing::debug!(
                        aggregator_path = %aggregator_path,
                        "Aggregator already created by another handler"
                    );
                } else {
                    tracing::warn!(
                        error = %e,
                        aggregator_path = %aggregator_path,
                        "Failed to create aggregator via direct storage (non-critical)"
                    );
                }
            }
        }
    }
}
