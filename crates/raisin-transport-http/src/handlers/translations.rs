//! Translation management HTTP handlers
//!
//! These endpoints manage translations for nodes in a multi-language content system.
//! Translations are stored separately from base content and applied via locale fallback chains.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use raisin_core::{TranslationService, TranslationUpdateResult};
use raisin_hlc::HLC;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::translations::{JsonPointer, LocaleCode};
use raisin_storage::{BranchRepository, Storage, TranslationRepository};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::{error::ApiError, state::AppState};

/// Request to update a node's translation for a specific locale
#[derive(Debug, Deserialize)]
pub struct UpdateTranslationRequest {
    /// Property translations as JSON pointers to values
    /// Example: {"/title": "Bonjour", "/description": "Ceci est une description"}
    pub translations: HashMap<String, serde_json::Value>,

    /// Optional commit message explaining the translation change
    #[serde(default)]
    pub message: Option<String>,

    /// Actor performing the translation (user ID, system, etc.)
    #[serde(default = "default_actor")]
    pub actor: String,
}

fn default_actor() -> String {
    "system".to_string()
}

/// Response from translation update operations
#[derive(Debug, Serialize)]
pub struct TranslationResponse {
    pub node_id: String,
    pub locale: String,
    pub revision: HLC,
    pub timestamp: String,
}

/// Update or create a translation for a node in a specific locale
///
/// # Endpoint
/// PUT /api/repository/{repo}/branch/{branch}/workspace/{workspace}/node/{node_id}/translations/{locale}
///
/// # Headers
/// X-Tenant-ID: {tenant_id}
///
/// # Body
/// ```json
/// {
///   "translations": {
///     "/title": "Bonjour",
///     "/description": "Ceci est une description",
///     "/properties/author": "Jean Dupont"
///   },
///   "message": "Updated French translation",
///   "actor": "user-123"
/// }
/// ```
pub async fn update_translation(
    State(state): State<AppState>,
    Path((repo, branch, workspace, node_id, locale_str)): Path<(
        String,
        String,
        String,
        String,
        String,
    )>,
    Json(req): Json<UpdateTranslationRequest>,
) -> Result<(StatusCode, Json<TranslationResponse>), ApiError> {
    let tenant_id = "default"; // TODO: Extract from headers/middleware

    // Parse locale
    let locale = LocaleCode::parse(&locale_str)
        .map_err(|e| ApiError::validation_failed(format!("Invalid locale code: {}", e)))?;

    // Convert JSON translations to PropertyValue translations
    let mut translations = HashMap::new();
    for (pointer_str, json_value) in req.translations {
        let pointer = JsonPointer::parse(&pointer_str).map_err(|e| {
            ApiError::validation_failed(format!("Invalid JSON pointer {}: {}", pointer_str, e))
        })?;

        let property_value: PropertyValue = serde_json::from_value(json_value)
            .map_err(|e| ApiError::validation_failed(format!("Invalid property value: {}", e)))?;

        translations.insert(pointer, property_value);
    }

    // Create translation service with storage (handles revision internally)
    let translation_service = TranslationService::new(state.storage().clone());

    // Update translation
    // Service handles revision management internally
    let result: TranslationUpdateResult = translation_service
        .update_translation(
            tenant_id,
            &repo,
            &branch,
            &workspace,
            &node_id,
            &locale,
            translations,
            &req.actor,
            req.message,
        )
        .await?;

    // Convert to response
    let response = TranslationResponse {
        node_id: result.node_id,
        locale: result.locale.as_str().to_string(),
        revision: result.revision,
        timestamp: result.timestamp.to_rfc3339(),
    };

    Ok((StatusCode::OK, Json(response)))
}

