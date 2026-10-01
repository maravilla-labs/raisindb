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
//!
//! A tool may call a model itself. Those calls are collected while the tool
//! runs (`raisin_storage::jobs::ai_usage`) and recorded like a model turn's:
//! the operation's usage (so the run's token budget counts them), a
//! `raisin:AICostRecord` under the message that requested the tool, and the
//! conversation's `total_tokens_used` (see [`record_tool_usage`]).

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
        let execution_id = execution_id(ctx);
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

/// The id an operation's function call executes under: made here, by the
/// server, from the operation, never by a client or the function.
fn execution_id(ctx: &ExecContext) -> String {
    format!("{}#{}", ctx.op.op_id, ctx.op.attempt)
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
                let tracked = raisin_storage::jobs::ai_usage::track(&execution_id(&ctx));
                let called = self.caller.call(&ctx, auth, tool, args).await;
                let usage = tracked.take();
                let mut result = match called {
                    Ok(output) => tool_result(ctx.op.op_id.as_str(), output),
                    Err(message) => failed(OpOutcome::Failed, "tool_error", message),
                };
                if usage.total_tokens() > 0 {
                    result.usage = Some(OpUsage {
                        input_tokens: usage.input_tokens(),
                        output_tokens: usage.output_tokens(),
                    });
                    record_tool_usage(&self.storage, &ctx, tool, &usage).await;
                }
                result
            }
            other => failed(
                OpOutcome::Failed,
                "unsupported",
                format!("no executor for operation kind {other:?}"),
            ),
        }
    }
}

/// How long recording a tool's model usage may hold up its result. The
/// visitor's answer matters more than the ledger.
const RECORD_USAGE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Record what a tool's own model calls used: a `raisin:AICostRecord` under
/// the message that requested the tool, and the conversation's
/// `total_tokens_used`. Best effort: a failure is logged and never fails or
/// holds up the tool's result for longer than [`RECORD_USAGE_TIMEOUT`].
async fn record_tool_usage(
    storage: &Arc<RocksDBStorage>,
    ctx: &ExecContext,
    tool: &str,
    usage: &raisin_storage::jobs::ai_usage::UsageTotals,
) {
    let Some(subject) = ctx.subject.as_ref() else {
        tracing::debug!(op = %ctx.op.op_id, "tool model usage: the run has no conversation to record it in");
        return;
    };
    match tokio::time::timeout(
        RECORD_USAGE_TIMEOUT,
        write_tool_usage(storage, ctx, &subject.workspace, &subject.path, tool, usage),
    )
    .await
    {
        Ok(Ok(())) => {}
        Ok(Err(e)) => tracing::warn!(
            op = %ctx.op.op_id, chat = %subject.path, error = %e,
            "tool model usage could not be recorded"
        ),
        Err(_) => tracing::warn!(
            op = %ctx.op.op_id, chat = %subject.path,
            "tool model usage: recording timed out"
        ),
    }
}

/// The cost record's name: one per operation attempt, so a retried write
/// finds it and counts nothing twice.
fn tool_cost_record_name(ctx: &ExecContext) -> String {
    let op: String = ctx
        .op
        .op_id
        .as_str()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    format!("tool-cost-{op}-{}", ctx.op.attempt)
}

/// The message that requested this tool call: the run's assistant message
/// whose `run_tool_calls` names the call, else the run's newest message.
fn requesting_message<'a>(
    children: &'a [raisin_models::nodes::Node],
    run_id: &str,
    call_id: Option<&str>,
) -> Option<&'a raisin_models::nodes::Node> {
    use raisin_models::nodes::properties::PropertyValue;
    let of_run: Vec<&raisin_models::nodes::Node> = children
        .iter()
        .filter(|n| n.node_type == "raisin:Message")
        .filter(
            |n| matches!(n.properties.get("run_id"), Some(PropertyValue::String(r)) if r == run_id),
        )
        .collect();
    let names_call =
        |n: &&raisin_models::nodes::Node| match (call_id, n.properties.get("run_tool_calls")) {
            (Some(call), Some(PropertyValue::Array(calls))) => calls.iter().any(|c| match c {
                PropertyValue::Object(o) => {
                    matches!(o.get("call_id"), Some(PropertyValue::String(id)) if id == call)
                }
                _ => false,
            }),
            _ => false,
        };
    of_run
        .iter()
        .find(|n| names_call(n))
        .or_else(|| of_run.iter().max_by_key(|n| n.created_at))
        .copied()
}

