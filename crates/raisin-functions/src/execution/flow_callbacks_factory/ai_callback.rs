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
use raisin_ai::types::{FunctionCall, Message, StreamChunk, ToolCall, ToolDefinition, Usage};
use raisin_binary::BinaryStorage;
use raisin_models::nodes::properties::{Properties, PropertyValue};
use raisin_storage::{transactional::TransactionalStorage, NodeRepository, Storage, StorageScope};

use super::skills::{self, SelectedSkill};
use super::types::{AICallerCallback, AIStreamingCallerCallback, AiCallContext};

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
    /// The skills this call was granted, `[{name, workspace, path}]`. The
    /// runtime hands it to `load-skill` as `__raisin_context.skill_grant`,
    /// because a flow has no chat for the tool to derive the grant from.
    #[serde(skip_serializing_if = "Option::is_none", rename = "_skill_grant")]
    skill_grant: Option<serde_json::Value>,
}

impl CompletionResponseEnvelope {
    fn from_response(
        response: raisin_ai::types::CompletionResponse,
        tool_path_map: HashMap<String, String>,
        skill_grant: &[SelectedSkill],
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
            skill_grant: if skill_grant.is_empty() {
                None
            } else {
                Some(skills::grant_json(skill_grant))
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
                content: if has_content { Some(chunk.delta) } else { None },
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
    Arc::new(
        move |ctx: AiCallContext,
              messages: Vec<serde_json::Value>,
              response_format_json: Option<serde_json::Value>| {
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

                let (request, tool_path_map, routing_model_id, skill_grant) =
                    build_completion_request(&deps, &ctx, &messages, response_format_json, false)
                        .await?;

                let ai_config_store = deps.ai_config_store.as_ref().ok_or_else(|| {
                    "AI operations not configured - no ai_config_store available".to_string()
                })?;

                let provider = create_provider_for_model(
                    ai_config_store.as_ref(),
                    &ctx.tenant_id,
                    &routing_model_id,
                )
                .await
                .map_err(|e| format!("Failed to create AI provider: {}", e))?;

                let response = raisin_ai::complete_with_tool_repair(provider.as_ref(), request)
                    .await
                    .map_err(|e| format!("AI completion failed: {}", e))?;

                tracing::debug!(
                    model = %response.model,
                    finish_reason = ?response.stop_reason,
                    "AI completion successful"
                );

                let envelope = CompletionResponseEnvelope::from_response(
                    response,
                    tool_path_map,
                    &skill_grant,
                );
                serde_json::to_value(&envelope).map_err(|e| format!("Serialization failed: {}", e))
            })
        },
    )
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
    Arc::new(
        move |ctx: AiCallContext,
              messages: Vec<serde_json::Value>,
              response_format_json: Option<serde_json::Value>| {
            let deps = deps.clone();
            Box::pin(async move {
                let (request, tool_path_map, routing_model_id, skill_grant) =
                    build_completion_request(&deps, &ctx, &messages, response_format_json, true)
                        .await?;

                let ai_config_store = deps.ai_config_store.as_ref().ok_or_else(|| {
                    "AI operations not configured - no ai_config_store available".to_string()
                })?;
                let provider = create_provider_for_model(
                    ai_config_store.as_ref(),
                    &ctx.tenant_id,
                    &routing_model_id,
                )
                .await
                .map_err(|e| format!("Failed to create AI provider: {}", e))?;

                let mut stream =
                    raisin_ai::stream_complete_with_tool_repair(provider.as_ref(), request)
                        .await
                        .map_err(|e| format!("Stream AI completion failed: {}", e))?;

                let (tx, rx) = tokio::sync::mpsc::channel::<serde_json::Value>(32);

                // The first chunk carries the side-band maps. `_skill_grant`
                // rides with `_tool_map` so a streaming caller gets it too.
                if !tool_path_map.is_empty() || !skill_grant.is_empty() {
                    let mut first = serde_json::json!({ "_tool_map": tool_path_map });
                    if !skill_grant.is_empty() {
                        first["_skill_grant"] = skills::grant_json(&skill_grant);
                    }
                    let _ = tx.send(first).await;
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
                                let _ =
                                    tx.send(serde_json::json!({ "error": e.to_string() })).await;
                                break;
                            }
                        }
                    }
                });

                Ok(rx)
            })
        },
    )
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Shared helper: load agent, build CompletionRequest, tool-path map and the
/// skill grant.
///
/// The third element of the return is the model id used for ROUTING —
/// `<slug>:<model>` — which is what `create_provider_for_model` needs to find the
/// tenant entry. `request.model` is the same id with the slug stripped, because the
/// upstream API only knows its own model names. The two are returned separately
/// because they genuinely differ; passing the stripped one to the router would leave
/// it guessing, and passing the qualified one to the provider would ask e.g. Groq for
/// a model called `marvel:maravilla/smart`.
async fn build_completion_request<S, B>(
    deps: &Arc<ExecutionDependencies<S, B>>,
    ctx: &AiCallContext,
    messages: &[serde_json::Value],
    response_format_json: Option<serde_json::Value>,
    stream: bool,
) -> Result<
    (
        raisin_ai::types::CompletionRequest,
        HashMap<String, String>,
        String,
        Vec<SelectedSkill>,
    ),
    String,