/// Get all translations for a node
///
/// # Endpoint
/// GET /api/repository/{repo}/branch/{branch}/workspace/{workspace}/node/{node_id}/translations
///
/// # Response
/// ```json
/// {
///   "node_id": "page-1",
///   "locales": ["fr", "de", "es"]
/// }
/// ```
pub async fn list_translations(
    State(state): State<AppState>,
    Path((repo, branch, workspace, node_id)): Path<(String, String, String, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let tenant_id = "default";

    let translation_repo = state.storage().translations();

    // Get current branch head revision
    let current_revision = state
        .storage()
        .branches()
        .get_head(tenant_id, &repo, &branch)
        .await?;

    // List translations
    let locales = translation_repo
        .list_translations_for_node(
            tenant_id,
            &repo,
            &branch,
            &workspace,
            &node_id,
            &current_revision,
        )
        .await?;

    let response = serde_json::json!({
        "node_id": node_id,
        "locales": locales.iter().map(|l| l.as_str()).collect::<Vec<_>>()
    });

    Ok(Json(response))
}

/// Delete a translation for a node in a specific locale
///
/// # Endpoint
/// DELETE /api/repository/{repo}/branch/{branch}/workspace/{workspace}/node/{node_id}/translations/{locale}
pub async fn delete_translation(
    State(state): State<AppState>,
    Path((repo, branch, workspace, node_id, locale_str)): Path<(
        String,
        String,
        String,
        String,
        String,
    )>,
) -> Result<StatusCode, ApiError> {
    let tenant_id = "default";

    // Parse locale
    let locale = LocaleCode::parse(&locale_str)
        .map_err(|e| ApiError::validation_failed(format!("Invalid locale code: {}", e)))?;

    // Create translation service with storage (handles revision internally)
    let translation_service = TranslationService::new(state.storage().clone());

    // Delete translation
    // Service handles revision management internally
    translation_service
        .delete_translation(
            tenant_id, &repo, &branch, &workspace, &node_id, &locale, "system", None,
        )
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

/// Hide a node in a specific locale (makes it not appear in that locale)
///
/// # Endpoint
/// POST /api/repository/{repo}/branch/{branch}/workspace/{workspace}/node/{node_id}/translations/{locale}/hide
///
/// # Body
/// ```json
/// {
///   "message": "This content is not applicable in German market",
///   "actor": "user-123"
/// }
/// ```
pub async fn hide_node(
    State(state): State<AppState>,
    Path((repo, branch, workspace, node_id, locale_str)): Path<(
        String,
        String,
        String,
        String,
        String,
    )>,
    Json(req): Json<UpdateTranslationRequest>,
) -> Result<(StatusCode, Json<TranslationResponse>), ApiError> {
    let tenant_id = "default";

    // Parse locale
    let locale = LocaleCode::parse(&locale_str)
        .map_err(|e| ApiError::validation_failed(format!("Invalid locale code: {}", e)))?;

    // Create translation service with storage (handles revision internally)
    let translation_service = TranslationService::new(state.storage().clone());

    // Hide node in locale
    // Service handles revision management internally
    let result = translation_service
        .hide_node(
            tenant_id,
            &repo,
            &branch,
            &workspace,
            &node_id,
            &locale,
            &req.actor,
            req.message,
        )
        .await?;

    // Convert to response
    let response = TranslationResponse {
        node_id: result.node_id,
        locale: result.locale.as_str().to_string(),
        revision: result.revision,
        timestamp: result.timestamp.to_rfc3339(),
    };

    Ok((StatusCode::OK, Json(response)))
}

/// Unhide a node in a specific locale (removes the hidden marker)
///
/// # Endpoint
/// DELETE /api/repository/{repo}/branch/{branch}/workspace/{workspace}/node/{node_id}/translations/{locale}/hide
pub async fn unhide_node(
    State(state): State<AppState>,
    Path((repo, branch, workspace, node_id, locale_str)): Path<(
        String,
        String,
        String,
        String,
        String,
    )>,
) -> Result<StatusCode, ApiError> {
    let tenant_id = "default";

    // Parse locale
    let locale = LocaleCode::parse(&locale_str)
        .map_err(|e| ApiError::validation_failed(format!("Invalid locale code: {}", e)))?;

    // Create translation service with storage (handles revision internally)
    let translation_service = TranslationService::new(state.storage().clone());

    // Unhide node (delete the Hidden overlay)
    // Service handles revision management internally
    translation_service
        .delete_translation(
            tenant_id,
            &repo,
            &branch,
            &workspace,
            &node_id,
            &locale,
            "system",
            Some("Unhide node".to_string()),
        )
        .await?;

    Ok(StatusCode::NO_CONTENT)
}
