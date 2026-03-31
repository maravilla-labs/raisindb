//! AI provider implementations.
//!
//! This module contains implementations for various AI providers:
//! - OpenAI (GPT-4, chat completions)
//! - Anthropic (Claude models, chat completions)
//! - Google Gemini (Gemini models, chat completions with tools)
//! - Ollama (local models, chat completions)
//! - Azure OpenAI (Azure-hosted OpenAI models)
//! - AWS Bedrock (Claude, Nova, Llama models via AWS)
//! - OpenRouter (multi-provider router with unified API)
//! - Groq (fast inference for open-source models)
//! - Local (Candle-based local inference for Moondream, BLIP, CLIP)

pub mod anthropic;
pub mod azure_openai;
pub mod bedrock;
pub mod gemini;
pub mod groq;
pub mod http_helpers;
pub mod local;
pub mod ollama;
pub mod openai;
pub mod openrouter;
pub mod sse;
pub mod structured_output;

pub use anthropic::AnthropicProvider;
pub use azure_openai::AzureOpenAIProvider;
pub use bedrock::BedrockProvider;
pub use gemini::GeminiProvider;
pub use groq::GroqProvider;
pub use local::LocalCandleProvider;
pub use ollama::OllamaProvider;
pub use openai::OpenAIProvider;
pub use openrouter::OpenRouterProvider;
