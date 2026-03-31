// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! AI caller callback for flow execution

use std::collections::HashMap;
use std::sync::Arc;

use futures::StreamExt;
use serde::Serialize;

use crate::execution::ai_provider::create_provider_for_model;
use crate::execution::ExecutionDependencies;
use raisin_ai::types::{FunctionCall, Message, StreamChunk, ToolCall, Usage};
use raisin_binary::BinaryStorage;
use raisin_models::nodes::properties::{Properties, PropertyValue};
use raisin_storage::{transactional::TransactionalStorage, NodeRepository, Storage, StorageScope};

use super::types::{AiCallContext, AICallerCallback, AIStreamingCallerCallback};

// ---------------------------------------------------------------------------
// Envelope types – typed structs that serialize to the JSON shape the flow
// runtime expects, replacing manual `json!({})` construction.
// ---------------------------------------------------------------------------

/// Envelope for a non-streaming `CompletionResponse`.
#[derive(Serialize)]
struct CompletionResponseEnvelope {
    content: String,
    model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    finish_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ToolCallEnvelope>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<UsageEnvelope>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "_tool_map")]
    tool_map: Option<HashMap<String, String>>,
}

impl CompletionResponseEnvelope {
    fn from_response(
        response: raisin_ai::types::CompletionResponse,
        tool_path_map: HashMap<String, String>,
    ) -> Self {
        Self {
            content: response.message.content,
            model: response.model,
            finish_reason: response.stop_reason,
            tool_calls: response
                .message
                .tool_calls
                .map(|calls| calls.into_iter().map(ToolCallEnvelope::from).collect()),
            usage: response.usage.map(UsageEnvelope::from),
            tool_map: if tool_path_map.is_empty() {
                None
            } else {
                Some(tool_path_map)
            },
        }
    }
}

/// Envelope for a streaming chunk.
#[derive(Serialize)]
struct StreamChunkEnvelope {
    #[serde(skip_serializing_if = "Option::is_none")]
    delta: Option<StreamDelta>,
    #[serde(skip_serializing_if = "Option::is_none")]
    finish_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<UsageEnvelope>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
}

impl From<StreamChunk> for StreamChunkEnvelope {
    fn from(chunk: StreamChunk) -> Self {
        let has_content = !chunk.delta.is_empty();
        let has_tool_calls = chunk.tool_calls.is_some();

        let delta = if has_content || has_tool_calls {
            Some(StreamDelta {
                content: if has_content {
                    Some(chunk.delta)
                } else {
                    None
                },
                tool_calls: chunk.tool_calls.map(|calls| {
                    calls
                        .into_iter()
                        .enumerate()
                        .map(|(i, tc)| ToolCallDelta {
                            index: tc.index.unwrap_or(i),
                            id: tc.id,
                            function: FunctionCallEnvelope::from(tc.function),
                        })
                        .collect()
                }),
            })
        } else {
            None
        };

        Self {
            delta,
            finish_reason: chunk.stop_reason,
            usage: chunk.usage.map(UsageEnvelope::from),
            model: chunk.model,
        }
    }
}

#[derive(Serialize)]
struct StreamDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ToolCallDelta>>,
}

#[derive(Serialize)]
struct ToolCallDelta {
    index: usize,
    id: String,
    function: FunctionCallEnvelope,
}

#[derive(Serialize)]
struct ToolCallEnvelope {
    id: String,
    #[serde(rename = "type")]
    call_type: String,
    function: FunctionCallEnvelope,
}

impl From<ToolCall> for ToolCallEnvelope {
    fn from(tc: ToolCall) -> Self {
        Self {
            id: tc.id,
            call_type: tc.call_type,
            function: FunctionCallEnvelope::from(tc.function),
        }
    }
}

#[derive(Serialize)]
struct FunctionCallEnvelope {
    name: String,
    arguments: String,
}

impl From<FunctionCall> for FunctionCallEnvelope {
    fn from(fc: FunctionCall) -> Self {
        Self {
            name: fc.name,
            arguments: fc.arguments,
        }
    }
}

