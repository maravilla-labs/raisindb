// SPDX-License-Identifier: BSL-1.1

//! Multipart upload handlers (buffered, <100MB).
//!
//! Handles both inline uploads (content stored as PropertyValue::String)
//! and external storage uploads dispatched via [`super::post_external`].

use axum::{
    body::{Body, Bytes},
    extract::Json,
    http::StatusCode,
};
use multer::Multipart;
use raisin_core::NodeService;
use raisin_models as models;
use raisin_models::auth::AuthContext;
use raisin_storage::{transactional::TransactionalStorage, Storage};

use crate::{error::ApiError, state::AppState, types::RepoQuery};

use super::post_external::handle_external_upload;

/// Handle multipart file upload (buffered, <100MB).
pub(super) async fn handle_multipart_upload<S: Storage + TransactionalStorage + 'static>(
    state: &AppState,
    nodes_svc: &NodeService<S>,
    ctx: &crate::middleware::RaisinContext,
    repo: &str,
    branch: &str,
    ws: &str,
    path: &str,
    q: &RepoQuery,
    archetype_header: &str,
    bytes: Bytes,
    auth_context: Option<AuthContext>,
    tenant_id: &str,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let boundary = multer::parse_boundary(archetype_header)
        .map_err(|_| ApiError::validation_failed("Invalid multipart boundary"))?;
    let body = Body::from(bytes);
    let mut multipart = Multipart::new(body.into_data_stream(), boundary);

    let inline = q.inline.unwrap_or(false);
    let override_existing = q.override_existing.unwrap_or(false);

    if let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| ApiError::validation_failed("Invalid multipart field"))?
    {
        let file_name = field.file_name().map(|s| s.to_string());
        let archetype = field.content_type().map(|ct| ct.to_string());
        let ext = file_name.as_deref().and_then(|n| {
            std::path::Path::new(n)
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
        });

        if inline {
            return handle_inline_upload(
                state,
                nodes_svc,
                path,
                ws,
                q,
                field,
                file_name.as_deref(),
                archetype.as_deref(),
                auth_context,
            )
            .await;
        }

        // External storage upload
        return handle_external_upload(
            state,
            nodes_svc,
            path,
            ws,
            q,
            field,
            file_name,
            archetype,
            ext,
            override_existing,
            auth_context,
            tenant_id,
        )
        .await;
    }

    Err(ApiError::validation_failed(
        "No file field found in multipart request",
    ))
}

/// Handle inline file upload (content stored as PropertyValue::String).
async fn handle_inline_upload<S: Storage + TransactionalStorage + 'static>(
    state: &AppState,
    nodes_svc: &NodeService<S>,
    path: &str,
    ws: &str,
    q: &RepoQuery,
    field: multer::Field<'_>,
    file_name: Option<&str>,
    archetype: Option<&str>,
    auth_context: Option<AuthContext>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    const MAX_INLINE_SIZE: u64 = 11 * 1024 * 1024; // 11MB

    let bytes = field
        .bytes()
        .await
        .map_err(|_| ApiError::validation_failed("Failed to read file bytes"))?;

    if bytes.len() as u64 > MAX_INLINE_SIZE {
        return Err(ApiError::payload_too_large(
            bytes.len(),
            MAX_INLINE_SIZE as usize,
        ));
    }

    let content = String::from_utf8(bytes.to_vec())
        .map_err(|_| ApiError::validation_failed("File content must be valid UTF-8"))?;

    let value = raisin_models::nodes::properties::PropertyValue::String(content);

    let mut property_updates = serde_json::Map::new();
    property_updates.insert(
        "file".to_string(),
        serde_json::to_value(&value).map_err(|e| ApiError::serialization_error(e.to_string()))?,
    );
    if let Some(mime) = archetype {
        property_updates.insert("file_type".to_string(), serde_json::json!(mime));
    }
    property_updates.insert(
        "file_size".to_string(),
        serde_json::json!(bytes.len() as u64),
    );

    if let Some(message) = q.commit_message.clone() {
        let actor = q.commit_actor.clone().unwrap_or_else(|| {
            auth_context
                .as_ref()
                .map(|ctx| ctx.actor_id())
                .unwrap_or_else(|| "system".to_string())
        });

        let mut tx = nodes_svc.transaction();

        if let Some(existing) = nodes_svc.get_by_path(path).await? {
            tx.update(
                existing.id.clone(),
                serde_json::Value::Object(property_updates),
            );
        } else {
            let asset_name = file_name
                .or_else(|| path.rsplit('/').next())
                .unwrap_or("asset")
                .to_string();

            let mut props = std::collections::HashMap::new();
            props.insert(
                "title".to_string(),
                raisin_models::nodes::properties::PropertyValue::String(asset_name.clone()),
            );
            props.insert(
                "file".to_string(),
                serde_json::from_value(serde_json::Value::Object(property_updates))?,
            );

            let new_node = models::nodes::Node {
                id: nanoid::nanoid!(),
                name: asset_name,
                path: path.to_string(),
                node_type: "raisin:Asset".into(),
                archetype: None,
                properties: props,
                children: vec![],
                order_key: String::new(),
                has_children: None,
                parent: None,
                version: 1,
                created_at: None,
                updated_at: None,
                published_at: None,
                published_by: None,
                updated_by: None,
                created_by: None,
                translations: None,
                tenant_id: None,
                workspace: Some(ws.to_string()),
                owner_id: None,
                relations: Vec::new(),
            };
            tx.create(new_node);
        }

        let revision = tx.commit(message, actor).await?;

        return Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "revision": revision,
                "committed": true
            })),
        ));
    }

    // Direct mode: update without commit
    let asset_name = file_name
        .or_else(|| path.rsplit('/').next())
        .unwrap_or("asset")
        .to_string();

    let mut node = nodes_svc
        .get_by_path(path)
        .await
        .map_err(ApiError::from)?
        .unwrap_or_else(|| {
            let mut props = std::collections::HashMap::new();
            props.insert(
                "title".to_string(),
                raisin_models::nodes::properties::PropertyValue::String(asset_name.clone()),
            );
            models::nodes::Node {
                id: nanoid::nanoid!(),
                name: asset_name.clone(),
                path: path.to_string(),
                node_type: "raisin:Asset".into(),
                archetype: None,
                properties: props,
                children: vec![],
                order_key: String::new(),
                has_children: None,
                parent: None,
                version: 1,
                created_at: None,
                updated_at: None,
                published_at: None,
                published_by: None,
                updated_by: None,
                created_by: None,
                translations: None,
                tenant_id: None,
                workspace: Some(ws.to_string()),
                owner_id: None,
                relations: Vec::new(),
            }
        });
    node.properties.insert("file".to_string(), value);
    nodes_svc.put(node).await?;
    Ok((
        StatusCode::OK,
        Json(serde_json::json!({"status": "inline file uploaded"})),
    ))
}
