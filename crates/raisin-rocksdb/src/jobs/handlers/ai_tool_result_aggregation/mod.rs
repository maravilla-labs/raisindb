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

//! AIToolResult aggregation job handler
//!
//! This module handles the aggregation of parallel AI tool call results.
//! When an AIToolSingleCallResult node is created, this handler:
//! 1. Finds the AIToolResultAggregator sibling node under the parent message
//! 2. Atomically increments completed_count
//! 3. If all tools complete, creates an aggregated AIToolResult node
//!
//! The aggregated AIToolResult node triggers the JS agent-continue-handler
//! which handles the LLM call and creates the response. This keeps the
//! architecture fully event-driven.

mod aggregation;
mod helpers;

use raisin_error::{Error, Result};
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::jobs::{JobContext, JobInfo, JobType};
use raisin_storage::{NodeRepository, Storage, StorageScope};
use std::sync::Arc;

use super::ai_tool_call_execution::NodeCreatorCallback;

use helpers::get_int_property;

/// Handler for AIToolResult aggregation jobs
///
/// This handler processes AIToolResultAggregation jobs by:
/// 1. Finding the aggregator node
/// 2. Atomically incrementing completed_count
/// 3. If all tools complete, creating an aggregated AIToolResult node
///
/// The aggregated result triggers the JS continuation handler.
pub struct AIToolResultAggregationHandler<S: Storage> {
    /// Storage for node operations
    pub(super) storage: Arc<S>,
    /// Optional node creator callback for creating nodes through NodeService
    pub(super) node_creator: Option<NodeCreatorCallback>,
}

impl<S: Storage + 'static> AIToolResultAggregationHandler<S> {
    /// Create a new AIToolResult aggregation handler
    pub fn new(storage: Arc<S>) -> Self {
        Self {
            storage,
            node_creator: None,
        }
    }

    /// Set the node creator callback
    ///
    /// This should be called by the transport layer after initialization
    /// to provide NodeService-based node creation.
    pub fn with_node_creator(mut self, node_creator: NodeCreatorCallback) -> Self {
        self.node_creator = Some(node_creator);
        self
    }

    /// Handle AIToolResult aggregation job
    pub async fn handle(
        &self,
        job: &JobInfo,
        context: &JobContext,
    ) -> Result<Option<serde_json::Value>> {
        // Extract job parameters
        let (single_result_path, workspace) = match &job.job_type {
            JobType::AIToolResultAggregation {
                single_result_path,
                workspace,
            } => (single_result_path.clone(), workspace.clone()),
            _ => {
                return Err(Error::Validation(
                    "Expected AIToolResultAggregation job type".to_string(),
                ))
            }
        };

        tracing::info!(
            job_id = %job.id,
            single_result_path = %single_result_path,
            workspace = %workspace,
            tenant_id = %context.tenant_id,
            repo_id = %context.repo_id,
            "Processing AIToolResult aggregation job"
        );

        // Navigate: /chat/msg/tool-call/result -> /chat/msg (assistant message)
        // Normalize the path first to handle any double slashes
        let normalized_path = single_result_path
            .split('/')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("/");
        let normalized_path = format!("/{}", normalized_path);

        let path_parts: Vec<&str> = normalized_path.split('/').collect();
        if path_parts.len() < 4 {
            return Err(Error::Validation(format!(
                "Invalid single_result_path: {} (expected at least 3 path segments)",
                single_result_path
            )));
        }

        // Go up two levels: result -> tool-call -> assistant-msg
        let assistant_msg_path = path_parts[..path_parts.len() - 2].join("/");

        tracing::debug!(
            single_result_path = %single_result_path,
            assistant_msg_path = %assistant_msg_path,
            "Navigating to assistant message"
        );

        // Find aggregator node (sibling of tool calls)
        let aggregator_path = format!("{}/tool_aggregator", assistant_msg_path);
        let aggregator_opt = self
            .storage
            .nodes()
            .get_by_path(
                StorageScope::new(
                    &context.tenant_id,
                    &context.repo_id,
                    &context.branch,
                    &workspace,
                ),
                &aggregator_path,
                None,
            )
            .await?;

        // Completion is decided by RECOUNTING persisted nodes, not by a
        // stored counter: property updates are last-writer-wins, so a
        // read-modify-write counter loses updates when two results complete
        // concurrently (both read N, both write N+1, the turn stalls forever).
        //
        // Recounting is race-free here because every result node is committed
        // BEFORE its aggregation job is enqueued — the job for the last
        // result therefore always observes the full set.
        let (tool_results, skip_continuation, tool_call_count) = self
            .collect_all_results(
                &context.tenant_id,
                &context.repo_id,
                &context.branch,
                &workspace,
                &assistant_msg_path,
            )
            .await?;

        // expected_count was computed when the aggregator was created and may
        // undercount if tool-call nodes were still being created; take the
        // max with the live tool-call count.
        let expected_count = match &aggregator_opt {
            Some(aggregator) => {
                get_int_property(aggregator, "expected_count")?.max(tool_call_count as i64)
            }
            None => tool_call_count as i64,
        };
        let completed_count = tool_results.len() as i64;

        if aggregator_opt.is_some() {
            self.store_completed_count(
                &context.tenant_id,
                &context.repo_id,
                &context.branch,
                &workspace,
                &aggregator_path,
                completed_count,
            )
            .await;
        }

        tracing::info!(
            aggregator_path = %aggregator_path,
            completed = completed_count,
            expected = expected_count,
            has_aggregator = aggregator_opt.is_some(),
            "Recounted aggregator state"
        );

        // Not all complete yet - exit early
        if completed_count < expected_count {
            return Ok(Some(serde_json::json!({
                "status": "waiting",
                "completed": completed_count,
                "expected": expected_count
            })));
        }

        // ALL COMPLETE - create aggregated AIToolResult node
        tracing::info!(
            assistant_msg_path = %assistant_msg_path,
            "All {} tools complete, creating aggregated result",
            expected_count
        );

        self.create_aggregated_result(
            &context.tenant_id,
            &context.repo_id,
            &context.branch,
            &workspace,
            &assistant_msg_path,
            tool_results,
            skip_continuation,
        )
        .await?;

        Ok(Some(serde_json::json!({
            "status": "complete",
            "result_count": expected_count
        })))
    }
}
