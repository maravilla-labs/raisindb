//! Integration tests for Moondream (`vikhyatk/moondream2`) image captioning.
//!
//! Moondream is a promptable vision-language model that generates captions
//! based on user prompts. This enables separate alt-text vs description generation.
//!
//! Run with: `cargo test -p raisin-ai --features integration-tests`

#![cfg(feature = "integration-tests")]

use candle_core::Device;
use raisin_ai::candle::moondream::{
    is_moondream_model, MoondreamCaptioner, ALT_TEXT_PROMPT, DEFAULT_MOONDREAM_MODEL,
    DESCRIPTION_PROMPT, KEYWORDS_PROMPT, MOONDREAM2_MODEL, MOONDREAM_IMAGE_SIZE,
    QUANTIZED_MOONDREAM_MODEL,
};
use raisin_ai::candle::{
    default_caption_model, is_blip_model, CandleError, AVAILABLE_CAPTION_MODELS,
};
use std::path::PathBuf;

/// Verify the Moondream model constant is correct.
#[test]
fn test_moondream_model_constants() {
    // Default is the quantized candle-compatible model
    assert_eq!(DEFAULT_MOONDREAM_MODEL, "santiagomed/candle-moondream");
    assert_eq!(QUANTIZED_MOONDREAM_MODEL, "santiagomed/candle-moondream");
    // Original moondream2 is available but requires tensor name mapping
    assert_eq!(MOONDREAM2_MODEL, "vikhyatk/moondream2");
    assert_eq!(MOONDREAM_IMAGE_SIZE, 378);
    assert!(!ALT_TEXT_PROMPT.is_empty());
    assert!(!DESCRIPTION_PROMPT.is_empty());
}

/// Verify Moondream model ID detection.
#[test]
fn test_is_moondream_model() {
    // Should match Moondream models
    assert!(is_moondream_model("vikhyatk/moondream2"));
    assert!(is_moondream_model("santiagomed/candle-moondream"));
    assert!(is_moondream_model("some/moondream-variant"));

    // Should NOT match BLIP models
    assert!(!is_moondream_model(
        "Salesforce/blip-image-captioning-large"
    ));
    assert!(!is_moondream_model("lmz/candle-blip"));

    // Should NOT match other models
    assert!(!is_moondream_model("openai/clip-vit-base-patch32"));
    assert!(!is_moondream_model("microsoft/git-large-coco"));
}

/// Verify the model registry correctly identifies Moondream models.
#[test]
fn test_model_registry_identifies_moondream() {
    // Find Moondream in the registry
    let moondream_model = AVAILABLE_CAPTION_MODELS
        .iter()
        .find(|m| m.id == DEFAULT_MOONDREAM_MODEL);

    assert!(
        moondream_model.is_some(),
        "Moondream should be in the model registry"
    );
    let moondream = moondream_model.unwrap();
    assert!(
        moondream.supported,
        "Moondream should be marked as supported"
    );
    assert!(
        is_moondream_model(moondream.id),
        "Moondream model should be detected as Moondream"
    );
    assert!(
        !is_blip_model(moondream.id),
        "Moondream model should not be detected as BLIP"
    );
}

/// Verify Moondream is now the default caption model.
#[test]
fn test_default_caption_model_is_moondream() {
    let default = default_caption_model();
    assert!(
        is_moondream_model(default),
        "Default caption model should be Moondream, got: {}",
        default
    );
    assert_eq!(
        default, DEFAULT_MOONDREAM_MODEL,
        "Default should be vikhyatk/moondream2"
    );
}

/// Verify the BLIP models are still available as fallback.
#[test]
fn test_blip_models_still_available() {
    let blip_models: Vec<_> = AVAILABLE_CAPTION_MODELS
        .iter()
        .filter(|m| is_blip_model(m.id))
        .collect();

    assert!(
        blip_models.len() >= 2,
        "Should have at least 2 BLIP models as fallback"
    );

    for model in blip_models {
        assert!(
            model.supported,
            "BLIP model {} should be supported",
            model.id
        );
    }
}

/// Verify that alt-text and description prompts are different.
#[test]
fn test_prompts_are_different() {
    assert_ne!(
        ALT_TEXT_PROMPT, DESCRIPTION_PROMPT,
        "Alt-text and description prompts should be different"
    );
    assert!(
        ALT_TEXT_PROMPT.contains("briefly") || ALT_TEXT_PROMPT.contains("one sentence"),
        "Alt-text prompt should encourage brevity"
    );
    assert!(
        DESCRIPTION_PROMPT.contains("detail"),
        "Description prompt should encourage detail"
    );
}