async fn write_tool_usage(
    storage: &Arc<RocksDBStorage>,
    ctx: &ExecContext,
    workspace: &str,
    chat_path: &str,
    tool: &str,
    usage: &raisin_storage::jobs::ai_usage::UsageTotals,
) -> raisin_error::Result<()> {
    use raisin_models::nodes::properties::PropertyValue;

    let svc = raisin_core::NodeService::new_with_context(
        storage.clone(),
        ctx.scope.tenant_id.clone(),
        ctx.scope.repo_id.clone(),
        ctx.scope.branch.clone(),
        workspace.to_string(),
    )
    .with_auth(AuthContext::system_as("agent-run-tool-usage"));

    let children = svc.list_children(chat_path).await?;
    let call_id = ctx.op.for_call_id.as_ref().map(|c| c.0.as_str());
    let Some(message) = requesting_message(&children, ctx.run_id.as_str(), call_id) else {
        return Err(raisin_error::Error::NotFound(format!(
            "no message of run {} under {chat_path}",
            ctx.run_id
        )));
    };
    let name = tool_cost_record_name(ctx);
    if svc
        .get_by_path(&format!("{}/{name}", message.path))
        .await?
        .is_some()
    {
        return Ok(()); // a retry: recorded and counted already
    }

    let models: Vec<&str> = {
        let mut m: Vec<&str> = usage.calls.iter().map(|c| c.model.as_str()).collect();
        m.dedup();
        m
    };
    let int = |n: u64| PropertyValue::Integer(n.min(i64::MAX as u64) as i64);
    let mut props = std::collections::HashMap::new();
    props.insert("model".to_string(), PropertyValue::String(models.join(",")));
    props.insert(
        "provider".to_string(),
        PropertyValue::String("tool".to_string()),
    );
    props.insert(
        "source".to_string(),
        PropertyValue::String("tool".to_string()),
    );
    props.insert("tool".to_string(), PropertyValue::String(tool.to_string()));
    props.insert("calls".to_string(), int(usage.calls.len() as u64));
    props.insert("input_tokens".to_string(), int(usage.input_tokens()));
    props.insert("output_tokens".to_string(), int(usage.output_tokens()));
    props.insert("total_tokens".to_string(), int(usage.total_tokens()));
    props.insert(
        "timestamp".to_string(),
        PropertyValue::String(
            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        ),
    );
    let node = raisin_models::nodes::Node {
        name: name.clone(),
        node_type: "raisin:AICostRecord".to_string(),
        properties: props,
        ..Default::default()
    };
    svc.add_node(&message.path, node).await?;

    // The conversation's running total, as the model turn keeps it. A run's
    // operations run one at a time, so this does not race the turn's own
    // read-modify-write.
    let current = match svc.get_by_path(chat_path).await? {
        Some(chat) => match chat.properties.get("total_tokens_used") {
            Some(PropertyValue::Integer(i)) => *i as f64,
            Some(PropertyValue::Float(f)) => *f,
            _ => 0.0,
        },
        None => 0.0,
    };
    svc.update_property_by_path(
        chat_path,
        "total_tokens_used",
        PropertyValue::Integer((current as i64).saturating_add(usage.total_tokens() as i64)),
    )
    .await?;
    Ok(())
}

/// A tool's own model usage, recorded against a real RocksDB conversation.
#[cfg(test)]
mod tool_usage_tests {
    use std::collections::HashMap;

