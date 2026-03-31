// SPDX-License-Identifier: BSL-1.1

//! Commit-mode external upload: creates a transaction and commits.

use axum::{extract::Json, http::StatusCode};
use raisin_core::NodeService;
use raisin_models as models;
use raisin_models::auth::AuthContext;
use raisin_storage::{transactional::TransactionalStorage, Storage};

use crate::{error::ApiError, types::RepoQuery};

/// Commit-mode external upload: creates a transaction and commits.
#[allow(clippy::too_many_arguments)]
pub(super) async fn handle_external_upload_commit<S: Storage + TransactionalStorage + 'static>(
    nodes_svc: &NodeService<S>,
    path: &str,
    ws: &str,
    q: &RepoQuery,
    param_node_type: &str,
    stored: &raisin_binary::StoredObject,
    property_updates: serde_json::Map<String, serde_json::Value>,
    extra_properties: &std::collections::HashMap<
        String,
        raisin_models::nodes::properties::PropertyValue,
    >,
    node_id_override: &Option<String>,
    node_name_override: &Option<String>,
    file_name: Option<&str>,
    auth_context: Option<AuthContext>,
    message: String,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let actor = q.commit_actor.clone().unwrap_or_else(|| {
        auth_context
            .as_ref()
            .map(|ctx| ctx.actor_id())
            .unwrap_or_else(|| "system".to_string())
    });

    let mut tx = nodes_svc.transaction();

    if let Some(existing) = nodes_svc.get_by_path(path).await? {
        let mut updates = property_updates.clone();
        for (k, v) in extra_properties {
            updates.insert(
                k.clone(),
                serde_json::to_value(v)
                    .map_err(|e| ApiError::serialization_error(e.to_string()))?,
            );
        }
        tx.update(existing.id.clone(), serde_json::Value::Object(updates));
    } else {
        let asset_name = node_name_override.clone().unwrap_or_else(|| {
            file_name
                .or_else(|| path.rsplit('/').next())
                .unwrap_or("asset")
                .to_string()
        });

        let node_id = node_id_override
            .clone()
            .unwrap_or_else(|| nanoid::nanoid!());

        let mut props = extra_properties.clone();
        if !props.contains_key("title") {
            props.insert(
                "title".to_string(),
                raisin_models::nodes::properties::PropertyValue::String(asset_name.clone()),
            );
        }
        for (k, v) in property_updates {
            props.insert(k, serde_json::from_value(v)?);
        }

        let new_node = models::nodes::Node {
            id: node_id.clone(),
            name: asset_name,
            path: path.to_string(),
            node_type: param_node_type.to_string(),
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

    Ok((
        StatusCode::OK,
        Json(serde_json::json!({
            "revision": revision,
            "committed": true
        })),
    ))
}
