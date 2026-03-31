//! Azure OpenAI Service provider implementation.
//!
//! Provides chat completions and tool calling using Azure's hosted OpenAI models.
//!
//! Key differences from standard OpenAI:
//! - Base URL format: `https://{resource}.openai.azure.com/openai/deployments/{deployment}`
//! - API version query parameter: `?api-version=2024-02-15-preview`
//! - API key header: `api-key` instead of `Authorization: Bearer`
//!
//! Supported models (deployment names match OpenAI model names):
//! - gpt-4o, gpt-4-turbo, gpt-4, gpt-35-turbo

#[cfg(test)]
mod tests;
mod trait_impl;
pub(crate) mod types;

use super::http_helpers::SecretKey;
use crate::model_cache::{ModelCache, ModelCapabilities, ModelInfo};
use crate::types::{Message, Role};
use reqwest::Client;
use std::sync::Arc;
use std::time::Duration;

use types::*;

const DEFAULT_API_VERSION: &str = "2024-02-15-preview";
const MODEL_CACHE_TTL: Duration = Duration::from_secs(3600); // 1 hour

/// Azure OpenAI provider configuration
#[derive(Debug, Clone)]
pub struct AzureOpenAIProvider {
    api_key: SecretKey,
    client: Client,
    /// Azure OpenAI endpoint (e.g., https://my-resource.openai.azure.com)
    endpoint: String,
    /// API version to use
    api_version: String,
    cache: Arc<ModelCache>,
}

impl AzureOpenAIProvider {
    /// Creates a new Azure OpenAI provider.
    ///
    /// # Arguments
    /// * `api_key` - Azure OpenAI API key
    /// * `endpoint` - Azure OpenAI endpoint (e.g., https://my-resource.openai.azure.com)
    pub fn new(api_key: impl Into<String>, endpoint: impl Into<String>) -> Self {
        Self {
            api_key: SecretKey::new(api_key),
            client: super::http_helpers::build_client(),
            endpoint: endpoint.into().trim_end_matches('/').to_string(),
            api_version: DEFAULT_API_VERSION.to_string(),
            cache: Arc::new(ModelCache::with_ttl(MODEL_CACHE_TTL)),
        }
    }

    /// Creates a new Azure OpenAI provider with a custom API version.
    pub fn with_api_version(
        api_key: impl Into<String>,
        endpoint: impl Into<String>,
        api_version: impl Into<String>,
    ) -> Self {
        Self {
            api_key: SecretKey::new(api_key),
            client: super::http_helpers::build_client(),
            endpoint: endpoint.into().trim_end_matches('/').to_string(),
            api_version: api_version.into(),
            cache: Arc::new(ModelCache::with_ttl(MODEL_CACHE_TTL)),
        }
    }

    /// Builds model info based on deployment name
    fn build_model_info(&self, deployment: &str) -> ModelInfo {
        let supports_tools =
            !deployment.contains("35-turbo-instruct") && !deployment.starts_with("text-");

        let supports_vision = deployment.contains("gpt-4o")
            || deployment.contains("gpt-4-turbo")
            || deployment.contains("vision");

        let context_window = if deployment.contains("gpt-4o") || deployment.contains("gpt-4-turbo")
        {
            128000
        } else if deployment.contains("gpt-4-32k") {
            32768
        } else if deployment.contains("gpt-4") {
            8192
        } else if deployment.contains("gpt-35-turbo-16k") {
            16384
        } else {
            4096
        };

        let capabilities = ModelCapabilities {
            chat: true,
            embeddings: deployment.contains("embedding"),
            vision: supports_vision,
            tools: supports_tools,
            streaming: true,
        };

        ModelInfo::new(deployment, deployment)
            .with_capabilities(capabilities)
            .with_context_window(context_window)
    }

    /// Converts our Message type to Azure OpenAI chat format
    fn convert_message(msg: &Message) -> AzureChatMessage {
        match msg.role {
            Role::User => AzureChatMessage::User {
                content: msg.content.clone(),
            },
            Role::Assistant => {
                if let Some(tool_calls) = &msg.tool_calls {
                    let azure_tool_calls: Vec<AzureToolCall> = tool_calls
                        .iter()
                        .map(|tc| AzureToolCall {
                            id: tc.id.clone(),
                            call_type: "function".to_string(),
                            function: AzureFunctionCall {
                                name: tc.function.name.clone(),
                                arguments: tc.function.arguments.clone(),
                            },
                        })
                        .collect();

                    AzureChatMessage::AssistantWithTools {
                        content: if msg.content.is_empty() {
                            None
                        } else {
                            Some(msg.content.clone())
                        },
                        tool_calls: azure_tool_calls,
                    }
                } else {
                    AzureChatMessage::Assistant {
                        content: msg.content.clone(),
                    }
                }
            }
            Role::System => AzureChatMessage::System {
                content: msg.content.clone(),
            },
            Role::Tool => AzureChatMessage::Tool {
                tool_call_id: msg.tool_call_id.clone().unwrap_or_default(),
                content: msg.content.clone(),
            },
        }
    }

    /// Converts our ToolDefinition to Azure OpenAI format
    fn convert_tools(tools: &Option<Vec<crate::types::ToolDefinition>>) -> Option<Vec<AzureTool>> {
        tools.as_ref().map(|tool_defs| {
            tool_defs
                .iter()
                .map(|tool| AzureTool {
                    tool_type: "function".to_string(),
                    function: AzureFunctionDefinition {
                        name: tool.function.name.clone(),
                        description: Some(tool.function.description.clone()),
                        parameters: tool.function.parameters.clone(),
                    },
                })
                .collect()
        })
    }
}
