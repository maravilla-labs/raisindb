// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Human task handler struct and property extraction helpers

use crate::types::{FlowContext, FlowError, FlowNode, FlowResult, TaskOption, TaskType};
use serde_json::Value;
use tracing::{debug, instrument};

/// Wait reason for human tasks
pub(super) const WAIT_REASON_HUMAN_TASK: &str = "human_task";

/// Handler for human task steps
///
/// Creates inbox tasks for user interaction and pauses flow execution.
/// The flow resumes when the user completes the task.
#[derive(Debug)]
pub struct HumanTaskHandler;

impl HumanTaskHandler {
    /// Create a new human task handler
    pub fn new() -> Self {
        Self
    }

    /// Extract task type from step properties
    pub(super) fn get_task_type(&self, step: &FlowNode) -> FlowResult<TaskType> {
        let task_type_str = step.get_string("task_type").ok_or_else(|| {
            FlowError::MissingProperty(format!(
                "Human task step '{}' missing required property: task_type",
                step.id
            ))
        })?;

        match task_type_str.as_str() {
            "approval" => Ok(TaskType::Approval),
            "input" => Ok(TaskType::Input),
            "review" => Ok(TaskType::Review),
            "action" => Ok(TaskType::Action),
            _ => Err(FlowError::InvalidNodeConfiguration(format!(
                "Invalid task_type '{}' for human task step '{}'",
                task_type_str, step.id
            ))),
        }
    }

    /// Extract task title from step properties
    pub(super) fn get_title(&self, step: &FlowNode) -> FlowResult<String> {
        step.get_string("title").ok_or_else(|| {
            FlowError::MissingProperty(format!(
                "Human task step '{}' missing required property: title",
                step.id
            ))
        })
    }

    /// Extract assignee from step properties
    pub(super) fn get_assignee(&self, step: &FlowNode) -> FlowResult<String> {
        step.get_string("assignee").ok_or_else(|| {
            FlowError::MissingProperty(format!(
                "Human task step '{}' missing required property: assignee",
                step.id
            ))
        })
    }

    /// Extract task options from step properties
    pub(super) fn get_options(&self, step: &FlowNode) -> FlowResult<Vec<TaskOption>> {
        if let Some(options_arr) = step.get_array("options") {
            let mut options = Vec::new();

            for (idx, option_value) in options_arr.iter().enumerate() {
                let option_obj = option_value.as_object().ok_or_else(|| {
                    FlowError::InvalidNodeConfiguration(format!(
                        "Option {} in human task step '{}' is not an object",
                        idx, step.id
                    ))
                })?;

                let value = option_obj
                    .get("value")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        FlowError::InvalidNodeConfiguration(format!(
                            "Option {} in human task step '{}' missing value",
                            idx, step.id
                        ))
                    })?
                    .to_string();

                let label = option_obj
                    .get("label")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        FlowError::InvalidNodeConfiguration(format!(
                            "Option {} in human task step '{}' missing label",
                            idx, step.id
                        ))
                    })?
                    .to_string();

                let style = option_obj
                    .get("style")
                    .and_then(|v| v.as_str())
                    .map(String::from);

                options.push(TaskOption {
                    value,
                    label,
                    style,
                });
            }

            Ok(options)
        } else {
            // Options are optional, return empty vec
            Ok(Vec::new())
        }
    }

    /// Build task node properties
    #[instrument(skip(self, step, context))]
    pub(super) fn build_task_properties(
        &self,
        step: &FlowNode,
        context: &FlowContext,
        task_type: TaskType,
    ) -> FlowResult<Value> {
        let title = self.get_title(step)?;
        let assignee = self.get_assignee(step)?;
        let description = step.get_string("description");
        let options = self.get_options(step)?;
        let due_in_seconds = step.get_i64_property("due_in_seconds");
        let priority = step
            .get_u32_property("priority")
            .map(|v| v as u8)
            .unwrap_or(3);
        let input_schema = step.get_property("input_schema").cloned();

        debug!(
            "Building task properties: type={:?}, title={}, assignee={}",
            task_type, title, assignee
        );

        // Build task properties JSON
        let mut task_props = serde_json::json!({
            "task_type": match task_type {
                TaskType::Approval => "approval",
                TaskType::Input => "input",
                TaskType::Review => "review",
                TaskType::Action => "action",
            },
            "title": title,
            "assignee": assignee,
            "priority": priority,
            "status": "pending",
            "flow_instance_id": context.instance_id,
            "step_id": step.id,
        });

        // Add optional fields
        if let Some(desc) = description {
            task_props["description"] = Value::String(desc);
        }

        if !options.is_empty() {
            task_props["options"] = serde_json::to_value(&options).map_err(|e| {
                FlowError::InvalidNodeConfiguration(format!("Failed to serialize options: {}", e))
            })?;
        }

        if let Some(schema) = input_schema {
            task_props["input_schema"] = schema;
        }

        if let Some(due_seconds) = due_in_seconds {
            task_props["due_in_seconds"] = Value::Number(due_seconds.into());
        }

        Ok(task_props)
    }

    /// Generate inbox task path based on assignee
    pub(super) fn generate_task_path(&self, assignee: &str, step_id: &str) -> String {
        // Generate a path like: /users/manager/inbox/task-{step_id}-{timestamp}
        let timestamp = chrono::Utc::now().timestamp_millis();
        format!("{}/inbox/task-{}-{}", assignee, step_id, timestamp)
    }
}

impl Default for HumanTaskHandler {
    fn default() -> Self {
        Self::new()
    }
}
