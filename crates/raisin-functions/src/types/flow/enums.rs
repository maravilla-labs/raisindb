// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Flow enumeration types, conditions, and validation errors

use serde::{Deserialize, Serialize};

/// Strategy for handling errors during flow execution
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorStrategy {
    /// Stop execution immediately on first error
    #[default]
    FailFast,

    /// Continue executing remaining steps even if some fail
    Continue,
}

/// Behavior when a step encounters an error
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepErrorBehavior {
    /// Stop the entire flow
    #[default]
    Stop,

    /// Skip this step and continue with the next
    Skip,

    /// Continue executing functions in this step despite errors
    Continue,
}

/// Condition for executing a step
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepCondition {
    /// Type of condition
    #[serde(rename = "type")]
    pub condition_type: ConditionType,

    /// Expression for expression-based conditions
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
}

/// Types of step execution conditions
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConditionType {
    /// All dependent steps must succeed
    AllSuccess,

    /// At least one dependent step must succeed
    AnySuccess,

    /// Evaluate a custom expression
    Expression,
}

/// Validation errors for function flows
#[derive(Debug, Clone, thiserror::Error)]
pub enum FlowValidationError {
    #[error("Flow must have at least one step")]
    EmptyFlow,

    #[error("Step '{step_id}' must have at least one function")]
    EmptyStep { step_id: String },

    #[error("Step '{step_id}' depends on non-existent step '{dependency_id}'")]
    MissingDependency {
        step_id: String,
        dependency_id: String,
    },

    #[error("Cyclic dependency detected at step '{step_id}'")]
    CyclicDependency { step_id: String },
}

/// Overall status of flow execution
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowStatus {
    /// Flow completed successfully
    Completed,
    /// Flow completed but some steps failed (with continue strategy)
    PartialSuccess,
    /// Flow failed
    Failed,
    /// Flow timed out
    TimedOut,
    /// Flow is currently running
    Running,
    /// Flow is pending execution
    Pending,
}

/// Status of a step execution
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    /// Step completed successfully
    Completed,
    /// Step failed
    Failed,
    /// Step was skipped due to condition or previous failure
    Skipped,
    /// Step is currently running
    Running,
    /// Step is pending execution
    Pending,
}
