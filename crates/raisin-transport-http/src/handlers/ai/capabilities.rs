// SPDX-License-Identifier: BSL-1.1

//! Per-provider model capabilities query handler.
//!
//! Returns detailed capabilities (chat, vision, tools, embeddings,
//! streaming) for a specific model from a given provider. Falls back
//! to heuristics based on the model name when the provider API is
//! unreachable or the model is not in the model list.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};

use raisin_ai::{
    config::AIProvider,
    crypto::ApiKeyEncryptor,
    provider::AIProviderTrait,
    providers::{
        AnthropicProvider, AzureOpenAIProvider, BedrockProvider, GeminiProvider, GroqProvider,
        OllamaProvider, OpenAIProvider, OpenRouterProvider,
    },
    storage::TenantAIConfigStore,
};

use crate::state::AppState;

use super::types::{CapabilitiesInfo, ErrorResponse, ModelCapabilitiesResponse};

/// Get capabilities for a specific model from a provider.
///
/// GET /api/tenants/{tenant_id}/ai/providers/{slug}/models/{model}/capabilities
///
/// Returns detailed capabilities information for a model, including whether it supports
/// tool calling. This endpoint queries the provider's model list and caches results.
///
/// The path segment is a provider SLUG. It resolves to one entry, and the entry's kind
/// picks the helper below — so two gateways with different endpoints report their own
/// capabilities instead of sharing whichever entry came first. Existing URLs like
/// `.../providers/ollama/...` keep working: pre-slug entries carry their kind's serde
/// name as their slug.
#[axum::debug_handler]
pub async fn get_model_capabilities(
    Path((tenant_id, slug, model_id)): Path<(String, String, String)>,
    State(state): State<AppState>,
) -> Result<Json<ModelCapabilitiesResponse>, (StatusCode, Json<ErrorResponse>)> {
    let repo_impl = state.storage().tenant_ai_config_repository();

    // "No config at all" and "a config with no entry under this slug" are different
    // cases here — see the fallback below — so keep them apart.
    let stored_config = match repo_impl.get_config(&tenant_id).await {
        Ok(config) => Some(config),
        Err(raisin_ai::storage::StorageError::NotFound(_)) => None,
        Err(e) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Storage error: {}", e),
                }),
            ));
        }
    };

    let provider_config = stored_config
        .as_ref()
        .and_then(|config| config.get_provider(&slug).cloned());

    let provider = match &provider_config {
        Some(pc) => pc.kind,
        // With no entry under this slug there is still a kind to answer for, as long as
        // the slug is one of the kind names: the helpers below fall back to name
        // heuristics when they have no key and no endpoint. That is what a tenant with
        // a config but no entry for the queried kind used to get, and it is what lets
        // the console describe a model before that provider has ever been saved.
        None => match AIProvider::from_serde_name(&slug) {
            // Ollama also answers for a tenant with no config at all: it is the one
            // kind that needs neither a credential nor a stored endpoint.
            Some(kind) if stored_config.is_some() || kind == AIProvider::Ollama => kind,
            _ => {
                return Err((
                    StatusCode::NOT_FOUND,
                    Json(ErrorResponse {
                        error: format!("Provider '{}' not configured", slug),
                    }),
                ));
            }
        },
    };

    // Decrypt API key if present
    let api_key = if let Some(encrypted) = provider_config
        .as_ref()
        .and_then(|pc| pc.api_key_encrypted.as_ref())
    {
        let master_key = state.get_master_key().map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Master key not configured: {}", e),
                }),
            )
        })?;
        let encryptor = ApiKeyEncryptor::new(&master_key);
        Some(encryptor.decrypt(encrypted).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to decrypt API key: {}", e),
                }),
            )
        })?)
    } else {
        None
    };

    let endpoint = provider_config
        .as_ref()
        .and_then(|pc| pc.api_endpoint.as_deref());

    // Query provider for model capabilities. The helpers know nothing about slugs, so
    // the slug is stamped onto the response once, here, on the way out.
    let response = match provider {
        AIProvider::OpenAI => {
            get_openai_model_capabilities(&model_id, api_key.as_deref(), endpoint).await
        }
        AIProvider::Anthropic => {
            get_anthropic_model_capabilities(&model_id, api_key.as_deref()).await
        }
        AIProvider::Google => {
            get_gemini_model_capabilities(&model_id, api_key.as_deref(), endpoint).await
        }
        AIProvider::AzureOpenAI => {
            get_azure_openai_model_capabilities(&model_id, api_key.as_deref(), endpoint).await
        }
        AIProvider::Ollama => get_ollama_model_capabilities(&model_id, endpoint).await,
        AIProvider::Groq => {
            get_groq_model_capabilities(&model_id, api_key.as_deref(), endpoint).await
        }
        AIProvider::OpenRouter => {
            get_openrouter_model_capabilities(&model_id, api_key.as_deref(), endpoint).await
        }
        AIProvider::Bedrock => {
            get_bedrock_model_capabilities(&model_id, api_key.as_deref(), endpoint).await
        }
        AIProvider::Local => Ok(Json(ModelCapabilitiesResponse {
            model_id: model_id.clone(),
            provider,
            provider_slug: String::new(),
            capabilities: local_model_capabilities(&model_id),
        })),
        AIProvider::Custom => Ok(Json(ModelCapabilitiesResponse {
            model_id: model_id.clone(),
            provider,
            provider_slug: String::new(),
            // An OpenAI-compatible endpoint serves `/embeddings` and supports tool calls
            // as part of the shape it implements. Reporting `tools: false` here made the
            // console hide tool configuration for agents that would have worked.
            capabilities: CapabilitiesInfo {
                chat: true,
                embeddings: true,
                vision: false,
                tools: true,
                streaming: true,
            },
        })),
    };

    response.map(|mut r| {
        r.0.provider_slug = slug;
        r
    })
}

