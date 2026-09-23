// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! `FunctionDomainReducer`: an AgentRun domain reducer that is an ORDINARY
//! RaisinDB function, in any supported language.
//!
//! There is one function contract for every runtime (`input -> output`), and a
//! reducer is simply a function whose input is a `raisin.agent-run.reducer/1`
//! request and whose output is the response. Nothing here knows or cares
//! whether the function is JavaScript, Starlark or a WebAssembly component:
//!
//! - **Pure by policy, not by language.** Every call runs under
//!   [`ExecutionPolicy::Deterministic`], which every runtime enforces: no host
//!   calls, a clock that reads the epoch, fixed entropy.
//! - **Inline, never a queued job.** The driver calls it synchronously inside
//!   its step; a job hop costs about a second, a reducer call must not.
//! - **Pinned.** The run binds a hash of the function's artifact (the component
//!   bytes, or the source plus its module set). If a later call resolves a
//!   different artifact it answers [`ReducerCallError::Changed`] and the run
//!   pauses with `reducer_changed` instead of silently switching reducers.
//! - **Validated.** Every response goes through `validate_response` before core
//!   sees it. A timeout, a trap or a missing function is `Unavailable` (the run
//!   pauses, resumable after a redeploy); the function's own error or a
//!   contract violation is deterministic and fails the run.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use async_trait::async_trait;
use raisin_agent_runtime::contract::{validate_response, ReducerRequest, ReducerResponse, Refusal};
use raisin_agent_runtime::domain::{artifact_hash, DomainReducer, ReducerCallError, ReducerRef};
use serde_json::Value;

use crate::api::{FunctionApi, MockFunctionApi};
use crate::runtime::FunctionRuntime;
use crate::types::{
    ExecutionContext, ExecutionPolicy, ExecutionResult, FunctionCode, FunctionMetadata,
};

/// One deterministic invocation: which artifact ran, and what it returned.
#[derive(Debug, Clone)]
pub struct DeterministicRun {
    /// Hash of the artifact that ran (see [`function_artifact_hash`]).
    pub artifact_hash: String,
    /// The runtime's result.
    pub result: ExecutionResult,
}

/// Runs a function under [`ExecutionPolicy::Deterministic`]. Implemented over
/// storage for the server, and over a runtime + source for embedding/tests.
#[async_trait]
pub trait DeterministicInvoker: Send + Sync {
    /// The CURRENT artifact hash of the function at `function_path`.
    async fn artifact_hash(&self, function_path: &str) -> Result<String, String>;

    /// Execute it on `input`. `Err` is a transport failure (not found, cannot
    /// load, runtime unavailable) — never the function's own error.
    async fn invoke(&self, function_path: &str, input: Value) -> Result<DeterministicRun, String>;
}

/// The hash a run pins: language, entry, artifact, and the module set in
/// name order — so a changed import changes the pin too.
pub fn function_artifact_hash(
    metadata: &FunctionMetadata,
    code: &FunctionCode,
    files: &HashMap<String, String>,
) -> String {
    let language = metadata.language.to_string();
    let bytes: &[u8] = match code {
        FunctionCode::Text(t) => t.as_bytes(),
        FunctionCode::Bytes(b) => b,
    };
    let sorted: BTreeMap<&String, &String> = files.iter().collect();
    let mut parts: Vec<&[u8]> = vec![language.as_bytes(), metadata.entry_file.as_bytes(), bytes];
    for (name, body) in &sorted {
        parts.push(name.as_bytes());
        parts.push(body.as_bytes());
    }
    artifact_hash(&parts)
}

/// Whether a failed execution is a transport failure (retry after a redeploy)
/// rather than the function's own deterministic error.
fn is_unavailable(result: &ExecutionResult) -> bool {
    let Some(err) = &result.error else {
        return false;
    };
    matches!(
        err.code.as_str(),
        "TIMEOUT" | "MEMORY_LIMIT" | "STACK_OVERFLOW"
    ) || err.message.starts_with("wasm trap:")
        || err.message.starts_with("wasm execution failed:")
}

/// A domain reducer backed by a function, in any runtime.
pub struct FunctionDomainReducer {
    reducer: ReducerRef,
    invoker: Arc<dyn DeterministicInvoker>,
}

impl FunctionDomainReducer {
    /// A reducer pinned to `reducer.artifact_hash`.
    pub fn new(reducer: ReducerRef, invoker: Arc<dyn DeterministicInvoker>) -> Self {
        Self { reducer, invoker }
    }