>
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

    // Qualify the model ID with the provider slug if not already qualified.
    // The agent node stores `provider` and `model` as separate properties
    // (e.g., provider="ollama", model="qwen2.5-coder:latest"), but
    // `create_provider_for_model` expects a qualified ID like "ollama:qwen2.5-coder:latest"
    // for dynamic model resolution.
    //
    // "Already qualified" means the part before the first colon is one of THIS tenant's
    // provider slugs, which is why the config has to be loaded here rather than matched
    // against the `AIProvider` enum: a tenant whose gateway is slugged `marvel` needs
    // `marvel:…` recognised though no enum variant is called that, and a tenant with no
    // `anthropic` entry must not have `anthropic:…` treated as routable. Model names
    // carry colons of their own (`qwen2.5-coder:latest`), so the distinction decides
    // whether the agent's `provider` property gets prepended at all.
    //
    // No config store (or an unreadable config) means no slugs exist, hence nothing can
    // be qualified — the `provider` property is prepended and the router reports the
    // miss with a message listing what the tenant does have.
    let ai_config = match deps.ai_config_store.as_ref() {
        Some(store) => store.get_config(&ctx.tenant_id).await.ok(),
        None => None,
    };
    let has_slug_prefix = |id: &str| {
        ai_config
            .as_ref()
            .is_some_and(|c| c.parse_model_id(id).0.is_some())
    };

    let model = match props.get_string("provider") {
        Some(provider) if !has_slug_prefix(&raw_model) => format!("{}:{}", provider, raw_model),
        _ => raw_model,
    };

    // RULES FIX. The system prompt used to be `system_prompt` ALONE here, so an
    // agent's `rules` applied in chat (agent-handler appends them as
    // `## Rules`) and were silently dropped whenever the same agent ran as a
    // workflow step. This is the one function every flow surface reaches —
    // `create_ai_caller` and `create_ai_streaming_caller` both build here,
    // whichever `call_ai*` trait method the step used — so no step needs
    // wiring of its own. Rules apply with skills ON or OFF.
    //
    // SKILLS share the same tail: the index of the granted raisin:Skill nodes
    // goes before `## Rules`, which stays last. No skills and no rules leave
    // `system_prompt` byte for byte.
    //
    // Skills are offered only when the caller RUNS A TOOL LOOP
    // (`ctx.offer_skills`). A decision / competition / agent-assignee call
    // makes one request and needs its structured answer back; told to load a
    // skill, it would return a `tool_call` instead. With the switch off the
    // grant is never resolved, the prompt is the rules-only one and no
    // `load-skill` is offered — whatever global skills exist.
    let temperature = props.get_number("temperature").map(|n| n as f32);
    let max_tokens = props.get_number("max_tokens").map(|n| n as u32);

    let (mut tools, mut tool_path_map) =
        load_agent_tools(deps, &props, &ctx.tenant_id, &ctx.repo_id, &ctx.branch).await;
    let (system_prompt, skill_grant) = skill_surface(
        ctx,
        &props,
        &mut tools,
        &mut tool_path_map,
        || resolve_skill_grant(deps, ctx, &props),
        || load_skill_tool_definition(deps, ctx),
    )
    .await;
    let ai_messages = build_ai_messages(system_prompt.as_deref(), messages);

    // CONTROL TOOLS, appended after the agent's own. These have no function
    // behind them — the flow runtime intercepts the call by name before it
    // would reach `execute_function` — so they are deliberately absent from
    // `tool_path_map`: a control tool that acquired a path would be executed as
    // one, and `end_session` would be invoked as the function `/end_session`.
    //
    // They come from the STEP (via `AiCallContext::extra_tools`) rather than
    // from the agent node, because whether an agent may end a conversation, or
    // park a flow on a person, is a property of the flow it is running in.
    for extra in &ctx.extra_tools {
        match serde_json::from_value::<ToolDefinition>(extra.clone()) {
            Ok(def) => tools.push(def),
            Err(e) => tracing::warn!("Ignoring malformed control tool definition: {}", e),
        }
    }

    // The STEP's response_format wins; the AGENT's declared `output_schema`
    // is the default beneath it.
    //
    // An agent has no output schema of its own in the engine — structured
    // output has always been a per-CALL setting — and that layering is right:
    // the same translator agent returns a different shape in a QA flow than in
    // a publishing one. But an agent that ALWAYS answers in one shape had no
    // way to say so, leaving every step to restate it and any step that forgot
    // to get prose back. Declaring it on the agent makes the common case
    // declarative without taking the override away.
    let response_format = response_format_json
        .or_else(|| {
            props.get("output_schema").map(|schema| {
                let schema = serde_json::to_value(schema).unwrap_or_default();
                serde_json::json!({
                    "type": "json_schema",
                    "json_schema": { "name": "agent_output", "schema": schema, "strict": false },
                })
            })
        })
        .and_then(|rf| {
            serde_json::from_value::<raisin_ai::types::ResponseFormat>(rf)
                .map_err(|e| {
                    tracing::warn!("Invalid response_format, ignoring: {}", e);
                    e
                })
                .ok()
        });

    // Strip the provider slug (e.g. "marvel:") — it routes the call, the upstream API
    // never sees it. Same tenant-slug test as the qualification above, so a colon that
    // belongs to the model name (`qwen2.5-coder:latest`) survives untouched.
    let api_model = ai_config
        .as_ref()
        .map(|c| c.parse_model_id(&model).1.to_string())
        .unwrap_or_else(|| model.clone());

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

    Ok((request, tool_path_map, model, skill_grant))
}