/// Verify the keywords prompt exists and is properly configured.
#[test]
fn test_keywords_prompt_exists() {
    assert!(
        !KEYWORDS_PROMPT.is_empty(),
        "Keywords prompt should not be empty"
    );
    assert!(
        KEYWORDS_PROMPT.contains("keyword"),
        "Keywords prompt should mention keywords"
    );
    assert!(
        KEYWORDS_PROMPT.contains("comma") || KEYWORDS_PROMPT.contains(","),
        "Keywords prompt should request comma-separated format"
    );
}

/// Test Moondream captioner construction fails gracefully without model files.
#[test]
fn test_moondream_captioner_not_found() {
    let model_path = PathBuf::from("/tmp/nonexistent-moondream-model");
    let result = MoondreamCaptioner::new(&model_path, Device::Cpu);

    assert!(result.is_err(), "Should fail when model not found");

    if let Err(CandleError::ModelNotDownloaded(msg)) = result {
        assert!(
            msg.contains("not found") || msg.contains("Tokenizer"),
            "Error should mention model/tokenizer not found, got: {}",
            msg
        );
    } else {
        // May also fail with ModelLoad error depending on what's missing
        let err = result.unwrap_err();
        println!("Got error: {:?}", err);
    }
}

/// Future test for when Moondream model is downloaded.
/// This test downloads the model and generates captions.
#[tokio::test]
#[ignore = "Requires ~3.6GB model download - run manually"]
async fn test_moondream_model_download() {
    use raisin_ai::huggingface::ModelRegistry;

    let registry = ModelRegistry::new().expect("Failed to create registry");
    let model_path = registry
        .download_model(DEFAULT_MOONDREAM_MODEL, None)
        .await
        .expect("Failed to download Moondream model");

    assert!(model_path.exists());
    assert!(model_path.join("tokenizer.json").exists());
}

/// Future test for Moondream caption generation.
/// This test requires the model to be downloaded first.
#[tokio::test]
#[ignore = "Requires model download - run manually"]
async fn test_moondream_generate_caption() {
    use raisin_ai::huggingface::ModelRegistry;

    let registry = ModelRegistry::new().expect("Failed to create registry");

    // Ensure model is downloaded
    let model_path = if registry.is_model_ready(DEFAULT_MOONDREAM_MODEL).await {
        registry.model_path(DEFAULT_MOONDREAM_MODEL)
    } else {
        registry
            .download_model(DEFAULT_MOONDREAM_MODEL, None)
            .await
            .expect("Failed to download model")
    };

    // Create test image (red square)
    let test_image = create_test_image();

    // Load captioner
    let device = raisin_ai::candle::select_device(false).expect("Failed to select device");
    let mut captioner =
        MoondreamCaptioner::new(&model_path, device).expect("Failed to load Moondream");

    // Generate description
    let description = captioner
        .generate_description(&test_image)
        .expect("Failed to generate description");
    assert!(!description.is_empty(), "Description should not be empty");
    println!("Description: {}", description);

    // Generate alt-text
    let alt_text = captioner
        .generate_alt_text(&test_image)
        .expect("Failed to generate alt-text");
    assert!(!alt_text.is_empty(), "Alt-text should not be empty");
    assert!(
        alt_text.len() <= 128,
        "Alt-text should be concise (<=128 chars)"
    );
    println!("Alt-text: {}", alt_text);
}

/// Future test for custom prompt support.
#[tokio::test]
#[ignore = "Requires model download - run manually"]
async fn test_moondream_custom_prompt() {
    use raisin_ai::huggingface::ModelRegistry;

    let registry = ModelRegistry::new().expect("Failed to create registry");

    let model_path = if registry.is_model_ready(DEFAULT_MOONDREAM_MODEL).await {
        registry.model_path(DEFAULT_MOONDREAM_MODEL)
    } else {
        registry
            .download_model(DEFAULT_MOONDREAM_MODEL, None)
            .await
            .expect("Failed to download model")
    };

    let test_image = create_test_image();

    let device = raisin_ai::select_device(true).expect("Failed to select device");
    let mut captioner =
        MoondreamCaptioner::new(&model_path, device).expect("Failed to load Moondream");

    // Test custom prompt
    let custom_prompt = "What colors do you see in this image?";
    let response = captioner
        .caption_with_prompt(&test_image, custom_prompt)
        .expect("Failed with custom prompt");

    assert!(
        !response.is_empty(),
        "Custom prompt response should not be empty"
    );
    println!("Custom prompt response: {}", response);
}