    /// Bind to whatever `function_path` resolves to NOW, pinning its hash.
    pub async fn bind(
        function_path: &str,
        handler: &str,
        invoker: Arc<dyn DeterministicInvoker>,
    ) -> Result<Self, ReducerCallError> {
        let hash = invoker
            .artifact_hash(function_path)
            .await
            .map_err(ReducerCallError::Unavailable)?;
        let reducer = ReducerRef {
            function_path: function_path.into(),
            handler: handler.into(),
            artifact_hash: hash,
        };
        Ok(Self::new(reducer, invoker))
    }
}

#[async_trait]
impl DomainReducer for FunctionDomainReducer {
    fn reducer_ref(&self) -> &ReducerRef {
        &self.reducer
    }

    async fn reduce(&self, req: &ReducerRequest) -> Result<ReducerResponse, ReducerCallError> {
        let input = serde_json::to_value(req)
            .map_err(|e| ReducerCallError::Unavailable(format!("request not serializable: {e}")))?;
        let run = self
            .invoker
            .invoke(&self.reducer.function_path, input)
            .await
            .map_err(ReducerCallError::Unavailable)?;
        // The call was pure, so running it before comparing is harmless; its
        // answer is simply discarded when the artifact moved.
        if run.artifact_hash != self.reducer.artifact_hash {
            return Err(ReducerCallError::Changed {
                actual_hash: run.artifact_hash,
            });
        }
        let result = run.result;
        if !result.success {
            let message = result
                .error
                .as_ref()
                .map(|e| e.message.clone())
                .unwrap_or_else(|| "function failed".into());
            return Err(if is_unavailable(&result) {
                ReducerCallError::Unavailable(message)
            } else {
                // The function's own error: deterministic, so it fails the run.
                ReducerCallError::Refused {
                    code: "handler_error".into(),
                    message,
                }
            });
        }
        let output = result.output.unwrap_or(Value::Null);
        let resp: ReducerResponse = serde_json::from_value(output).map_err(|e| {
            ReducerCallError::Invalid(Refusal::new("response_undecodable", e.to_string()))
        })?;
        if let Some(r) = &resp.refused {
            return Err(ReducerCallError::Refused {
                code: r.code.clone(),
                message: r.message.clone(),
            });
        }
        validate_response(req, &resp).map_err(ReducerCallError::Invalid)?;
        Ok(resp)
    }
}

/// Runs ONE function artifact directly on a runtime, with no storage: for
/// embedding a reducer whose source the caller already holds, and for tests.
pub struct DirectRuntimeInvoker {
    runtime: Arc<dyn FunctionRuntime>,
    metadata: FunctionMetadata,
    code: FunctionCode,
    files: HashMap<String, String>,
}

impl DirectRuntimeInvoker {
    /// An invoker running `code` (entry `metadata.entry_file`) on `runtime`.
    pub fn new(
        runtime: Arc<dyn FunctionRuntime>,
        metadata: FunctionMetadata,
        code: FunctionCode,
        files: HashMap<String, String>,
    ) -> Self {
        Self {
            runtime,
            metadata,
            code,
            files,
        }
    }
}

#[async_trait]
impl DeterministicInvoker for DirectRuntimeInvoker {
    async fn artifact_hash(&self, _function_path: &str) -> Result<String, String> {
        Ok(function_artifact_hash(
            &self.metadata,
            &self.code,
            &self.files,
        ))
    }

    async fn invoke(&self, _function_path: &str, input: Value) -> Result<DeterministicRun, String> {
        let context = ExecutionContext::new("reducer", "reducer", "main", "reducer")
            .with_input(input)
            .with_policy(ExecutionPolicy::Deterministic);
        // Nothing real is reachable even in principle: the API is a mock, and
        // the policy refuses every host call before it would reach it.
        let api: Arc<dyn FunctionApi> = Arc::new(MockFunctionApi::new(serde_json::json!({})));
        let result = self
            .runtime
            .execute(
                &self.code,
                self.metadata.entry_function_name(),
                context,
                &self.metadata,
                api,
                self.files.clone(),
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(DeterministicRun {
            artifact_hash: function_artifact_hash(&self.metadata, &self.code, &self.files),
            result,
        })
    }
}

#[cfg(test)]
#[path = "agent_reducer_tests.rs"]
mod tests;
