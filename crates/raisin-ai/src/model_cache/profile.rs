//! Extended model profiles with cost tracking and factory methods.

use super::model_info::{ModelCapabilities, ModelInfo, SchemaTransformerType};

/// Extended profile information for an AI model.
///
/// Includes cost tracking, extended thinking support, and structured output capabilities
/// beyond the basic ModelInfo struct.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ModelProfile {
    /// Unique identifier for the model
    pub id: String,

    /// Human-readable name
    pub name: String,

    /// Model capabilities
    pub capabilities: ModelCapabilities,

    /// Context window size (in tokens)
    pub context_window: u32,

    /// Maximum output tokens
    pub max_output_tokens: Option<u32>,

    /// Cost per 1,000 input tokens (in USD)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_per_1k_input: Option<f64>,

    /// Cost per 1,000 output tokens (in USD)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_per_1k_output: Option<f64>,

    /// Extended thinking tags (e.g., ("<thinking>", "</thinking>") for Claude, o-series)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_tags: Option<(String, String)>,

    /// Supports native JSON mode (Claude 3.5+, GPT-4o, etc.)
    pub supports_native_json: bool,

    /// Schema transformation type for structured output
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json_schema_transformer: Option<SchemaTransformerType>,

    /// Whether this model is currently available
    pub available: bool,

    /// Additional metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

