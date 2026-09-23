// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Operation executors: every operation of a run is an ordinary RaisinDB
//! function call, executed AS THE RUN'S PRINCIPAL.
//!
//! - **Tool call** → the function the operation names (`input.tool`), with the
//!   operation's args, through the generic function executor — any language.
//! - **Model turn** → a [`ModelTurnExecutor`]. The default one is itself a
//!   function (`executor_config.model_turn_function`, else the server default),
//!   so the agent harness (`ai-tools`) implements model turns as a function and
//!   core stays free of any provider knowledge.
//!
//! The auth context always comes from [`resolve_principal_auth`], which fails
//! closed: an operation whose principal cannot be resolved FAILS, it never
//! runs with system rights.

use std::sync::Arc;

use async_trait::async_trait;
use raisin_agent_runtime::driver::{ExecContext, OperationExecutor};
use raisin_agent_runtime::events::{OpOutcome, OpUsage};
use raisin_agent_runtime::ids::CallId;
use raisin_agent_runtime::lifecycle::OperationResult;
use raisin_agent_runtime::record::OperationKind;
use raisin_models::auth::AuthContext;
use serde_json::{json, Value};

use super::principal_auth::resolve_principal_auth;
use crate::FunctionExecutorCallback;
use crate::RocksDBStorage;

/// The function a model turn runs when the run does not name one.
pub const DEFAULT_MODEL_TURN_FUNCTION: &str = "/lib/raisin/ai/agent-run-model-turn";

/// The seam the agent harness implements: one model turn of a run.
///
/// Input is the reducer's `request_model_turn` effect body (tools offered,
/// instructions, tool results, output schema). The payload of a successful
/// result is `{ message: {text?}, tool_calls: [{call_id, name, args}],
/// finish_reason, usage: {input_tokens, output_tokens} }`; its `tool_calls`
/// ids must also be returned in `OperationResult::tool_calls`.
#[async_trait]
pub trait ModelTurnExecutor: Send + Sync {
    /// Run one turn under `auth`.
    async fn run_turn(
        &self,
        ctx: &ExecContext,
        auth: AuthContext,
        request: Value,
    ) -> OperationResult;
}

fn failed(outcome: OpOutcome, class: &str, message: impl Into<String>) -> OperationResult {
    OperationResult {
        outcome: Some(outcome),
        payload: Some(json!({ "error_class": class, "message": message.into() })),
        ..OperationResult::default()
    }
}

/// Calls functions for a run.
#[derive(Clone)]
pub struct FunctionCaller {
    functions: FunctionExecutorCallback,
}

impl FunctionCaller {
    /// Over the server's function executor.
    pub fn new(functions: FunctionExecutorCallback) -> Self {
        Self { functions }
    }

    /// Run `path` on `input` under `auth`; `Err` is a failure message.
    pub async fn call(
        &self,
        ctx: &ExecContext,
        auth: AuthContext,
        path: &str,
        input: Value,
    ) -> Result<Value, String> {
        let execution_id = format!("{}#{}", ctx.op.op_id, ctx.op.attempt);
        let result = (self.functions)(
            path.to_string(),
            execution_id,
            input,
            ctx.scope.tenant_id.clone(),
            ctx.scope.repo_id.clone(),
            ctx.scope.branch.clone(),
            "functions".to_string(),
            Some(auth),
            None,
        )
        .await
        .map_err(|e| e.to_string())?;
        if result.success {
            Ok(result.result.unwrap_or(Value::Null))
        } else {
            Err(result.error.unwrap_or_else(|| "function failed".into()))
        }
    }
}

/// The default model-turn executor: a function implements the turn.
pub struct FunctionModelTurnExecutor {
    caller: FunctionCaller,
    default_function: String,
}

impl FunctionModelTurnExecutor {
    /// Model turns run `default_function` unless the run names another.
    pub fn new(caller: FunctionCaller, default_function: impl Into<String>) -> Self {
        Self {
            caller,
            default_function: default_function.into(),
        }
    }
}

#[async_trait]
impl ModelTurnExecutor for FunctionModelTurnExecutor {
    async fn run_turn(
        &self,
        ctx: &ExecContext,
        auth: AuthContext,
        request: Value,
    ) -> OperationResult {
        let path = ctx
            .executor_config
            .as_ref()
            .and_then(|c| c.get("model_turn_function"))
            .and_then(Value::as_str)
            .unwrap_or(&self.default_function)
            .to_string();
        let input = json!({
            "run_id": ctx.run_id, "operation_id": ctx.op.op_id, "attempt": ctx.op.attempt,
            "agent_ref": ctx.agent_ref, "subject": ctx.subject,
            "executor_config": ctx.executor_config, "request": request,
        });
        match self.caller.call(ctx, auth, &path, input).await {
            Ok(output) => model_turn_result(output),
            Err(message) => failed(OpOutcome::Retryable, "provider", message),
        }
    }
}

