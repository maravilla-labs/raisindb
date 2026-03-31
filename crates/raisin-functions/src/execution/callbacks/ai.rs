// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! AI operation callbacks for function execution.
//!
//! These callbacks implement the `raisin.ai.*` API available to JavaScript functions.

use std::sync::Arc;
use std::time::Instant;

use futures::StreamExt;
use raisin_ai::{
    streaming::{self, StreamEvent},
    types::CompletionRequest,
    TenantAIConfigStore,
};
use raisin_storage::jobs::{global_conversation_broadcaster, ConversationEvent};
use serde_json::Value;

use crate::api::{
    AICompletionCallback, AIEmbedCallback, AIGetDefaultModelCallback, AIListModelsCallback,
};
use crate::execution::ai_provider::create_provider_for_model;

/// Create ai_completion callback: `raisin.ai.completion(request)`
///
/// This callback handles AI completion requests from JavaScript functions by:
/// 1. Parsing the request JSON into a CompletionRequest
/// 2. Using the shared factory to create the provider
/// 3. Calling the provider's complete() or stream_complete() method
/// 4. Transforming the response back to JSON
///
/// ## Streaming support
///
/// Two optional fields control streaming behaviour:
///
/// - `stream: true` — explicit opt-in to streaming (uses provider's SSE API)
/// - `conversation_path: "..."` — where to emit events (independent of stream)
/// - `conversation_channel: "..."` — shared stream channel key (preferred)
///
/// If `stream` is true but the provider doesn't support it, the callback falls
/// back to non-streaming with a synthetic `TextChunk` event containing the full
/// response text.
pub fn create_ai_completion(
    ai_config_store: Option<Arc<dyn TenantAIConfigStore>>,
    tenant_id: String,
) -> AICompletionCallback {
    Arc::new(move |request_json: Value| {
        let store = ai_config_store.clone();
        let tenant = tenant_id.clone();

        Box::pin(async move {
            // Check if AI is configured
            let store = store.ok_or_else(|| {
                raisin_error::Error::Backend("AI operations not configured".to_string())
            })?;

            // Extract streaming control fields BEFORE parsing into CompletionRequest
            let wants_stream = request_json
                .get("stream")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let conversation_path = request_json
                .get("conversation_path")
                .and_then(|v| v.as_str())
                .map(String::from);
            let conversation_channel = request_json
                .get("conversation_channel")
                .and_then(|v| v.as_str())
                .map(String::from);
            let tools_requested = request_json
                .get("tools")
                .and_then(|v| v.as_array())
                .map(|arr| !arr.is_empty())
                .unwrap_or(false);

            let msg_count = request_json
                .get("messages")
                .and_then(|v| v.as_array())
                .map(|arr| arr.len())
                .unwrap_or(0);
            let requested_model = request_json
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");

            tracing::info!(
                target: "ai_completion",
                model = %requested_model,
                stream = wants_stream,
                tools_requested = tools_requested,
                message_count = msg_count,
                "AI completion requested"
            );

            let start = Instant::now();

            // Strip non-standard fields so CompletionRequest deserialization works
            let mut clean_json = request_json.clone();
            if let Some(obj) = clean_json.as_object_mut() {
                obj.remove("conversation_path");
                obj.remove("conversation_channel");
            }

            // Hard-switch routing: prefer shared channel; fall back to path only if channel is absent.
            let conversation_event_key = conversation_channel.clone().or(conversation_path.clone());

            // Coerce non-string/non-array message content to strings to prevent deserialization errors
            if let Some(messages) = clean_json
                .get_mut("messages")
                .and_then(|m| m.as_array_mut())
            {
                for msg in messages.iter_mut() {
                    if let Some(content) = msg.get_mut("content") {
                        match content {
                            Value::String(_) | Value::Array(_) | Value::Null => {
                                // Valid types — leave as-is
                            }
                            other => {
                                tracing::warn!(
                                    "Coercing non-string message content to string: {:?}",
                                    other
                                );
                                *other = Value::String(other.to_string());
                            }
                        }
                    }
                }
            }

            // Parse request from JSON
            let request: CompletionRequest = serde_json::from_value(clean_json).map_err(|e| {
                raisin_error::Error::Validation(format!("Invalid completion request: {}", e))
            })?;

            // Use shared factory - single source of truth!
            let provider =
                create_provider_for_model(store.as_ref(), &tenant, &request.model).await?;

            // Strip provider prefix from model name before sending to API
            let mut api_request = request;
            if let Some((_prefix, model_name)) = api_request.model.split_once(':') {
                api_request.model = model_name.to_string();
            }

            if wants_stream {
                // ---- Streaming path ----
                if provider.supports_streaming() {
                    // Ollama currently does not reliably support tool calls in streaming mode.
                    // Hard-switch to non-streaming for tool-capable turns to preserve tool calls.
                    if tools_requested && provider.provider_name() == "ollama" {
                        tracing::warn!(
                            provider = %provider.provider_name(),
                            "Disabling streaming for tool-enabled turn (provider limitation)"
                        );

                        let response = provider.complete(api_request).await.map_err(|e| {
                            raisin_error::Error::Backend(format!("AI completion failed: {}", e))
                        })?;

                        // Preserve realtime UX by emitting one final chunk when routing is set.
                        if let Some(key) = &conversation_event_key {
                            if !response.message.content.is_empty() {
                                let ts = chrono::Utc::now().to_rfc3339();
                                global_conversation_broadcaster().emit(
                                    key,
                                    ConversationEvent::TextChunk {
                                        text: response.message.content.clone(),
                                        timestamp: ts,
                                    },
                                );
                            }
                        }

                        let elapsed = start.elapsed().as_millis() as u64;
                        let has_tool_calls = response.message.tool_calls.as_ref().map_or(false, |tc| !tc.is_empty());
                        tracing::info!(
                            target: "ai_completion",
                            model = %response.model,
                            finish_reason = ?response.stop_reason,
                            has_tool_calls = has_tool_calls,
                            duration_ms = elapsed,
                            "AI completion response (ollama non-stream fallback)"
                        );

                        let response_json = build_response_json(&response);
                        return Ok(response_json);
                    }

                    // Set stream flag on the request
                    api_request.stream = true;

                    // Save a non-streaming fallback copy in case streaming loses tool calls
                    let fallback_request = {
                        let mut r = api_request.clone();
                        r.stream = false;
                        r
                    };

                    let stream = provider.stream_complete(api_request).await.map_err(|e| {
                        raisin_error::Error::Backend(format!("Stream AI completion failed: {}", e))
                    })?;

                    // Forward stream chunks through mpsc channel
                    let (tx, mut rx) = tokio::sync::mpsc::channel::<Value>(32);
                    tokio::spawn(async move {
                        futures::pin_mut!(stream);
                        while let Some(chunk_result) = stream.next().await {
                            match chunk_result {
                                Ok(chunk) => {
                                    let chunk_json = streaming::stream_chunk_to_json(chunk);
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

                    // Accumulate, emitting events to conversation broadcaster
                    let mut tool_map = std::collections::HashMap::new();
                    let mut response = streaming::accumulate_stream(
                        &mut rx,
                        |event| {
                            let routing_key = conversation_event_key.clone();
                            async move {
                                if let Some(key) = routing_key {
                                    let ts = chrono::Utc::now().to_rfc3339();
                                    let conv_event = match event {
                                        StreamEvent::TextChunk(text) => {
                                            ConversationEvent::TextChunk {
                                                text,
                                                timestamp: ts,
                                            }
                                        }
                                        StreamEvent::ThoughtChunk(text) => {
                                            ConversationEvent::ThoughtChunk {
                                                text,
                                                timestamp: ts,
                                            }
                                        }
                                    };
                                    global_conversation_broadcaster().emit(&key, conv_event);
                                }
                            }
                        },
                        &mut tool_map,
                    )
                    .await;

                    // Keep JS/runtime compatibility: expose finish_reason alongside stop_reason.
                    if response.get("finish_reason").is_none() {
                        if let Some(stop_reason) = response.get("stop_reason").cloned() {
                            response["finish_reason"] = stop_reason;
                        }
                    }

                    // Defensive fallback: if the provider signalled tool use but
                    // streaming failed to accumulate any tool calls, retry without
                    // streaming so the caller still gets tool_calls in the response.
                    let finish = response
                        .get("finish_reason")
                        .or(response.get("stop_reason"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let content_empty = response
                        .get("content")
                        .and_then(|v| v.as_str())
                        .map(|s| s.trim().is_empty())
                        .unwrap_or(true);
                    let tc_empty = response.get("tool_calls").map_or(true, |v| {
                        v.is_null() || v.as_array().map(|arr| arr.is_empty()).unwrap_or(false)
                    });

                    let needs_tool_fallback = ((finish == "tool_calls" || finish == "tool_use")
                        && tc_empty)
                        || (tools_requested && tc_empty && content_empty);

                    if needs_tool_fallback {
                        tracing::warn!(
                            finish_reason = %finish,
                            tools_requested = tools_requested,
                            content_empty = content_empty,
                            "Streaming response missing expected tool calls — falling back to non-streaming call"
                        );
                        let fallback = provider.complete(fallback_request).await.map_err(|e| {
                            raisin_error::Error::Backend(format!("AI fallback failed: {}", e))
                        })?;

                        let mut resp = serde_json::json!({
                            "content": fallback.message.content,
                            "model": fallback.model,
                        });
                        if let Some(tcs) = &fallback.message.tool_calls {
                            resp["tool_calls"] = serde_json::to_value(tcs).unwrap_or_default();
                        }
                        if let Some(reason) = &fallback.stop_reason {
                            resp["finish_reason"] = serde_json::json!(reason);
                        }
                        if let Some(usage) = &fallback.usage {
                            resp["usage"] = serde_json::json!({
                                "prompt_tokens": usage.prompt_tokens,
                                "completion_tokens": usage.completion_tokens,
                                "total_tokens": usage.total_tokens,
                            });
                        }

                        let elapsed = start.elapsed().as_millis() as u64;
                        let has_tool_calls = fallback.message.tool_calls.as_ref().map_or(false, |tc| !tc.is_empty());
                        tracing::info!(
                            target: "ai_completion",
                            model = %fallback.model,
                            finish_reason = ?fallback.stop_reason,
                            has_tool_calls = has_tool_calls,
                            duration_ms = elapsed,
                            "AI completion response (stream tool fallback)"
                        );

                        return Ok(resp);
                    }

                    let elapsed = start.elapsed().as_millis() as u64;
                    tracing::info!(
                        target: "ai_completion",
                        finish_reason = %finish,
                        has_tool_calls = !tc_empty,
                        duration_ms = elapsed,
                        "AI completion response (streamed)"
                    );

                    Ok(response)
                } else {
                    // Provider doesn't support streaming — fall back to non-streaming
                    tracing::warn!(
                        provider = %provider.provider_name(),
                        "Provider does not support streaming, using non-streaming fallback"
                    );

                    let response = provider.complete(api_request).await.map_err(|e| {
                        raisin_error::Error::Backend(format!("AI completion failed: {}", e))
                    })?;

                    // Emit full text as single TextChunk if conversation routing is set
                    if let Some(key) = &conversation_event_key {
                        if !response.message.content.is_empty() {
                            let ts = chrono::Utc::now().to_rfc3339();
                            global_conversation_broadcaster().emit(
                                key,
                                ConversationEvent::TextChunk {
                                    text: response.message.content.clone(),
                                    timestamp: ts,
                                },
                            );
                        }
                    }

                    let elapsed = start.elapsed().as_millis() as u64;
                    let has_tool_calls = response.message.tool_calls.as_ref().map_or(false, |tc| !tc.is_empty());
                    tracing::info!(
                        target: "ai_completion",
                        model = %response.model,
                        finish_reason = ?response.stop_reason,
                        has_tool_calls = has_tool_calls,
                        duration_ms = elapsed,
                        "AI completion response (non-stream fallback)"
                    );

                    let response_json = build_response_json(&response);
                    Ok(response_json)
                }
            } else {
                // ---- Non-streaming path (unchanged) ----
                let response = provider.complete(api_request).await.map_err(|e| {
                    raisin_error::Error::Backend(format!("AI completion failed: {}", e))
                })?;

                let elapsed = start.elapsed().as_millis() as u64;
                let has_tool_calls = response.message.tool_calls.as_ref().map_or(false, |tc| !tc.is_empty());
                tracing::info!(
                    target: "ai_completion",
                    model = %response.model,
                    finish_reason = ?response.stop_reason,
                    has_tool_calls = has_tool_calls,
                    duration_ms = elapsed,
                    "AI completion response"
                );

                let response_json = build_response_json(&response);
                Ok(response_json)
            }
        })
    })
}

/// Build the standard JSON response envelope from a CompletionResponse.
///
/// Includes provider-agnostic post-processing: if no structured tool calls
/// exist but the content contains raw `<function=...>` syntax, extract them.
fn build_response_json(response: &raisin_ai::types::CompletionResponse) -> Value {
    let tc_empty = response
        .message
        .tool_calls
        .as_ref()
        .map_or(true, |tc| tc.is_empty());

    // Strip control tokens from content
    let mut content =
        raisin_ai::tool_call_extraction::strip_model_control_tokens(&response.message.content);
    let mut tool_calls_json: Value = serde_json::to_value(&response.message.tool_calls)
        .unwrap_or(Value::Null);
    let mut stop_reason = response.stop_reason.clone();

    // Provider-agnostic: extract raw tool call syntax from content
    if tc_empty && !content.is_empty() {
        if let Some(extracted) =
            raisin_ai::tool_call_extraction::extract_tool_calls_from_content(&content)
        {
            tracing::warn!(
                "build_response_json: extracted {} tool call(s) from raw content",
                extracted.len()
            );
            tool_calls_json = serde_json::to_value(&extracted).unwrap_or(Value::Null);
            content = raisin_ai::tool_call_extraction::strip_tool_call_syntax(&content);
            if stop_reason.as_deref() == Some("stop") || stop_reason.is_none() {
                stop_reason = Some("tool_calls".to_string());
            }
        }
    }

    serde_json::json!({
        "content": content,
        "model": response.model,
        "finish_reason": stop_reason,
        "tool_calls": tool_calls_json,
        "usage": response.usage.as_ref().map(|u| serde_json::json!({
            "prompt_tokens": u.prompt_tokens,
            "completion_tokens": u.completion_tokens,
            "total_tokens": u.total_tokens,
        })),
    })
}

/// Create ai_list_models callback: `raisin.ai.listModels()`
///
/// Returns all models configured for the tenant across all enabled providers.
pub fn create_ai_list_models(
    ai_config_store: Option<Arc<dyn TenantAIConfigStore>>,
    tenant_id: String,
) -> AIListModelsCallback {
    Arc::new(move || {
        let store = ai_config_store.clone();
        let tenant = tenant_id.clone();

        Box::pin(async move {
            // Check if AI is configured
            let store = store.ok_or_else(|| {
                raisin_error::Error::Backend("AI operations not configured".to_string())
            })?;

            // Get tenant AI config
            let config = store.get_config(&tenant).await.map_err(|e| {
                raisin_error::Error::Backend(format!("Failed to get AI config: {}", e))
            })?;

            // Collect all models from all enabled providers
            let mut models = Vec::new();
            for provider_config in config.providers {
                if !provider_config.enabled {
                    continue;
                }

                for model in provider_config.models {
                    models.push(serde_json::json!({
                        "id": model.model_id,
                        "name": model.display_name,
                        "provider": provider_config.provider,
                        "use_cases": model.use_cases,
                        "default_temperature": model.default_temperature,
                        "default_max_tokens": model.default_max_tokens,
                        "is_default": model.is_default,
                    }));
                }
            }

            Ok(models)
        })
    })
}

/// Create ai_get_default_model callback: `raisin.ai.getDefaultModel(useCase)`
///
/// Returns the default model ID for a specific use case, or None if no default is configured.
pub fn create_ai_get_default_model(
    ai_config_store: Option<Arc<dyn TenantAIConfigStore>>,
    tenant_id: String,
) -> AIGetDefaultModelCallback {
    Arc::new(move |use_case_str: String| {
        let store = ai_config_store.clone();
        let tenant = tenant_id.clone();

        Box::pin(async move {
            // Check if AI is configured
            let store = store.ok_or_else(|| {
                raisin_error::Error::Backend("AI operations not configured".to_string())
            })?;

            // Parse use case from string
            let use_case = match use_case_str.as_str() {
                "embedding" => raisin_ai::config::AIUseCase::Embedding,
                "chat" => raisin_ai::config::AIUseCase::Chat,
                "agent" => raisin_ai::config::AIUseCase::Agent,
                "completion" => raisin_ai::config::AIUseCase::Completion,
                "classification" => raisin_ai::config::AIUseCase::Classification,
                _ => {
                    return Err(raisin_error::Error::Validation(format!(
                        "Invalid use case: {}",
                        use_case_str
                    )))
                }
            };

            // Get tenant AI config
            let config = store.get_config(&tenant).await.map_err(|e| {
                raisin_error::Error::Backend(format!("Failed to get AI config: {}", e))
            })?;

            // Find default model for the use case
            for provider_config in config.providers {
                if !provider_config.enabled {
                    continue;
                }

                for model in provider_config.models {
                    if model.use_cases.contains(&use_case) && model.is_default {
                        return Ok(Some(model.model_id));
                    }
                }
            }

            Ok(None)
        })
    })
}

/// Create ai_embed callback: `raisin.ai.embed(request)`
///
/// Generates vector embeddings for text or image input.
pub fn create_ai_embed(
    ai_config_store: Option<Arc<dyn TenantAIConfigStore>>,
    tenant_id: String,
) -> AIEmbedCallback {
    Arc::new(move |request_json: Value| {
        let store = ai_config_store.clone();
        let tenant = tenant_id.clone();

        Box::pin(async move {
            // Check if AI is configured
            let store = store.ok_or_else(|| {
                raisin_error::Error::Backend("AI operations not configured".to_string())
            })?;

            // Parse request
            let model = request_json
                .get("model")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    raisin_error::Error::Validation("Missing 'model' in embed request".to_string())
                })?
                .to_string();

            let input = request_json
                .get("input")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    raisin_error::Error::Validation("Missing 'input' in embed request".to_string())
                })?
                .to_string();

            // Use shared factory to create provider
            let provider = create_provider_for_model(store.as_ref(), &tenant, &model).await?;

            // Strip provider prefix from model name before sending to API
            let api_model = if let Some((_prefix, model_name)) = model.split_once(':') {
                model_name.to_string()
            } else {
                model.clone()
            };

            // Generate embedding with stripped model name
            let embedding = provider
                .generate_embedding(&input, &api_model)
                .await
                .map_err(|e| {
                    raisin_error::Error::Backend(format!("Embedding generation failed: {}", e))
                })?;

            let dimensions = embedding.len();

            // Return response
            Ok(serde_json::json!({
                "embedding": embedding,
                "model": model,
                "dimensions": dimensions
            }))
        })
    })
}