#[derive(Serialize)]
struct UsageEnvelope {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

impl From<Usage> for UsageEnvelope {
    fn from(u: Usage) -> Self {
        Self {
            prompt_tokens: u.prompt_tokens,
            completion_tokens: u.completion_tokens,
            total_tokens: u.total_tokens,
        }
    }
}

// ---------------------------------------------------------------------------
// Callback factories
// ---------------------------------------------------------------------------

/// Create AI caller callback - invokes AI agents (non-streaming)
pub(super) fn create_ai_caller<S, B>(deps: &Arc<ExecutionDependencies<S, B>>) -> AICallerCallback
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    let deps = deps.clone();
    Arc::new(move |ctx: AiCallContext, messages: Vec<serde_json::Value>, response_format_json: Option<serde_json::Value>| {
        let deps = deps.clone();
        Box::pin(async move {
            tracing::debug!(
                tenant_id = %ctx.tenant_id,
                repo_id = %ctx.repo_id,
                branch = %ctx.branch,
                agent_ref = %ctx.agent_ref,
                message_count = messages.len(),
                "Flow ai_caller callback"
            );

            let (request, tool_path_map) = build_completion_request(
                &deps,
                &ctx,
                &messages,
                response_format_json,
                false,
            )
            .await?;

            let model = request.model.clone();
            let ai_config_store = deps.ai_config_store.as_ref().ok_or_else(|| {
                "AI operations not configured - no ai_config_store available".to_string()
            })?;

            let provider =
                create_provider_for_model(ai_config_store.as_ref(), &ctx.tenant_id, &model)
                    .await
                    .map_err(|e| format!("Failed to create AI provider: {}", e))?;

            let response = provider
                .complete(request)
                .await
                .map_err(|e| format!("AI completion failed: {}", e))?;

            tracing::debug!(
                model = %response.model,
                finish_reason = ?response.stop_reason,
                "AI completion successful"
            );

            let envelope = CompletionResponseEnvelope::from_response(response, tool_path_map);
            serde_json::to_value(&envelope).map_err(|e| format!("Serialization failed: {}", e))
        })
    })
}

