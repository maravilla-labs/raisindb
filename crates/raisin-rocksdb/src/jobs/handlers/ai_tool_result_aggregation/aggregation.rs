//! Aggregation logic: collecting results, atomic counting, and creating aggregated nodes.

use super::helpers::{json_to_property_value, property_value_to_json};
use super::AIToolResultAggregationHandler;
use raisin_error::{Error, Result};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::{ListOptions, NodeRepository, Storage, StorageScope};
use std::collections::HashMap;
use std::sync::Arc;

impl<S: Storage + 'static> AIToolResultAggregationHandler<S> {
    /// Atomically increment the completed_count on the aggregator node
    ///
    /// Uses compare-and-swap to ensure exactly one handler wins when multiple
    /// results complete simultaneously.
    pub(super) async fn atomic_increment_completed_count(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        aggregator_path: &str,
    ) -> Result<i64> {
        let max_retries = 10;
        for attempt in 0..max_retries {
            let node = self
                .storage
                .nodes()
                .get_by_path(
                    StorageScope::new(tenant_id, repo_id, branch, workspace),
                    aggregator_path,
                    None,
                )
                .await?
                .ok_or_else(|| {
                    Error::NotFound(format!("Aggregator not found: {}", aggregator_path))
                })?;

            let current = super::helpers::get_int_property(&node, "completed_count").unwrap_or(0);
            let new_value = current + 1;

            let result = self
                .storage
                .nodes()
                .update_property_by_path(
                    StorageScope::new(tenant_id, repo_id, branch, workspace),
                    aggregator_path,
                    "completed_count",
                    PropertyValue::Integer(new_value),
                )
                .await;

            match result {
                Ok(_) => {
                    tracing::debug!(
                        aggregator_path = %aggregator_path,
                        previous = current,
                        new = new_value,
                        attempt = attempt,
                        "Successfully incremented completed_count"
                    );
                    return Ok(new_value);
                }
                Err(e) => {
                    let err_str = e.to_string();
                    if err_str.contains("version") || err_str.contains("conflict") {
                        tracing::debug!(
                            aggregator_path = %aggregator_path,
                            attempt = attempt,
                            "CAS conflict, retrying"
                        );
                        tokio::time::sleep(tokio::time::Duration::from_millis(
                            10 * (attempt as u64 + 1),
                        ))
                        .await;
                        continue;
                    }
                    return Err(e);
                }
            }
        }

        Err(Error::Validation(format!(
            "Failed to increment completed_count after {} retries",
            max_retries
        )))
    }

    /// Collect all AIToolSingleCallResult nodes under the assistant message
    ///
    /// Returns `(results, skip_continuation)`. If any single result has
    /// `skip_continuation: true`, the flag is propagated so the aggregated
    /// AIToolResult can suppress the continuation trigger.
    pub(super) async fn collect_all_results(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        assistant_msg_path: &str,
    ) -> Result<(Vec<serde_json::Value>, bool)> {
        let children = self
            .storage
            .nodes()
            .list_children(
                StorageScope::new(tenant_id, repo_id, branch, workspace),
                assistant_msg_path,
                ListOptions::default(),
            )
            .await?;

        let tool_calls: Vec<_> = children
            .into_iter()
            .filter(|n| n.node_type == "raisin:AIToolCall")
            .collect();

        let mut results = Vec::new();
        let mut skip_continuation = false;

        for tool_call in tool_calls {
            let tc_children = self
                .storage
                .nodes()
                .list_children(
                    StorageScope::new(tenant_id, repo_id, branch, workspace),
                    &tool_call.path,
                    ListOptions::default(),
                )
                .await?;

            if let Some(result_node) = tc_children.iter().find(|c| {
                c.node_type == "raisin:AIToolSingleCallResult"
                    || c.node_type == "raisin:AIToolResult"
            }) {
                if matches!(
                    result_node.properties.get("skip_continuation"),
                    Some(PropertyValue::Boolean(true))
                ) {
                    skip_continuation = true;
                }

                let tool_call_id = extract_tool_call_id(&tool_call);
                let function_name = extract_function_name(&tool_call);

                let result_content = result_node
                    .properties
                    .get("result")
                    .map(property_value_to_json)
                    .unwrap_or(serde_json::Value::Null);

                let error = result_node.properties.get("error").and_then(|v| match v {
                    PropertyValue::String(s) => Some(s.clone()),
                    _ => None,
                });

                results.push(serde_json::json!({
                    "tool_call_id": tool_call_id,
                    "function_name": function_name,
                    "result": result_content,
                    "error": error
                }));
            }
        }

        tracing::debug!(
            assistant_msg_path = %assistant_msg_path,
            result_count = results.len(),
            skip_continuation = skip_continuation,
            "Collected tool results"
        );

        Ok((results, skip_continuation))
    }

    /// Create an aggregated AIToolResult node
    ///
    /// This node is created as a sibling of the tool calls (under the assistant message).
    /// Its creation triggers the agent-continue-handler.js via the event system.
    pub(super) async fn create_aggregated_result(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        assistant_msg_path: &str,
        results: Vec<serde_json::Value>,
        skip_continuation: bool,
    ) -> Result<()> {
        let node_creator = self.node_creator.as_ref().ok_or_else(|| {
            Error::Validation(
                "Node creator callback not configured. The transport layer must provide it."
                    .to_string(),
            )
        })?;

        let result_path = format!("{}/aggregated_result", assistant_msg_path);

        // Check if already exists (idempotency)
        if self
            .storage
            .nodes()
            .get_by_path(
                StorageScope::new(tenant_id, repo_id, branch, workspace),
                &result_path,
                None,
            )
            .await?
            .is_some()
        {
            tracing::debug!(
                result_path = %result_path,
                "Aggregated result already exists, skipping creation"
            );
            return Ok(());
        }

        let mut properties = HashMap::new();
        properties.insert(
            "results".to_string(),
            json_to_property_value(serde_json::Value::Array(results.clone()))?,
        );
        properties.insert(
            "result_count".to_string(),
            PropertyValue::Integer(results.len() as i64),
        );
        properties.insert("aggregated".to_string(), PropertyValue::Boolean(true));
        if skip_continuation {
            properties.insert(
                "skip_continuation".to_string(),
                PropertyValue::Boolean(true),
            );
        }

        let result_node = Node {
            id: uuid::Uuid::new_v4().to_string(),
            name: "aggregated_result".to_string(),
            path: result_path.clone(),
            node_type: "raisin:AIToolResult".to_string(),
            properties,
            created_at: Some(chrono::Utc::now()),
            ..Default::default()
        };

        node_creator(
            result_node,
            tenant_id.to_string(),
            repo_id.to_string(),
            branch.to_string(),
            workspace.to_string(),
        )
        .await?;

        tracing::info!(
            result_path = %result_path,
            result_count = results.len(),
            "Created aggregated AIToolResult node - will trigger JS continuation"
        );

        Ok(())
    }
}