/// Local Candle model capabilities lookup.
fn local_model_capabilities(model_id: &str) -> CapabilitiesInfo {
    match model_id {
        "moondream" => CapabilitiesInfo {
            chat: true,
            embeddings: false,
            vision: true,
            tools: false,
            streaming: false,
        },
        "blip" => CapabilitiesInfo {
            chat: false,
            embeddings: false,
            vision: true,
            tools: false,
            streaming: false,
        },
        "clip" => CapabilitiesInfo {
            chat: false,
            embeddings: true,
            vision: true,
            tools: false,
            streaming: false,
        },
        _ => CapabilitiesInfo {
            chat: false,
            embeddings: false,
            vision: false,
            tools: false,
            streaming: false,
        },
    }
}

// ============================================================================
// Per-provider capabilities helpers
// ============================================================================

/// Helper: build a `ModelCapabilitiesResponse` from a provider's model list or heuristics.
macro_rules! capabilities_from_provider_or_heuristic {
    ($provider_enum:expr, $model_id:expr, $provider_instance:expr, $heuristic:expr) => {{
        match $provider_instance.list_available_models().await {
            Ok(models) => {
                if let Some(model) = models.iter().find(|m| m.id == $model_id) {
                    Ok(Json(ModelCapabilitiesResponse {
                        model_id: $model_id.to_string(),
                        provider: $provider_enum,
                        provider_slug: String::new(),
                        capabilities: CapabilitiesInfo {
                            chat: model.capabilities.chat,
                            embeddings: model.capabilities.embeddings,
                            vision: model.capabilities.vision,
                            tools: model.capabilities.tools,
                            streaming: model.capabilities.streaming,
                        },
                    }))
                } else {
                    Ok(Json(ModelCapabilitiesResponse {
                        model_id: $model_id.to_string(),
                        provider: $provider_enum,
                        provider_slug: String::new(),
                        capabilities: $heuristic,
                    }))
                }
            }
            Err(e) => {
                tracing::warn!("Failed to fetch {:?} models: {}", $provider_enum, e);
                Ok(Json(ModelCapabilitiesResponse {
                    model_id: $model_id.to_string(),
                    provider: $provider_enum,
                    provider_slug: String::new(),
                    capabilities: $heuristic,
                }))
            }
        }
    }};
}

