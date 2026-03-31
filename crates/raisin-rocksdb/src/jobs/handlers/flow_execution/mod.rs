// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

// TODO(v0.2): Flow execution callbacks used by flow runtime
#![allow(dead_code)]

//! Flow execution job handler
//!
//! This module handles the orchestrated execution of multi-function flows
//! when a trigger with `function_flow` fires. It manages sequential and
//! parallel function execution, error handling, and result aggregation.
//!
//! Note: Flow types are defined locally to avoid circular dependency with raisin-functions.

mod handler;
mod step_execution;
mod types;

pub use types::*;

use raisin_storage::jobs::JobRegistry;
use std::sync::Arc;

use super::function_execution::{FunctionEnabledChecker, FunctionExecutorCallback};
use crate::jobs::data_store::JobDataStore;

/// Functions are always stored in the "functions" workspace
const FUNCTIONS_WORKSPACE: &str = "functions";

/// Handler for flow execution jobs
///
/// This handler processes FlowExecution jobs by orchestrating the execution
/// of multiple functions according to the flow definition. It handles:
/// - Sequential step execution (respecting dependencies)
/// - Parallel function execution within steps
/// - Error handling according to the flow's error strategy
/// - Result aggregation between steps
pub struct FlowExecutionHandler {
    /// Job registry for enqueueing function execution jobs
    #[allow(dead_code)]
    pub(super) job_registry: Arc<JobRegistry>,
    /// Job data store for storing job context
    #[allow(dead_code)]
    pub(super) job_data_store: Arc<JobDataStore>,
    /// Optional function executor callback (set by transport layer)
    pub(super) executor: Option<FunctionExecutorCallback>,
    /// Optional function enabled checker callback (set by transport layer)
    pub(super) enabled_checker: Option<FunctionEnabledChecker>,
}

impl FlowExecutionHandler {
    /// Create a new flow execution handler
    pub fn new(job_registry: Arc<JobRegistry>, job_data_store: Arc<JobDataStore>) -> Self {
        Self {
            job_registry,
            job_data_store,
            executor: None,
            enabled_checker: None,
        }
    }

    /// Set the function executor callback
    pub fn with_executor(mut self, executor: FunctionExecutorCallback) -> Self {
        self.executor = Some(executor);
        self
    }

    /// Set the function enabled checker callback
    pub fn with_enabled_checker(mut self, checker: FunctionEnabledChecker) -> Self {
        self.enabled_checker = Some(checker);
        self
    }
}
