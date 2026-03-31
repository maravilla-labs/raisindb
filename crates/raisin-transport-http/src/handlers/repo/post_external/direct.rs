// SPDX-License-Identifier: BSL-1.1

//! Direct-mode external upload: upsert without creating a revision.

use axum::{extract::Json, http::StatusCode};
use raisin_core::NodeService;
use raisin_models as models;
use raisin_storage::{transactional::TransactionalStorage, Storage};

use crate::{error::ApiError, state::AppState};

/// Direct-mode external upload: upsert without creating a revision.
#[allow(clippy::too_many_arguments)]
pub(super) async fn handle_external_upload_direct<S: Storage + TransactionalStorage + 'static>(
    state: &AppState,
    nodes_svc: &NodeService<S>,
    path: &str,
    ws: &str,
    param_node_type: &str,
    stored: &raisin_binary::StoredObject,
    param_prop_path: &str,
    resource_value: raisin_models::nodes::properties::PropertyValue,
    property_updates: serde_json::Map<String, serde_json::Value>,
    extra_properties: &std::collections::HashMap<
        String,
        raisin_models::nodes::properties::PropertyValue,
    >,
    node_id_override: &Option<String>,
    node_name_override: &Option<String>,
    file_name: Option<&str>,
    tenant_id: &str,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let asset_name = node_name_override.clone().unwrap_or_else(|| {
        file_name
            .or_else(|| path.rsplit('/').next())
            .unwrap_or("asset")
            .to_string()
    });

    // Build the effective path
    let effective_path = if param_node_type == "raisin:Package" {
        if let Some(ref name) = node_name_override {
            let parent = path.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
            if parent.is_empty() {
                format!("/{}", name)
            } else {
                format!("{}/{}", parent, name)
            }
        } else {
            path.to_string()
        }
    } else {
        path.to_string()
    };

    // Check if node exists at the upload path
    let existing_node = nodes_svc
        .get_by_path(&effective_path)
        .await
        .map_err(ApiError::from)?;

    let created_node_id = if let Some(mut node) = existing_node {
        let mut updated_props = node.properties.clone();
        updated_props.insert(param_prop_path.to_string(), resource_value);
        for (k, v) in extra_properties.clone() {
            updated_props.insert(k, v);
        }

        if param_node_type == "raisin:Package" {
            updated_props.insert(
                "upload_state".to_string(),
                raisin_models::nodes::properties::PropertyValue::String("updated".to_string()),
            );
            updated_props.insert(
                "installed".to_string(),
                raisin_models::nodes::properties::PropertyValue::Boolean(false),
            );

            if node.path != effective_path {
                tracing::info!(
                    node_path = %node.path,
                    expected_path = %effective_path,
                    "Updating legacy package (keeping existing path)"
                );
            }
        }

        let node_id = node.id.clone();
        node.properties = updated_props;
        nodes_svc.update_node(node).await?;
        node_id
    } else {
        let mut props = extra_properties.clone();
        if !props.contains_key("title") {
            props.insert(
                "title".to_string(),
                raisin_models::nodes::properties::PropertyValue::String(asset_name.clone()),
            );
        }
        props.insert(param_prop_path.to_string(), resource_value);

        let node_id = node_id_override
            .clone()
            .unwrap_or_else(|| nanoid::nanoid!());

        let node = models::nodes::Node {
            id: node_id.clone(),
            name: asset_name.clone(),
            path: effective_path.clone(),
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
        nodes_svc.create(node).await?;
        node_id
    };

    // Enqueue PackageProcess job for raisin:Package uploads
    #[cfg(feature = "storage-rocksdb")]
    if param_node_type == "raisin:Package" {
        super::jobs::enqueue_package_process_job(state, &created_node_id, stored, ws, tenant_id)
            .await;
    }

    let response = if node_id_override.is_some() {
        serde_json::json!({
            "storedKey": stored.key,
            "url": stored.url,
            "node_id": node_id_override
        })
    } else {
        serde_json::json!({"storedKey": stored.key, "url": stored.url})
    };

    Ok((StatusCode::OK, Json(response)))
}
