// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Function execution management

use std::sync::Arc;

use raisin_error::Result;

use crate::api::FunctionApi;
use crate::runtime::RuntimeRegistry;
use crate::types::{ExecutionContext, ExecutionResult, LoadedFunction};

/// Handle to a running or completed function execution
#[derive(Debug, Clone)]
pub struct ExecutionHandle {
    /// Execution ID
    pub execution_id: String,
    /// Whether execution is complete
    pub complete: bool,
}

/// Function executor
///
/// Manages the execution of user-defined functions using the appropriate runtime.
pub struct FunctionExecutor {
    /// Runtime registry
    runtimes: RuntimeRegistry,
}

impl FunctionExecutor {
    /// Create a new function executor
    pub fn new() -> Self {
        Self {
            runtimes: RuntimeRegistry::new(),
        }
    }

    /// Create with custom runtime registry
    pub fn with_runtimes(runtimes: RuntimeRegistry) -> Self {
        Self { runtimes }
    }

    /// Execute a function synchronously
    ///
    /// This blocks until the function completes or times out.
    /// Use for WebAPI endpoints that need immediate response.
    pub async fn execute(
        &self,
        function: &LoadedFunction,
        context: ExecutionContext,
        api: Arc<dyn FunctionApi>,
    ) -> Result<ExecutionResult> {
        let runtime = self
            .runtimes
            .get(function.metadata.language)
            .ok_or_else(|| {
                raisin_error::Error::Validation(format!(
                    "No runtime available for language: {}",
                    function.metadata.language
                ))
            })?;

        tracing::info!(
            execution_id = %context.execution_id,
            function_name = %function.metadata.name,
            language = %function.metadata.language,
            runtime = %runtime.name(),
            "Executing function"
        );

        runtime
            .execute(
                &function.code,
                function.metadata.entry_function_name(),
                context,
                &function.metadata,
                api,
                function.files.clone(),
            )
            .await
    }

    /// Validate function code without executing
    pub fn validate(&self, function: &LoadedFunction) -> Result<()> {
        let runtime = self
            .runtimes
            .get(function.metadata.language)
            .ok_or_else(|| {
                raisin_error::Error::Validation(format!(
                    "No runtime available for language: {}",
                    function.metadata.language
                ))
            })?;

        runtime.validate(&function.code)
    }

    /// Get available languages
    pub fn available_languages(&self) -> Vec<crate::types::FunctionLanguage> {
        self.runtimes.available_languages()
    }
}

impl Default for FunctionExecutor {
    fn default() -> Self {
        Self::new()
    }
}
