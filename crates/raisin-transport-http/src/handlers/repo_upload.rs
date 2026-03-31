// SPDX-License-Identifier: BSL-1.1
//! Upload handlers for RaisinDB binary storage.
//!
//! This module contains handlers for file uploads, including large multipart
//! uploads that stream directly to storage without buffering.

use axum::body::Body;
use axum::http::StatusCode;
use axum::Json;
use futures_util::StreamExt;
use multer::Multipart;
use raisin_binary::BinaryStorage;
use raisin_models as models;
use raisin_models::auth::AuthContext;
use raisin_storage::Storage;

use crate::error::ApiError;
use crate::middleware::RaisinContext;
use crate::state::AppState;
use crate::types::RepoQuery;

/// Threshold for considering a file as a "large upload" (100MB).
/// Files larger than this are streamed directly to storage without buffering.
pub const BUFFER_THRESHOLD: u64 = 100 * 1024 * 1024;

/// Handle large multipart uploads (>=100MB) by streaming directly to storage.
///
/// For large packages:
/// - Streams file to storage with a temp name (upload_{timestamp}_{random})
/// - Creates a raisin:Package node with status="processing"
/// - Background job (PackageProcess) extracts manifest, renames to correct name, handles upsert
///
/// This avoids buffering 40GB+ files in memory.
pub async fn handle_large_multipart_upload(
    state: AppState,
    ctx: RaisinContext,
    repo: String,
    branch: String,
    ws: String,
    path: String,
    q: RepoQuery,
    request: axum::http::Request<Body>,
    archetype_header: &str,
    auth_context: Option<AuthContext>,
    tenant_id: &str,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let nodes_svc = state.node_service_for_context(tenant_id, &repo, &branch, &ws, auth_context.clone());

    // Parse multipart boundary
    let boundary = multer::parse_boundary(archetype_header)
        .map_err(|_| ApiError::validation_failed("Invalid multipart boundary"))?;

    // Create multipart from request body stream (no buffering!)
    let body = request.into_body();
    let mut multipart = Multipart::new(body.into_data_stream(), boundary);

    // Get the node type from query params
    let param_node_type = q.node_type.clone().unwrap_or_else(|| "raisin:Asset".to_string());

    // Process the first file field
    if let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| ApiError::validation_failed("Invalid multipart field"))?
    {
        let file_name = field.file_name().map(|s| s.to_string());
        let archetype = field.content_type().map(|ct| ct.to_string());
        let ext = file_name
            .as_deref()
            .and_then(|n| std::path::Path::new(n).extension().and_then(|s| s.to_str()));

        // Stream the file directly to storage
        let stream = field.map(|res| res.map_err(|e| std::io::Error::other(e.to_string())));
        let stored = state
            .bin
            .put_stream(
                stream,
                archetype.as_deref(),
                ext,
                file_name.as_deref(),
                None,
                None, // tenant_context
            )
            .await
            .map_err(|e| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "STORAGE_ERROR",
                    format!("Failed to store file: {}", e),
                )
            })?;

        tracing::info!(
            stored_key = %stored.key,
            size = stored.size,
            "Large file streamed to storage"
        );

        // For packages, create a node with temporary name and "processing" status
        // The background job will extract manifest and update the node
        if param_node_type == "raisin:Package" {
            // Generate temp name - will be updated by background job after manifest extraction
            let temp_name = format!("upload_{}_{}", chrono::Utc::now().timestamp(), nanoid::nanoid!(8));
            let temp_path = format!("/{}", temp_name);

            // Build resource property (Object format for packages)
            let mut resource_obj = std::collections::HashMap::new();
            resource_obj.insert(
                "key".to_string(),
                raisin_models::nodes::properties::PropertyValue::String(stored.key.clone()),
            );
            resource_obj.insert(
                "url".to_string(),
                raisin_models::nodes::properties::PropertyValue::String(stored.url.clone()),
            );
            if let Some(mime) = &stored.mime_type {
                resource_obj.insert(
                    "mime_type".to_string(),
                    raisin_models::nodes::properties::PropertyValue::String(mime.clone()),
                );
            }
            resource_obj.insert(
                "size".to_string(),
                raisin_models::nodes::properties::PropertyValue::Integer(stored.size),
            );
            let resource_value = raisin_models::nodes::properties::PropertyValue::Object(resource_obj);

            // Create initial properties for the processing package
            let mut props = std::collections::HashMap::new();
            props.insert(
                "title".to_string(),
                raisin_models::nodes::properties::PropertyValue::String(temp_name.clone()),
            );
            props.insert("resource".to_string(), resource_value);
            props.insert(
                "status".to_string(),
                raisin_models::nodes::properties::PropertyValue::String("processing".to_string()),
            );
            props.insert(
                "installed".to_string(),
                raisin_models::nodes::properties::PropertyValue::Boolean(false),
            );
            props.insert(
                "upload_state".to_string(),
                raisin_models::nodes::properties::PropertyValue::String("new".to_string()),
            );
            // Mark as large upload so job knows to extract manifest and rename
            props.insert(
                "large_upload".to_string(),
                raisin_models::nodes::properties::PropertyValue::Boolean(true),
            );
            props.insert(
                "progress".to_string(),
                raisin_models::nodes::properties::PropertyValue::Float(0.0),
            );

            let node_id = nanoid::nanoid!();

            let node = models::nodes::Node {
                id: node_id.clone(),
                name: temp_name.clone(),
                path: temp_path.clone(),
                node_type: "raisin:Package".to_string(),
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
                workspace: Some(ws.clone()),
                owner_id: None,
                relations: Vec::new(),
            };

            nodes_svc.create(node).await?;

            // Enqueue PackageProcess job - it will handle manifest extraction and renaming
            #[cfg(feature = "storage-rocksdb")]
            let job_id = {
                if let Some(rocksdb) = state.rocksdb_storage.as_ref() {
                    let job_registry = rocksdb.job_registry();
                    let job_data_store = rocksdb.job_data_store();

                    let job_type = raisin_storage::jobs::JobType::PackageProcess {
                        package_node_id: node_id.clone(),
                    };

                    // Create job context with resource_key in metadata
                    let mut metadata = std::collections::HashMap::new();
                    metadata.insert("resource_key".to_string(), serde_json::json!(stored.key.clone()));
                    metadata.insert("large_upload".to_string(), serde_json::json!(true));

                    let job_context = raisin_storage::jobs::JobContext {
                        tenant_id: tenant_id.to_string(),
                        repo_id: repo.clone(),
                        branch: branch.clone(),
                        workspace_id: ws.clone(),
                        revision: raisin_hlc::HLC::now(),
                        metadata,
                    };

                    match job_registry
                        .register_job(job_type, Some(tenant_id.to_string()), None, None, None)
                        .await
                    {
                        Ok(job_id) => {
                            if let Err(e) = job_data_store.put(&job_id, &job_context) {
                                tracing::warn!(
                                    job_id = %job_id,
                                    error = %e,
                                    "Failed to store job context for large package processing"
                                );
                            } else {
                                tracing::info!(
                                    job_id = %job_id,
                                    package_node_id = %node_id,
                                    "Enqueued PackageProcess job for large upload"
                                );
                            }
                            Some(job_id)
                        }
                        Err(e) => {
                            tracing::warn!(
                                package_node_id = %node_id,
                                error = %e,
                                "Failed to register PackageProcess job for large upload"
                            );
                            None
                        }
                    }
                } else {
                    None
                }
            };

            #[cfg(not(feature = "storage-rocksdb"))]
            let job_id: Option<String> = None;

            return Ok((
                StatusCode::OK,
                Json(serde_json::json!({
                    "storedKey": stored.key,
                    "url": stored.url,
                    "node_id": node_id,
                    "job_id": job_id,
                    "status": "processing",
                    "message": "Large file uploaded. Processing in background."
                })),
            ));
        }

        // For non-package large uploads, just store and create standard node
        let param_prop_path = q.property_path.clone().unwrap_or_else(|| "file".to_string());

        // Build resource property (standard Resource format)
        let mut metadata = std::collections::HashMap::new();
        metadata.insert(
            "storage_key".to_string(),
            raisin_models::nodes::properties::PropertyValue::String(stored.key.clone()),
        );

        let resource = raisin_models::nodes::properties::value::Resource {
            uuid: nanoid::nanoid!(),
            name: stored.name.clone(),
            size: Some(stored.size),
            mime_type: stored.mime_type.clone(),
            url: Some(stored.key.clone()),
            metadata: Some(metadata),
            is_loaded: Some(true),
            is_external: Some(false),
            created_at: stored.created_at.into(),
            updated_at: stored.updated_at.into(),
        };
        let resource_value = raisin_models::nodes::properties::PropertyValue::Resource(resource);

        let asset_name = file_name
            .as_deref()
            .or_else(|| path.rsplit('/').next())
            .unwrap_or("asset")
            .to_string();

        let mut props = std::collections::HashMap::new();
        props.insert(
            "title".to_string(),
            raisin_models::nodes::properties::PropertyValue::String(asset_name.clone()),
        );
        props.insert(param_prop_path.clone(), resource_value);

        let node_id = nanoid::nanoid!();

        let node = models::nodes::Node {
            id: node_id.clone(),
            name: asset_name,
            path: ctx.cleaned_path.clone(),
            node_type: param_node_type,
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
            workspace: Some(ws.clone()),
            owner_id: None,
            relations: Vec::new(),
        };

        nodes_svc.create(node).await?;

        return Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "storedKey": stored.key,
                "url": stored.url,
                "node_id": node_id
            })),
        ));
    }

    Err(ApiError::validation_failed(
        "No file field found in multipart request",
    ))
}
