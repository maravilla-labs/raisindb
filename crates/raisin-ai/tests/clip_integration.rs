//! Integration tests for CLIP image embeddings.
//!
//! These tests require model downloads (~605MB) and are gated behind the
//! `integration-tests` feature flag.
//!
//! Run with: `cargo test -p raisin-ai --features integration-tests`

#![cfg(feature = "integration-tests")]

mod fixtures;

use image::{DynamicImage, Rgb, RgbImage};
use raisin_ai::candle::{select_device, ClipEmbedder, CLIP_EMBEDDING_DIM};
use raisin_ai::huggingface::ModelRegistry;

/// The LAION CLIP model with native safetensors support.
const CLIP_MODEL_ID: &str = "laion/CLIP-ViT-B-32-laion2B-s34B-b79K";

/// Create a test JPEG image with a specific color.
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
async fn test_clip_model_download() {
    // Initialize tracing for debugging
    let _ = tracing_subscriber::fmt()
        .with_env_filter("raisin_ai=debug")
        .try_init();

    let registry = ModelRegistry::new().expect("Failed to create registry");
    let model_path = registry
        .download_model(CLIP_MODEL_ID, None)
        .await
        .expect("Failed to download CLIP model");

    assert!(model_path.exists());
    // LAION model uses open_clip_model.safetensors or model.safetensors
    let has_weights = model_path.join("model.safetensors").exists()
        || model_path.join("open_clip_model.safetensors").exists();
    assert!(has_weights, "Model safetensors should exist");
}

#[tokio::test]
async fn test_clip_embed_image() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("raisin_ai=debug")
        .try_init();

    // Download model
    let registry = ModelRegistry::new().expect("Failed to create registry");
    let model_path = registry
        .download_model(CLIP_MODEL_ID, None)
        .await
        .expect("Failed to download CLIP model");

    // Load embedder (use CPU for CI compatibility)
    let device = select_device(false).expect("Failed to select device");
    let embedder = ClipEmbedder::new(&model_path, device).expect("Failed to create embedder");

    // Create and embed test image
    let image_bytes = create_gradient_jpeg(224, 224);
    let embedding = embedder
        .embed_image(&image_bytes)
        .expect("Failed to embed image");

    // Verify embedding properties
    assert_eq!(
        embedding.len(),
        CLIP_EMBEDDING_DIM,
        "Embedding should be 512-dimensional"
    );

    // All values should be finite
    assert!(
        embedding.iter().all(|v| v.is_finite()),
        "All embedding values should be finite"
    );

    // Should be L2 normalized (unit length)
    let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
    assert!(
        (norm - 1.0).abs() < 0.01,
        "Embedding should be L2 normalized, got norm={}",
        norm
    );
}

#[tokio::test]
async fn test_clip_embed_different_images_produce_different_embeddings() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("raisin_ai=debug")
        .try_init();

    let registry = ModelRegistry::new().expect("Failed to create registry");
    let model_path = registry
        .download_model(CLIP_MODEL_ID, None)
        .await
        .expect("Failed to download CLIP model");

    let device = select_device(false).expect("Failed to select device");
    let embedder = ClipEmbedder::new(&model_path, device).expect("Failed to create embedder");

    // Create two different colored images
    let red_image = create_colored_jpeg(224, 224, 255, 0, 0);
    let blue_image = create_colored_jpeg(224, 224, 0, 0, 255);

    let red_embedding = embedder
        .embed_image(&red_image)
        .expect("Failed to embed red");
    let blue_embedding = embedder
        .embed_image(&blue_image)
        .expect("Failed to embed blue");

    // Calculate cosine similarity (dot product of normalized vectors)
    let similarity: f32 = red_embedding
        .iter()
        .zip(blue_embedding.iter())
        .map(|(a, b)| a * b)
        .sum();

    // Different images should have different embeddings (similarity < 1.0)
    assert!(
        similarity < 0.99,
        "Different images should produce different embeddings, got similarity={}",
        similarity
    );
}

#[tokio::test]
async fn test_clip_embed_same_image_produces_identical_embeddings() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("raisin_ai=debug")
        .try_init();

    let registry = ModelRegistry::new().expect("Failed to create registry");
    let model_path = registry
        .download_model(CLIP_MODEL_ID, None)
        .await
        .expect("Failed to download CLIP model");

    let device = select_device(false).expect("Failed to select device");
    let embedder = ClipEmbedder::new(&model_path, device).expect("Failed to create embedder");

    let image = create_gradient_jpeg(224, 224);

    let embedding1 = embedder
        .embed_image(&image)
        .expect("First embedding failed");
    let embedding2 = embedder
        .embed_image(&image)
        .expect("Second embedding failed");

    // Same image should produce identical embeddings
    let similarity: f32 = embedding1
        .iter()
        .zip(embedding2.iter())
        .map(|(a, b)| a * b)
        .sum();

    assert!(
        (similarity - 1.0).abs() < 0.001,
        "Same image should produce identical embeddings, got similarity={}",
        similarity
    );
}

#[tokio::test]
async fn test_clip_batch_embedding() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("raisin_ai=debug")
        .try_init();

    let registry = ModelRegistry::new().expect("Failed to create registry");
    let model_path = registry
        .download_model(CLIP_MODEL_ID, None)
        .await
        .expect("Failed to download CLIP model");

    let device = select_device(false).expect("Failed to select device");
    let embedder = ClipEmbedder::new(&model_path, device).expect("Failed to create embedder");

    // Create multiple test images
    let images: Vec<Vec<u8>> = vec![
        create_colored_jpeg(224, 224, 255, 0, 0),
        create_colored_jpeg(224, 224, 0, 255, 0),
        create_colored_jpeg(224, 224, 0, 0, 255),
    ];

    let image_refs: Vec<&[u8]> = images.iter().map(|v| v.as_slice()).collect();

    let embeddings = embedder
        .embed_images(&image_refs)
        .expect("Batch embedding failed");

    assert_eq!(embeddings.len(), 3, "Should get 3 embeddings");

    for (i, embedding) in embeddings.iter().enumerate() {
        assert_eq!(
            embedding.len(),
            CLIP_EMBEDDING_DIM,
            "Embedding {} should be 512-dimensional",
            i
        );

        let norm: f32 = embedding.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 0.01,
            "Embedding {} should be L2 normalized",
            i
        );
    }
}

#[tokio::test]
async fn test_clip_handles_various_image_sizes() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("raisin_ai=debug")
        .try_init();

    let registry = ModelRegistry::new().expect("Failed to create registry");
    let model_path = registry
        .download_model(CLIP_MODEL_ID, None)
        .await
        .expect("Failed to download CLIP model");

    let device = select_device(false).expect("Failed to select device");
    let embedder = ClipEmbedder::new(&model_path, device).expect("Failed to create embedder");

    // Test various image sizes
    let sizes = [(50, 50), (224, 224), (640, 480), (1920, 1080)];

    for (width, height) in sizes {
        let image = create_gradient_jpeg(width, height);
        let embedding = embedder
            .embed_image(&image)
            .unwrap_or_else(|_| panic!("Failed to embed {}x{} image", width, height));

        assert_eq!(
            embedding.len(),
            CLIP_EMBEDDING_DIM,
            "{}x{} image should produce 512-dim embedding",
            width,
            height
        );
    }
}
