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

//! Flow instance creation and builder pattern
//!
//! Provides functions and a builder for creating `FlowInstance` values
//! from trigger events, including full tenant/repo/branch context.

use serde_json::Value;

use crate::types::{FlowDefinition, FlowInstance, TriggerEventType, TriggerInfo};
use crate::FlowError;

use super::events::FlowTriggerEvent;

/// Create a flow instance from a trigger event
///
/// This function creates a new FlowInstance ready to be saved and executed.
/// It sets up the instance with:
/// - A snapshot of the flow definition
/// - TriggerInfo containing event details
/// - The triggering node data as input
///
/// # Arguments
///
/// * `flow_ref` - Path to the flow definition node
/// * `flow_version` - Version of the flow definition
/// * `flow_definition` - Complete flow definition as JSON
/// * `trigger_event` - The event that triggered this flow
/// * `input` - The triggering node data (will be available as `context.input`)
///
/// # Returns
///
/// A new FlowInstance ready to be saved and executed
///
/// # Example
///
/// ```rust,ignore
/// let instance = create_flow_instance_from_trigger(
///     "/flows/my-flow".to_string(),
///     1,
///     flow_definition_json,
///     trigger_event,
///     triggering_node_data,
/// )?;
///
/// // Save instance and queue execution job
/// callbacks.save_instance(&instance).await?;
/// callbacks.queue_job("flow_execution", serde_json::json!({
///     "instance_id": instance.id,
/// })).await?;
/// ```
pub fn create_flow_instance_from_trigger(
    flow_ref: String,
    flow_version: i32,
    flow_definition: Value,
    trigger_event: &FlowTriggerEvent,
    input: Value,
) -> Result<FlowInstance, FlowError> {
    // Parse the flow definition to find the start node
    // Supports both runtime format (step_type) and designer format (node_type)
    let definition = FlowDefinition::from_workflow_data(flow_definition.clone())?;

    // Find the start node
    let start_node = definition
        .start_node()
        .ok_or_else(|| FlowError::InvalidDefinition("Flow has no start node".to_string()))?;

    // Create the instance
    // Note: The trigger info will be set later by the builder or by the caller
    // when creating the FlowContext with `FlowContext::with_trigger`
    let instance = FlowInstance::new(
        flow_ref,
        flow_version,
        flow_definition,
        input,
        start_node.id.clone(),
    );

    Ok(instance)
}

/// Build TriggerInfo from a FlowTriggerEvent
///
/// Extracts the relevant fields from the trigger event and creates
/// a TriggerInfo struct that will be attached to the flow context.
///
/// # Arguments
///
/// * `event` - The trigger event to convert
///
/// # Returns
///
/// TriggerInfo with event type and metadata
///
/// # Errors
///
/// Returns an error if the event doesn't contain required fields for trigger info
pub fn build_trigger_info_from_event(event: &FlowTriggerEvent) -> Result<TriggerInfo, FlowError> {
    match event {
        FlowTriggerEvent::NodeEvent {
            event_type,
            node_id,
            node_type,
            node_path,
            ..
        } => {
            // Map event type string to TriggerEventType
            let trigger_event_type = match event_type.as_str() {
                "Created" => TriggerEventType::Created,
                "Updated" => TriggerEventType::Updated,
                "Deleted" => TriggerEventType::Deleted,
                "Published" => TriggerEventType::Manual, // Map Published to Manual for now
                _ => TriggerEventType::Manual,
            };

            // For node events, we need additional context fields that aren't in the event
            // These will need to be provided by the caller or extracted from the node
            // For now, we create a minimal TriggerInfo that can be enhanced later
            Ok(TriggerInfo {
                event_type: trigger_event_type,
                node_id: node_id.clone(),
                node_type: node_type.clone(),
                workspace: String::new(), // Will be filled by caller
                node_path: Some(node_path.clone()),
                tenant_id: String::new(), // Will be filled by caller
                repo_id: String::new(),   // Will be filled by caller
                branch: String::new(),    // Will be filled by caller
            })
        }
        FlowTriggerEvent::ScheduledTime { schedule_id, .. } => Ok(TriggerInfo {
            event_type: TriggerEventType::Scheduled,
            node_id: schedule_id.clone(),
            node_type: "schedule".to_string(),
            workspace: String::new(),
            node_path: None,
            tenant_id: String::new(),
            repo_id: String::new(),
            branch: String::new(),
        }),
        FlowTriggerEvent::ToolResult {
            tool_call_id,
            tool_name,
            ..
        } => Ok(TriggerInfo {
            event_type: TriggerEventType::Resume,
            node_id: tool_call_id.clone(),
            node_type: tool_name.clone(),
            workspace: String::new(),
            node_path: None,
            tenant_id: String::new(),
            repo_id: String::new(),
            branch: String::new(),
        }),
        FlowTriggerEvent::HumanTaskCompleted {
            task_id, task_type, ..
        } => Ok(TriggerInfo {
            event_type: TriggerEventType::Resume,
            node_id: task_id.clone(),
            node_type: task_type.clone(),
            workspace: String::new(),
            node_path: None,
            tenant_id: String::new(),
            repo_id: String::new(),
            branch: String::new(),
        }),
        FlowTriggerEvent::CustomEvent { event_name, .. } => Ok(TriggerInfo {
            event_type: TriggerEventType::Webhook,
            node_id: event_name.clone(),
            node_type: "custom_event".to_string(),
            workspace: String::new(),
            node_path: None,
            tenant_id: String::new(),
            repo_id: String::new(),
            branch: String::new(),
        }),
        FlowTriggerEvent::Manual {
            actor, actor_home, ..
        } => Ok(TriggerInfo {
            event_type: TriggerEventType::Manual,
            node_id: actor.clone(),
            node_type: "manual".to_string(),
            workspace: String::new(),
            node_path: actor_home.clone(),
            tenant_id: String::new(),
            repo_id: String::new(),
            branch: String::new(),
        }),
    }
}

