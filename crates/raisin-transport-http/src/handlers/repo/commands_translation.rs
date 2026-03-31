// SPDX-License-Identifier: BSL-1.1

//! Translation-related command handlers for repository nodes.
//!
//! Handles translate, delete-translation, hide-in-locale, and unhide-in-locale
//! commands via the `raisin:cmd` pattern.

use axum::{extract::Json, http::StatusCode};
use raisin_core::{NodeService, NodeTypeResolver, TranslationService, TranslationStalenessService};
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::translations::{JsonPointer, LocaleCode};
use raisin_storage::{transactional::TransactionalStorage, Storage};
use std::collections::HashMap;

use crate::{error::ApiError, state::AppState, types::CommandBody};

/// Handle the translate command.
///
/// POST /path/raisin:cmd/translate
/// Body: { "locale": "fr", "translations": { "/title": "...", "/description": "..." }, "message": "...", "actor": "..." }
pub(crate) async fn handle_translate<S: Storage + TransactionalStorage + 'static>(
    state: &AppState,
    nodes_svc: &NodeService<S>,
    tenant_id: &str,
    repository: &str,
    branch: &str,
    ws: &str,
    path: &str,
    params: &CommandBody,
    auth: Option<AuthContext>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let locale_str = params
        .locale
        .as_ref()
        .ok_or_else(|| ApiError::validation_failed("locale is required for translate command"))?;
    let locale = LocaleCode::parse(locale_str)
        .map_err(|e| ApiError::validation_failed(format!("Invalid locale code: {}", e)))?;

    let translations_json = params.translations.as_ref().ok_or_else(|| {
        ApiError::validation_failed("translations is required for translate command")
    })?;

    // Convert JSON translations to PropertyValue translations
    let translations_map: HashMap<String, serde_json::Value> =
        serde_json::from_value(translations_json.clone()).map_err(|e| {
            ApiError::validation_failed(format!("Invalid translations format: {}", e))
        })?;

    let mut translations = HashMap::new();
    for (pointer_str, json_value) in translations_map {
        let pointer = JsonPointer::parse(&pointer_str).map_err(|e| {
            ApiError::validation_failed(format!("Invalid JSON pointer {}: {}", pointer_str, e))
        })?;

        let property_value: PropertyValue = serde_json::from_value(json_value)
            .map_err(|e| ApiError::validation_failed(format!("Invalid property value: {}", e)))?;

        translations.insert(pointer, property_value);
    }

    // Get node to verify it exists
    let node = nodes_svc
        .get_by_path(path)
        .await?
        .ok_or_else(|| ApiError::node_not_found(path))?;

    // Create translation service with storage (handles revisions internally)
    let translation_service = TranslationService::new(state.storage().clone());

    let actor = params.actor.clone().unwrap_or_else(|| {
        auth.as_ref()
            .map(|ctx| ctx.actor_id())
            .unwrap_or_else(|| "system".to_string())
    });
    let message = params.message.clone();

    // Update translation (service handles revision management)
    let result = translation_service
        .update_translation(
            tenant_id,
            repository,
            branch,
            ws,
            &node.id,
            &locale,
            translations,
            &actor,
            message,
        )
        .await?;

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({
            "node_id": result.node_id,
            "locale": result.locale.as_str(),
            "revision": result.revision,
            "timestamp": result.timestamp.to_rfc3339(),
        })),
    ))
}