/// The skills side of a request: the system prompt (with the skills index
/// when anything is granted, and always with `rules`), `load-skill` added to
/// `tools` / `tool_path_map`, and the grant for `_skill_grant`.
///
/// When `ctx.offer_skills` is off, `resolve` and `load_tool` are never called:
/// the grant is empty, the prompt is `system_prompt` plus rules exactly as
/// before skills existed, and `tools` is left as the agent declared it.
/// The IO is passed in as closures so this one path is what the tests run.
async fn skill_surface<R, RF, L, LF>(
    ctx: &AiCallContext,
    props: &Properties<'_>,
    tools: &mut Vec<ToolDefinition>,
    tool_path_map: &mut HashMap<String, String>,
    resolve: R,
    load_tool: L,
) -> (Option<String>, Vec<SelectedSkill>)
where
    R: FnOnce() -> RF,
    RF: std::future::Future<Output = Vec<SelectedSkill>>,
    L: FnOnce() -> LF,
    LF: std::future::Future<Output = Option<ToolDefinition>>,
{
    let skill_grant = if ctx.offer_skills {
        resolve().await
    } else {
        Vec::new()
    };

    let omitted = skills::skill_index_omitted(&skill_grant);
    if !skill_grant.is_empty() {
        tracing::info!(
            agent_ref = %ctx.agent_ref,
            granted = skill_grant.len(),
            listed = skill_grant.len() - omitted,
            omitted,
            skills = ?skill_grant.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
            "Agent skills index"
        );
    }
    if omitted > 0 {
        tracing::warn!(
            agent_ref = %ctx.agent_ref,
            omitted,
            "Skill index truncated: {} granted skills are not listed in the prompt",
            omitted
        );
    }
    let system_prompt = agent_system_prompt(props, &skill_grant);

    // LOAD-SKILL. The index names skills; their bodies load through this tool.
    // It is a real function, so unlike the control tools it gets a
    // `tool_path_map` entry and the runtime executes it — with the grant put
    // into `__raisin_context` from `_skill_grant`, never from the model.
    if needs_load_skill(tools, &skill_grant) {
        match load_tool().await {
            Some(def) => offer_load_skill(tools, tool_path_map, def),
            None => tracing::warn!(
                agent_ref = %ctx.agent_ref,
                path = skills::LOAD_SKILL_PATH,
                "Skills are granted but the load-skill function is missing; the index lists skills the agent cannot open"
            ),
        }
    }

    (system_prompt, skill_grant)
}

