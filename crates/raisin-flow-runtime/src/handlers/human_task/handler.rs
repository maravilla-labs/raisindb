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

/// Priority assigned when a step does not specify one (1-5, 5 highest)
const DEFAULT_TASK_PRIORITY: i64 = 3;

/// Coerce a post-template-resolution property value to an integer.
///
/// Templates always resolve to JSON strings, so a resolved deadline arrives
/// as `"259200"` rather than `259200`; both forms are accepted. `Ok(None)`
/// means absent (or an unresolvable expression); `Err` carries the offending
/// value, which is an authoring error worth reporting rather than ignoring.
fn coerce_i64(value: Option<&Value>) -> Result<Option<i64>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        // A float truncates rather than failing (e.g. 1.5 -> 1)
        Some(Value::Number(n)) => n
            .as_i64()
            .or_else(|| n.as_f64().map(|f| f as i64))
            .map(Some)
            .ok_or_else(|| n.to_string()),
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            trimmed
                .parse::<i64>()
                .ok()
                .or_else(|| trimmed.parse::<f64>().ok().map(|f| f as i64))
                .map(Some)
                .ok_or_else(|| s.clone())
        }
        Some(other) => Err(other.to_string()),
    }
}

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

        // The named types (approval/input/review/action) are the ones the
        // runtime understands semantically; any other valid slug is an
        // application-defined type and is carried through verbatim.
        TaskType::parse(&task_type_str).ok_or_else(|| {
            FlowError::InvalidNodeConfiguration(format!(
                "Invalid task_type '{}' for human task step '{}': expected a \
                 slug of 1-64 chars matching [a-z][a-z0-9_-]*",
                task_type_str, step.id
            ))
        })
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
        task_type: &TaskType,
    ) -> FlowResult<Value> {
        let title = self.get_title(step)?;
        let assignee = self.get_assignee(step)?;
        let description = step.get_string("description");
        let options = self.get_options(step)?;
        let input_schema = step.get_property("input_schema").cloned();

        debug!(
            "Building task properties: type={}, title={}, assignee={}",
            task_type, title, assignee
        );

        // Build task properties JSON
        let mut task_props = serde_json::json!({
            "task_type": task_type.as_str(),
            "title": title,
            "assignee": assignee,
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

        // Arbitrary structured payload for the task UI (raisin:InboxTask v4
        // `data` slot). Lets a flow-created human task carry the same renderable
        // payload standalone raisin.tasks.create tasks do; template expressions
        // inside it are resolved with the rest of task_props below.
        if let Some(data) = step.get_property("data").cloned() {
            task_props["data"] = data;
        }

        // `due_in_seconds` and `priority` are inserted RAW (they may be
        // `${...}` / `{{...}}` expressions, e.g. a per-policy deadline read
        // from a node) and coerced to numbers only AFTER the mapper below
        // has resolved them. Reading them as numbers here instead would
        // silently drop any templated value - a template resolves to a
        // JSON string, and a string is not an i64.
        if let Some(due) = step.get_property("due_in_seconds").cloned() {
            task_props["due_in_seconds"] = due;
        }
        task_props["priority"] = step
            .get_property("priority")
            .cloned()
            .unwrap_or_else(|| Value::from(DEFAULT_TASK_PRIORITY));

        // Resolve template expressions (e.g. "Approve order {{ input.order_id }}")
        // in title, description, assignee, option labels, deadline, etc.
        let mut resolved = crate::runtime::DataMapper::map(&task_props, context)?;

        // Coerce the two numeric fields post-resolution. A template that
        // resolved to a numeric string ("259200") counts as a number here;
        // anything genuinely non-numeric is an authoring error and is
        // reported rather than silently ignored.
        let bad_value = |field: &str, raw: String| {
            FlowError::InvalidNodeConfiguration(format!(
                "Human task step '{}' property '{}' resolved to a non-numeric value: {}",
                step.id, field, raw
            ))
        };

        match coerce_i64(resolved.get("due_in_seconds"))
            .map_err(|raw| bad_value("due_in_seconds", raw))?
        {
            // An unresolvable deadline leaves the task WITHOUT one, which is
            // the safe outcome; a bogus deadline would not be.
            None => {
                if let Some(obj) = resolved.as_object_mut() {
                    obj.remove("due_in_seconds");
                }
            }
            Some(secs) => resolved["due_in_seconds"] = Value::from(secs),
        }

        let priority = coerce_i64(resolved.get("priority"))
            .map_err(|raw| bad_value("priority", raw))?
            .unwrap_or(DEFAULT_TASK_PRIORITY);
        resolved["priority"] = Value::from(priority.clamp(1, 5));

        Ok(resolved)
    }

    /// Generate the inbox task path for a human task wait.
    ///
    /// The path is scoped to the flow INSTANCE and the loop ITERATION, not
    /// just the step:
    ///
    ///   `{assignee}/inbox/task-{step_id}-{instance_slug}-it{iteration}`
    ///
    /// - `instance_slug` makes the path unique per flow run, so a new run
    ///   can never alias (and be satisfied by) an earlier run's task node.
    /// - `it{iteration}` (`__loop_index`, 0 outside loops) makes each loop
    ///   iteration of the same step id its own task.
    /// - The path is deterministic per (instance, step, iteration), so a
    ///   re-delivered execution of the same wait is idempotent; the create
    ///   path in `step_handler_impl` verifies any existing node at this
    ///   path actually belongs to this instance/step and is still pending
    ///   before reusing it.
    pub(super) fn generate_task_path(
        &self,
        assignee: &str,
        step_id: &str,
        context: &FlowContext,
    ) -> String {
        // Short path-safe slug of the instance id (UUIDs: first 8 hex chars)
        let mut instance_slug: String = context
            .instance_id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .take(8)
            .collect();
        if instance_slug.is_empty() {
            instance_slug = chrono::Utc::now().timestamp_millis().to_string();
        }

        // Loop iteration (set by the loop handler); 0 outside loops
        let iteration = context
            .variables
            .get("__loop_index")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        format!(
            "{}/inbox/task-{}-{}-it{}",
            assignee, step_id, instance_slug, iteration
        )
    }
}

impl Default for HumanTaskHandler {
    fn default() -> Self {
        Self::new()
    }
}
