// SPDX-License-Identifier: BSL-1.1
//! Translation operations: translate, delete-translation, hide-in-locale, unhide-in-locale.

use axum::http::StatusCode;
use axum::Json;
use raisin_core::{NodeTypeResolver, TranslationService, TranslationStalenessService};
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::translations::{JsonPointer, LocaleCode};
use raisin_storage::Storage;
use std::collections::HashMap;

use crate::error::ApiError;

use super::common::{CommandContext, CommandResult};

/// Handle the translate command.
pub async fn handle_translate<S: Storage>(ctx: &mut CommandContext<'_, S>) -> CommandResult {
    // POST /path/raisin:cmd/translate
    // { "locale": "fr", "translations": { "/title": "...", "/description": "..." }, "message": "...", "actor": "..." }

    let locale_str = ctx.params.locale.clone().ok_or_else(|| {
        ApiError::validation_failed("locale is required for translate command")
    })?;
    let locale = LocaleCode::parse(&locale_str)
        .map_err(|e| ApiError::validation_failed(format!("Invalid locale code: {}", e)))?;

    let translations_json = ctx.params.translations.clone().ok_or_else(|| {
        ApiError::validation_failed("translations is required for translate command")
    })?;

    // Convert JSON translations to PropertyValue translations
    let translations_map: HashMap<String, serde_json::Value> =
        serde_json::from_value(translations_json).map_err(|e| {
            ApiError::validation_failed(format!("Invalid translations format: {}", e))
        })?;

    let mut translations = HashMap::new();
    for (pointer_str, json_value) in translations_map {
        let pointer = JsonPointer::parse(&pointer_str).map_err(|e| {
            ApiError::validation_failed(format!(
                "Invalid JSON pointer {}: {}",
                pointer_str, e
            ))
        })?;

        let property_value: PropertyValue =
            serde_json::from_value(json_value).map_err(|e| {
                ApiError::validation_failed(format!("Invalid property value: {}", e))
            })?;

        translations.insert(pointer, property_value);
    }

    // Get node to verify it exists
    let node = ctx
        .nodes_svc
        .get_by_path(ctx.path)
        .await?
        .ok_or_else(|| ApiError::node_not_found(ctx.path))?;

    // Create translation service with storage (handles revisions internally)
    let translation_service = TranslationService::new(ctx.state.storage().clone());

    let actor = ctx.get_actor();
    let message = ctx.params.message.clone();

    // Update translation (service handles revision management)
    let result = translation_service
        .update_translation(
            ctx.tenant_id,
            ctx.repository,
            ctx.branch,
            ctx.ws,
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
pub async fn handle_delete_translation<S: Storage>(
    ctx: &mut CommandContext<'_, S>,
) -> CommandResult {
    // POST /path/raisin:cmd/delete-translation
    // { "locale": "fr", "message": "...", "actor": "..." }

    let locale_str = ctx.params.locale.clone().ok_or_else(|| {
        ApiError::validation_failed("locale is required for delete-translation command")
    })?;
    let locale = LocaleCode::parse(&locale_str)
        .map_err(|e| ApiError::validation_failed(format!("Invalid locale code: {}", e)))?;

    // Get node to verify it exists
    let node = ctx
        .nodes_svc
        .get_by_path(ctx.path)
        .await?
        .ok_or_else(|| ApiError::node_not_found(ctx.path))?;

    // Create translation service (handles revisions internally)
    let translation_service = TranslationService::new(ctx.state.storage().clone());

    let actor = ctx.get_actor();
    let message = ctx.params.message.clone();

    // Delete translation (service handles revision management)
    translation_service
        .delete_translation(
            ctx.tenant_id,
            ctx.repository,
            ctx.branch,
            ctx.ws,
            &node.id,
            &locale,
            &actor,
            message,
        )
        .await?;

    CommandContext::<S>::no_content()
}

/// Handle the hide-in-locale command.
pub async fn handle_hide_in_locale<S: Storage>(ctx: &mut CommandContext<'_, S>) -> CommandResult {
    // POST /path/raisin:cmd/hide-in-locale
    // { "locale": "fr", "message": "...", "actor": "..." }

    let locale_str = ctx.params.locale.clone().ok_or_else(|| {
        ApiError::validation_failed("locale is required for hide-in-locale command")
    })?;
    let locale = LocaleCode::parse(&locale_str)
        .map_err(|e| ApiError::validation_failed(format!("Invalid locale code: {}", e)))?;

    // Get node to verify it exists
    let node = ctx
        .nodes_svc
        .get_by_path(ctx.path)
        .await?
        .ok_or_else(|| ApiError::node_not_found(ctx.path))?;

    // Create translation service (handles revisions internally)
    let translation_service = TranslationService::new(ctx.state.storage().clone());

    let actor = ctx.get_actor();
    let message = ctx.params.message.clone();

    // Hide node in locale (service handles revision management)
    let result = translation_service
        .hide_node(
            ctx.tenant_id,
            ctx.repository,
            ctx.branch,
            ctx.ws,
            &node.id,
            &locale,
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

/// Handle the unhide-in-locale command.
pub async fn handle_unhide_in_locale<S: Storage>(
    ctx: &mut CommandContext<'_, S>,
) -> CommandResult {
    // POST /path/raisin:cmd/unhide-in-locale
    // { "locale": "fr" }

    let locale_str = ctx.params.locale.clone().ok_or_else(|| {
        ApiError::validation_failed("locale is required for unhide-in-locale command")
    })?;
    let locale = LocaleCode::parse(&locale_str)
        .map_err(|e| ApiError::validation_failed(format!("Invalid locale code: {}", e)))?;

    // Get node to verify it exists
    let node = ctx
        .nodes_svc
        .get_by_path(ctx.path)
        .await?
        .ok_or_else(|| ApiError::node_not_found(ctx.path))?;

    // Create translation service with storage (handles revision internally)
    let translation_service = TranslationService::new(ctx.state.storage().clone());

    let actor = ctx.get_actor();

    // Unhide node (delete the Hidden overlay)
    // Service handles revision management internally
    translation_service
        .delete_translation(
            ctx.tenant_id,
            ctx.repository,
            ctx.branch,
            ctx.ws,
            &node.id,
            &locale,
            &actor,
            Some("Unhide node".to_string()),
        )
        .await?;

    CommandContext::<S>::no_content()
}

/// Handle the translation-staleness command.
pub async fn handle_translation_staleness<S: Storage>(
    ctx: &mut CommandContext<'_, S>,
) -> CommandResult {
    // GET /path/raisin:cmd/translation-staleness?locale=fr
    // Response: { stale: [...], missing: [...], fresh: [...], unknown: [...] }

    let locale_str = ctx.params.locale.clone().ok_or_else(|| {
        ApiError::validation_failed("locale is required for translation-staleness command")
    })?;
    let locale = LocaleCode::parse(&locale_str)
        .map_err(|e| ApiError::validation_failed(format!("Invalid locale code: {}", e)))?;

    // Get node to check staleness for
    let node = ctx
        .nodes_svc
        .get_by_path(ctx.path)
        .await?
        .ok_or_else(|| ApiError::node_not_found(ctx.path))?;

    // Resolve the node type schema to get is_translatable flags
    let resolved_schema = if !node.node_type.is_empty() {
        let resolver = NodeTypeResolver::new(
            ctx.state.storage().clone(),
            ctx.tenant_id.to_string(),
            ctx.repository.to_string(),
            ctx.branch.to_string(),
        );
        resolver.resolve(&node.node_type).await.ok()
    } else {
        None
    };

    // Create staleness service and check
    let staleness_service = TranslationStalenessService::new(ctx.state.storage().clone());

    let report = staleness_service
        .check_staleness(
            ctx.tenant_id,
            ctx.repository,
            ctx.branch,
            ctx.ws,
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
pub async fn handle_acknowledge_staleness<S: Storage>(
    ctx: &mut CommandContext<'_, S>,
) -> CommandResult {
    // POST /path/raisin:cmd/acknowledge-staleness
    // { "locale": "fr", "pointer": "/title" }

    let locale_str = ctx.params.locale.clone().ok_or_else(|| {
        ApiError::validation_failed("locale is required for acknowledge-staleness command")
    })?;
    let locale = LocaleCode::parse(&locale_str)
        .map_err(|e| ApiError::validation_failed(format!("Invalid locale code: {}", e)))?;

    let pointer_str = ctx.params.pointer.clone().ok_or_else(|| {
        ApiError::validation_failed("pointer is required for acknowledge-staleness command")
    })?;
    let pointer = JsonPointer::parse(&pointer_str)
        .map_err(|e| ApiError::validation_failed(format!("Invalid JSON pointer: {}", e)))?;

    // Get node
    let node = ctx
        .nodes_svc
        .get_by_path(ctx.path)
        .await?
        .ok_or_else(|| ApiError::node_not_found(ctx.path))?;

    // Resolve the node type schema to get is_translatable flags
    let resolved_schema = if !node.node_type.is_empty() {
        let resolver = NodeTypeResolver::new(
            ctx.state.storage().clone(),
            ctx.tenant_id.to_string(),
            ctx.repository.to_string(),
            ctx.branch.to_string(),
        );
        resolver.resolve(&node.node_type).await.ok()
    } else {
        None
    };

    // Acknowledge the staleness
    let staleness_service = TranslationStalenessService::new(ctx.state.storage().clone());

    staleness_service
        .acknowledge_staleness(
            ctx.tenant_id,
            ctx.repository,
            ctx.branch,
            ctx.ws,
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