/// Builder for creating flow instances with full trigger context
///
/// Use this when you have all the context information (tenant, repo, branch, workspace).
/// This provides a more ergonomic API than calling `create_flow_instance_from_trigger`
/// and manually filling in the context fields.
pub struct FlowInstanceBuilder {
    flow_ref: String,
    flow_version: i32,
    flow_definition: Value,
    trigger_event: FlowTriggerEvent,
    input: Value,
    tenant_id: Option<String>,
    repo_id: Option<String>,
    branch: Option<String>,
    workspace: Option<String>,
    test_config: Option<crate::types::TestRunConfig>,
}

impl FlowInstanceBuilder {
    /// Create a new builder
    pub fn new(
        flow_ref: String,
        flow_version: i32,
        flow_definition: Value,
        trigger_event: FlowTriggerEvent,
        input: Value,
    ) -> Self {
        Self {
            flow_ref,
            flow_version,
            flow_definition,
            trigger_event,
            input,
            tenant_id: None,
            repo_id: None,
            branch: None,
            workspace: None,
            test_config: None,
        }
    }

    /// Set the tenant ID
    pub fn tenant_id(mut self, tenant_id: String) -> Self {
        self.tenant_id = Some(tenant_id);
        self
    }

    /// Set the repository ID
    pub fn repo_id(mut self, repo_id: String) -> Self {
        self.repo_id = Some(repo_id);
        self
    }

    /// Set the branch
    pub fn branch(mut self, branch: String) -> Self {
        self.branch = Some(branch);
        self
    }

    /// Set the workspace
    pub fn workspace(mut self, workspace: String) -> Self {
        self.workspace = Some(workspace);
        self
    }

    /// Set the test run configuration for testing workflows
    ///
    /// When set, the flow instance will be created as a test run with:
    /// - Function mocking support
    /// - Optional isolated branch execution
    /// - Auto-discard of changes on completion
    pub fn test_config(mut self, test_config: crate::types::TestRunConfig) -> Self {
        self.test_config = Some(test_config);
        self
    }

    /// Build the flow instance with complete trigger info
    pub fn build(self) -> Result<FlowInstance, FlowError> {
        let mut instance = create_flow_instance_from_trigger(
            self.flow_ref,
            self.flow_version,
            self.flow_definition,
            &self.trigger_event,
            self.input,
        )?;

        // Store the trigger context as metadata in the instance
        // We'll add this to the instance's variables for now
        if let Value::Object(ref mut vars) = instance.variables {
            let mut trigger_info = build_trigger_info_from_event(&self.trigger_event)?;

            // Fill in the context fields
            if let Some(tenant_id) = self.tenant_id {
                trigger_info.tenant_id = tenant_id;
            }
            if let Some(repo_id) = self.repo_id {
                trigger_info.repo_id = repo_id;
            }
            if let Some(branch) = self.branch {
                trigger_info.branch = branch;
            }
            if let Some(workspace) = self.workspace {
                trigger_info.workspace = workspace;
            }

            // Store trigger info as a variable
            vars.insert(
                "__trigger_info".to_string(),
                serde_json::to_value(trigger_info)
                    .map_err(|e| FlowError::Serialization(e.to_string()))?,
            );
        }

        // Apply test configuration if provided
        if let Some(test_config) = self.test_config {
            instance.test_config = Some(test_config);
        }

        Ok(instance)
    }
}