    use raisin_agent_runtime::ids::{
        LeaseEpoch, OperationId, Principal, RunId, RunScope, SubjectRef,
    };
    use raisin_agent_runtime::record::ActiveOperation;
    use raisin_context::RepositoryConfig;
    use raisin_core::services::workspace_service::WorkspaceService;
    use raisin_models::nodes::properties::PropertyValue;
    use raisin_models::nodes::types::node_type::NodeType;
    use raisin_models::nodes::Node;
    use raisin_models::workspace::Workspace;
    use raisin_storage::jobs::ai_usage::{ModelCall, UsageTotals};
    use raisin_storage::scope::BranchScope;
    use raisin_storage::{
        BranchRepository, CommitMetadata, NodeTypeRepository, RegistryRepository,
        RepositoryManagementRepository, Storage,
    };

    use super::*;
    use crate::RocksDBConfig;

    const RUN: &str = "6c45e796-2942-4b8f-b5f5-601791bb9091";
    const CHAT: &str = "/chat";

    fn node_type(name: &str, children: &[&str]) -> NodeType {
        serde_json::from_value(json!({
            "id": name, "name": name, "strict": false, "version": 1,
            "allowed_children": children, "versionable": false, "publishable": false,
            "auditable": false, "indexable": true,
        }))
        .unwrap()
    }

    async fn storage(dir: &tempfile::TempDir) -> Arc<RocksDBStorage> {
        let mut config = RocksDBConfig::default();
        config.path = dir.path().to_path_buf();
        let storage = Arc::new(RocksDBStorage::with_config(config).unwrap());
        storage
            .registry()
            .register_tenant("t", HashMap::new())
            .await
            .unwrap();
        storage
            .repository_management()
            .create_repository(
                "t",
                "repo",
                RepositoryConfig {
                    default_language: "en".into(),
                    supported_languages: vec!["en".into()],
                    locale_fallback_chains: HashMap::new(),
                    default_branch: "main".into(),
                    description: None,
                    tags: HashMap::new(),
                },
            )
            .await
            .unwrap();
        storage
            .branches()
            .create_branch("t", "repo", "main", "system", None, None, false, false)
            .await
            .unwrap();
        for nt in [
            node_type(
                "raisin:Conversation",
                &["raisin:Message", "raisin:AICompaction"],
            ),
            node_type(
                "raisin:Message",
                &["raisin:AICostRecord", "raisin:AIToolCall"],
            ),
            node_type("raisin:AICostRecord", &[]),
        ] {
            storage
                .node_types()
                .upsert(
                    BranchScope::new("t", "repo", "main"),
                    nt,
                    CommitMetadata::system("seed"),
                )
                .await
                .unwrap();
        }
        let mut ws = Workspace::new("ai".to_string());
        ws.config.default_branch = "main".into();
        WorkspaceService::new(storage.clone())
            .put("t", "repo", ws)
            .await
            .unwrap();
        storage
    }

    fn svc(storage: &Arc<RocksDBStorage>) -> raisin_core::NodeService<RocksDBStorage> {
        raisin_core::NodeService::new_with_context(
            storage.clone(),
            "t".into(),
            "repo".into(),
            "main".into(),
            "ai".into(),
        )
        .with_auth(AuthContext::system_as("test"))
    }