/// Handle the delete-translation command.
///
/// POST /path/raisin:cmd/delete-translation
/// Body: { "locale": "fr", "message": "...", "actor": "..." }
pub(crate) async fn handle_delete_translation<S: Storage + TransactionalStorage + 'static>(
    state: &AppState,
    nodes_svc: &NodeService<S>,
    tenant_id: &str,
    repository: &str,
    branch: &str,
    ws: &str,
    path: &str,
    params: &CommandBody,
    auth: Option<AuthContext>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let locale_str = params.locale.as_ref().ok_or_else(|| {
        ApiError::validation_failed("locale is required for delete-translation command")
    })?;
    let locale = LocaleCode::parse(locale_str)
        .map_err(|e| ApiError::validation_failed(format!("Invalid locale code: {}", e)))?;

    // Get node to verify it exists
    let node = nodes_svc
        .get_by_path(path)
        .await?
        .ok_or_else(|| ApiError::node_not_found(path))?;

    // Create translation service (handles revisions internally)
    let translation_service = TranslationService::new(state.storage().clone());

    let actor = params.actor.clone().unwrap_or_else(|| {
        auth.as_ref()
            .map(|ctx| ctx.actor_id())
            .unwrap_or_else(|| "system".to_string())
    });
    let message = params.message.clone();

    // Delete translation (service handles revision management)
    translation_service
        .delete_translation(
            tenant_id, repository, branch, ws, &node.id, &locale, &actor, message,
        )
        .await?;

    Ok((StatusCode::NO_CONTENT, Json(serde_json::json!({}))))
}

/// Handle the hide-in-locale command.
///
/// POST /path/raisin:cmd/hide-in-locale
/// Body: { "locale": "fr", "message": "...", "actor": "..." }
pub(crate) async fn handle_hide_in_locale<S: Storage + TransactionalStorage + 'static>(
    state: &AppState,
    nodes_svc: &NodeService<S>,
    tenant_id: &str,
    repository: &str,
    branch: &str,
    ws: &str,
    path: &str,
    params: &CommandBody,
    auth: Option<AuthContext>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let locale_str = params.locale.as_ref().ok_or_else(|| {
        ApiError::validation_failed("locale is required for hide-in-locale command")
    })?;
    let locale = LocaleCode::parse(locale_str)
        .map_err(|e| ApiError::validation_failed(format!("Invalid locale code: {}", e)))?;

    // Get node to verify it exists
    let node = nodes_svc
        .get_by_path(path)
        .await?
        .ok_or_else(|| ApiError::node_not_found(path))?;

    // Create translation service (handles revisions internally)
    let translation_service = TranslationService::new(state.storage().clone());

    let actor = params.actor.clone().unwrap_or_else(|| {
        auth.as_ref()
            .map(|ctx| ctx.actor_id())
            .unwrap_or_else(|| "system".to_string())
    });
    let message = params.message.clone();

    // Hide node in locale (service handles revision management)
    let result = translation_service
        .hide_node(
            tenant_id, repository, branch, ws, &node.id, &locale, &actor, message,
        )
        .await?;

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({
            "node_id": result.node_id,
            "locale": result.locale.as_str(),
            "revision": result.revision,
            "timestamp": result.timestamp.to_rfc3339(),
        })),
    ))
}

/// Handle the unhide-in-locale command.
///
/// POST /path/raisin:cmd/unhide-in-locale
/// Body: { "locale": "fr" }
pub(crate) async fn handle_unhide_in_locale<S: Storage + TransactionalStorage + 'static>(
    state: &AppState,
    nodes_svc: &NodeService<S>,
    tenant_id: &str,
    repository: &str,
    branch: &str,
    ws: &str,
    path: &str,
    params: &CommandBody,
    auth: Option<AuthContext>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let locale_str = params.locale.as_ref().ok_or_else(|| {
        ApiError::validation_failed("locale is required for unhide-in-locale command")
    })?;
    let locale = LocaleCode::parse(locale_str)
        .map_err(|e| ApiError::validation_failed(format!("Invalid locale code: {}", e)))?;

    // Get node to verify it exists
    let node = nodes_svc
        .get_by_path(path)
        .await?
        .ok_or_else(|| ApiError::node_not_found(path))?;

    // Create translation service with storage (handles revision internally)
    let translation_service = TranslationService::new(state.storage().clone());

    let actor = params.actor.clone().unwrap_or_else(|| {
        auth.as_ref()
            .map(|ctx| ctx.actor_id())
            .unwrap_or_else(|| "system".to_string())
    });

    // Unhide node (delete the Hidden overlay)
    translation_service
        .delete_translation(
            tenant_id,
            repository,
            branch,
            ws,
            &node.id,
            &locale,
            &actor,
            Some("Unhide node".to_string()),
        )
        .await?;

    Ok((StatusCode::NO_CONTENT, Json(serde_json::json!({}))))
}

