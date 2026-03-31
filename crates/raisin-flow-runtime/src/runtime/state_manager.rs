// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! State persistence for flow instances with OCC (Optimistic Concurrency Control).
//!
//! This module provides functions to:
//! - Load flow instances from storage
//! - Save flow instances with version checking
//! - Handle version conflicts

use crate::types::{FlowCallbacks, FlowError, FlowInstance, FlowResult};
use tracing::{debug, info};

/// Load a flow instance from storage.
///
/// # Arguments
///
/// * `instance_id` - The flow instance ID to load
/// * `callbacks` - Callbacks for storage operations
///
/// # Returns
///
/// Returns the flow instance if found, or an error if not found or deserialization fails.
pub async fn load_instance(
    instance_id: &str,
    callbacks: &dyn FlowCallbacks,
) -> FlowResult<FlowInstance> {
    debug!("Loading flow instance: {}", instance_id);

    let path = format!("/flows/instances/{}", instance_id);
    callbacks.load_instance(&path).await
}

/// Save a flow instance to storage with version check (OCC).
///
/// This function implements Optimistic Concurrency Control by verifying that
/// the instance's version hasn't changed since it was loaded. This prevents
/// the "double click" problem where multiple processes try to advance the
/// same flow simultaneously.
///
/// # Arguments
///
/// * `instance` - The flow instance to save
/// * `expected_version` - The version we expect the instance to have
/// * `callbacks` - Callbacks for storage operations
///
/// # Returns
///
/// Returns `Ok(())` if the save succeeds, or `Err(FlowError::VersionConflict)`
/// if another process has modified the instance.
pub async fn save_instance_with_version(
    instance: &FlowInstance,
    expected_version: i32,
    callbacks: &dyn FlowCallbacks,
) -> FlowResult<()> {
    info!(
        "Saving flow instance {} (expected version: {})",
        instance.id, expected_version
    );

    callbacks
        .save_instance_with_version(instance, expected_version)
        .await
}

/// Save a flow instance without version check.
///
/// This should only be used when creating a new instance or when you're certain
/// no concurrent modifications are possible.
///
/// # Arguments
///
/// * `instance` - The flow instance to save
/// * `callbacks` - Callbacks for storage operations
///
/// # Returns
///
/// Returns `Ok(())` if the save succeeds.
pub async fn save_instance(
    instance: &FlowInstance,
    callbacks: &dyn FlowCallbacks,
) -> FlowResult<()> {
    debug!("Saving flow instance {} (no version check)", instance.id);

    callbacks.save_instance(instance).await
}

