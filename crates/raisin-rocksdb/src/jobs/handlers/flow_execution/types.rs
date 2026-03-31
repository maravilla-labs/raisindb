// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Flow type definitions (local mirrors of raisin_functions::types::flow)
//!
//! These are duplicated here to avoid circular dependency with raisin-functions.

use raisin_error::Error;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Function execution flow definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionFlow {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub error_strategy: ErrorStrategy,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    pub steps: Vec<FlowStep>,
}

fn default_version() -> u32 {
    1
}

impl FunctionFlow {
    /// Get the execution order of steps using topological sort
    pub fn execution_order(&self) -> raisin_error::Result<Vec<&FlowStep>> {
        if self.steps.is_empty() {
            return Err(Error::Validation("Flow must have at least one step".into()));
        }

        let step_ids: HashSet<_> = self.steps.iter().map(|s| &s.id).collect();

        // Validate dependencies exist
        for step in &self.steps {
            for dep in &step.depends_on {
                if !step_ids.contains(dep) {
                    return Err(Error::Validation(format!(
                        "Step '{}' depends on non-existent step '{}'",
                        step.id, dep
                    )));
                }
            }
        }

        // Simple topological sort
        let mut result = Vec::new();
        let mut visited = HashSet::new();
        let step_map: HashMap<_, _> = self.steps.iter().map(|s| (&s.id, s)).collect();

        fn visit<'a>(
            step_id: &str,
            step_map: &HashMap<&String, &'a FlowStep>,
            visited: &mut HashSet<String>,
            result: &mut Vec<&'a FlowStep>,
            visiting: &mut HashSet<String>,
        ) -> std::result::Result<(), String> {
            if visiting.contains(step_id) {
                return Err(format!("Cyclic dependency detected at step '{}'", step_id));
            }
            if visited.contains(step_id) {
                return Ok(());
            }

            visiting.insert(step_id.to_string());

            if let Some(step) = step_map.get(&step_id.to_string()) {
                for dep in &step.depends_on {
                    visit(dep, step_map, visited, result, visiting)?;
                }
                visited.insert(step_id.to_string());
                visiting.remove(step_id);
                result.push(step);
            }

            Ok(())
        }

        let mut visiting = HashSet::new();
        for step in &self.steps {
            visit(
                &step.id,
                &step_map,
                &mut visited,
                &mut result,
                &mut visiting,
            )
            .map_err(Error::Validation)?;
        }

        Ok(result)
    }
}

/// A single step in the execution flow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowStep {
    pub id: String,
    pub name: String,
    pub functions: Vec<FunctionRef>,
    #[serde(default)]
    pub parallel: bool,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub on_error: StepErrorBehavior,
}

/// Reference to a function to execute
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionRef {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

/// Strategy for handling errors during flow execution
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorStrategy {
    #[default]
    FailFast,
    Continue,
}

/// Behavior when a step encounters an error
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepErrorBehavior {
    #[default]
    Stop,
    Skip,
    Continue,
}

/// Result of executing a flow step
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    pub step_id: String,
    pub status: StepStatus,
    pub function_results: Vec<FunctionResult>,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Status of a step execution
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Completed,
    Failed,
    Skipped,
    Running,
    Pending,
}

/// Result of executing a single function
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionResult {
    pub function_path: String,
    pub execution_id: String,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub duration_ms: u64,
}

/// Overall flow execution result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowExecutionResult {
    pub flow_execution_id: String,
    pub trigger_path: String,
    pub status: FlowStatus,
    pub started_at: chrono::DateTime<chrono::Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub duration_ms: u64,
    pub step_results: Vec<StepResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_output: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Overall status of flow execution
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowStatus {
    Completed,
    PartialSuccess,
    Failed,
    TimedOut,
    Running,
    Pending,
}
