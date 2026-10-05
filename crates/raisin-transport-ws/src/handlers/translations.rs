// SPDX-License-Identifier: BSL-1.1

//! Translation operation handlers.
//!
//! Thin adapters over the translation commands on `NodeService`
//! (`raisin-core`, `services/translation_service/commands.rs`), the one
//! implementation the HTTP `raisin:cmd/translate|delete-translation|
//! hide-in-locale|unhide-in-locale` commands call too. The node is resolved
//! under the connection's RLS, writes require `Update` on it, the overlay is
//! keyed by the resolved node ID, and the actor is the connection's
//! authenticated user. This file only parses payloads and shapes responses.
//!
//! The payload field is `node_path` for wire compatibility. It accepts an
//! absolute path (`/a/b`) or a node id.

use parking_lot::RwLock;
use raisin_core::{parse_translation_fields, NodeRef, NodeService, TranslationUpdateResult};
use raisin_models::translations::LocaleCode;
use raisin_storage::transactional::TransactionalStorage;
use std::sync::Arc;

use super::nodes::helpers::{build_node_service, extract_context};
use crate::{
    connection::ConnectionState,
    error::WsError,
    handler::WsState,
    protocol::{
        RequestEnvelope, ResponseEnvelope, TranslationDeletePayload, TranslationHidePayload,
        TranslationListPayload, TranslationUnhidePayload, TranslationUpdatePayload,
    },
};

/// The connection's node service for the request's tenant/repo/branch/workspace.
fn node_service<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: &RequestEnvelope,
) -> Result<NodeService<S>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let ctx = extract_context(request)?;
    Ok(build_node_service(state, connection_state, &ctx))
}

fn parse_locale(locale: &str) -> Result<LocaleCode, WsError> {
    LocaleCode::parse(locale)
        .map_err(|e| WsError::InvalidRequest(format!("Invalid locale code: {}", e)))
}

/// Core errors to WS errors. A refusal must read as one, not as a storage fault.
fn map_err(err: raisin_error::Error) -> WsError {
    match err {
        raisin_error::Error::PermissionDenied(_) | raisin_error::Error::Forbidden(_) => {
            WsError::PermissionDenied
        }
        raisin_error::Error::NotFound(msg) | raisin_error::Error::Validation(msg) => {
            WsError::InvalidRequest(msg)
        }
        other => other.into(),
    }
}

fn update_result_json(result: TranslationUpdateResult) -> serde_json::Value {
    serde_json::json!({
        "node_id": result.node_id,
        "locale": result.locale.as_str(),
        "revision": result.revision,
        "timestamp": result.timestamp.to_rfc3339(),
    })
}

/// Handle translation update operation
pub async fn handle_translation_update<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: TranslationUpdatePayload = serde_json::from_value(request.payload.clone())?;
    let svc = node_service(state, connection_state, &request)?;
    let locale = parse_locale(&payload.locale)?;
    let translations = parse_translation_fields(payload.properties).map_err(map_err)?;

    let result = svc
        .translate(
            NodeRef::parse(&payload.node_path),
            &locale,
            translations,
            None,
        )
        .await
        .map_err(map_err)?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        update_result_json(result),
    )))
}

/// Handle list translations operation
pub async fn handle_translation_list<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: TranslationListPayload = serde_json::from_value(request.payload.clone())?;
    let svc = node_service(state, connection_state, &request)?;

    let listed = svc
        .list_node_translations(NodeRef::parse(&payload.node_path))
        .await
        .map_err(map_err)?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::json!({
            "node_id": listed.node_id,
            "locales": listed.locales.iter().map(|l| l.as_str()).collect::<Vec<_>>()
        }),
    )))
}

/// Handle translation delete operation
pub async fn handle_translation_delete<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: TranslationDeletePayload = serde_json::from_value(request.payload.clone())?;
    let svc = node_service(state, connection_state, &request)?;
    let locale = parse_locale(&payload.locale)?;

    svc.delete_locale_translation(NodeRef::parse(&payload.node_path), &locale, None)
        .await
        .map_err(map_err)?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::json!({ "success": true }),
    )))
}

/// Handle translation hide operation
pub async fn handle_translation_hide<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: TranslationHidePayload = serde_json::from_value(request.payload.clone())?;
    let svc = node_service(state, connection_state, &request)?;
    let locale = parse_locale(&payload.locale)?;

    let result = svc
        .hide_in_locale(NodeRef::parse(&payload.node_path), &locale, None)
        .await
        .map_err(map_err)?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        update_result_json(result),
    )))
}

/// Handle translation unhide operation
pub async fn handle_translation_unhide<S, B>(
    state: &Arc<WsState<S, B>>,
    connection_state: &Arc<RwLock<ConnectionState>>,
    request: RequestEnvelope,
) -> Result<Option<ResponseEnvelope>, WsError>
where
    S: raisin_storage::Storage + TransactionalStorage,
    B: raisin_binary::BinaryStorage,
{
    let payload: TranslationUnhidePayload = serde_json::from_value(request.payload.clone())?;
    let svc = node_service(state, connection_state, &request)?;
    let locale = parse_locale(&payload.locale)?;

    svc.unhide_in_locale(NodeRef::parse(&payload.node_path), &locale, None)
        .await
        .map_err(map_err)?;

    Ok(Some(ResponseEnvelope::success(
        request.request_id,
        serde_json::json!({ "success": true }),
    )))
}
