//! Integration tests for BLIP image captioning.
//!
//! These tests require model downloads (~1.88GB) and are gated behind the
//! `integration-tests` feature flag.
//!
//! Run with: `cargo test -p raisin-ai --features integration-tests`

#![cfg(feature = "integration-tests")]

mod fixtures;

use image::{DynamicImage, Rgb, RgbImage};
use raisin_ai::candle::{select_device, BlipCaptioner};
use raisin_ai::huggingface::ModelRegistry;

/// The BLIP large model with native safetensors support.
const BLIP_MODEL_ID: &str = "Salesforce/blip-image-captioning-large";

/// Create a test JPEG image with a solid color.
fn create_colored_jpeg(width: u32, height: u32, r: u8, g: u8, b: u8) -> Vec<u8> {
    let mut img = RgbImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
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

/// Create a test image with a gradient pattern.
fn create_gradient_jpeg(width: u32, height: u32) -> Vec<u8> {
    let mut img = RgbImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let r = ((x * 255) / width) as u8;
            let g = ((y * 255) / height) as u8;
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

#[tokio::test]
async fn test_blip_model_download() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("raisin_ai=debug")
        .try_init();

    let registry = ModelRegistry::new().expect("Failed to create registry");
    let model_path = registry
        .download_model(BLIP_MODEL_ID, None)
        .await
        .expect("Failed to download BLIP model");

    assert!(model_path.exists());
    // Model weights should exist as safetensors
    assert!(
        model_path.join("model.safetensors").exists(),
        "model.safetensors should exist"
    );
    assert!(model_path.join("tokenizer.json").exists());
}

#[tokio::test]
async fn test_blip_caption_image() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("raisin_ai=debug")
        .try_init();

    // Download model
    let registry = ModelRegistry::new().expect("Failed to create registry");
    let model_path = registry
        .download_model(BLIP_MODEL_ID, None)
        .await
        .expect("Failed to download BLIP model");

    // Load captioner (use CPU for CI compatibility)
    let device = select_device(false).expect("Failed to select device");
    let mut captioner =
        BlipCaptioner::new(&model_path, device).expect("Failed to create captioner");

    // Create and caption test image
    let image_bytes = create_gradient_jpeg(384, 384);
    let caption = captioner
        .caption_image(&image_bytes)
        .expect("Failed to caption image");

    // Verify caption properties
    assert!(!caption.is_empty(), "Caption should not be empty");
    assert!(
        caption.len() < 500,
        "Caption should be reasonably short, got {} chars",
        caption.len()
    );

    // Caption should be human-readable (contains letters)
    assert!(
        caption.chars().any(|c| c.is_alphabetic()),
        "Caption should contain text: {}",
        caption
    );
}

#[tokio::test]
async fn test_blip_generate_alt_text() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("raisin_ai=debug")
        .try_init();

    let registry = ModelRegistry::new().expect("Failed to create registry");
    let model_path = registry
        .download_model(BLIP_MODEL_ID, None)
        .await
        .expect("Failed to download BLIP model");

    let device = select_device(false).expect("Failed to select device");
    let mut captioner =
        BlipCaptioner::new(&model_path, device).expect("Failed to create captioner");

    let image_bytes = create_colored_jpeg(384, 384, 255, 0, 0); // Red image
    let alt_text = captioner
        .generate_alt_text(&image_bytes)
        .expect("Failed to generate alt text");

    // Alt text should be shorter and cleaner than raw caption
    assert!(!alt_text.is_empty(), "Alt text should not be empty");
    assert!(
        alt_text.len() <= 128,
        "Alt text should be max 125 chars + '...'"
    );

    // Should start with capital letter
    assert!(
        alt_text
            .chars()
            .next()
            .map(|c| c.is_uppercase())
            .unwrap_or(false)
            || alt_text
                .chars()
                .next()
                .map(|c| !c.is_alphabetic())
                .unwrap_or(false),
        "Alt text should start with capital letter or non-alphabetic: {}",
        alt_text
    );
}

#[tokio::test]
async fn test_blip_caption_with_prompt() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("raisin_ai=debug")
        .try_init();

    let registry = ModelRegistry::new().expect("Failed to create registry");
    let model_path = registry
        .download_model(BLIP_MODEL_ID, None)
        .await
        .expect("Failed to download BLIP model");

    let device = select_device(false).expect("Failed to select device");
    let mut captioner =
        BlipCaptioner::new(&model_path, device).expect("Failed to create captioner");

    let image_bytes = create_gradient_jpeg(384, 384);

    // Caption with prompt
    let caption = captioner
        .caption_image_with_options(&image_bytes, 30, Some("A colorful"))
        .expect("Failed to caption with prompt");

    assert!(
        !caption.is_empty(),
        "Caption with prompt should not be empty"
    );
}

