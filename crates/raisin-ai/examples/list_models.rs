//! Example demonstrating dynamic model discovery from AI providers.
//!
//! This example shows how to:
//! - List available models from OpenAI, Anthropic, and Ollama
//! - Display model capabilities (chat, tools, vision, embeddings)
//! - Show model metadata (context window, etc.)
//! - Use caching to avoid repeated API calls

use raisin_ai::provider::AIProviderTrait;
use raisin_ai::providers::{
    anthropic::AnthropicProvider, ollama::OllamaProvider, openai::OpenAIProvider,
};
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Dynamic Model Discovery Example\n");
    println!("================================\n");

    // OpenAI Models
    if let Ok(api_key) = env::var("OPENAI_API_KEY") {
        println!("\n--- OpenAI Models ---\n");
        let provider = OpenAIProvider::new(api_key);

        match provider.list_available_models().await {
            Ok(models) => {
                println!("Found {} OpenAI models:\n", models.len());
                for model in models {
                    println!("  {} ({})", model.id, model.name);
                    println!("    Chat: {}", model.capabilities.chat);
                    println!("    Tools: {}", model.capabilities.tools);
                    println!("    Vision: {}", model.capabilities.vision);
                    println!("    Streaming: {}", model.capabilities.streaming);
                    if let Some(ctx) = model.context_window {
                        println!("    Context: {} tokens", ctx);
                    }
                    println!();
                }
            }
            Err(e) => eprintln!("Failed to fetch OpenAI models: {}", e),
        }
    } else {
        println!("Skipping OpenAI (OPENAI_API_KEY not set)");
    }

    // Anthropic Models
    if let Ok(api_key) = env::var("ANTHROPIC_API_KEY") {
        println!("\n--- Anthropic Models ---\n");
        let provider = AnthropicProvider::new(api_key);

        match provider.list_available_models().await {
            Ok(models) => {
                println!("Found {} Anthropic models:\n", models.len());
                for model in models {
                    println!("  {} ({})", model.id, model.name);
                    println!("    Chat: {}", model.capabilities.chat);
                    println!("    Tools: {}", model.capabilities.tools);
                    println!("    Context: {} tokens", model.context_window.unwrap_or(0));
                    if let Some(metadata) = &model.metadata {
                        if let Some(family) = metadata.get("family") {
                            println!("    Family: {}", family);
                        }
                        if let Some(tier) = metadata.get("tier") {
                            println!("    Tier: {}", tier);
                        }
                    }
                    println!();
                }
            }
            Err(e) => eprintln!("Failed to fetch Anthropic models: {}", e),
        }
    } else {
        println!("Skipping Anthropic (ANTHROPIC_API_KEY not set)");
    }

    // Ollama Models (local)
    println!("\n--- Ollama Models (Local) ---\n");
    let provider = OllamaProvider::new();

    match provider.list_available_models().await {
        Ok(models) => {
            if models.is_empty() {
                println!("No models installed. Install models with: ollama pull <model-name>");
            } else {
                println!("Found {} locally installed models:\n", models.len());
                for model in models {
                    println!("  {}", model.id);
                    println!("    Chat: {}", model.capabilities.chat);
                    println!("    Tools: {}", model.capabilities.tools);
                    println!("    Vision: {}", model.capabilities.vision);
                    if let Some(metadata) = &model.metadata {
                        if let Some(size) = metadata.get("size") {
                            if let Some(size_bytes) = size.as_u64() {
                                println!("    Size: {:.2} GB", size_bytes as f64 / 1_000_000_000.0);
                            }
                        }
                    }
                    println!();
                }
            }
        }
        Err(e) => {
            eprintln!("Failed to fetch Ollama models: {}", e);
            eprintln!("Make sure Ollama is running: https://ollama.ai");
        }
    }

    println!("\n--- Caching Demo ---\n");
    println!("The second call to list_available_models() will use cached results:");

    if let Ok(api_key) = env::var("ANTHROPIC_API_KEY") {
        let provider = AnthropicProvider::new(api_key);

        // First call - fetches from API
        let start = std::time::Instant::now();
        let _ = provider.list_available_models().await?;
        let first_duration = start.elapsed();

        // Second call - uses cache
        let start = std::time::Instant::now();
        let _ = provider.list_available_models().await?;
        let second_duration = start.elapsed();

        println!("First call:  {:?}", first_duration);
        println!("Second call: {:?} (cached)", second_duration);
        println!(
            "Cache speedup: {:.1}x",
            first_duration.as_secs_f64() / second_duration.as_secs_f64()
        );
    }

    Ok(())
}
