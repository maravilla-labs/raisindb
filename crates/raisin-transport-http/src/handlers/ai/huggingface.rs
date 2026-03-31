// SPDX-License-Identifier: BSL-1.1

//! HuggingFace model management handlers.
//!
//! Endpoints for listing, inspecting, downloading, and deleting
//! HuggingFace models used for local inference (BLIP, CLIP, etc.).

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use percent_encoding::percent_decode_str;

use raisin_ai::ModelRegistry;

use crate::{error::ApiError, state::AppState};

use super::types::{
    HuggingFaceModelDeleteResponse, HuggingFaceModelDownloadResponse, HuggingFaceModelResponse,
    HuggingFaceModelsListResponse, LocalCaptionModelResponse, LocalCaptionModelsResponse,
};

/// List all available HuggingFace models.
///
/// GET /api/tenants/{tenant}/ai/models/huggingface
pub async fn list_huggingface_models(
    State(_state): State<AppState>,
    Path(_tenant): Path<String>,
) -> Result<Json<HuggingFaceModelsListResponse>, ApiError> {
    // Create a temporary registry (in production, this would be shared via AppState)
    let registry = ModelRegistry::new()
        .map_err(|e| ApiError::internal(format!("Failed to initialize model registry: {}", e)))?;

    // Refresh download status
    registry.refresh_download_status().await;

    let models = registry.list_models().await;
    let disk_usage = registry.disk_usage_display().await;

    Ok(Json(HuggingFaceModelsListResponse {
        models: models
            .into_iter()
            .map(HuggingFaceModelResponse::from)
            .collect(),
        total_disk_usage: disk_usage,
    }))
}

/// Get info for a specific HuggingFace model.
///
/// GET /api/tenants/{tenant}/ai/models/huggingface/{model_id}
pub async fn get_huggingface_model(
    State(_state): State<AppState>,
    Path((_tenant, model_id)): Path<(String, String)>,
) -> Result<Json<HuggingFaceModelResponse>, ApiError> {
    // URL decode the model_id (e.g., "openai%2Fclip-vit-base-patch32" -> "openai/clip-vit-base-patch32")
    let model_id = percent_decode_str(&model_id)
        .decode_utf8()
        .map_err(|e| ApiError::validation_failed(format!("Invalid model_id encoding: {}", e)))?
        .into_owned();

    let registry = ModelRegistry::new()
        .map_err(|e| ApiError::internal(format!("Failed to initialize model registry: {}", e)))?;

    registry.refresh_download_status().await;

    let model = registry
        .get_model(&model_id)
        .await
        .ok_or_else(|| ApiError::not_found(format!("Model not found: {}", model_id)))?;

    Ok(Json(HuggingFaceModelResponse::from(model)))
}

/// Start downloading a HuggingFace model.
///
/// POST /api/tenants/{tenant}/ai/models/huggingface/{model_id}/download
#[cfg(feature = "storage-rocksdb")]
pub async fn download_huggingface_model(
    State(state): State<AppState>,
    Path((tenant, model_id)): Path<(String, String)>,
) -> Result<Json<HuggingFaceModelDownloadResponse>, ApiError> {
    use raisin_storage::jobs::JobType;

    // URL decode the model_id
    let model_id = percent_decode_str(&model_id)
        .decode_utf8()
        .map_err(|e| ApiError::validation_failed(format!("Invalid model_id encoding: {}", e)))?
        .into_owned();

    // Verify model exists in registry
    let registry = ModelRegistry::new()
        .map_err(|e| ApiError::internal(format!("Failed to initialize model registry: {}", e)))?;

    let model = registry
        .get_model(&model_id)
        .await
        .ok_or_else(|| ApiError::not_found(format!("Model not found in registry: {}", model_id)))?;

    // Check if already downloaded
    if model.is_downloaded() {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "MODEL_ALREADY_DOWNLOADED",
            format!("Model {} is already downloaded", model_id),
        ));
    }

    // Queue download job
    let storage = state
        .rocksdb_storage
        .as_ref()
        .ok_or_else(|| ApiError::internal("RocksDB storage not available"))?;

    let job_type = JobType::HuggingFaceModelDownload {
        model_id: model_id.clone(),
    };

    // Use unified job queue
    let job_id = storage
        .job_registry()
        .register_job(
            job_type,
            Some(tenant.clone()),
            None, // no handle
            None, // no cancel token
            None, // default retries
        )
        .await
        .map_err(|e| ApiError::internal(format!("Failed to queue download job: {}", e)))?;

    Ok(Json(HuggingFaceModelDownloadResponse {
        model_id,
        job_id: job_id.to_string(),
        message: "Model download queued".to_string(),
    }))
}

/// Delete a downloaded HuggingFace model.
///
/// DELETE /api/tenants/{tenant}/ai/models/huggingface/{model_id}
pub async fn delete_huggingface_model(
    State(_state): State<AppState>,
    Path((_tenant, model_id)): Path<(String, String)>,
) -> Result<Json<HuggingFaceModelDeleteResponse>, ApiError> {
    // URL decode the model_id
    let model_id = percent_decode_str(&model_id)
        .decode_utf8()
        .map_err(|e| ApiError::validation_failed(format!("Invalid model_id encoding: {}", e)))?
        .into_owned();

    let registry = ModelRegistry::new()
        .map_err(|e| ApiError::internal(format!("Failed to initialize model registry: {}", e)))?;

    // Delete the model
    registry
        .delete_model(&model_id)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to delete model: {}", e)))?;

    Ok(Json(HuggingFaceModelDeleteResponse {
        model_id,
        success: true,
        message: "Model deleted successfully".to_string(),
    }))
}

/// List available local image captioning models.
///
/// GET /api/ai/models/local/caption
///
/// Returns the list of local image captioning models available for processing.
/// This includes models like BLIP and GIT that run locally without cloud APIs.
#[cfg(feature = "storage-rocksdb")]
pub async fn list_local_caption_models(
    State(_state): State<AppState>,
) -> Result<Json<LocalCaptionModelsResponse>, ApiError> {
    use raisin_ai::{AVAILABLE_CAPTION_MODELS, DEFAULT_CAPTION_MODEL};

    let models: Vec<LocalCaptionModelResponse> = AVAILABLE_CAPTION_MODELS
        .iter()
        .map(|m| LocalCaptionModelResponse {
            id: m.id.to_string(),
            name: m.name.to_string(),
            size_mb: m.size_mb,
            supported: m.supported,
            description: m.description.to_string(),
        })
        .collect();

    Ok(Json(LocalCaptionModelsResponse {
        models,
        default_model: DEFAULT_CAPTION_MODEL.to_string(),
    }))
}
