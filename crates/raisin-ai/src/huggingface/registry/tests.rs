//! Tests for the HuggingFace model registry.

use super::super::types::*;
use super::*;
use tempfile::TempDir;

#[tokio::test]
async fn test_registry_creation() {
    let temp_dir = TempDir::new().unwrap();
    let registry = ModelRegistry::with_cache_dir(temp_dir.path().to_path_buf()).unwrap();

    let models = registry.list_models().await;
    assert!(!models.is_empty());

    // Check that we have the default CLIP model (LAION)
    let clip = registry
        .get_model("laion/CLIP-ViT-B-32-laion2B-s34B-b79K")
        .await;
    assert!(clip.is_some());
    let clip = clip.unwrap();
    assert_eq!(clip.model_type, ModelType::Clip);
    assert!(!clip.is_downloaded());
}

#[tokio::test]
async fn test_model_path() {
    let temp_dir = TempDir::new().unwrap();
    let registry = ModelRegistry::with_cache_dir(temp_dir.path().to_path_buf()).unwrap();

    let path = registry.model_path("laion/CLIP-ViT-B-32-laion2B-s34B-b79K");
    assert!(path.ends_with("laion--CLIP-ViT-B-32-laion2B-s34B-b79K"));
}

#[tokio::test]
async fn test_disk_usage() {
    let temp_dir = TempDir::new().unwrap();
    let registry = ModelRegistry::with_cache_dir(temp_dir.path().to_path_buf()).unwrap();

    let usage = registry.total_disk_usage().await;
    assert_eq!(usage, 0);

    // Create a fake downloaded model
    let model_dir = temp_dir.path().join("test--model");
    std::fs::create_dir_all(&model_dir).unwrap();
    std::fs::write(model_dir.join("model.bin"), vec![0u8; 1000]).unwrap();

    // Add to registry
    let mut model = ModelInfo::new(
        "test/model",
        "Test Model",
        ModelType::Clip,
        vec![ModelCapability::ImageEmbedding],
    );
    model.actual_size_bytes = Some(1000);
    model.status = DownloadStatus::Ready;
    registry.register_model(model).await;

    let usage = registry.total_disk_usage().await;
    assert_eq!(usage, 1000);
}
