// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! StarlarkRuntime struct and FunctionRuntime trait implementation

use async_trait::async_trait;
use raisin_error::{Error, Result};
use starlark::environment::{FrozenModule, Globals, GlobalsBuilder, LibraryExtension, Module};
use starlark::eval::{Evaluator, FileLoader};
use starlark::syntax::{AstModule, Dialect};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::Mutex;
use tokio::runtime::Handle;

use super::conversions::{json_to_starlark, starlark_value_to_json};
use super::gateway::raisin_gateway_module;
use super::setup_code::generate_setup_code;
use super::thread_local::{clear_logs, clear_thread_api, set_thread_api, take_logs};
use crate::api::FunctionApi;
use crate::runtime::FunctionRuntime;
use crate::types::{
    ExecutionContext, ExecutionError, ExecutionResult, ExecutionStats, FunctionLanguage,
    FunctionMetadata, LogEntry,
};

/// Starlark-based Python-like runtime
pub struct StarlarkRuntime {
    /// Tokio runtime handle for async operations
    handle: Option<Handle>,
    /// Pre-built globals with the gateway function
    globals: Globals,
}

impl StarlarkRuntime {
    /// Create a new Starlark runtime
    pub fn new() -> Self {
        // Try to get the current tokio runtime handle
        let handle = Handle::try_current().ok();

        // Build globals with the gateway module
        // Use extended_by to add struct support (needed for building the raisin namespace)
        let globals = GlobalsBuilder::extended_by(&[LibraryExtension::StructType])
            .with(raisin_gateway_module)
            .build();

        Self { handle, globals }
    }
}

/// Guard that tracks the current module evaluation stack for relative load() resolution.
struct ModuleStackGuard<'a> {
    stack: &'a Mutex<Vec<String>>,
}

impl<'a> ModuleStackGuard<'a> {
    fn push(stack: &'a Mutex<Vec<String>>, module_path: String) -> Self {
        if let Ok(mut stack_guard) = stack.lock() {
            stack_guard.push(module_path);
        }
        Self { stack }
    }
}

impl Drop for ModuleStackGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut stack_guard) = self.stack.lock() {
            stack_guard.pop();
        }
    }
}

/// File loader for Starlark load() statements backed by function sibling files.
struct StarlarkFunctionLoader<'a> {
    globals: &'a Globals,
    files: HashMap<String, String>,
    setup_code: String,
    cache: Mutex<HashMap<String, FrozenModule>>,
    in_progress: Mutex<HashSet<String>>,
    module_stack: Mutex<Vec<String>>,
}

impl<'a> StarlarkFunctionLoader<'a> {
    fn new(globals: &'a Globals, files: HashMap<String, String>, setup_code: String) -> Self {
        let mut normalized_files = HashMap::new();
        for (path, source) in files {
            let key = normalize_module_path(&path);
            normalized_files.insert(key, source);
        }
        Self {
            globals,
            files: normalized_files,
            setup_code,
            cache: Mutex::new(HashMap::new()),
            in_progress: Mutex::new(HashSet::new()),
            module_stack: Mutex::new(Vec::new()),
        }
    }

    fn with_entry_module(self, entry_module_path: &str) -> Self {
        let entry = normalize_module_path(entry_module_path);
        if !entry.is_empty() {
            if let Ok(mut stack_guard) = self.module_stack.lock() {
                stack_guard.push(entry);
            }
        }
        self
    }

    fn current_module_path(&self) -> Option<String> {
        self.module_stack
            .lock()
            .ok()
            .and_then(|stack_guard| stack_guard.last().cloned())
    }

    fn resolve_module_id(&self, requested: &str) -> String {
        let parent = self.current_module_path();
        resolve_module_path(parent.as_deref(), requested)
    }