impl ModelProfile {
    /// Creates a new ModelProfile with the given ID and name.
    pub fn new(id: impl Into<String>, name: impl Into<String>, context_window: u32) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            capabilities: ModelCapabilities::default(),
            context_window,
            max_output_tokens: None,
            cost_per_1k_input: None,
            cost_per_1k_output: None,
            thinking_tags: None,
            supports_native_json: false,
            json_schema_transformer: None,
            available: true,
            metadata: None,
        }
    }

    /// Sets the model capabilities.
    pub fn with_capabilities(mut self, capabilities: ModelCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }

    /// Sets the maximum output tokens.
    pub fn with_max_output_tokens(mut self, tokens: u32) -> Self {
        self.max_output_tokens = Some(tokens);
        self
    }

    /// Sets the cost per 1,000 input tokens.
    pub fn with_cost_per_1k_input(mut self, cost: f64) -> Self {
        self.cost_per_1k_input = Some(cost);
        self
    }

    /// Sets the cost per 1,000 output tokens.
    pub fn with_cost_per_1k_output(mut self, cost: f64) -> Self {
        self.cost_per_1k_output = Some(cost);
        self
    }

    /// Sets both input and output costs.
    pub fn with_costs(mut self, input_cost: f64, output_cost: f64) -> Self {
        self.cost_per_1k_input = Some(input_cost);
        self.cost_per_1k_output = Some(output_cost);
        self
    }

    /// Sets the thinking tags for extended thinking support.
    pub fn with_thinking_tags(
        mut self,
        open_tag: impl Into<String>,
        close_tag: impl Into<String>,
    ) -> Self {
        self.thinking_tags = Some((open_tag.into(), close_tag.into()));
        self
    }

    /// Sets whether the model supports native JSON mode.
    pub fn with_native_json(mut self, supports: bool) -> Self {
        self.supports_native_json = supports;
        self
    }

    /// Sets the JSON schema transformer type.
    pub fn with_json_schema_transformer(mut self, transformer: SchemaTransformerType) -> Self {
        self.json_schema_transformer = Some(transformer);
        self
    }

    /// Sets whether the model is available.
    pub fn with_availability(mut self, available: bool) -> Self {
        self.available = available;
        self
    }

    /// Sets additional metadata.
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = Some(metadata);
        self
    }

    /// Calculates the total cost for the given token counts.
    ///
    /// Returns `None` if cost information is not available for this model.
    pub fn calculate_cost(&self, input_tokens: u32, output_tokens: u32) -> Option<f64> {
        match (self.cost_per_1k_input, self.cost_per_1k_output) {
            (Some(input_cost), Some(output_cost)) => {
                let input_cost_total = (input_tokens as f64 / 1000.0) * input_cost;
                let output_cost_total = (output_tokens as f64 / 1000.0) * output_cost;
                Some(input_cost_total + output_cost_total)
            }
            _ => None,
        }
    }

    /// Converts this ModelProfile to a basic ModelInfo.
    ///
    /// This is useful for backward compatibility with code expecting ModelInfo.
    pub fn to_model_info(&self) -> ModelInfo {
        ModelInfo {
            id: self.id.clone(),
            name: self.name.clone(),
            description: None,
            capabilities: self.capabilities.clone(),
            context_window: Some(self.context_window),
            max_output_tokens: self.max_output_tokens,
            available: self.available,
            metadata: self.metadata.clone(),
        }
    }

    /// Creates a profile for Claude Sonnet 4.
    pub fn claude_sonnet_4() -> Self {
        Self::new("claude-sonnet-4-20250514", "Claude Sonnet 4", 200_000)
            .with_capabilities(ModelCapabilities {
                chat: true,
                embeddings: false,
                vision: true,
                tools: true,
                streaming: true,
            })
            .with_max_output_tokens(8192)
            .with_costs(3.0, 15.0)
            .with_thinking_tags("<thinking>", "</thinking>")
            .with_native_json(true)
            .with_json_schema_transformer(SchemaTransformerType::Anthropic)
    }

    /// Creates a profile for GPT-4o.
    pub fn gpt_4o() -> Self {
        Self::new("gpt-4o", "GPT-4o", 128_000)
            .with_capabilities(ModelCapabilities {
                chat: true,
                embeddings: false,
                vision: true,
                tools: true,
                streaming: true,
            })
            .with_max_output_tokens(16_384)
            .with_costs(2.5, 10.0)
            .with_native_json(true)
            .with_json_schema_transformer(SchemaTransformerType::OpenAI)
    }

    /// Creates a profile for Llama 3.3 70B.
    pub fn llama_3_3_70b() -> Self {
        Self::new("llama-3.3-70b-versatile", "Llama 3.3 70B", 128_000)
            .with_capabilities(ModelCapabilities {
                chat: true,
                embeddings: false,
                vision: false,
                tools: true,
                streaming: true,
            })
            .with_max_output_tokens(32_768)
            .with_costs(0.59, 0.79)
            .with_native_json(true)
            .with_json_schema_transformer(SchemaTransformerType::None)
    }

    /// Creates a profile for GPT-4 Turbo.
    pub fn gpt_4_turbo() -> Self {
        Self::new("gpt-4-turbo", "GPT-4 Turbo", 128_000)
            .with_capabilities(ModelCapabilities {
                chat: true,
                embeddings: false,
                vision: true,
                tools: true,
                streaming: true,
            })
            .with_max_output_tokens(4096)
            .with_costs(10.0, 30.0)
            .with_native_json(true)
            .with_json_schema_transformer(SchemaTransformerType::OpenAI)
    }

    /// Creates a profile for Claude Opus 4.
    pub fn claude_opus_4() -> Self {
        Self::new("claude-opus-4-20250514", "Claude Opus 4", 200_000)
            .with_capabilities(ModelCapabilities {
                chat: true,
                embeddings: false,
                vision: true,
                tools: true,
                streaming: true,
            })
            .with_max_output_tokens(16_384)
            .with_costs(15.0, 75.0)
            .with_thinking_tags("<thinking>", "</thinking>")
            .with_native_json(true)
            .with_json_schema_transformer(SchemaTransformerType::Anthropic)
    }

    /// Creates a profile for Gemini 2.0 Flash.
    pub fn gemini_2_flash() -> Self {
        Self::new("gemini-2.0-flash-exp", "Gemini 2.0 Flash", 1_048_576)
            .with_capabilities(ModelCapabilities {
                chat: true,
                embeddings: false,
                vision: true,
                tools: true,
                streaming: true,
            })
            .with_max_output_tokens(8192)
            .with_costs(0.075, 0.30)
            .with_native_json(true)
            .with_json_schema_transformer(SchemaTransformerType::Google)
    }
}
