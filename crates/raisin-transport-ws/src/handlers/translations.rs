// SPDX-License-Identifier: BSL-1.1

//! Translation operation handlers

use parking_lot::RwLock;
use raisin_core::TranslationService;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::translations::{JsonPointer, LocaleCode};
use raisin_storage::{
    transactional::TransactionalStorage, BranchRepository, TranslationRepository,
};
use std::collections::HashMap;
use std::sync::Arc;

use crate::{
    connection::ConnectionState,
    error::WsError,
    handler::WsState,
    protocol::{
        RequestEnvelope, ResponseEnvelope, TranslationDeletePayload, TranslationHidePayload,
        TranslationListPayload, TranslationUnhidePayload, TranslationUpdatePayload,
    },
};

/// Convert serde_json::Value to PropertyValue (from request payload)
fn json_to_property_value(value: &serde_json::Value) -> PropertyValue {
    match value {
        serde_json::Value::String(s) => PropertyValue::String(s.clone()),
        serde_json::Value::Number(n) => {
            // Check if the number is an integer or float
            if n.is_i64() || n.is_u64() {
                PropertyValue::Integer(n.as_i64().unwrap_or(0))
            } else {
                PropertyValue::Float(n.as_f64().unwrap_or(0.0))
            }
        }
        serde_json::Value::Bool(b) => PropertyValue::Boolean(*b),
        serde_json::Value::Array(arr) => {
            PropertyValue::Array(arr.iter().map(json_to_property_value).collect())
        }
        serde_json::Value::Object(obj) => PropertyValue::Object(
            obj.iter()
                .map(|(k, v)| (k.clone(), json_to_property_value(v)))
                .collect(),
        ),
        serde_json::Value::Null => PropertyValue::String(String::new()),
    }
}

/// Handle translation update operation
pub async fn handle_translation_update<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: TranslationUpdatePayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;
    let branch = request.context.branch.as_deref().unwrap_or("main");
    let workspace = request
        .context
        .workspace
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Workspace required".to_string()))?;

    // Parse locale
    let locale = LocaleCode::parse(&payload.locale)
        .map_err(|e| WsError::InvalidRequest(format!("Invalid locale code: {}", e)))?;

    // Convert properties HashMap to translation format
    let mut translations = HashMap::new();
    for (property_name, value) in payload.properties {
        let pointer = JsonPointer::parse(format!("/{}", property_name)).map_err(|e| {
            WsError::InvalidRequest(format!("Invalid property name '{}': {}", property_name, e))
        })?;
        let property_value = json_to_property_value(&value);
        translations.insert(pointer, property_value);
    }

    // Create translation service
    let translation_service = TranslationService::new(state.storage.clone());

    // Update translation
    let result = translation_service
        .update_translation(
            tenant_id,
            repo,
            branch,
            workspace,
            &payload.node_path,
            &locale,
            translations,
            "system", // TODO: Get actor from connection state
            None,
        )
        .await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::json!({
            "node_id": result.node_id,
            "locale": result.locale.as_str(),
            "revision": result.revision,
            "timestamp": result.timestamp.to_rfc3339(),
        }),
    )))
}

/// Handle list translations operation
pub async fn handle_translation_list<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: TranslationListPayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;
    let branch = request.context.branch.as_deref().unwrap_or("main");
    let workspace = request
        .context
        .workspace
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Workspace required".to_string()))?;

    // Get current branch head revision
    let current_revision = state
        .storage
        .branches()
        .get_head(tenant_id, repo, branch)
        .await?;

    // List translations
    let locales = state
        .storage
        .translations()
        .list_translations_for_node(
            tenant_id,
            repo,
            branch,
            workspace,
            &payload.node_path,
            &current_revision,
        )
        .await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::json!({
            "node_id": payload.node_path,
            "locales": locales.iter().map(|l| l.as_str()).collect::<Vec<_>>()
        }),
    )))
}

/// Handle translation delete operation
pub async fn handle_translation_delete<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: TranslationDeletePayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;
    let branch = request.context.branch.as_deref().unwrap_or("main");
    let workspace = request
        .context
        .workspace
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Workspace required".to_string()))?;

    // Parse locale
    let locale = LocaleCode::parse(&payload.locale)
        .map_err(|e| WsError::InvalidRequest(format!("Invalid locale code: {}", e)))?;

    // Create translation service
    let translation_service = TranslationService::new(state.storage.clone());

    // Delete translation
    translation_service
        .delete_translation(
            tenant_id,
            repo,
            branch,
            workspace,
            &payload.node_path,
            &locale,
            "system",
            None,
        )
        .await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::json!({ "success": true }),
    )))
}

/// Handle translation hide operation
pub async fn handle_translation_hide<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: TranslationHidePayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;
    let branch = request.context.branch.as_deref().unwrap_or("main");
    let workspace = request
        .context
        .workspace
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Workspace required".to_string()))?;

    // Parse locale
    let locale = LocaleCode::parse(&payload.locale)
        .map_err(|e| WsError::InvalidRequest(format!("Invalid locale code: {}", e)))?;

    // Create translation service
    let translation_service = TranslationService::new(state.storage.clone());

    // Hide node in locale
    let result = translation_service
        .hide_node(
            tenant_id,
            repo,
            branch,
            workspace,
            &payload.node_path,
            &locale,
            "system",
            None,
        )
        .await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::json!({
            "node_id": result.node_id,
            "locale": result.locale.as_str(),
            "revision": result.revision,
            "timestamp": result.timestamp.to_rfc3339(),
        }),
    )))
}

/// Handle translation unhide operation
pub async fn handle_translation_unhide<S, B>(
    state: &Arc<WsState<S, B>>,
    _connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: TranslationUnhidePayload = serde_json::from_value(request.payload.clone())?;

    let tenant_id = &request.context.tenant_id;
    let repo = request
        .context
        .repository
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Repository required".to_string()))?;
    let branch = request.context.branch.as_deref().unwrap_or("main");
    let workspace = request
        .context
        .workspace
        .as_ref()
        .ok_or_else(|| WsError::InvalidRequest("Workspace required".to_string()))?;

    // Parse locale
    let locale = LocaleCode::parse(&payload.locale)
        .map_err(|e| WsError::InvalidRequest(format!("Invalid locale code: {}", e)))?;

    // Create translation service
    let translation_service = TranslationService::new(state.storage.clone());

    // Unhide node (delete the Hidden overlay)
    translation_service
        .delete_translation(
            tenant_id,
            repo,
            branch,
            workspace,
            &payload.node_path,
            &locale,
            "system",
            Some("Unhide node".to_string()),
        )
        .await?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::json!({ "success": true }),
    )))
}
