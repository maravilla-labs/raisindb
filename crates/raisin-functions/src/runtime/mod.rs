// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Function runtime implementations
//!
//! This module provides the runtime engines for executing user-defined functions
//! in different languages (JavaScript, Starlark, SQL).

pub mod bindings;
pub mod fetch;
mod quickjs;
mod sandbox;
mod sql;
mod starlark;
pub mod temp;
pub mod timers;

pub use quickjs::QuickJsRuntime;
pub use sandbox::{Sandbox, SandboxConfig};
pub use sql::SqlRuntime;
pub use starlark::StarlarkRuntime;

use crate::api::FunctionApi;
use crate::types::{ExecutionContext, ExecutionResult, FunctionLanguage, FunctionMetadata};
use async_trait::async_trait;
use raisin_error::Result;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use tokio::sync::Semaphore;

/// Shared semaphore limiting concurrent function executions across all runtimes.
/// Each execution uses `block_in_place()` which pins a tokio worker thread.
/// Configurable via `RAISIN_MAX_CONCURRENT_FUNCTIONS` env var (default: 15).
pub(crate) static FUNCTION_EXECUTION_SEMAPHORE: LazyLock<Semaphore> = LazyLock::new(|| {
    let max_concurrent = std::env::var("RAISIN_MAX_CONCURRENT_FUNCTIONS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(15);
    tracing::info!(
        max_concurrent,
        "Function execution concurrency limit initialized"
    );
    Semaphore::new(max_concurrent)
});

/// Trait for function runtime engines
///
/// Each runtime implements execution for a specific language (JavaScript, Starlark, SQL).
#[async_trait]
pub trait FunctionRuntime: Send + Sync {
    /// Execute a function with the given code and context
    ///
    /// # Arguments
    /// * `code` - The function source code (entry file content)
    /// * `entrypoint` - The name of the function to call (e.g., "handler")
    /// * `context` - Execution context with input, tenant, etc.
    /// * `metadata` - Function metadata (resource limits, network policy)
    /// * `api` - API implementation for node/SQL/HTTP operations
    /// * `files` - All function files for module resolution (path -> content)
    ///
    /// # Returns
    /// Execution result with output, stats, and any errors
    async fn execute(
        &self,
        code: &str,
        entrypoint: &str,
        context: ExecutionContext,
        metadata: &FunctionMetadata,
        api: Arc<dyn FunctionApi>,
        files: HashMap<String, String>,
    ) -> Result<ExecutionResult>;

    /// Validate function code syntax without executing
    ///
    /// Used when creating/updating functions to catch syntax errors early.
    fn validate(&self, code: &str) -> Result<()>;

    /// Get the language this runtime supports
    fn language(&self) -> FunctionLanguage;

    /// Get runtime name for logging/debugging
    fn name(&self) -> &'static str;
}

/// Registry of available function runtimes
pub struct RuntimeRegistry {
    runtimes: HashMap<FunctionLanguage, Arc<dyn FunctionRuntime>>,
}

impl RuntimeRegistry {
    /// Create a new runtime registry with default runtimes
    pub fn new() -> Self {
        let mut runtimes: HashMap<FunctionLanguage, Arc<dyn FunctionRuntime>> = HashMap::new();

        // Register QuickJS for JavaScript
        runtimes.insert(
            FunctionLanguage::JavaScript,
            Arc::new(QuickJsRuntime::new()),
        );

        // Register Starlark for Python-like (placeholder)
        runtimes.insert(FunctionLanguage::Starlark, Arc::new(StarlarkRuntime::new()));

        // Register SQL passthrough
        runtimes.insert(FunctionLanguage::Sql, Arc::new(SqlRuntime::new()));

        Self { runtimes }
    }

    /// Get runtime for a specific language
    pub fn get(&self, language: FunctionLanguage) -> Option<Arc<dyn FunctionRuntime>> {
        self.runtimes.get(&language).cloned()
    }

    /// Register a custom runtime
    pub fn register(&mut self, runtime: Arc<dyn FunctionRuntime>) {
        self.runtimes.insert(runtime.language(), runtime);
    }

    /// List available languages
    pub fn available_languages(&self) -> Vec<FunctionLanguage> {
        self.runtimes.keys().cloned().collect()
    }
}

impl Default for RuntimeRegistry {
    fn default() -> Self {
        Self::new()
    }
}