/// Extract tool_call_id from the tool call node properties.
fn extract_tool_call_id(tool_call: &Node) -> String {
    let raw_prop = tool_call.properties.get("tool_call_id");

    tracing::info!(
        tool_call_path = %tool_call.path,
        tool_call_id_property = ?raw_prop,
        all_properties = ?tool_call.properties.keys().collect::<Vec<_>>(),
        "TRACE: Extracting tool_call_id from AIToolCall properties"
    );

    raw_prop
        .and_then(|v| match v {
            PropertyValue::String(s) => {
                tracing::info!(
                    tool_call_path = %tool_call.path,
                    tool_call_id_value = %s,
                    "TRACE: Found tool_call_id as String"
                );
                Some(s.clone())
            }
            _ => {
                tracing::error!(
                    tool_call_path = %tool_call.path,
                    property_type = ?v,
                    "tool_call_id property is not a String!"
                );
                None
            }
        })
        .unwrap_or_else(|| {
            tracing::error!(
                tool_call_path = %tool_call.path,
                node_id = %tool_call.id,
                "tool_call_id property NOT FOUND, falling back to node UUID (THIS IS A BUG)"
            );
            tool_call.id.clone()
        })
}

/// Extract function name from a tool call node.
///
/// Tries `function_name` property first, then extracts from `function_ref` path as fallback.
fn extract_function_name(tool_call: &Node) -> String {
    tool_call
        .properties
        .get("function_name")
        .and_then(|v| match v {
            PropertyValue::String(s) => Some(s.clone()),
            _ => None,
        })
        .or_else(|| {
            tool_call
                .properties
                .get("function_ref")
                .and_then(|v| match v {
                    PropertyValue::Reference(r) => {
                        if r.path.is_empty() {
                            None
                        } else {
                            r.path.rsplit('/').next().map(|s| s.to_string())
                        }
                    }
                    _ => None,
                })
        })
        .unwrap_or_else(|| "unknown".to_string())
}
