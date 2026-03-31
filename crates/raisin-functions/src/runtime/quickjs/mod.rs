// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! QuickJS JavaScript runtime implementation.
//!
//! This module provides JavaScript function execution using the QuickJS engine
//! via the rquickjs crate. It exposes the RaisinDB API to JavaScript code.
//!
//! # Module Structure
//!
//! - `helpers` - JSON error formatting and async-to-sync bridging
//! - `module_loader` - ES6 module resolution and loading
//! - `console` - Console API, timer setup, and JS error formatting
//! - `environment` - JS environment and raisin API setup
//! - `api_nodes` - Node CRUD API registration
//! - `api_resources` - Resource, PDF, and temp file API registration
//! - `api_transaction` - Transaction API registration
//! - `api_admin` - Admin (RLS-bypass) API registration
//! - `api_fetch` - W3C Fetch API registration
//! - `api_misc` - SQL, HTTP, events, AI, functions, tasks, crypto API registration

mod api_admin;
mod api_fetch;
mod api_misc;
mod api_nodes;
mod api_resources;
mod api_transaction;
mod console;
mod environment;
mod helpers;
mod module_loader;

#[cfg(test)]
mod tests;

use async_trait::async_trait;
use raisin_error::{Error, Result};
use rquickjs::{AsyncContext, AsyncRuntime, CatchResultExt, Function, Module, Promise, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tracing::debug;

use super::FUNCTION_EXECUTION_SEMAPHORE;

use crate::api::FunctionApi;
use crate::types::{
    ExecutionContext, ExecutionError, ExecutionResult, ExecutionStats, FunctionLanguage,
    FunctionMetadata, LogEntry,
};

use super::FunctionRuntime;
use console::format_js_error;
use environment::setup_js_environment;
use module_loader::{has_es6_modules, FunctionModuleLoader, FunctionModuleResolver};

/// QuickJS-based JavaScript runtime.
///
/// Uses the rquickjs crate to execute JavaScript code in a sandboxed environment.
pub struct QuickJsRuntime {
    /// Shared async runtime
    runtime: AsyncRuntime,
}

impl QuickJsRuntime {
    /// Create a new QuickJS runtime.
    pub fn new() -> Self {
        let runtime = AsyncRuntime::new().expect("Failed to create QuickJS runtime");
        Self { runtime }
    }

    /// Set runtime limits (async).
    async fn configure_limits(&self, memory_bytes: usize, stack_bytes: usize) {
        self.runtime.set_memory_limit(memory_bytes).await;
        self.runtime.set_max_stack_size(stack_bytes).await;
    }
}

impl Default for QuickJsRuntime {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl FunctionRuntime for QuickJsRuntime {
    async fn execute(
        &self,
        code: &str,
        entrypoint: &str,
        context: ExecutionContext,
        metadata: &FunctionMetadata,
        api: Arc<dyn FunctionApi>,
        files: HashMap<String, String>,
    ) -> Result<ExecutionResult> {
        let start = Instant::now();
        let execution_id = context.execution_id.clone();
        let timeout_ms = metadata.resource_limits.timeout_ms;

        // Acquire semaphore permit to limit concurrent executions.
        // Timeout after 60s to avoid indefinite blocking when all slots are busy.
        let _permit = tokio::time::timeout(
            std::time::Duration::from_secs(60),
            FUNCTION_EXECUTION_SEMAPHORE.acquire(),
        )
        .await
        .map_err(|_| {
            Error::Internal(
                "Function execution timed out waiting for available slot (all execution slots busy)"
                    .to_string(),
            )
        })?
        .map_err(|_| Error::Internal("Function execution semaphore closed".to_string()))?;

        // Set memory and stack limits from metadata
        self.configure_limits(
            metadata.resource_limits.max_memory_bytes as usize,
            metadata.resource_limits.max_stack_bytes as usize,
        )
        .await;

        // Check if code uses ES6 modules
        let uses_modules = has_es6_modules(code);
        let files_arc = Arc::new(files);

        tracing::debug!(
            execution_id = %execution_id,
            entrypoint = %entrypoint,
            timeout_ms = %timeout_ms,
            language = "javascript",
            uses_modules = %uses_modules,
            file_count = %files_arc.len(),
            "Executing JavaScript function"
        );

        // Set up module loader if we have files or code uses imports
        if uses_modules || !files_arc.is_empty() {
            let resolver = FunctionModuleResolver::new(files_arc.clone());
            let loader = FunctionModuleLoader::new(files_arc.clone());
            self.runtime.set_loader(resolver, loader).await;
        }

        // Create log collector
        let logs = Arc::new(Mutex::new(Vec::<LogEntry>::new()));

        // Create context for this execution
        let ctx = AsyncContext::full(&self.runtime)
            .await
            .map_err(|e| Error::Internal(format!("Failed to create JS context: {}", e)))?;

        // Prepare input JSON
        let input_json = serde_json::to_string(&context.input).unwrap_or_else(|_| "{}".to_string());

        // Clone for closure
        let code = code.to_string();
        debug!("JavaScript function code: {}", code);
        let entrypoint = entrypoint.to_string();
        let logs_clone = logs.clone();
        let context_data = api.get_context();
        debug!("JavaScript function context data: {:?}", context_data);
        let api_clone = api.clone();
        let log_emitter = context.log_emitter.clone();

        // Execute with timeout
        let execution_future = async {
            ctx.with(|ctx| {
                // Setup environment with full API bindings
                if let Err(e) = setup_js_environment(
                    &ctx,
                    logs_clone.clone(),
                    &context_data,
                    api_clone.clone(),
                    log_emitter,
                ) {
                    tracing::error!(error = %e, "Failed to setup JS environment");
                    return Err(Error::Internal(format!(
                        "Failed to setup JS environment: {}",
                        e
                    )));
                }

                // Evaluate the function code and get the entrypoint
                let handler: Function = if uses_modules {
                    // Module mode: declare and evaluate as ES6 module
                    debug!("Evaluating as ES6 module");
                    let module_result: std::result::Result<Module, rquickjs::CaughtError> =
                        Module::declare(ctx.clone(), "entry", code.as_bytes()).catch(&ctx);

                    let module = match module_result {
                        Ok(m) => m,
                        Err(e) => {
                            let error_msg = format_js_error(&ctx, e);
                            tracing::error!(error = %error_msg, "JavaScript module syntax error");
                            return Err(Error::Validation(format!(
                                "JavaScript module syntax error: {}",
                                error_msg
                            )));
                        }
                    };

                    let eval_result = module.eval().catch(&ctx);

                    let (module, promise) = match eval_result {
                        Ok(pair) => pair,
                        Err(e) => {
                            let error_msg = format_js_error(&ctx, e);
                            tracing::error!(error = %error_msg, "JavaScript module eval error");
                            return Err(Error::Validation(format!(
                                "JavaScript module eval error: {}",
                                error_msg
                            )));
                        }
                    };

                    // Resolve top-level await in the module
                    if let Err(e) = promise.finish::<()>() {
                        let error_msg = format!("{}", e);
                        tracing::error!(error = %error_msg, "Module top-level await failed");
                        return Err(Error::Internal(format!(
                            "Module top-level await failed: {}",
                            error_msg
                        )));
                    }

                    // Get the entrypoint from module exports
                    let ns = module.namespace()
                        .map_err(|e| Error::Internal(format!(
                            "Failed to get module namespace: {}", e
                        )))?;

                    let handler_result: std::result::Result<Function, _> =
                        ns.get(&*entrypoint);

                    match handler_result {
                        Ok(f) => f,
                        Err(_) => {
                            return Err(Error::Validation(format!(
                                "Entrypoint function '{}' not found in module exports. \
                                 Make sure it is exported with: export {{ {} }}",
                                entrypoint, entrypoint
                            )));
                        }
                    }
                } else {
                    // Script mode: evaluate as classic script (existing behavior)
                    let code_bytes: Vec<u8> = code.into_bytes();
                    let eval_result: std::result::Result<(), rquickjs::CaughtError> =
                        ctx.eval(code_bytes).catch(&ctx);

                    debug!(
                        "JavaScript function code evaluated with result : {:?}",
                        eval_result
                    );

                    if let Err(e) = eval_result {
                        let error_msg = format_js_error(&ctx, e);
                        tracing::error!(error = %error_msg, "JavaScript syntax/eval error");
                        return Err(Error::Validation(format!(
                            "JavaScript syntax error: {}",
                            error_msg
                        )));
                    }

                    // Get the entrypoint function from globals
                    let globals = ctx.globals();
                    let handler: std::result::Result<Function, _> = globals.get(&*entrypoint);
                    match handler {
                        Ok(f) => f,
                        Err(_) => {
                            return Err(Error::Validation(format!(
                                "Entrypoint function '{}' not found",
                                entrypoint
                            )));
                        }
                    }
                };

                // Parse input
                let input_bytes: Vec<u8> = input_json.into_bytes();
                let input_val: Value = ctx
                    .json_parse(input_bytes)
                    .map_err(|e| Error::Validation(format!("Invalid input JSON: {}", e)))?;

                // Call the handler function
                let result: std::result::Result<Value, rquickjs::CaughtError> =
                    handler.call((input_val,)).catch(&ctx);

                match result {
                    Ok(value) => {
                        // Check if the result is a Promise and resolve it
                        let resolved_value = if value.is_promise() {
                            debug!("Handler returned a Promise, resolving...");
                            let promise: Promise = value
                                .into_promise()
                                .ok_or_else(|| {
                                    Error::Internal(
                                        "Failed to convert to Promise".to_string(),
                                    )
                                })?;

                            match promise.finish::<Value>() {
                                Ok(v) => {
                                    debug!("Promise resolved successfully");
                                    v
                                }
                                Err(e) => {
                                    let error_msg = if e.is_exception() {
                                        if let Some(caught) = ctx.catch().into_exception() {
                                            let msg = caught
                                                .message()
                                                .unwrap_or_else(|| "Unknown error".to_string());
                                            if let Some(stack) = caught.stack() {
                                                format!("{}\n{}", msg, stack)
                                            } else {
                                                msg
                                            }
                                        } else {
                                            let raw_msg = format!("{}", e);
                                            if raw_msg
                                                .contains("Exception generated by QuickJS")
                                            {
                                                "Promise rejected".to_string()
                                            } else {
                                                raw_msg
                                            }
                                        }
                                    } else {
                                        format!("{}", e)
                                    };
                                    tracing::error!(error = %error_msg, "JavaScript Promise rejection");
                                    return Err(Error::Internal(format!(
                                        "[JS] {}",
                                        error_msg
                                    )));
                                }
                            }
                        } else {
                            value
                        };

                        // Convert result to JSON
                        let result_str: String = match ctx.json_stringify(resolved_value) {
                            Ok(Some(s)) => {
                                s.to_string().unwrap_or_else(|_| "null".to_string())
                            }
                            Ok(None) => "null".to_string(),
                            Err(_) => "null".to_string(),
                        };

                        let output: serde_json::Value =
                            serde_json::from_str(&result_str).unwrap_or(serde_json::Value::Null);

                        Ok(output)
                    }
                    Err(e) => {
                        let error_msg = format_js_error(&ctx, e);
                        tracing::error!(error = %error_msg, "JavaScript handler execution failed");
                        Err(Error::Internal(format!("[JS] {}", error_msg)))
                    }
                }
            })
            .await
        };

        // Apply timeout
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms),
            execution_future,
        )
        .await;

        let duration_ms = start.elapsed().as_millis() as u64;

        // Collect logs
        let captured_logs = logs.lock().unwrap_or_else(|e| e.into_inner()).clone();

        let stats = ExecutionStats {
            duration_ms,
            memory_used_bytes: 0,
            instructions_executed: None,
            http_requests_made: 0,
            node_operations: 0,
            sql_queries: 0,
        };

        match result {
            Ok(Ok(output)) => {
                tracing::debug!(
                    execution_id = %execution_id,
                    output_is_null = output.is_null(),
                    output_type = %match &output {
                        serde_json::Value::Null => "null",
                        serde_json::Value::Bool(_) => "bool",
                        serde_json::Value::Number(_) => "number",
                        serde_json::Value::String(_) => "string",
                        serde_json::Value::Array(_) => "array",
                        serde_json::Value::Object(_) => "object",
                    },
                    duration_ms = duration_ms,
                    "QuickJS execution completed successfully"
                );
                Ok(ExecutionResult::success(execution_id, output, stats).with_logs(captured_logs))
            }
            Ok(Err(e)) => {
                tracing::debug!(
                    execution_id = %execution_id,
                    error = %e,
                    duration_ms = duration_ms,
                    "QuickJS execution failed with error"
                );
                Ok(ExecutionResult::failure(
                    execution_id,
                    ExecutionError::runtime(e.to_string()),
                    stats,
                )
                .with_logs(captured_logs))
            }
            Err(_) => {
                tracing::debug!(
                    execution_id = %execution_id,
                    timeout_ms = timeout_ms,
                    "QuickJS execution timed out"
                );
                Ok(ExecutionResult::failure(
                    execution_id,
                    ExecutionError::timeout(timeout_ms),
                    stats,
                )
                .with_logs(captured_logs))
            }
        }
    }

    fn validate(&self, code: &str) -> Result<()> {
        if code.trim().is_empty() {
            return Err(Error::Validation(
                "Function code cannot be empty".to_string(),
            ));
        }

        if !code.contains("function")
            && !code.contains("=>")
            && !code.contains("async")
            && !code.contains("const")
            && !code.contains("let")
        {
            tracing::warn!("Code may not contain a valid function definition");
        }

        Ok(())
    }

    fn language(&self) -> FunctionLanguage {
        FunctionLanguage::JavaScript
    }

    fn name(&self) -> &'static str {
        "QuickJS"
    }
}