/// Normalize a model-turn function's output into an operation result.
pub fn model_turn_result(output: Value) -> OperationResult {
    if let Some(class) = output.get("error_class").and_then(Value::as_str) {
        let retryable = output.get("retryable").and_then(Value::as_bool) == Some(true);
        let outcome = if retryable {
            OpOutcome::Retryable
        } else {
            OpOutcome::Failed
        };
        return OperationResult {
            outcome: Some(outcome),
            payload: Some(json!({ "error_class": class, "message": output.get("message") })),
            ..OperationResult::default()
        };
    }
    let tool_calls = output
        .get("tool_calls")
        .and_then(Value::as_array)
        .map(|calls| {
            calls
                .iter()
                .filter_map(|c| c.get("call_id").and_then(Value::as_str))
                .map(|id| CallId(id.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let tokens = |k: &str| {
        output
            .get("usage")
            .and_then(|u| u.get(k))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    OperationResult {
        outcome: Some(OpOutcome::Succeeded),
        usage: Some(OpUsage {
            input_tokens: tokens("input_tokens"),
            output_tokens: tokens("output_tokens"),
        }),
        tool_calls,
        payload: Some(output),
        ..OperationResult::default()
    }
}

/// Normalize a tool function's output into an operation result. A native
/// `raisin.tool-result/1` envelope keeps its status; anything else is wrapped
/// as a LEGACY envelope, whose writes and evidence a reducer must not trust.
pub fn tool_result(op_id: &str, output: Value) -> OperationResult {
    let native = output.get("envelope").and_then(Value::as_str) == Some("raisin.tool-result/1");
    if !native {
        let envelope = json!({
            "envelope": "raisin.tool-result/1", "operation_id": op_id,
            "status": "succeeded", "legacy": true, "payload": output,
        });
        return OperationResult {
            outcome: Some(OpOutcome::Succeeded),
            payload: Some(envelope),
            ..OperationResult::default()
        };
    }
    let status = output
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("failed");
    let outcome = match status {
        "succeeded" => OpOutcome::Succeeded,
        "waiting" => OpOutcome::Waiting,
        "retryable" => OpOutcome::Retryable,
        "blocked" => OpOutcome::Blocked,
        _ => OpOutcome::Failed,
    };
    let resume_key = output
        .get("resume_key")
        .and_then(Value::as_str)
        .map(str::to_owned);
    OperationResult {
        outcome: Some(outcome),
        resume_key,
        payload: Some(output),
        ..OperationResult::default()
    }
}

/// Executes every operation kind of a run through functions.
pub struct FunctionOperationExecutor {
    storage: Arc<RocksDBStorage>,
    caller: FunctionCaller,
    model_turns: Arc<dyn ModelTurnExecutor>,
}

impl FunctionOperationExecutor {
    /// Tool calls through `caller`, model turns through `model_turns`.
    pub fn new(
        storage: Arc<RocksDBStorage>,
        caller: FunctionCaller,
        model_turns: Arc<dyn ModelTurnExecutor>,
    ) -> Self {
        Self {
            storage,
            caller,
            model_turns,
        }
    }
}

#[async_trait]
impl OperationExecutor for FunctionOperationExecutor {
    async fn execute(&self, ctx: ExecContext) -> OperationResult {
        let marker = format!("agent_run:{}", ctx.run_id);
        let auth = match resolve_principal_auth(&self.storage, &ctx.scope, &ctx.principal, &marker)
            .await
        {
            Ok(auth) => auth,
            Err(message) => return failed(OpOutcome::Blocked, "unauthorized", message),
        };
        let input = ctx.op.input.clone().unwrap_or(Value::Null);
        match &ctx.op.kind {
            OperationKind::ModelTurn => self.model_turns.run_turn(&ctx, auth, input).await,
            OperationKind::ToolCall => {
                let Some(tool) = input.get("tool").and_then(Value::as_str) else {
                    return failed(OpOutcome::Failed, "tool_error", "operation names no tool");
                };
                let args = input.get("args").cloned().unwrap_or_else(|| json!({}));
                match self.caller.call(&ctx, auth, tool, args).await {
                    Ok(output) => tool_result(ctx.op.op_id.as_str(), output),
                    Err(message) => failed(OpOutcome::Failed, "tool_error", message),
                }
            }
            other => failed(
                OpOutcome::Failed,
                "unsupported",
                format!("no executor for operation kind {other:?}"),
            ),
        }
    }
}