    fn load_uncached(&self, module_id: &str) -> starlark::Result<FrozenModule> {
        let source = self.files.get(module_id).ok_or_else(|| {
            starlark::Error::new_other(anyhow::anyhow!(
                "Starlark load() module not found: {}",
                module_id
            ))
        })?;

        let full_code = format!("{}\n\n# User code\n{}", self.setup_code, source);
        let ast = AstModule::parse(module_id, full_code, &Dialect::Standard)?;

        let module = Module::new();
        let mut eval = Evaluator::new(&module);
        eval.set_loader(self);

        let _stack_guard = ModuleStackGuard::push(&self.module_stack, module_id.to_string());
        eval.eval_module(ast, self.globals)?;
        drop(eval);

        Ok(module.freeze()?)
    }
}

impl FileLoader for StarlarkFunctionLoader<'_> {
    fn load(&self, path: &str) -> starlark::Result<FrozenModule> {
        let module_id = self.resolve_module_id(path);

        if let Ok(cache_guard) = self.cache.lock() {
            if let Some(cached) = cache_guard.get(&module_id) {
                return Ok(cached.clone());
            }
        }

        {
            let mut in_progress_guard = self.in_progress.lock().map_err(|_| {
                starlark::Error::new_other(anyhow::anyhow!("Loader state poisoned"))
            })?;

            if in_progress_guard.contains(&module_id) {
                return Err(starlark::Error::new_other(anyhow::anyhow!(
                    "Cyclic starlark load() detected for module {}",
                    module_id
                )));
            }
            in_progress_guard.insert(module_id.clone());
        }

        let loaded = self.load_uncached(&module_id);

        if let Ok(mut in_progress_guard) = self.in_progress.lock() {
            in_progress_guard.remove(&module_id);
        }

        if let Ok(frozen) = &loaded {
            if let Ok(mut cache_guard) = self.cache.lock() {
                cache_guard.insert(module_id, frozen.clone());
            }
        }

        loaded
    }
}

fn normalize_module_path(path: &str) -> String {
    let mut parts = Vec::new();
    for part in path.replace('\\', "/").split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if !parts.is_empty() {
                    parts.pop();
                }
            }
            segment => parts.push(segment.to_string()),
        }
    }
    parts.join("/")
}

fn resolve_module_path(parent_module: Option<&str>, requested: &str) -> String {
    let requested_normalized = requested.replace('\\', "/");

    if requested_normalized.starts_with("./") || requested_normalized.starts_with("../") {
        if let Some(parent) = parent_module {
            let parent_dir = if let Some(idx) = parent.rfind('/') {
                &parent[..idx]
            } else {
                ""
            };
            let combined = if parent_dir.is_empty() {
                requested_normalized
            } else {
                format!("{}/{}", parent_dir, requested_normalized)
            };
            return normalize_module_path(&combined);
        }
    }

    normalize_module_path(&requested_normalized)
}

