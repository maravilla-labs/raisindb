//! Tests for the local Candle provider.

use super::*;
use crate::provider::AIProviderTrait;

#[test]
fn test_local_model_from_model_id() {
    assert_eq!(
        LocalModel::from_model_id("moondream"),
        Some(LocalModel::Moondream)
    );
    assert_eq!(LocalModel::from_model_id("blip"), Some(LocalModel::Blip));
    assert_eq!(LocalModel::from_model_id("clip"), Some(LocalModel::Clip));

    assert_eq!(
        LocalModel::from_model_id("local:moondream"),
        Some(LocalModel::Moondream)
    );
    assert_eq!(
        LocalModel::from_model_id("local:clip"),
        Some(LocalModel::Clip)
    );

    assert_eq!(
        LocalModel::from_model_id("moondream-quantized"),
        Some(LocalModel::MoondreamQuantized)
    );
    assert_eq!(
        LocalModel::from_model_id("blip-quantized"),
        Some(LocalModel::BlipQuantized)
    );

    assert_eq!(
        LocalModel::from_model_id("moondream2"),
        Some(LocalModel::Moondream)
    );
    assert_eq!(
        LocalModel::from_model_id("blip-large"),
        Some(LocalModel::Blip)
    );
    assert_eq!(
        LocalModel::from_model_id("clip-vit-b-32"),
        Some(LocalModel::Clip)
    );

    assert_eq!(LocalModel::from_model_id("gpt-4"), None);
    assert_eq!(LocalModel::from_model_id("unknown"), None);
}

#[test]
fn test_local_model_capabilities() {
    assert!(LocalModel::Moondream.supports_vision());
    assert!(LocalModel::Blip.supports_vision());
    assert!(LocalModel::Clip.supports_vision());

    assert!(!LocalModel::Moondream.supports_embeddings());
    assert!(!LocalModel::Blip.supports_embeddings());
    assert!(LocalModel::Clip.supports_embeddings());

    assert!(LocalModel::Moondream.is_promptable());
    assert!(!LocalModel::Blip.is_promptable());
    assert!(!LocalModel::Clip.is_promptable());
}

#[test]
fn test_local_model_names() {
    assert_eq!(LocalModel::Moondream.name(), "moondream");
    assert_eq!(LocalModel::MoondreamQuantized.name(), "moondream-quantized");
    assert_eq!(LocalModel::Blip.name(), "blip");
    assert_eq!(LocalModel::BlipQuantized.name(), "blip-quantized");
    assert_eq!(LocalModel::Clip.name(), "clip");
}

#[test]
fn test_provider_name() {
    let provider = LocalCandleProvider::new("/tmp/models");
    assert_eq!(provider.provider_name(), "local");
}

#[test]
fn test_provider_capabilities() {
    let provider = LocalCandleProvider::new("/tmp/models");
    assert!(!provider.supports_streaming());
    assert!(!provider.supports_tools());
}

#[test]
fn test_available_models() {
    let provider = LocalCandleProvider::new("/tmp/models");
    let models = provider.available_models();
    assert!(models.contains(&"moondream".to_string()));
    assert!(models.contains(&"blip".to_string()));
    assert!(models.contains(&"clip".to_string()));
}

#[test]
fn test_extract_prompt_from_messages() {
    use crate::types::Message;

    let messages = vec![Message::user("What is this?")];
    assert_eq!(
        LocalCandleProvider::extract_prompt_from_messages(&messages),
        "What is this?"
    );

    let messages = vec![
        Message::user("Hello"),
        Message::assistant("Hi!"),
        Message::user("Describe this image."),
    ];
    assert_eq!(
        LocalCandleProvider::extract_prompt_from_messages(&messages),
        "Describe this image."
    );

    let messages = vec![Message::assistant("Hi!")];
    assert_eq!(
        LocalCandleProvider::extract_prompt_from_messages(&messages),
        "Describe this image."
    );
}