/// Create streaming AI caller callback — invokes AI agents with streaming.
///
/// Returns a `mpsc::Receiver<Value>` that yields streaming chunks. The
/// caller spawns a background task that reads from the provider's SSE
/// stream and forwards JSON-serialized `StreamChunk` values.
pub(super) fn create_ai_streaming_caller<S, B>(
    deps: &Arc<ExecutionDependencies<S, B>>,
) -> AIStreamingCallerCallback
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    let deps = deps.clone();
    Arc::new(move |ctx: AiCallContext, messages: Vec<serde_json::Value>, response_format_json: Option<serde_json::Value>| {
        let deps = deps.clone();
        Box::pin(async move {
            let (request, tool_path_map) = build_completion_request(
                &deps,
                &ctx,
                &messages,
                response_format_json,
                true,
            )
            .await?;

            let model = request.model.clone();
            let ai_config_store = deps.ai_config_store.as_ref().ok_or_else(|| {
                "AI operations not configured - no ai_config_store available".to_string()
            })?;
            let provider =
                create_provider_for_model(ai_config_store.as_ref(), &ctx.tenant_id, &model)
                    .await
                    .map_err(|e| format!("Failed to create AI provider: {}", e))?;

            let mut stream = provider
                .stream_complete(request)
                .await
                .map_err(|e| format!("Stream AI completion failed: {}", e))?;

            let (tx, rx) = tokio::sync::mpsc::channel::<serde_json::Value>(32);

            if !tool_path_map.is_empty() {
                let _ = tx
                    .send(serde_json::json!({ "_tool_map": tool_path_map }))
                    .await;
            }

            tokio::spawn(async move {
                while let Some(chunk_result) = stream.next().await {
                    match chunk_result {
                        Ok(chunk) => {
                            let envelope = StreamChunkEnvelope::from(chunk);
                            let chunk_json =
                                serde_json::to_value(&envelope).unwrap_or_default();
                            if tx.send(chunk_json).await.is_err() {
                                break;
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Stream chunk error: {}", e);
                            let _ = tx
                                .send(serde_json::json!({ "error": e.to_string() }))
                                .await;
                            break;
                        }
                    }
                }
            });

            Ok(rx)
        })
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Shared helper: load agent, build CompletionRequest and tool-path map.
async fn build_completion_request<S, B>(
    deps: &Arc<ExecutionDependencies<S, B>>,
    ctx: &AiCallContext,
    messages: &[serde_json::Value],
    response_format_json: Option<serde_json::Value>,
    stream: bool,
) -> Result<(raisin_ai::types::CompletionRequest, HashMap<String, String>), String>
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    let agent_node = deps
        .storage
        .nodes()
        .get_by_path(
            StorageScope::new(&ctx.tenant_id, &ctx.repo_id, &ctx.branch, "functions"),
            &ctx.agent_ref,
            None,
        )
        .await
        .map_err(|e| format!("Failed to load agent node: {}", e))?
        .ok_or_else(|| format!("Agent not found at path: {}", ctx.agent_ref))?;

    let props = Properties::new(&agent_node.properties);
    let raw_model = props
        .get_string("model")
        .ok_or_else(|| "Agent missing 'model' property".to_string())?;

    // Qualify the model ID with the provider prefix if not already qualified.
    // The agent node stores `provider` and `model` as separate properties
    // (e.g., provider="ollama", model="qwen2.5-coder:latest"), but
    // `create_provider_for_model` expects a qualified ID like "ollama:qwen2.5-coder:latest"
    // for dynamic model resolution.
    let model = if raw_model.contains(':') {
        // Check if the prefix is already a known provider
        let prefix = raw_model.split_once(':').map(|(p, _)| p).unwrap_or("");
        if raisin_ai::config::AIProvider::from_serde_name(prefix).is_some() {
            raw_model
        } else if let Some(provider) = props.get_string("provider") {
            format!("{}:{}", provider, raw_model)
        } else {
            raw_model
        }
    } else if let Some(provider) = props.get_string("provider") {
        format!("{}:{}", provider, raw_model)
    } else {
        raw_model
    };

    let system_prompt = props.get_string("system_prompt");
    let temperature = props.get_number("temperature").map(|n| n as f32);
    let max_tokens = props.get_number("max_tokens").map(|n| n as u32);

    let ai_messages = build_ai_messages(system_prompt.as_deref(), messages);
    let (tools, tool_path_map) =
        load_agent_tools(deps, &props, &ctx.tenant_id, &ctx.repo_id, &ctx.branch).await;

    let response_format = response_format_json.and_then(|rf| {
        serde_json::from_value::<raisin_ai::types::ResponseFormat>(rf)
            .map_err(|e| {
                tracing::warn!("Invalid response_format, ignoring: {}", e);
                e
            })
            .ok()
    });

    // Strip provider prefix (e.g. "groq:") — only needed for provider routing, not the API call
    let api_model = model
        .split_once(':')
        .filter(|(prefix, _)| raisin_ai::config::AIProvider::from_serde_name(prefix).is_some())
        .map(|(_, name)| name.to_string())
        .unwrap_or(model);

    let request = raisin_ai::types::CompletionRequest {
        model: api_model,
        messages: ai_messages,
        system: None,
        tools: if tools.is_empty() { None } else { Some(tools) },
        temperature,
        max_tokens,
        stream,
        response_format,
    };

    Ok((request, tool_path_map))
}

/// Build AI messages from system prompt and input JSON messages.
///
/// Uses `Message` constructors and `serde_json::from_value` for tool_calls
/// instead of manual field-by-field parsing.
fn build_ai_messages(
    system_prompt: Option<&str>,
    messages: &[serde_json::Value],
) -> Vec<Message> {
    let mut ai_messages: Vec<Message> = Vec::new();

    if let Some(system) = system_prompt {
        ai_messages.push(Message::system(system));
    }

    for msg in messages {
        let role_str = msg.get("role").and_then(|v| v.as_str()).unwrap_or("user");
        let content = msg
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let mut message = match role_str {
            "system" => Message::system(&content),
            "assistant" => Message::assistant(&content),
            "tool" => {
                let tool_call_id = msg
                    .get("tool_call_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                Message::tool(&content, tool_call_id, None)
            }
            _ => Message::user(&content),
        };

        if let Some(tool_calls) = parse_tool_calls(msg) {
            message = message.with_tool_calls(tool_calls);
        }

        // Carry over tool_call_id for non-tool roles (e.g. assistant messages
        // that reference a tool call) — `Message::tool` already sets it above.
        if role_str != "tool" {
            message.tool_call_id = msg
                .get("tool_call_id")
                .and_then(|v| v.as_str())
                .map(String::from);
        }

        ai_messages.push(message);
    }

    ai_messages
}

/// Parse tool_calls array from a JSON message value.
fn parse_tool_calls(msg: &serde_json::Value) -> Option<Vec<ToolCall>> {
    let arr = msg.get("tool_calls")?.as_array()?;
    let calls: Vec<ToolCall> = arr
        .iter()
        .filter_map(|tc| {
            Some(ToolCall {
                id: tc.get("id")?.as_str()?.to_string(),
                call_type: tc
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("function")
                    .to_string(),
                function: FunctionCall {
                    name: tc.get("function")?.get("name")?.as_str()?.to_string(),
                    arguments: tc
                        .get("function")?
                        .get("arguments")
                        .map(|a| {
                            if a.is_string() {
                                a.as_str().unwrap_or("{}").to_string()
                            } else {
                                serde_json::to_string(a).unwrap_or_default()
                            }
                        })
                        .unwrap_or_default(),
                },
                index: None,
            })
        })
        .collect();
    if calls.is_empty() {
        None
    } else {
        Some(calls)
    }
}

/// Load tool definitions from agent configuration.
///
/// Returns the tool definitions AND a mapping of tool name -> function path
/// so the flow runtime can resolve tool names back to executable paths.
async fn load_agent_tools<S, B>(
    deps: &Arc<ExecutionDependencies<S, B>>,
    props: &Properties<'_>,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
) -> (Vec<raisin_ai::types::ToolDefinition>, HashMap<String, String>)
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    use raisin_ai::types::{FunctionDefinition, ToolDefinition};

    let mut tools: Vec<ToolDefinition> = Vec::new();
    let mut tool_path_map: HashMap<String, String> = Default::default();

    if let Some(tool_refs) = props.get_array("tools") {
        for tool_ref in tool_refs {
            let tool_path = match tool_ref {
                PropertyValue::String(path) => path.clone(),
                PropertyValue::Reference(r) => r.path.clone(),
                _ => continue,
            };

            if tool_path.is_empty() {
                continue;
            }

            if let Ok(Some(func_node)) = deps
                .storage
                .nodes()
                .get_by_path(
                    StorageScope::new(tenant_id, repo_id, branch, "functions"),
                    &tool_path,
                    None,
                )
                .await
            {
                let func_props = Properties::new(&func_node.properties);

                let tool_name = func_props
                    .get_string("name")
                    .unwrap_or_else(|| func_node.name.clone());

                let tool_description = func_props.get_string("description").unwrap_or_default();

                let parameters = func_props
                    .get("input_schema")
                    .map(|v| serde_json::to_value(v).unwrap_or_default())
                    .unwrap_or_else(|| {
                        serde_json::json!({
                            "type": "object",
                            "properties": {},
                        })
                    });

                tool_path_map.insert(tool_name.clone(), tool_path);

                tools.push(ToolDefinition {
                    tool_type: "function".to_string(),
                    function: FunctionDefinition {
                        name: tool_name,
                        description: tool_description,
                        parameters,
                    },
                });
            }
        }
    }

    (tools, tool_path_map)
}