impl Default for StarlarkRuntime {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl FunctionRuntime for StarlarkRuntime {
    async fn execute(
        &self,
        code: &str,
        entrypoint: &str,
        context: ExecutionContext,
        metadata: &FunctionMetadata,
        api: Arc<dyn FunctionApi>,
        files: HashMap<String, String>,
    ) -> Result<ExecutionResult> {
        let start = std::time::Instant::now();
        let execution_id = context.execution_id.clone();
        let mut logs = Vec::new();

        // Acquire semaphore permit to limit concurrent executions and prevent
        // tokio worker thread exhaustion from block_in_place() calls.
        // Timeout after 60s to avoid indefinite blocking when all slots are busy.
        let _permit = tokio::time::timeout(
            std::time::Duration::from_secs(60),
            crate::runtime::FUNCTION_EXECUTION_SEMAPHORE.acquire(),
        )
        .await
        .map_err(|_| {
            Error::Internal(
                "Function execution timed out waiting for available slot (all execution slots busy)"
                    .to_string(),
            )
        })?
        .map_err(|_| Error::Internal("Function execution semaphore closed".to_string()))?;

        tracing::debug!(
            execution_id = %execution_id,
            entrypoint = %entrypoint,
            "Executing Starlark function"
        );

        // Get tokio runtime handle
        let handle = self
            .handle
            .clone()
            .or_else(|| Handle::try_current().ok())
            .ok_or_else(|| Error::Internal("No tokio runtime available".to_string()))?;

        // Set thread-local API, handle, and log emitter for the gateway function
        set_thread_api(api.clone(), handle.clone(), context.log_emitter.clone());

        // Clear any stale logs from previous executions
        clear_logs();

        // Ensure we clear thread-local on exit using a drop guard
        struct ClearApiOnDrop;
        impl Drop for ClearApiOnDrop {
            fn drop(&mut self) {
                clear_thread_api();
            }
        }
        let _guard = ClearApiOnDrop;

        // Create module and evaluator
        let module = Module::new();

        // Generate setup code that creates the raisin namespace with full API
        let setup_code = generate_setup_code(&context);

        // Combine setup code with user code
        let full_code = format!("{}\n\n# User code\n{}", setup_code, code);

        let entry_module_path = normalize_module_path(metadata.entry_file_path());

        // Parse the combined code
        let ast = AstModule::parse(&entry_module_path, full_code, &Dialect::Standard)
            .map_err(|e| Error::Validation(format!("Starlark parse error: {}", e)))?;

        // Set instruction limit based on metadata
        if let Some(max_instructions) = metadata.resource_limits.max_instructions {
            tracing::debug!(
                max_instructions = max_instructions,
                "Instruction limit set (not enforced in current Starlark version)"
            );
        }

        // Evaluate the module to define functions
        let loader = StarlarkFunctionLoader::new(&self.globals, files, setup_code)
            .with_entry_module(&entry_module_path);
        let mut eval = Evaluator::new(&module);
        eval.set_loader(&loader);

        eval.eval_module(ast, &self.globals).map_err(|e| {
            tracing::error!(
                execution_id = %execution_id,
                error = %e,
                "Starlark module eval failed"
            );
            Error::Internal(format!("Starlark eval error: {}", e))
        })?;

        // Get the handler function
        let handler_fn = module.get(entrypoint).ok_or_else(|| {
            Error::NotFound(format!(
                "Function '{}' not found in Starlark module",
                entrypoint
            ))
        })?;

        // Prepare input argument
        let heap = module.heap();
        let input_val = json_to_starlark(heap, &context.input);

        // Call the handler function
        let result = eval.eval_function(handler_fn, &[input_val], &[]);

        // Always capture logs from thread-local storage first (even on error)
        let captured_logs = take_logs();
        logs.extend(captured_logs);

        // Handle the result - use match to ensure logs are attached in ALL cases
        let duration_ms = start.elapsed().as_millis() as u64;
        let stats = ExecutionStats {
            duration_ms,
            ..Default::default()
        };

        match result {
            Ok(result) => {
                // Convert result to JSON
                let output = starlark_value_to_json(result);

                logs.push(LogEntry::info(format!(
                    "Function completed in {}ms",
                    duration_ms
                )));

                Ok(ExecutionResult::success(execution_id, output, stats).with_logs(logs))
            }
            Err(e) => {
                tracing::error!(
                    execution_id = %execution_id,
                    error = %e,
                    "Starlark handler execution failed"
                );
                logs.push(LogEntry::error(format!("Execution error: {}", e)));

                // Return failure with logs attached (key fix: logs are preserved on failure)
                Ok(ExecutionResult::failure(
                    execution_id,
                    ExecutionError::runtime(e.to_string()),
                    stats,
                )
                .with_logs(logs))
            }
        }
    }

    fn validate(&self, code: &str) -> Result<()> {
        if code.trim().is_empty() {
            return Err(Error::Validation(
                "Function code cannot be empty".to_string(),
            ));
        }

        // Parse to check for syntax errors
        AstModule::parse("validate.star", code.to_string(), &Dialect::Standard)
            .map_err(|e| Error::Validation(format!("Starlark syntax error: {}", e)))?;

        // Basic check for function definition
        if !code.contains("def ") {
            return Err(Error::Validation(
                "Starlark code must contain a function definition (def)".to_string(),
            ));
        }

        Ok(())
    }

    fn language(&self) -> FunctionLanguage {
        FunctionLanguage::Starlark
    }

    fn name(&self) -> &'static str {
        "Starlark"
    }
}