/// Create a new flow instance from a flow definition.
///
/// This function:
/// 1. Loads the flow definition
/// 2. Snapshots the workflow_data
/// 3. Creates a new FlowInstance with pending status
/// 4. Saves it to storage
///
/// # Arguments
///
/// * `flow_ref` - Reference to the flow definition
/// * `input` - Initial input for the flow
/// * `callbacks` - Callbacks for storage operations
///
/// # Returns
///
/// Returns the newly created flow instance.
pub async fn create_flow_instance(
    flow_ref: String,
    input: serde_json::Value,
    callbacks: &dyn FlowCallbacks,
) -> FlowResult<FlowInstance> {
    info!("Creating new flow instance for flow: {}", flow_ref);

    // Load flow definition
    let flow_node = callbacks
        .get_node(&flow_ref)
        .await?
        .ok_or_else(|| FlowError::NodeNotFound(flow_ref.clone()))?;

    // Extract workflow_data (flow definition snapshot)
    let workflow_data = flow_node
        .get("workflow_data")
        .cloned()
        .ok_or_else(|| FlowError::InvalidDefinition("Missing workflow_data".to_string()))?;

    // Get flow version
    let flow_version = flow_node
        .get("_version")
        .and_then(|v| v.as_i64())
        .unwrap_or(1) as i32;

    // Get start node ID from workflow
    let start_node_id = workflow_data
        .get("nodes")
        .and_then(|n| n.as_array())
        .and_then(|nodes| {
            nodes.iter().find(|n| {
                n.get("step_type")
                    .and_then(|t| t.as_str())
                    .map(|t| t == "start")
                    .unwrap_or(false)
            })
        })
        .and_then(|n| n.get("id"))
        .and_then(|id| id.as_str())
        .map(String::from)
        .unwrap_or_else(|| "start".to_string());

    // Create instance
    let instance = FlowInstance::new(flow_ref, flow_version, workflow_data, input, start_node_id);

    // Save to storage
    save_instance(&instance, callbacks).await?;

    info!("Created flow instance: {}", instance.id);
    Ok(instance)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::FlowStatus;
    use async_trait::async_trait;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    // Mock callbacks for testing
    struct MockCallbacks {
        instances: Arc<Mutex<std::collections::HashMap<String, FlowInstance>>>,
    }

    impl MockCallbacks {
        fn new() -> Self {
            Self {
                instances: Arc::new(Mutex::new(std::collections::HashMap::new())),
            }
        }
    }

    #[async_trait]
    impl FlowCallbacks for MockCallbacks {
        async fn load_instance(&self, path: &str) -> FlowResult<FlowInstance> {
            let instances = self.instances.lock().unwrap();
            instances
                .get(path)
                .cloned()
                .ok_or_else(|| FlowError::NodeNotFound(path.to_string()))
        }

        async fn save_instance(&self, instance: &FlowInstance) -> FlowResult<()> {
            let mut instances = self.instances.lock().unwrap();
            let path = format!("/flows/instances/{}", instance.id);
            instances.insert(path, instance.clone());
            Ok(())
        }

        async fn save_instance_with_version(
            &self,
            instance: &FlowInstance,
            expected_version: i32,
        ) -> FlowResult<()> {
            let mut instances = self.instances.lock().unwrap();
            let path = format!("/flows/instances/{}", instance.id);

            if let Some(existing) = instances.get(&path) {
                if existing.flow_version != expected_version {
                    return Err(FlowError::VersionConflict);
                }
            }

            instances.insert(path, instance.clone());
            Ok(())
        }

        async fn create_node(
            &self,
            _node_type: &str,
            _path: &str,
            _properties: serde_json::Value,
        ) -> FlowResult<serde_json::Value> {
            Ok(json!({}))
        }

        async fn update_node(
            &self,
            _path: &str,
            _properties: serde_json::Value,
        ) -> FlowResult<serde_json::Value> {
            Ok(json!({}))
        }

        async fn get_node(&self, _path: &str) -> FlowResult<Option<serde_json::Value>> {
            Ok(Some(json!({
                "_version": 1,
                "workflow_data": {
                    "nodes": [
                        {"id": "start", "step_type": "start"},
                        {"id": "end", "step_type": "end"}
                    ],
                    "edges": []
                }
            })))
        }

        async fn queue_job(
            &self,
            _job_type: &str,
            _payload: serde_json::Value,
        ) -> FlowResult<String> {
            Ok("job-123".to_string())
        }

        async fn call_ai(
            &self,
            _agent_workspace: &str,
            _agent_ref: &str,
            _messages: Vec<serde_json::Value>,
            _response_format: Option<serde_json::Value>,
        ) -> FlowResult<serde_json::Value> {
            Ok(json!({}))
        }

        async fn execute_function(
            &self,
            _function_ref: &str,
            _input: serde_json::Value,
        ) -> FlowResult<serde_json::Value> {
            Ok(json!({}))
        }
    }

    #[tokio::test]
    async fn test_save_and_load_instance() {
        let callbacks = MockCallbacks::new();
        let instance = FlowInstance::new(
            "/flows/test-flow".to_string(),
            1,
            json!({"nodes": [], "edges": []}),
            json!({"test": "input"}),
            "start".to_string(),
        );

        // Save instance
        save_instance(&instance, &callbacks).await.unwrap();

        // Load instance
        let loaded = load_instance(&instance.id, &callbacks).await.unwrap();

        assert_eq!(loaded.id, instance.id);
        assert_eq!(loaded.flow_ref, instance.flow_ref);
        assert_eq!(loaded.status, FlowStatus::Pending);
    }

    #[tokio::test]
    async fn test_version_conflict() {
        let callbacks = MockCallbacks::new();
        let mut instance = FlowInstance::new(
            "/flows/test-flow".to_string(),
            1,
            json!({"nodes": [], "edges": []}),
            json!({}),
            "start".to_string(),
        );

        // Save initial version
        save_instance(&instance, &callbacks).await.unwrap();

        // Modify instance
        instance.status = FlowStatus::Running;
        save_instance(&instance, &callbacks).await.unwrap();

        // Try to save with old version (should fail)
        let result = save_instance_with_version(&instance, 0, &callbacks).await;
        assert!(matches!(result, Err(FlowError::VersionConflict)));
    }
}