/// The agent's system prompt with the skills index and its `rules` appended,
/// in exactly the shape the chat loop writes (see `skills.rs`). No skills and
/// no rules return `system_prompt` untouched, byte for byte.
fn agent_system_prompt(props: &Properties<'_>, skill_grant: &[SelectedSkill]) -> Option<String> {
    let rules: Vec<String> = match props.get("rules") {
        Some(PropertyValue::Array(items)) => items
            .iter()
            .filter_map(|v| match v {
                PropertyValue::String(s) => Some(s.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    let tail = skills::compose_instruction_tail(&rules, skill_grant);
    skills::apply_tail(props.get_string("system_prompt"), &tail)
}

// ---------------------------------------------------------------------------
// Skills — IO only; every decision is in `skills.rs`
// ---------------------------------------------------------------------------

/// Where an explicit skill reference points.
#[derive(Debug, Clone, PartialEq)]
struct SkillRef {
    workspace: String,
    /// A path (leading `/`) or a node id.
    target: String,
}

impl SkillRef {
    fn new(workspace: Option<&str>, path: Option<&str>, id: Option<&str>) -> Option<Self> {
        fn non_empty(s: Option<&str>) -> Option<&str> {
            s.map(str::trim).filter(|s| !s.is_empty())
        }
        let target = non_empty(path).or_else(|| non_empty(id))?;
        Some(Self {
            workspace: non_empty(workspace)
                .unwrap_or(skills::SKILL_WORKSPACE)
                .to_string(),
            target: target.to_string(),
        })
    }
}

/// One entry of an agent's `skills:` — the envelope `tools:` uses, or a bare
/// path.
fn parse_skill_ref_property(value: &PropertyValue) -> Option<SkillRef> {
    match value {
        PropertyValue::String(path) => SkillRef::new(None, Some(path), None),
        PropertyValue::Reference(r) => {
            SkillRef::new(Some(&r.workspace), Some(&r.path), Some(&r.id))
        }
        PropertyValue::Object(fields) => {
            let field = |key: &str| match fields.get(key) {
                Some(PropertyValue::String(s)) => Some(s.as_str()),
                _ => None,
            };
            SkillRef::new(
                field("raisin:workspace").or_else(|| field("workspace")),
                field("raisin:path").or_else(|| field("path")),
                field("raisin:ref"),
            )
        }
        _ => None,
    }
}

/// One entry of a step's `skills:` (JSON from the flow definition).
fn parse_skill_ref_json(value: &serde_json::Value) -> Option<SkillRef> {
    match value {
        serde_json::Value::String(path) => SkillRef::new(None, Some(path), None),
        serde_json::Value::Object(fields) => {
            let field = |key: &str| fields.get(key).and_then(|v| v.as_str());
            SkillRef::new(
                field("raisin:workspace").or_else(|| field("workspace")),
                field("raisin:path").or_else(|| field("path")),
                field("raisin:ref"),
            )
        }
        _ => None,
    }
}

/// What the rule reads of a node.
fn skill_node_of(node: &raisin_models::nodes::Node) -> skills::SkillNode {
    let props = Properties::new(&node.properties);
    let mut properties = serde_json::Map::new();
    if let Some(name) = props.get_string("name") {
        properties.insert("name".into(), name.into());
    }
    if let Some(description) = props.get_string("description") {
        properties.insert("description".into(), description.into());
    }
    if let Some(PropertyValue::Boolean(enabled)) = node.properties.get("enabled") {
        properties.insert("enabled".into(), (*enabled).into());
    }
    skills::SkillNode {
        path: node.path.clone(),
        node_type: node.node_type.clone(),
        properties: serde_json::Value::Object(properties),
    }
}

/// Read one explicit reference. A failed read is absent, logged at debug.
async fn load_declared_skill<S, B>(
    deps: &Arc<ExecutionDependencies<S, B>>,
    ctx: &AiCallContext,
    source: &str,
    reference: SkillRef,
) -> skills::DeclaredSkill
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    let scope = StorageScope::new(
        &ctx.tenant_id,
        &ctx.repo_id,
        &ctx.branch,
        &reference.workspace,
    );
    let read = if reference.target.starts_with('/') {
        deps.storage
            .nodes()
            .get_by_path(scope, &reference.target, None)
            .await
    } else {
        deps.storage
            .nodes()
            .get(scope, &reference.target, None)
            .await
    };
    let node = match read {
        Ok(node) => node,
        Err(e) => {
            tracing::debug!(
                workspace = %reference.workspace,
                target = %reference.target,
                error = %e,
                "Skill reference unreadable; treated as absent"
            );
            None
        }
    };
    skills::DeclaredSkill {
        source: source.to_string(),
        workspace: reference.workspace,
        path: node
            .as_ref()
            .map(|n| n.path.clone())
            .unwrap_or(reference.target),
        node: node.as_ref().map(skill_node_of),
    }
}

/// The children of a global skills folder. A missing or unreadable folder is
/// empty, logged at debug.
async fn list_global_skills<S, B>(
    deps: &Arc<ExecutionDependencies<S, B>>,
    ctx: &AiCallContext,
    folder: &str,
) -> Vec<skills::SkillNode>
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    match deps
        .storage
        .nodes()
        .list_children(
            StorageScope::new(
                &ctx.tenant_id,
                &ctx.repo_id,
                &ctx.branch,
                skills::SKILL_WORKSPACE,
            ),
            folder,
            raisin_storage::ListOptions::default(),
        )
        .await
    {
        Ok(children) => children.iter().map(skill_node_of).collect(),
        Err(e) => {
            tracing::debug!(folder, error = %e, "Global skills folder unreadable; treated as empty");
            Vec::new()
        }
    }
}

/// The skills this call is granted: the agent's `skills:`, the step's
/// `skills:` (`ctx.skills`), then — unless the agent says
/// `global_skills: false` — the installation and package globals.
async fn resolve_skill_grant<S, B>(
    deps: &Arc<ExecutionDependencies<S, B>>,
    ctx: &AiCallContext,
    props: &Properties<'_>,
) -> Vec<SelectedSkill>
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    let mut declared = Vec::new();
    for entry in props.get_array("skills").into_iter().flatten() {
        if let Some(reference) = parse_skill_ref_property(entry) {
            declared.push(load_declared_skill(deps, ctx, "agent", reference).await);
        }
    }
    for entry in &ctx.skills {
        if let Some(reference) = parse_skill_ref_json(entry) {
            declared.push(load_declared_skill(deps, ctx, "step", reference).await);
        }
    }

    let globals_enabled = !matches!(
        props.get("global_skills"),
        Some(PropertyValue::Boolean(false))
    );
    let (installation, pkg) = if globals_enabled {
        (
            list_global_skills(deps, ctx, skills::INSTALLATION_SKILLS_PATH).await,
            list_global_skills(deps, ctx, skills::PACKAGE_SKILLS_PATH).await,
        )
    } else {
        (Vec::new(), Vec::new())
    };

    skills::select_skills(&skills::SkillSelection {
        declared,
        installation,
        pkg,
        globals_enabled,
    })
}

/// Offer `load-skill` only when something is granted and the agent does not
/// already list a tool of that name.
fn needs_load_skill(tools: &[ToolDefinition], skill_grant: &[SelectedSkill]) -> bool {
    !skill_grant.is_empty()
        && !tools
            .iter()
            .any(|t| t.function.name == skills::LOAD_SKILL_TOOL)
}

/// Add the `load-skill` tool and route it to its function.
fn offer_load_skill(
    tools: &mut Vec<ToolDefinition>,
    tool_path_map: &mut HashMap<String, String>,
    def: ToolDefinition,
) {
    tool_path_map.insert(
        skills::LOAD_SKILL_TOOL.to_string(),
        skills::LOAD_SKILL_PATH.to_string(),
    );
    tools.push(def);
}

/// The `load-skill` function node as a tool, read the way `load_agent_tools`
/// reads any tool. `None` when the node is missing or unreadable.
async fn load_skill_tool_definition<S, B>(
    deps: &Arc<ExecutionDependencies<S, B>>,
    ctx: &AiCallContext,
) -> Option<ToolDefinition>
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    let node = deps
        .storage
        .nodes()
        .get_by_path(
            StorageScope::new(
                &ctx.tenant_id,
                &ctx.repo_id,
                &ctx.branch,
                skills::SKILL_WORKSPACE,
            ),
            skills::LOAD_SKILL_PATH,
            None,
        )
        .await
        .ok()??;
    Some(tool_definition_of(
        skills::LOAD_SKILL_TOOL.to_string(),
        &Properties::new(&node.properties),
    ))
}

