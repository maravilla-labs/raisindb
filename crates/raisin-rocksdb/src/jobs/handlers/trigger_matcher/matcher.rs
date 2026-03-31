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

//! Trigger matcher callback factory
//!
//! Creates the main trigger matcher callback that queries storage for
//! both inline function triggers and standalone trigger nodes.

use raisin_storage::{ListOptions, NodeRepository, Storage, StorageScope};
use std::sync::Arc;

use super::inline_triggers::{process_inline_triggers, InlineTriggerContext};
use super::standalone_triggers::process_standalone_triggers;
use crate::jobs::handlers::TriggerMatcherCallback;

/// Create a trigger matcher callback
///
/// This callback queries raisin:Function nodes and standalone raisin:Trigger nodes
/// to find triggers matching the given event. Returns both the matching triggers
/// and detailed evaluation results for debugging (showing which filters passed/failed).
///
/// # Arguments
///
/// * `storage` - Arc to the storage implementation (RocksDB or other)
///
/// # Returns
///
/// A TriggerMatcherCallback that can be used with TriggerEvaluationHandler
pub fn create_trigger_matcher<S: Storage + 'static>(storage: Arc<S>) -> TriggerMatcherCallback {
    Arc::new(
        move |event_type: String,
              node_id: String,
              node_type: String,
              node_path: String,
              tenant_id: String,
              repo_id: String,
              branch: String,
              workspace: String,
              _node_properties: Option<serde_json::Value>| {
            let storage = storage.clone();

            Box::pin(async move {
                // Defensive check: reject empty tenant_id or repo_id
                if tenant_id.is_empty() || repo_id.is_empty() {
                    tracing::error!(
                        tenant_id = %tenant_id,
                        repo_id = %repo_id,
                        "Trigger matcher called with empty tenant_id or repo_id"
                    );
                    return Ok((vec![], vec![]));
                }

                tracing::debug!(
                    event_type = %event_type,
                    node_type = %node_type,
                    node_path = %node_path,
                    "Looking for matching triggers (with debug)"
                );

                let mut matches = Vec::new();
                let mut all_results = Vec::new();

                let ctx = InlineTriggerContext {
                    event_type: &event_type,
                    node_id: &node_id,
                    node_type: &node_type,
                    node_path: &node_path,
                    tenant_id: &tenant_id,
                    repo_id: &repo_id,
                    branch: &branch,
                    workspace: &workspace,
                };

                // Query all raisin:Function nodes in the functions workspace
                let functions = storage
                    .nodes()
                    .list_by_type(
                        StorageScope::new(&tenant_id, &repo_id, &branch, "functions"),
                        "raisin:Function",
                        ListOptions::default(),
                    )
                    .await?;

                // Process inline triggers on raisin:Function nodes
                process_inline_triggers(&storage, functions, &ctx, &mut matches, &mut all_results)
                    .await?;

                // Query and process standalone raisin:Trigger nodes
                let standalone_triggers = storage
                    .nodes()
                    .list_by_type(
                        StorageScope::new(&tenant_id, &repo_id, &branch, "functions"),
                        "raisin:Trigger",
                        ListOptions::default(),
                    )
                    .await
                    .unwrap_or_default();

                process_standalone_triggers(
                    &storage,
                    standalone_triggers,
                    &ctx,
                    &mut matches,
                    &mut all_results,
                )
                .await?;

                tracing::info!(
                    event_type = %event_type,
                    node_type = %node_type,
                    match_count = matches.len(),
                    total_evaluated = all_results.len(),
                    "Trigger matching with debug complete"
                );

                Ok((matches, all_results))
            })
        },
    )
}