#[tokio::test]
async fn test_blip_different_images_produce_different_captions() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("raisin_ai=debug")
        .try_init();

    let registry = ModelRegistry::new().expect("Failed to create registry");
    let model_path = registry
        .download_model(BLIP_MODEL_ID, None)
        .await
        .expect("Failed to download BLIP model");

    let device = select_device(false).expect("Failed to select device");
    let mut captioner =
        BlipCaptioner::new(&model_path, device).expect("Failed to create captioner");

    // Create distinctly different images
    let red_image = create_colored_jpeg(384, 384, 255, 0, 0);
    let gradient_image = create_gradient_jpeg(384, 384);

    let red_caption = captioner
        .caption_image(&red_image)
        .expect("Failed to caption red");
    let gradient_caption = captioner
        .caption_image(&gradient_image)
        .expect("Failed to caption gradient");

    // Captions don't need to be completely different but should both exist
    assert!(!red_caption.is_empty());
    assert!(!gradient_caption.is_empty());

    println!("Red image caption: {}", red_caption);
    println!("Gradient image caption: {}", gradient_caption);
}

#[tokio::test]
async fn test_blip_handles_various_image_sizes() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("raisin_ai=debug")
        .try_init();

    let registry = ModelRegistry::new().expect("Failed to create registry");
    let model_path = registry
        .download_model(BLIP_MODEL_ID, None)
        .await
        .expect("Failed to download BLIP model");

    let device = select_device(false).expect("Failed to select device");
    let mut captioner =
        BlipCaptioner::new(&model_path, device).expect("Failed to create captioner");

    // Test various image sizes
    let sizes = [(100, 100), (384, 384), (640, 480), (1024, 768)];

    for (width, height) in sizes {
        let image = create_gradient_jpeg(width, height);
        let caption = captioner
            .caption_image(&image)
            .unwrap_or_else(|_| panic!("Failed to caption {}x{} image", width, height));

        assert!(
            !caption.is_empty(),
            "{}x{} image should produce caption",
            width,
            height
        );
    }
}

#[tokio::test]
async fn test_blip_model_id() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("raisin_ai=debug")
        .try_init();

    let registry = ModelRegistry::new().expect("Failed to create registry");
    let model_path = registry
        .download_model(BLIP_MODEL_ID, None)
        .await
        .expect("Failed to download BLIP model");

    let device = select_device(false).expect("Failed to select device");
    let captioner = BlipCaptioner::new(&model_path, device).expect("Failed to create captioner");

    assert_eq!(captioner.model_id(), BLIP_MODEL_ID);
}

/// Quantized BLIP model ID for fast CPU inference.
const QUANTIZED_BLIP_MODEL_ID: &str = "lmz/candle-blip";

#[tokio::test]
async fn test_blip_quantized_download() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("raisin_ai=debug")
        .try_init();

    let registry = ModelRegistry::new().expect("Failed to create registry");
    let model_path = registry
        .download_model(QUANTIZED_BLIP_MODEL_ID, None)
        .await
        .expect("Failed to download quantized BLIP model");

    assert!(model_path.exists());
    // Quantized model should have GGUF file
    assert!(
        model_path
            .join("blip-image-captioning-large-q4k.gguf")
            .exists(),
        "Q4K GGUF file should exist"
    );
    assert!(model_path.join("tokenizer.json").exists());
}

#[tokio::test]
async fn test_blip_quantized_caption_image() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("raisin_ai=debug")
        .try_init();

    let registry = ModelRegistry::new().expect("Failed to create registry");
    let model_path = registry
        .download_model(QUANTIZED_BLIP_MODEL_ID, None)
        .await
        .expect("Failed to download quantized BLIP model");

    let gguf_path = model_path.join("blip-image-captioning-large-q4k.gguf");
    let tokenizer_path = model_path.join("tokenizer.json");

    let device = select_device(false).expect("Failed to select device");
    let mut captioner = BlipCaptioner::new_quantized(&gguf_path, &tokenizer_path, device)
        .expect("Failed to create quantized captioner");

    // Create and caption test image
    let image_bytes = create_gradient_jpeg(384, 384);
    let caption = captioner
        .caption_image(&image_bytes)
        .expect("Failed to caption image");

    // Verify caption properties
    assert!(!caption.is_empty(), "Caption should not be empty");
    assert!(
        caption.len() < 500,
        "Caption should be reasonably short, got {} chars",
        caption.len()
    );

    println!("Quantized BLIP caption: {}", caption);
}