/// Handle the translation-staleness command.
///
/// GET /path/raisin:cmd/translation-staleness?locale=fr
/// Response: {
///   "stale": [...],
///   "missing": [...],
///   "fresh": [...],
///   "unknown": [...]
/// }
pub(crate) async fn handle_translation_staleness<S: Storage + TransactionalStorage + 'static>(
    state: &AppState,
    nodes_svc: &NodeService<S>,
    tenant_id: &str,
    repository: &str,
    branch: &str,
    ws: &str,
    path: &str,
    params: &CommandBody,
    _auth: Option<AuthContext>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let locale_str = params.locale.as_ref().ok_or_else(|| {
        ApiError::validation_failed("locale is required for translation-staleness command")
    })?;
    let locale = LocaleCode::parse(locale_str)
        .map_err(|e| ApiError::validation_failed(format!("Invalid locale code: {}", e)))?;

    // Get node to check staleness for
    let node = nodes_svc
        .get_by_path(path)
        .await?
        .ok_or_else(|| ApiError::node_not_found(path))?;

    // Resolve the node type schema to get is_translatable flags
    let resolved_schema = if !node.node_type.is_empty() {
        let resolver = NodeTypeResolver::new(
            state.storage().clone(),
            tenant_id.to_string(),
            repository.to_string(),
            branch.to_string(),
        );
        resolver.resolve(&node.node_type).await.ok()
    } else {
        None
    };

    // Create staleness service and check
    let staleness_service = TranslationStalenessService::new(state.storage().clone());

    let report = staleness_service
        .check_staleness(
            tenant_id,
            repository,
            branch,
            ws,
            &node,
            &locale,
            resolved_schema
                .as_ref()
                .map(|s| s.resolved_properties.as_slice()),
        )
        .await?;

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({
            "stale": report.stale_fields,
            "missing": report.missing_fields,
            "fresh": report.fresh_fields,
            "unknown": report.unknown_fields,
        })),
    ))
}

/// Handle the acknowledge-staleness command.
///
/// POST /path/raisin:cmd/acknowledge-staleness
/// Body: { "locale": "fr", "pointer": "/title" }
///
/// Marks a stale translation as acknowledged without requiring re-translation.
pub(crate) async fn handle_acknowledge_staleness<S: Storage + TransactionalStorage + 'static>(
    state: &AppState,
    nodes_svc: &NodeService<S>,
    tenant_id: &str,
    repository: &str,
    branch: &str,
    ws: &str,
    path: &str,
    params: &CommandBody,
    _auth: Option<AuthContext>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let locale_str = params.locale.as_ref().ok_or_else(|| {
        ApiError::validation_failed("locale is required for acknowledge-staleness command")
    })?;
    let locale = LocaleCode::parse(locale_str)
        .map_err(|e| ApiError::validation_failed(format!("Invalid locale code: {}", e)))?;

    let pointer_str = params.pointer.as_ref().ok_or_else(|| {
        ApiError::validation_failed("pointer is required for acknowledge-staleness command")
    })?;
    let pointer = JsonPointer::parse(pointer_str)
        .map_err(|e| ApiError::validation_failed(format!("Invalid JSON pointer: {}", e)))?;

    // Get node
    let node = nodes_svc
        .get_by_path(path)
        .await?
        .ok_or_else(|| ApiError::node_not_found(path))?;

    // Resolve the node type schema to get is_translatable flags
    let resolved_schema = if !node.node_type.is_empty() {
        let resolver = NodeTypeResolver::new(
            state.storage().clone(),
            tenant_id.to_string(),
            repository.to_string(),
            branch.to_string(),
        );
        resolver.resolve(&node.node_type).await.ok()
    } else {
        None
    };

    // Acknowledge the staleness
    let staleness_service = TranslationStalenessService::new(state.storage().clone());

    staleness_service
        .acknowledge_staleness(
            tenant_id,
            repository,
            branch,
            ws,
            &node,
            &locale,
            &pointer,
            resolved_schema
                .as_ref()
                .map(|s| s.resolved_properties.as_slice()),
        )
        .await?;

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({
            "acknowledged": true,
            "pointer": pointer_str,
            "locale": locale_str,
        })),
    ))
}