/// A function node's tool definition under `name`: its `description` and its
/// `input_schema` (an empty object schema when it declares none).
fn tool_definition_of(name: String, func_props: &Properties<'_>) -> ToolDefinition {
    let parameters = func_props
        .get("input_schema")
        .map(|v| serde_json::to_value(v).unwrap_or_default())
        .unwrap_or_else(|| {
            serde_json::json!({
                "type": "object",
                "properties": {},
            })
        });
    ToolDefinition {
        tool_type: "function".to_string(),
        function: raisin_ai::types::FunctionDefinition {
            name,
            description: func_props.get_string("description").unwrap_or_default(),
            parameters,
        },
    }
}

/// Build AI messages from system prompt and input JSON messages.
///
/// Uses `Message` constructors and `serde_json::from_value` for tool_calls
/// instead of manual field-by-field parsing.
fn build_ai_messages(system_prompt: Option<&str>, messages: &[serde_json::Value]) -> Vec<Message> {
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
                // The function this result answers for. Groq's gpt-oss
                // (harmony) renderer refuses a tool message without a name.
                let name = msg.get("name").and_then(|v| v.as_str()).map(String::from);
                Message::tool(&content, tool_call_id, name)
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
) -> (
    Vec<raisin_ai::types::ToolDefinition>,
    HashMap<String, String>,
)
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    use raisin_ai::types::FunctionDefinition;

    let mut tools: Vec<ToolDefinition> = Vec::new();
    let mut tool_path_map: HashMap<String, String> = Default::default();

    if let Some(tool_refs) = props.get_array("tools") {
        for tool_ref in tool_refs {
            let Some(entry) = parse_agent_tool_entry(tool_ref) else {
                continue;
            };

            let tool_path = entry.path;
            let workspace = entry.workspace.as_deref().unwrap_or("functions");

            if let Ok(Some(func_node)) = deps
                .storage
                .nodes()
                .get_by_path(
                    StorageScope::new(tenant_id, repo_id, branch, workspace),
                    &tool_path,
                    None,
                )
                .await
            {
                let func_props = Properties::new(&func_node.properties);

                let tool_name = entry.alias.clone().unwrap_or_else(|| {
                    func_props
                        .get_string("name")
                        .unwrap_or_else(|| func_node.name.clone())
                });

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

/// One entry of an agent's `tools` array, in any of the three shapes the
/// property accepts.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AgentToolEntry {
    /// Path to the function node.
    pub path: String,
    /// Workspace holding it. `None` means the `functions` default.
    pub workspace: Option<String>,
    /// Name to advertise the tool under, overriding the function's own.
    pub alias: Option<String>,
    /// Whether the tool requires explicit invocation.
    pub explicit: bool,
}

/// Read one `tools` entry.
///
/// `raisin_agent.yaml` declares the items as OBJECTS
/// (`{path, workspace, alias, explicit}`), but this loader only ever matched
/// `String` and `Reference` and `continue`d on everything else — so an agent
/// authored the way its own nodetype documents advertised NO tools, silently
/// and with nothing logged. All three shapes are read here:
///
/// - `PropertyValue::String("/path")` — the bare path.
/// - `PropertyValue::Reference` — the path it points at.
/// - `PropertyValue::Object` — the declared shape; `path` is required, and
///   `workspace`/`alias`/`explicit` are optional. A nested `Reference` under
///   `path` is accepted too, since that is how a reference-typed field
///   round-trips.
///
/// Returns `None` for an entry with no usable path, which is the one case the
/// caller skips.
pub(crate) fn parse_agent_tool_entry(value: &PropertyValue) -> Option<AgentToolEntry> {
    let non_empty = |s: &str| {
        let t = s.trim();
        (!t.is_empty()).then(|| t.to_string())
    };

    match value {
        PropertyValue::String(path) => non_empty(path).map(|path| AgentToolEntry {
            path,
            workspace: None,
            alias: None,
            explicit: false,
        }),
        PropertyValue::Reference(r) => non_empty(&r.path).map(|path| AgentToolEntry {
            path,
            workspace: None,
            alias: None,
            explicit: false,
        }),
        PropertyValue::Object(fields) => {
            let string_field = |key: &str| match fields.get(key) {
                Some(PropertyValue::String(s)) => non_empty(s),
                Some(PropertyValue::Reference(r)) => non_empty(&r.path),
                _ => None,
            };

            let path = string_field("path")?;

            Some(AgentToolEntry {
                path,
                workspace: string_field("workspace"),
                alias: string_field("alias"),
                explicit: matches!(fields.get("explicit"), Some(PropertyValue::Boolean(true))),
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod agent_tool_entry_tests {
    use super::*;
    use std::collections::HashMap;

    fn obj(pairs: &[(&str, PropertyValue)]) -> PropertyValue {
        let mut map = HashMap::new();
        for (k, v) in pairs {
            map.insert((*k).to_string(), v.clone());
        }
        PropertyValue::Object(map)
    }

    #[test]
    fn a_bare_string_is_a_path() {
        let entry = parse_agent_tool_entry(&PropertyValue::String("/tools/greet".into())).unwrap();
        assert_eq!(entry.path, "/tools/greet");
        assert_eq!(entry.workspace, None);
        assert_eq!(entry.alias, None);
        assert!(!entry.explicit);
    }

    #[test]
    fn the_declared_object_form_is_read() {
        // Regression: this shape is what `raisin_agent.yaml` declares, and the
        // loader used to skip it, so the agent advertised no tools at all.
        let entry = parse_agent_tool_entry(&obj(&[
            ("path", PropertyValue::String("/tools/greet".into())),
            ("workspace", PropertyValue::String("shared".into())),
            ("alias", PropertyValue::String("say_hello".into())),
            ("explicit", PropertyValue::Boolean(true)),
        ]))
        .unwrap();

        assert_eq!(entry.path, "/tools/greet");
        assert_eq!(entry.workspace.as_deref(), Some("shared"));
        assert_eq!(entry.alias.as_deref(), Some("say_hello"));
        assert!(entry.explicit);
    }

    #[test]
    fn an_object_with_only_a_path_takes_the_defaults() {
        let entry = parse_agent_tool_entry(&obj(&[(
            "path",
            PropertyValue::String("/tools/greet".into()),
        )]))
        .unwrap();
        assert_eq!(entry.path, "/tools/greet");
        assert_eq!(entry.workspace, None);
        assert_eq!(entry.alias, None);
        assert!(!entry.explicit);
    }

    #[test]
    fn an_entry_with_no_usable_path_is_skipped() {
        assert!(parse_agent_tool_entry(&PropertyValue::String("   ".into())).is_none());
        assert!(parse_agent_tool_entry(&PropertyValue::Boolean(true)).is_none());
        assert!(parse_agent_tool_entry(&obj(&[(
            "workspace",
            PropertyValue::String("shared".into())
        )]))
        .is_none());
    }
}

#[cfg(test)]
mod skill_prompt_tests {
    use super::*;
    use raisin_models::nodes::properties::RaisinReference;

    fn props(pairs: &[(&str, PropertyValue)]) -> HashMap<String, PropertyValue> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), v.clone()))
            .collect()
    }

    fn granted(name: &str, source: &str) -> SelectedSkill {
        SelectedSkill {
            name: name.to_string(),
            description: format!("{name} does things."),
            workspace: "functions".to_string(),
            path: format!("/skills/{name}"),
            source: source.to_string(),
        }
    }

    fn tool(name: &str) -> ToolDefinition {
        ToolDefinition {
            tool_type: "function".to_string(),
            function: raisin_ai::types::FunctionDefinition {
                name: name.to_string(),
                description: String::new(),
                parameters: serde_json::json!({ "type": "object", "properties": {} }),
            },
        }
    }

    #[test]
    fn an_empty_grant_with_no_rules_is_the_system_prompt_byte_for_byte() {
        for sp in ["", "You are a probe.\n", "  trailing  \n\n"] {
            let data = props(&[("system_prompt", PropertyValue::String(sp.into()))]);
            assert_eq!(
                agent_system_prompt(&Properties::new(&data), &[]),
                Some(sp.to_string())
            );
        }
        let none = props(&[]);
        assert_eq!(agent_system_prompt(&Properties::new(&none), &[]), None);
        // An empty rules array is no rules.
        let empty_rules = props(&[
            ("system_prompt", PropertyValue::String("sp".into())),
            ("rules", PropertyValue::Array(vec![])),
        ]);
        assert_eq!(
            agent_system_prompt(&Properties::new(&empty_rules), &[]),
            Some("sp".to_string())
        );
    }

    #[test]
    fn rules_only_matches_the_chat_text() {
        // agent-handler: systemPrompt += '\n\n## Rules\n' + rules.map(r => `- ${r}`).join('\n')
        let data = props(&[
            ("system_prompt", PropertyValue::String("sp".into())),
            (
                "rules",
                PropertyValue::Array(vec![
                    PropertyValue::String("be brief".into()),
                    PropertyValue::String("cite".into()),
                ]),
            ),
        ]);
        assert_eq!(
            agent_system_prompt(&Properties::new(&data), &[]),
            Some("sp\n\n## Rules\n- be brief\n- cite".to_string())
        );
    }

    #[test]
    fn the_index_goes_before_the_rules() {
        let data = props(&[
            ("system_prompt", PropertyValue::String("sp".into())),
            (
                "rules",
                PropertyValue::Array(vec![PropertyValue::String("r".into())]),
            ),
        ]);
        let prompt =
            agent_system_prompt(&Properties::new(&data), &[granted("pdf", "agent")]).unwrap();
        assert_eq!(
            prompt,
            format!(
                "sp{}\n- pdf \u{2014} pdf does things.\n\n## Rules\n- r",
                skills::SKILL_INDEX_HEADER
            )
        );
    }

    #[test]
    fn a_step_skill_is_added_to_the_agents() {
        let node = |name: &str| skills::SkillNode {
            path: format!("/skills/{name}"),
            node_type: skills::SKILL_NODE_TYPE.to_string(),
            properties: serde_json::json!({ "name": name, "description": "D." }),
        };
        let grant = skills::select_skills(&skills::SkillSelection {
            declared: vec![
                skills::DeclaredSkill {
                    source: "agent".into(),
                    workspace: "functions".into(),
                    path: "/skills/agent-one".into(),
                    node: Some(node("agent-one")),
                },
                skills::DeclaredSkill {
                    source: "step".into(),
                    workspace: "functions".into(),
                    path: "/skills/step-one".into(),
                    node: Some(node("step-one")),
                },
            ],
            installation: vec![],
            pkg: vec![],
            globals_enabled: true,
        });
        let names: Vec<_> = grant
            .iter()
            .map(|s| (s.name.as_str(), s.source.as_str()))
            .collect();
        assert_eq!(names, vec![("agent-one", "agent"), ("step-one", "step")]);
    }

    #[test]
    fn step_skill_refs_parse_from_the_envelope() {
        let r = parse_skill_ref_json(&serde_json::json!({
            "raisin:ref": "/skills/pdf", "raisin:workspace": "functions"
        }))
        .unwrap();
        assert_eq!(r.workspace, "functions");
        assert_eq!(r.target, "/skills/pdf");
        let r = parse_skill_ref_json(&serde_json::json!({
            "raisin:ref": "0b6c…uuid", "raisin:workspace": "shared", "raisin:path": "/s/x"
        }))
        .unwrap();
        assert_eq!(
            (r.workspace.as_str(), r.target.as_str()),
            ("shared", "/s/x")
        );
        assert!(parse_skill_ref_json(&serde_json::json!(42)).is_none());
        assert!(parse_skill_ref_json(&serde_json::json!({ "raisin:workspace": "x" })).is_none());

        let agent_ref = parse_skill_ref_property(&PropertyValue::Reference(RaisinReference {
            id: "abc".into(),
            workspace: "functions".into(),
            path: String::new(),
        }))
        .unwrap();
        assert_eq!(agent_ref.target, "abc", "an id-only reference reads by id");
    }

    #[test]
    fn load_skill_is_offered_only_when_something_is_granted() {
        let mut tools = vec![tool("lookup")];
        let mut map: HashMap<String, String> =
            [("lookup".to_string(), "/lib/lookup".to_string())].into();

        assert!(!needs_load_skill(&tools, &[]));
        // Nothing granted: the request is exactly the agent's tools.
        assert_eq!(tools.len(), 1);
        assert!(!map.contains_key(skills::LOAD_SKILL_TOOL));

        let grant = [granted("pdf", "package")];
        assert!(needs_load_skill(&tools, &grant));
        offer_load_skill(&mut tools, &mut map, tool(skills::LOAD_SKILL_TOOL));
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[1].function.name, skills::LOAD_SKILL_TOOL);
        assert_eq!(
            map.get(skills::LOAD_SKILL_TOOL).map(String::as_str),
            Some(skills::LOAD_SKILL_PATH)
        );
        // An agent that already lists it does not get it twice.
        assert!(!needs_load_skill(&tools, &grant));
    }

    #[test]
    fn the_envelope_carries_the_grant_only_when_there_is_one() {
        let response = || raisin_ai::types::CompletionResponse {
            message: Message::assistant("hi"),
            model: "m".into(),
            usage: None,
            stop_reason: None,
        };
        let bare = serde_json::to_value(CompletionResponseEnvelope::from_response(
            response(),
            HashMap::new(),
            &[],
        ))
        .unwrap();
        assert!(bare.get("_skill_grant").is_none());
        let with = serde_json::to_value(CompletionResponseEnvelope::from_response(
            response(),
            HashMap::new(),
            &[granted("pdf", "agent")],
        ))
        .unwrap();
        assert_eq!(
            with["_skill_grant"],
            serde_json::json!([{ "name": "pdf", "workspace": "functions", "path": "/skills/pdf" }])
        );
    }

    /// A global (package) skill, resolved the way the server resolves one.
    fn global_grant() -> Vec<SelectedSkill> {
        skills::select_skills(&skills::SkillSelection {
            declared: vec![],
            installation: vec![],
            pkg: vec![skills::SkillNode {
                path: format!("{}/pdf", skills::PACKAGE_SKILLS_PATH),
                node_type: skills::SKILL_NODE_TYPE.to_string(),
                properties: serde_json::json!({ "name": "pdf", "description": "Read PDFs." }),
            }],
            globals_enabled: true,
        })
    }

    fn agent_with_rules() -> HashMap<String, PropertyValue> {
        props(&[
            ("system_prompt", PropertyValue::String("Decide.".into())),
            (
                "rules",
                PropertyValue::Array(vec![PropertyValue::String("answer in JSON".into())]),
            ),
        ])
    }

    #[tokio::test]
    async fn a_decision_call_is_unchanged_by_a_global_skill() {
        // agent_decision / agent_competition / agent_assignee reach the caller
        // through `call_ai`, which leaves `offer_skills` at its default.
        let ctx = AiCallContext::default();
        assert!(!ctx.offer_skills, "skills are OFF by default");
        assert!(!global_grant().is_empty(), "a global skill exists");

        let data = agent_with_rules();
        let mut tools = vec![tool("lookup")];
        let mut map: HashMap<String, String> =
            [("lookup".to_string(), "/lib/lookup".to_string())].into();
        let resolved = std::cell::Cell::new(false);
        let loaded = std::cell::Cell::new(false);

        let (prompt, grant) = skill_surface(
            &ctx,
            &Properties::new(&data),
            &mut tools,
            &mut map,
            || {
                resolved.set(true);
                async { global_grant() }
            },
            || {
                loaded.set(true);
                async { Some(tool(skills::LOAD_SKILL_TOOL)) }
            },
        )
        .await;

        // The prompt a decision step got before skills existed — rules fix
        // included — and no index.
        assert_eq!(
            prompt.as_deref(),
            Some("Decide.\n\n## Rules\n- answer in JSON")
        );
        assert!(!prompt.unwrap().contains(skills::SKILL_INDEX_HEADER.trim()));
        assert!(grant.is_empty(), "no _skill_grant");
        let names: Vec<_> = tools.iter().map(|t| t.function.name.as_str()).collect();
        assert_eq!(names, vec!["lookup"], "no load-skill tool");
        assert!(!map.contains_key(skills::LOAD_SKILL_TOOL));
        assert!(!resolved.get(), "the grant is not even resolved");
        assert!(!loaded.get());
    }

    #[tokio::test]
    async fn a_tool_loop_call_gets_the_index_and_load_skill() {
        let ctx = AiCallContext {
            offer_skills: true,
            ..Default::default()
        };
        let data = agent_with_rules();
        let mut tools = vec![tool("lookup")];
        let mut map: HashMap<String, String> =
            [("lookup".to_string(), "/lib/lookup".to_string())].into();

        let (prompt, grant) = skill_surface(
            &ctx,
            &Properties::new(&data),
            &mut tools,
            &mut map,
            || async { global_grant() },
            || async { Some(tool(skills::LOAD_SKILL_TOOL)) },
        )
        .await;

        assert_eq!(
            prompt.as_deref(),
            Some(
                format!(
                    "Decide.{}\n- pdf \u{2014} Read PDFs.\n\n## Rules\n- answer in JSON",
                    skills::SKILL_INDEX_HEADER
                )
                .as_str()
            ),
            "index before the rules, rules kept"
        );
        let granted: Vec<_> = grant
            .iter()
            .map(|s| (s.name.as_str(), s.source.as_str()))
            .collect();
        assert_eq!(granted, vec![("pdf", "package")]);
        let names: Vec<_> = tools.iter().map(|t| t.function.name.as_str()).collect();
        assert_eq!(names, vec!["lookup", skills::LOAD_SKILL_TOOL]);
        assert_eq!(
            map.get(skills::LOAD_SKILL_TOOL).map(String::as_str),
            Some(skills::LOAD_SKILL_PATH)
        );
    }
}