/// Integration test for keyword generation.
/// This test downloads the model and generates keywords from an image.
#[tokio::test]
#[ignore = "Requires model download - run manually"]
async fn test_moondream_generate_keywords() {
    use raisin_ai::huggingface::ModelRegistry;

    let registry = ModelRegistry::new().expect("Failed to create registry");

    // Ensure model is downloaded
    let model_path = if registry.is_model_ready(DEFAULT_MOONDREAM_MODEL).await {
        registry.model_path(DEFAULT_MOONDREAM_MODEL)
    } else {
        registry
            .download_model(DEFAULT_MOONDREAM_MODEL, None)
            .await
            .expect("Failed to download model")
    };

    // Create test image (colorful gradient for more interesting keywords)
    let test_image = create_gradient_image();

    // Load captioner
    let device = raisin_ai::candle::select_device(false).expect("Failed to select device");
    let mut captioner =
        MoondreamCaptioner::new(&model_path, device).expect("Failed to load Moondream");

    // Generate keywords using default prompt
    let keywords = captioner
        .generate_keywords(&test_image)
        .expect("Failed to generate keywords");

    assert!(!keywords.is_empty(), "Keywords should not be empty");
    println!("Generated keywords: {:?}", keywords);

    // Keywords should be reasonable length (parsing truncates long entries)
    for keyword in &keywords {
        assert!(
            keyword.len() <= 50,
            "Each keyword/phrase should be truncated to 50 chars max, got: {} (len={})",
            keyword,
            keyword.len()
        );
    }

    // Test with custom prompt
    let custom_keywords = captioner
        .generate_keywords_with_prompt(
            &test_image,
            "List the main colors in this image, separated by commas.",
        )
        .expect("Failed to generate keywords with custom prompt");

    assert!(
        !custom_keywords.is_empty(),
        "Custom keywords should not be empty"
    );
    println!("Custom prompt keywords: {:?}", custom_keywords);
}

/// Integration test for full caption pipeline (description + alt-text + keywords).
#[tokio::test]
#[ignore = "Requires model download - run manually"]
async fn test_moondream_full_pipeline() {
    use raisin_ai::huggingface::ModelRegistry;

    let registry = ModelRegistry::new().expect("Failed to create registry");

    let model_path = if registry.is_model_ready(DEFAULT_MOONDREAM_MODEL).await {
        registry.model_path(DEFAULT_MOONDREAM_MODEL)
    } else {
        registry
            .download_model(DEFAULT_MOONDREAM_MODEL, None)
            .await
            .expect("Failed to download model")
    };

    let test_image = create_gradient_image();

    let device = raisin_ai::select_device(true).expect("Failed to select device");
    let mut captioner =
        MoondreamCaptioner::new(&model_path, device).expect("Failed to load Moondream");

    // Generate all three outputs
    let description = captioner
        .generate_description(&test_image)
        .expect("Failed to generate description");
    let alt_text = captioner
        .generate_alt_text(&test_image)
        .expect("Failed to generate alt-text");
    let keywords = captioner
        .generate_keywords(&test_image)
        .expect("Failed to generate keywords");

    println!("=== Full Pipeline Results ===");
    println!("Description: {}", description);
    println!("Alt-text: {}", alt_text);
    println!("Keywords: {:?}", keywords);

    // All should have content
    assert!(!description.is_empty(), "Description should not be empty");
    assert!(!alt_text.is_empty(), "Alt-text should not be empty");
    assert!(!keywords.is_empty(), "Keywords should not be empty");

    // Alt-text should be shorter than description (it's meant to be concise)
    assert!(
        alt_text.len() <= 128,
        "Alt-text should be concise (<=128 chars), got {} chars",
        alt_text.len()
    );
}

/// Helper function to create a simple test image (red square).
#[cfg(feature = "integration-tests")]
fn create_test_image() -> Vec<u8> {
    use image::{DynamicImage, Rgb, RgbImage};

    let mut img = RgbImage::new(100, 100);
    for y in 0..100 {
        for x in 0..100 {
            img.put_pixel(x, y, Rgb([255, 0, 0])); // Red pixel
        }
    }

    let mut buffer = std::io::Cursor::new(Vec::new());
    let dynamic = DynamicImage::ImageRgb8(img);
    dynamic
        .write_to(&mut buffer, image::ImageFormat::Jpeg)
        .expect("Failed to encode JPEG");

    buffer.into_inner()
}

/// Helper function to create a colorful gradient test image.
/// More interesting for keyword extraction than a solid color.
#[cfg(feature = "integration-tests")]
fn create_gradient_image() -> Vec<u8> {
    use image::{DynamicImage, Rgb, RgbImage};

    let mut img = RgbImage::new(200, 200);
    for y in 0..200 {
        for x in 0..200 {
            let r = ((x * 255) / 200) as u8;
            let g = ((y * 255) / 200) as u8;
            let b = 128u8;
            img.put_pixel(x, y, Rgb([r, g, b]));
        }
    }

    let mut buffer = std::io::Cursor::new(Vec::new());
    let dynamic = DynamicImage::ImageRgb8(img);
    dynamic
        .write_to(&mut buffer, image::ImageFormat::Jpeg)
        .expect("Failed to encode JPEG");

    buffer.into_inner()
}
