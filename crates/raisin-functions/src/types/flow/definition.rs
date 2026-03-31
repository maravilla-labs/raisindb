// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Flow and step definition types

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::enums::{ErrorStrategy, FlowValidationError, StepCondition, StepErrorBehavior};

/// Function execution flow definition
///
/// A flow defines how multiple functions should be executed when a trigger fires.
/// Functions can be organized into steps that execute sequentially, with each step
/// optionally containing multiple functions that execute in parallel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionFlow {
    /// Schema version for future compatibility
    #[serde(default = "default_version")]
    pub version: u32,

    /// How to handle errors during flow execution
    #[serde(default)]
    pub error_strategy: ErrorStrategy,

    /// Overall timeout for the entire flow in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,

    /// Ordered list of execution steps
    pub steps: Vec<FlowStep>,
}

fn default_version() -> u32 {
    1
}

impl FunctionFlow {
    /// Create a new flow with default settings
    pub fn new() -> Self {
        Self {
            version: 1,
            error_strategy: ErrorStrategy::default(),
            timeout_ms: None,
            steps: Vec::new(),
        }
    }

    /// Add a step to the flow
    pub fn add_step(mut self, step: FlowStep) -> Self {
        self.steps.push(step);
        self
    }

    /// Set the error strategy
    pub fn with_error_strategy(mut self, strategy: ErrorStrategy) -> Self {
        self.error_strategy = strategy;
        self
    }

    /// Set the overall timeout
    pub fn with_timeout(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = Some(timeout_ms);
        self
    }

    /// Validate the flow structure
    ///
    /// Checks for:
    /// - At least one step
    /// - Valid step dependencies (no cycles, dependencies exist)
    /// - At least one function per step
    pub fn validate(&self) -> Result<(), FlowValidationError> {
        if self.steps.is_empty() {
            return Err(FlowValidationError::EmptyFlow);
        }

        let step_ids: std::collections::HashSet<_> = self.steps.iter().map(|s| &s.id).collect();

        for step in &self.steps {
            if step.functions.is_empty() {
                return Err(FlowValidationError::EmptyStep {
                    step_id: step.id.clone(),
                });
            }

            // Check that all dependencies exist
            for dep in &step.depends_on {
                if !step_ids.contains(dep) {
                    return Err(FlowValidationError::MissingDependency {
                        step_id: step.id.clone(),
                        dependency_id: dep.clone(),
                    });
                }
            }

            // Check for self-dependency
            if step.depends_on.contains(&step.id) {
                return Err(FlowValidationError::CyclicDependency {
                    step_id: step.id.clone(),
                });
            }
        }

        Ok(())
    }

    /// Get the execution order of steps using topological sort
    ///
    /// Returns steps in an order that respects dependencies.
    pub fn execution_order(&self) -> Result<Vec<&FlowStep>, FlowValidationError> {
        self.validate()?;

        let mut result = Vec::new();
        let mut visited = std::collections::HashSet::new();
        let step_map: HashMap<_, _> = self.steps.iter().map(|s| (&s.id, s)).collect();

        fn visit<'a>(
            step_id: &str,
            step_map: &HashMap<&String, &'a FlowStep>,
            visited: &mut std::collections::HashSet<String>,
            result: &mut Vec<&'a FlowStep>,
            visiting: &mut std::collections::HashSet<String>,
        ) -> Result<(), FlowValidationError> {
            if visiting.contains(step_id) {
                return Err(FlowValidationError::CyclicDependency {
                    step_id: step_id.to_string(),
                });
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

        let mut visiting = std::collections::HashSet::new();
        for step in &self.steps {
            visit(
                &step.id,
                &step_map,
                &mut visited,
                &mut result,
                &mut visiting,
            )?;
        }

        Ok(result)
    }
}

impl Default for FunctionFlow {
    fn default() -> Self {
        Self::new()
    }
}

/// A single step in the execution flow
///
/// A step groups functions that should execute together. If `parallel` is true,
/// all functions in the step execute concurrently. Otherwise, they execute sequentially.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowStep {
    /// Unique identifier for this step within the flow
    pub id: String,

    /// Human-readable name for the step
    pub name: String,

    /// Functions to execute in this step
    pub functions: Vec<FunctionRef>,

    /// Whether functions in this step should run in parallel
    #[serde(default)]
    pub parallel: bool,

    /// IDs of steps that must complete before this step
    #[serde(default)]
    pub depends_on: Vec<String>,

    /// Timeout for this step in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,

    /// What to do if this step fails
    #[serde(default)]
    pub on_error: StepErrorBehavior,

    /// Optional condition for executing this step
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<StepCondition>,
}

impl FlowStep {
    /// Create a new step with a single function
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        function_path: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            functions: vec![FunctionRef::new(function_path)],
            parallel: false,
            depends_on: Vec::new(),
            timeout_ms: None,
            on_error: StepErrorBehavior::default(),
            condition: None,
        }
    }

    /// Add a function to this step
    pub fn add_function(mut self, function: FunctionRef) -> Self {
        self.functions.push(function);
        self
    }

    /// Set parallel execution
    pub fn parallel(mut self) -> Self {
        self.parallel = true;
        self
    }

    /// Add a dependency on another step
    pub fn depends_on(mut self, step_id: impl Into<String>) -> Self {
        self.depends_on.push(step_id.into());
        self
    }

    /// Set the error behavior
    pub fn on_error(mut self, behavior: StepErrorBehavior) -> Self {
        self.on_error = behavior;
        self
    }

    /// Set a timeout for this step
    pub fn with_timeout(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = Some(timeout_ms);
        self
    }
}

/// Reference to a function to execute
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionRef {
    /// Path to the function node
    pub path: String,

    /// Optional timeout override for this specific function
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,

    /// Optional retry configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry: Option<RetryConfig>,

    /// Optional input mapping/transformation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_mapping: Option<serde_json::Value>,
}

impl FunctionRef {
    /// Create a new function reference
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            timeout_ms: None,
            retry: None,
            input_mapping: None,
        }
    }

    /// Set a timeout for this function
    pub fn with_timeout(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = Some(timeout_ms);
        self
    }

    /// Set retry configuration
    pub fn with_retry(mut self, retry: RetryConfig) -> Self {
        self.retry = Some(retry);
        self
    }
}

/// Retry configuration for a function
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    /// Maximum number of retry attempts
    #[serde(default = "default_max_attempts")]
    pub max_attempts: u32,

    /// Delay between retries in milliseconds
    #[serde(default = "default_retry_delay")]
    pub delay_ms: u64,

    /// Backoff multiplier (delay *= multiplier after each retry)
    #[serde(default = "default_backoff_multiplier")]
    pub backoff_multiplier: f64,
}

fn default_max_attempts() -> u32 {
    3
}
fn default_retry_delay() -> u64 {
    1000
}
fn default_backoff_multiplier() -> f64 {
    2.0
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: default_max_attempts(),
            delay_ms: default_retry_delay(),
            backoff_multiplier: default_backoff_multiplier(),
        }
    }
}
