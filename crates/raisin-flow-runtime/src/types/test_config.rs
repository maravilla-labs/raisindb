// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Test run configuration for workflow execution.
//!
//! Allows configuring how functions behave during test runs:
//! - `real`: Execute the actual function
//! - `passthrough`: Return input as output (no execution)
//! - `mock_output`: Return a predefined mock value
//!
//! AI agents always run with real behavior and cannot be mocked.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Test run configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestRunConfig {
    /// Whether this is a test run
    #[serde(default)]
    pub is_test_run: bool,

    /// Mock configuration for functions
    #[serde(default)]
    pub mock_functions: HashMap<String, FunctionMock>,

    /// Whether to run in an isolated branch
    #[serde(default)]
    pub isolated_branch: bool,

    /// Name of the isolated branch (if created)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch_name: Option<String>,

    /// Whether to auto-discard changes on completion
    #[serde(default = "default_true")]
    pub auto_discard: bool,
}

fn default_true() -> bool {
    true
}

impl Default for TestRunConfig {
    fn default() -> Self {
        Self {
            is_test_run: false,
            mock_functions: HashMap::new(),
            isolated_branch: false,
            branch_name: None,
            auto_discard: true,
        }
    }
}

/// Mock behavior for a function
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionMock {
    /// How the function should behave
    pub behavior: MockBehavior,

    /// Mock output value (for `mock_output` behavior)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mock_output: Option<Value>,

    /// Artificial delay in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mock_delay_ms: Option<u64>,
}

impl Default for FunctionMock {
    fn default() -> Self {
        Self {
            behavior: MockBehavior::Real,
            mock_output: None,
            mock_delay_ms: None,
        }
    }
}

/// Mock behavior options
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MockBehavior {
    /// Execute the actual function
    #[default]
    Real,

    /// Return input as output (no execution)
    Passthrough,

    /// Return a predefined mock value
    MockOutput,
}

impl TestRunConfig {
    /// Create a new test run configuration
    pub fn new() -> Self {
        Self {
            is_test_run: true,
            ..Default::default()
        }
    }

    /// Create a test run with isolated branch
    pub fn with_isolated_branch(mut self) -> Self {
        self.isolated_branch = true;
        self
    }

    /// Add a function mock
    pub fn with_mock(mut self, function_path: String, mock: FunctionMock) -> Self {
        self.mock_functions.insert(function_path, mock);
        self
    }

    /// Get mock configuration for a function
    pub fn get_mock(&self, function_path: &str) -> Option<&FunctionMock> {
        self.mock_functions.get(function_path)
    }

    /// Check if a function should be mocked
    pub fn should_mock(&self, function_path: &str) -> bool {
        if !self.is_test_run {
            return false;
        }
        self.mock_functions
            .get(function_path)
            .map(|m| m.behavior != MockBehavior::Real)
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = TestRunConfig::default();
        assert!(!config.is_test_run);
        assert!(!config.isolated_branch);
        assert!(config.auto_discard);
        assert!(config.mock_functions.is_empty());
    }

    #[test]
    fn test_with_mock() {
        let config = TestRunConfig::new().with_mock(
            "/lib/my-function".to_string(),
            FunctionMock {
                behavior: MockBehavior::Passthrough,
                mock_output: None,
                mock_delay_ms: Some(100),
            },
        );

        assert!(config.is_test_run);
        assert!(config.should_mock("/lib/my-function"));
        assert!(!config.should_mock("/lib/other-function"));
    }

    #[test]
    fn test_mock_behavior_serialization() {
        let mock = FunctionMock {
            behavior: MockBehavior::MockOutput,
            mock_output: Some(serde_json::json!({"result": "mocked"})),
            mock_delay_ms: None,
        };

        let json = serde_json::to_string(&mock).unwrap();
        let parsed: FunctionMock = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.behavior, MockBehavior::MockOutput);
        assert_eq!(
            parsed.mock_output,
            Some(serde_json::json!({"result": "mocked"}))
        );
    }
}