    fn props(v: Value) -> HashMap<String, PropertyValue> {
        v.as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), PropertyValue::from_json(v)))
            .collect()
    }

    /// The chat: a visitor message, then two assistant turns of the run; the
    /// first asked for call `fc_1`.
    async fn chat(storage: &Arc<RocksDBStorage>) {
        let s = svc(storage);
        s.add_node(
            "/",
            Node {
                name: "chat".into(),
                node_type: "raisin:Conversation".into(),
                properties: props(json!({ "participants": [], "total_tokens_used": 5000 })),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        for (name, p) in [
            ("msg-visitor", json!({ "role": "user" })),
            (
                "run-6c45e796-op-1",
                json!({ "role": "assistant", "run_id": RUN,
                        "run_tool_calls": [{ "call_id": "fc_1", "name": "ask-website" }] }),
            ),
            (
                "run-6c45e796-op-3",
                json!({ "role": "assistant", "run_id": RUN, "run_tool_calls": [] }),
            ),
        ] {
            s.add_node(
                CHAT,
                Node {
                    name: name.into(),
                    node_type: "raisin:Message".into(),
                    properties: props(p),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        }
    }

    fn ctx(call: &str, attempt: u32) -> ExecContext {
        let run = RunId(RUN.into());
        ExecContext {
            scope: RunScope::new("t", "repo", "main"),
            op: ActiveOperation {
                op_id: OperationId::nth(&run, 2),
                kind: OperationKind::ToolCall,
                effect_id: None,
                idempotency_key: String::new(),
                replay_safe: true,
                interruptible: true,
                for_call_id: Some(CallId(call.into())),
                input: None,
                started_at_ms: 0,
                deadline_ms: None,
                lease_epoch: LeaseEpoch(1),
                attempt,
            },
            run_id: run,
            principal: Principal::user("visitor:x"),
            token: tokio_util::sync::CancellationToken::new(),
            agent_ref: None,
            subject: Some(SubjectRef {
                workspace: "ai".into(),
                path: CHAT.into(),
                node_id: None,
            }),
            executor_config: None,
        }
    }

    fn usage() -> UsageTotals {
        UsageTotals {
            calls: vec![
                ModelCall {
                    model: "gpt".into(),
                    input_tokens: 1000,
                    output_tokens: 200,
                },
                ModelCall {
                    model: "gpt".into(),
                    input_tokens: 800,
                    output_tokens: 50,
                },
            ],
        }
    }

    #[tokio::test]
    async fn a_tools_model_usage_lands_under_its_message_and_in_the_chat_total() {
        let dir = tempfile::tempdir().unwrap();
        let storage = storage(&dir).await;
        chat(&storage).await;

        record_tool_usage(&storage, &ctx("fc_1", 1), "/lib/site/ask-website", &usage()).await;

        let s = svc(&storage);
        let rec = s
            .get_by_path(&format!("{CHAT}/run-6c45e796-op-1/tool-cost-{RUN}-op-2-1"))
            .await
            .unwrap()
            .expect("the record sits under the message that asked for the tool");
        assert_eq!(rec.node_type, "raisin:AICostRecord");
        assert_eq!(
            rec.properties.get("total_tokens"),
            Some(&PropertyValue::Integer(2050))
        );
        assert_eq!(
            rec.properties.get("input_tokens"),
            Some(&PropertyValue::Integer(1800))
        );
        assert_eq!(
            rec.properties.get("calls"),
            Some(&PropertyValue::Integer(2))
        );
        // stored as a Date, which the daily budget's query compares as text
        assert!(matches!(
            rec.properties.get("timestamp"),
            Some(PropertyValue::Date(_))
        ));
        let chat = s.get_by_path(CHAT).await.unwrap().unwrap();
        assert_eq!(
            chat.properties.get("total_tokens_used"),
            Some(&PropertyValue::Integer(7050))
        );

        // the same attempt again (a retried write): nothing counted twice
        record_tool_usage(&storage, &ctx("fc_1", 1), "/lib/site/ask-website", &usage()).await;
        let chat = s.get_by_path(CHAT).await.unwrap().unwrap();
        assert_eq!(
            chat.properties.get("total_tokens_used"),
            Some(&PropertyValue::Integer(7050))
        );
    }

    #[tokio::test]
    async fn an_unknown_call_falls_back_to_the_runs_newest_message() {
        let dir = tempfile::tempdir().unwrap();
        let storage = storage(&dir).await;
        chat(&storage).await;
        record_tool_usage(&storage, &ctx("fc_unknown", 1), "/lib/t", &usage()).await;
        let s = svc(&storage);
        assert!(s
            .get_by_path(&format!("{CHAT}/run-6c45e796-op-3/tool-cost-{RUN}-op-2-1"))
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn a_missing_conversation_is_logged_not_raised() {
        let dir = tempfile::tempdir().unwrap();
        let storage = storage(&dir).await;
        // no chat at all: returns quietly, the tool result is not affected
        record_tool_usage(&storage, &ctx("fc_1", 1), "/lib/t", &usage()).await;
        let mut no_subject = ctx("fc_1", 1);
        no_subject.subject = None;
        record_tool_usage(&storage, &no_subject, "/lib/t", &usage()).await;
    }
}