async fn get_openai_model_capabilities(
    model_id: &str,
    api_key: Option<&str>,
    endpoint: Option<&str>,
) -> Result<Json<ModelCapabilitiesResponse>, (StatusCode, Json<ErrorResponse>)> {
    let provider = match endpoint {
        Some(url) => OpenAIProvider::with_base_url(api_key.unwrap_or_default(), url),
        None => OpenAIProvider::new(api_key.unwrap_or_default()),
    };
    let tools_supported = !model_id.contains("o1") && !model_id.contains("gpt-3.5");
    let heuristic = CapabilitiesInfo {
        chat: true,
        embeddings: model_id.contains("embedding"),
        vision: model_id.contains("vision") || model_id.contains("gpt-4"),
        tools: tools_supported,
        streaming: true,
    };
    capabilities_from_provider_or_heuristic!(AIProvider::OpenAI, model_id, provider, heuristic)
}

async fn get_anthropic_model_capabilities(
    model_id: &str,
    api_key: Option<&str>,
) -> Result<Json<ModelCapabilitiesResponse>, (StatusCode, Json<ErrorResponse>)> {
    let provider = AnthropicProvider::new(api_key.unwrap_or_default());
    let is_claude_3_plus = model_id.contains("claude-3")
        || model_id.contains("claude-opus")
        || model_id.contains("claude-sonnet");
    let heuristic = CapabilitiesInfo {
        chat: true,
        embeddings: false,
        vision: is_claude_3_plus,
        tools: is_claude_3_plus,
        streaming: true,
    };
    capabilities_from_provider_or_heuristic!(AIProvider::Anthropic, model_id, provider, heuristic)
}

async fn get_ollama_model_capabilities(
    model_id: &str,
    endpoint: Option<&str>,
) -> Result<Json<ModelCapabilitiesResponse>, (StatusCode, Json<ErrorResponse>)> {
    let provider = match endpoint {
        Some(url) => OllamaProvider::with_base_url(url),
        None => OllamaProvider::new(),
    };
    let heuristic = ollama_heuristic_capabilities(model_id);
    capabilities_from_provider_or_heuristic!(AIProvider::Ollama, model_id, provider, heuristic)
}

/// Ollama-specific heuristic capabilities based on model name patterns.
fn ollama_heuristic_capabilities(model_id: &str) -> CapabilitiesInfo {
    let base_name = model_id
        .split(':')
        .next()
        .unwrap_or(model_id)
        .to_lowercase();
    let tool_capable_models = [
        "llama3.2",
        "llama3.3",
        "qwen2.5",
        "qwen2.5-coder",
        "mistral",
        "mistral-nemo",
        "mixtral",
        "gemma2",
        "command-r",
        "command-r-plus",
        "firefunction",
        "hermes",
        "hermes3",
        "athene",
        "phi4",
        "granite",
        "deepseek-r1-tool",
        "deepseek-coder",
        "marco",
    ];
    let supports_tools = tool_capable_models
        .iter()
        .any(|&m| base_name.starts_with(m) || base_name.contains(m));

    CapabilitiesInfo {
        chat: true,
        embeddings: false,
        vision: model_id.contains("vision") || model_id.contains("llava"),
        tools: supports_tools,
        streaming: true,
    }
}

async fn get_gemini_model_capabilities(
    model_id: &str,
    api_key: Option<&str>,
    endpoint: Option<&str>,
) -> Result<Json<ModelCapabilitiesResponse>, (StatusCode, Json<ErrorResponse>)> {
    let provider = match endpoint {
        Some(url) => GeminiProvider::with_base_url(api_key.unwrap_or_default(), url),
        None => GeminiProvider::new(api_key.unwrap_or_default()),
    };
    let supports_tools = model_id.contains("gemini-1.5") || model_id.contains("gemini-2");
    let supports_vision = model_id.contains("pro") || model_id.contains("flash");
    let heuristic = CapabilitiesInfo {
        chat: true,
        embeddings: model_id.contains("embedding"),
        vision: supports_vision,
        tools: supports_tools,
        streaming: true,
    };
    capabilities_from_provider_or_heuristic!(AIProvider::Google, model_id, provider, heuristic)
}

