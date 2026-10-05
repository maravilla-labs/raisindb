// SPDX-License-Identifier: BSL-1.1

//! Translation-related command handlers for repository nodes.
//!
//! Handles translate, delete-translation, hide-in-locale, and unhide-in-locale
//! commands via the `raisin:cmd` pattern.
//!
//! The four write commands are thin adapters over the translation commands on
//! [`NodeService`] (`raisin-core`, `services/translation_service/commands.rs`),
//! the one implementation the WebSocket transport calls too. That is where the
//! node is resolved under RLS, `Update` permission is required, the overlay is
//! keyed by node id, and the actor is taken from the auth context. Nothing here
//! may re-implement any of it — in particular the request body's `actor` field
//! is NOT honoured for translations: the caller cannot name someone else as the
//! author of a write.

use axum::{extract::Json, http::StatusCode};
use raisin_core::{
    parse_translation_fields, NodeRef, NodeService, NodeTypeResolver, TranslationStalenessService,
    TranslationUpdateResult,
};
use raisin_models::auth::AuthContext;
use raisin_models::translations::{JsonPointer, LocaleCode};
use raisin_storage::{transactional::TransactionalStorage, Storage};
use std::collections::HashMap;

use crate::{error::ApiError, state::AppState, types::CommandBody};

/// The `locale` parameter, required and valid.
fn required_locale(params: &CommandBody, command: &str) -> Result<LocaleCode, ApiError> {
    let locale_str = params.locale.as_ref().ok_or_else(|| {
        ApiError::validation_failed(format!("locale is required for {command} command"))
    })?;
    LocaleCode::parse(locale_str)
        .map_err(|e| ApiError::validation_failed(format!("Invalid locale code: {}", e)))
}

/// Map a core error, keeping the path-naming 404 this endpoint always returned.
fn map_err(path: &str) -> impl Fn(raisin_error::Error) -> ApiError + '_ {
    move |e| match e {
        raisin_error::Error::NotFound(_) => ApiError::node_not_found(path),
        other => other.into(),
    }
}

fn update_result_json(result: TranslationUpdateResult) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "node_id": result.node_id,
            "locale": result.locale.as_str(),
            "revision": result.revision,
            "timestamp": result.timestamp.to_rfc3339(),
        })),
    )
}

/// Handle the translate command.
///
/// POST /path/raisin:cmd/translate
/// Body: { "locale": "fr", "translations": { "/title": "...", "/description": "..." }, "message": "..." }
pub(crate) async fn handle_translate<S: Storage + TransactionalStorage + 'static>(
    nodes_svc: &NodeService<S>,
    path: &str,
    params: &CommandBody,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let locale = required_locale(params, "translate")?;
    let translations_json = params.translations.as_ref().ok_or_else(|| {
        ApiError::validation_failed("translations is required for translate command")
    })?;
    let fields: HashMap<String, serde_json::Value> =
        serde_json::from_value(translations_json.clone()).map_err(|e| {
            ApiError::validation_failed(format!("Invalid translations format: {}", e))
        })?;
    let translations = parse_translation_fields(fields)?;

    let result = nodes_svc
        .translate(
            NodeRef::Path(path),
            &locale,
            translations,
            params.message.clone(),
        )
        .await
        .map_err(map_err(path))?;
    Ok(update_result_json(result))
}

/// Handle the delete-translation command.
///
/// POST /path/raisin:cmd/delete-translation
/// Body: { "locale": "fr", "message": "..." }
pub(crate) async fn handle_delete_translation<S: Storage + TransactionalStorage + 'static>(
    nodes_svc: &NodeService<S>,
    path: &str,
    params: &CommandBody,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let locale = required_locale(params, "delete-translation")?;
    nodes_svc
        .delete_locale_translation(NodeRef::Path(path), &locale, params.message.clone())
        .await
        .map_err(map_err(path))?;
    Ok((StatusCode::NO_CONTENT, Json(serde_json::json!({}))))
}

/// Handle the hide-in-locale command.
///
/// POST /path/raisin:cmd/hide-in-locale
/// Body: { "locale": "fr", "message": "..." }
pub(crate) async fn handle_hide_in_locale<S: Storage + TransactionalStorage + 'static>(
    nodes_svc: &NodeService<S>,
    path: &str,
    params: &CommandBody,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let locale = required_locale(params, "hide-in-locale")?;
    let result = nodes_svc
        .hide_in_locale(NodeRef::Path(path), &locale, params.message.clone())
        .await
        .map_err(map_err(path))?;
    Ok(update_result_json(result))
}

/// Handle the unhide-in-locale command.
///
/// POST /path/raisin:cmd/unhide-in-locale
/// Body: { "locale": "fr" }
pub(crate) async fn handle_unhide_in_locale<S: Storage + TransactionalStorage + 'static>(
    nodes_svc: &NodeService<S>,
    path: &str,
    params: &CommandBody,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let locale = required_locale(params, "unhide-in-locale")?;
    nodes_svc
        .unhide_in_locale(NodeRef::Path(path), &locale, None)
        .await
        .map_err(map_err(path))?;
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
