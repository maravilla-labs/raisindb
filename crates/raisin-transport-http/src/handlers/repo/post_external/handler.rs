// SPDX-License-Identifier: BSL-1.1

//! Main external upload handler entry point.

use axum::{extract::Json, http::StatusCode};
use futures_util::StreamExt;
use raisin_binary::BinaryStorage;
use raisin_core::NodeService;
use raisin_models::auth::AuthContext;
use raisin_storage::{transactional::TransactionalStorage, Storage};

use crate::{error::ApiError, state::AppState, types::RepoQuery, upload_processors::StorageFormat};

use super::commit::handle_external_upload_commit;
use super::direct::handle_external_upload_direct;
use super::resource::build_resource_value;

/// Handle external storage file upload.
///
/// This is the main path for multipart uploads under 100MB that store
/// content in binary storage (not inline).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_external_upload<S: Storage + TransactionalStorage + 'static>(
    state: &AppState,
    nodes_svc: &NodeService<S>,
    path: &str,
    ws: &str,
    q: &RepoQuery,
    field: multer::Field<'_>,
    file_name: Option<String>,
    archetype: Option<String>,
    ext: Option<String>,
    override_existing: bool,
    auth_context: Option<AuthContext>,
    tenant_id: &str,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let param_node_type = q
        .node_type
        .clone()
        .unwrap_or_else(|| "raisin:Asset".to_string());

    // Check if there's a processor for this node type
    let processor = state.upload_processors.get_processor(&param_node_type);

    // For processor-handled node types, buffer the file data to extract metadata
    let (stored, processed_upload) = if let Some(processor) = processor {
        let file_data = field
            .bytes()
            .await
            .map_err(|e| ApiError::validation_failed(format!("Failed to read file: {}", e)))?;

        let processed = processor.process(&file_data, file_name.as_deref(), path)?;

        let stored = state
            .bin
            .put_bytes(
                &file_data,
                archetype.as_deref(),
                ext.as_deref(),
                file_name.as_deref(),
                None,
            )
            .await
            .map_err(|e| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "STORAGE_ERROR",
                    format!("Failed to store file: {}", e),
                )
            })?;

        (stored, Some(processed))
    } else {
        // Handle override: delete old file if it exists
        if override_existing {
            #[allow(clippy::collapsible_match)]
            if let Ok(Some(prop_value)) = nodes_svc.get_property_by_path(path, "file").await {
                if let raisin_models::nodes::properties::PropertyValue::Resource(ref old_resource) =
                    prop_value
                {
                    if let Some(ref metadata) = old_resource.metadata {
                        if let Some(raisin_models::nodes::properties::PropertyValue::String(
                            storage_key,
                        )) = metadata.get("storage_key")
                        {
                            let _ = state.bin.delete(storage_key).await;
                        }
                    }
                }
            }
        }

        // Stream the file to storage without buffering the whole body
        let stream = field.map(|res| res.map_err(|e| std::io::Error::other(e.to_string())));
        let stored = state
            .bin
            .put_stream(
                stream,
                archetype.as_deref(),
                ext.as_deref(),
                file_name.as_deref(),
                None,
                None,
            )
            .await
            .map_err(|e| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "STORAGE_ERROR",
                    format!("Failed to store file: {}", e),
                )
            })?;

        (stored, None)
    };

    // Determine resource property path and storage format from processor or defaults
    let (param_prop_path, storage_format, extra_properties, node_id_override, node_name_override) =
        if let Some(ref processed) = processed_upload {
            (
                processed.resource_property.clone(),
                processed.storage_format,
                processed.properties.clone(),
                processed.node_id.clone(),
                processed.node_name.clone(),
            )
        } else {
            (
                q.property_path
                    .clone()
                    .unwrap_or_else(|| "file".to_string()),
                StorageFormat::Resource,
                std::collections::HashMap::new(),
                None,
                None,
            )
        };

    // Build the resource property value based on storage format
    let resource_value = build_resource_value(&stored, storage_format);

    let mut property_updates = serde_json::Map::new();
    property_updates.insert(
        param_prop_path.clone(),
        serde_json::to_value(&resource_value)
            .map_err(|e| ApiError::serialization_error(e.to_string()))?,
    );
    if let Some(mime) = &stored.mime_type {
        property_updates.insert(format!("{}_type", param_prop_path), serde_json::json!(mime));
    }
    property_updates.insert(
        format!("{}_size", param_prop_path),
        serde_json::json!(stored.size),
    );

    if let Some(message) = q.commit_message.clone() {
        return handle_external_upload_commit(
            nodes_svc,
            path,
            ws,
            q,
            &param_node_type,
            &stored,
            property_updates,
            &extra_properties,
            &node_id_override,
            &node_name_override,
            file_name.as_deref(),
            auth_context,
            message,
        )
        .await;
    }

    // Direct mode: upsert without commit
    handle_external_upload_direct(
        state,
        nodes_svc,
        path,
        ws,
        &param_node_type,
        &stored,
        &param_prop_path,
        resource_value,
        property_updates,
        &extra_properties,
        &node_id_override,
        &node_name_override,
        file_name.as_deref(),
        tenant_id,
    )
    .await
}