async fn get_azure_openai_model_capabilities(
    model_id: &str,
    api_key: Option<&str>,
    endpoint: Option<&str>,
) -> Result<Json<ModelCapabilitiesResponse>, (StatusCode, Json<ErrorResponse>)> {
    if let Some(azure_endpoint) = endpoint {
        let provider = AzureOpenAIProvider::new(api_key.unwrap_or_default(), azure_endpoint);
        let supports_tools =
            !model_id.contains("35-turbo-instruct") && !model_id.starts_with("text-");
        let supports_vision = model_id.contains("gpt-4o")
            || model_id.contains("gpt-4-turbo")
            || model_id.contains("vision");
        let heuristic = CapabilitiesInfo {
            chat: true,
            embeddings: model_id.contains("embedding"),
            vision: supports_vision,
            tools: supports_tools,
            streaming: true,
        };
        capabilities_from_provider_or_heuristic!(
            AIProvider::AzureOpenAI,
            model_id,
            provider,
            heuristic
        )
    } else {
        Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Azure OpenAI requires a custom endpoint".to_string(),
            }),
        ))
    }
}

async fn get_groq_model_capabilities(
    model_id: &str,
    api_key: Option<&str>,
    endpoint: Option<&str>,
) -> Result<Json<ModelCapabilitiesResponse>, (StatusCode, Json<ErrorResponse>)> {
    let provider = match endpoint {
        Some(url) => GroqProvider::with_base_url(api_key.unwrap_or_default(), url),
        None => GroqProvider::new(api_key.unwrap_or_default()),
    };
    // Per Groq's docs all chat/LLM models support tools; only STT/TTS/guard
    // models don't. Share the provider's classifier so this agrees with the list.
    let heuristic = CapabilitiesInfo {
        chat: true,
        embeddings: false,
        vision: model_id.contains("vision"),
        tools: raisin_ai::providers::groq_model_supports_tools(model_id),
        streaming: true,
    };
    capabilities_from_provider_or_heuristic!(AIProvider::Groq, model_id, provider, heuristic)
}

async fn get_openrouter_model_capabilities(
    model_id: &str,
    api_key: Option<&str>,
    endpoint: Option<&str>,
) -> Result<Json<ModelCapabilitiesResponse>, (StatusCode, Json<ErrorResponse>)> {
    let provider = match endpoint {
        Some(url) => OpenRouterProvider::with_base_url(api_key.unwrap_or_default(), url),
        None => OpenRouterProvider::new(api_key.unwrap_or_default()),
    };
    let supports_tools = model_id.contains("gpt")
        || model_id.contains("claude")
        || model_id.contains("gemini")
        || model_id.contains("llama");
    let supports_vision =
        model_id.contains("vision") || model_id.contains("gpt-4o") || model_id.contains("claude-3");
    let heuristic = CapabilitiesInfo {
        chat: true,
        embeddings: false,
        vision: supports_vision,
        tools: supports_tools,
        streaming: true,
    };
    capabilities_from_provider_or_heuristic!(AIProvider::OpenRouter, model_id, provider, heuristic)
}

async fn get_bedrock_model_capabilities(
    model_id: &str,
    api_key: Option<&str>,
    endpoint: Option<&str>,
) -> Result<Json<ModelCapabilitiesResponse>, (StatusCode, Json<ErrorResponse>)> {
    if let Some(region) = endpoint {
        let key = api_key.unwrap_or_default();
        let parts: Vec<&str> = key.splitn(2, ':').collect();
        if parts.len() == 2 {
            let provider = BedrockProvider::new(region, parts[0], parts[1]);
            let supports_tools = model_id.contains("claude")
                || model_id.contains("nova")
                || model_id.contains("llama");
            let supports_vision = model_id.contains("claude-3") || model_id.contains("nova");
            let heuristic = CapabilitiesInfo {
                chat: true,
                embeddings: model_id.contains("embed"),
                vision: supports_vision,
                tools: supports_tools,
                streaming: true,
            };
            capabilities_from_provider_or_heuristic!(
                AIProvider::Bedrock,
                model_id,
                provider,
                heuristic
            )
        } else {
            Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error:
                        "AWS Bedrock api_key must be in format 'access_key_id:secret_access_key'"
                            .to_string(),
                }),
            ))
        }
    } else {
        Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "AWS Bedrock requires a region in the endpoint field (e.g., 'us-east-1')"
                    .to_string(),
            }),
        ))
    }
}
