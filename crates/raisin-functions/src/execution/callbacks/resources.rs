// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Resource operation callbacks for function execution.
//!
//! These callbacks implement resource-related operations for the JavaScript SDK:
//! - `resource.getBinary()` - Get binary data from storage
//! - `node.addResource()` - Upload and attach a resource to a node

use std::collections::HashMap;
use std::sync::Arc;

use base64::Engine;
use raisin_binary::BinaryStorage;
use raisin_core::services::node_service::NodeService;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::value::Resource;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::Storage;
use serde_json::Value;

use crate::api::{NodeAddResourceCallback, ResourceGetBinaryCallback};

/// Create resource_get_binary callback: Gets binary data from storage key.
///
/// This callback is used by the JavaScript Resource class to fetch file contents.
/// The storage key comes from `node.properties.file.metadata.storage_key`.
///
/// # Arguments
/// - `storage_key`: The key to retrieve (e.g., "uploads/tenant/abc123.jpg")
///
/// # Returns
/// - Base64-encoded binary data
pub fn create_resource_get_binary<B>(binary_storage: Arc<B>) -> ResourceGetBinaryCallback
where
    B: BinaryStorage + 'static,
{
    Arc::new(move |storage_key: String| {
        let storage = binary_storage.clone();

        Box::pin(async move {
            // Get binary data from storage
            let data = storage.get(&storage_key).await.map_err(|e| {
                raisin_error::Error::Backend(format!(
                    "Failed to retrieve binary from '{}': {}",
                    storage_key, e
                ))
            })?;

            // Encode as base64
            let base64_data = base64::engine::general_purpose::STANDARD.encode(&data);

            Ok(base64_data)
        })
    })
}

/// Create node_add_resource callback: Uploads binary and attaches to node property.
///
/// This callback handles the `node.addResource(propertyPath, data)` JavaScript API.
/// It supports:
/// - Base64-encoded data with mimeType
/// - Temp file handles (from resize operations) - future
///
/// # Arguments
/// - `workspace`: Target workspace
/// - `node_path`: Path to the node
/// - `property_path`: Property name to set (e.g., "thumbnail")
/// - `upload_data`: JSON with either:
///   - `{ base64: "...", mimeType: "image/jpeg", name: "thumbnail.jpg" }`
///   - `{ tempHandle: "..." }` (future: for processed images)
///
/// # Returns
/// - Updated resource metadata as JSON
pub fn create_node_add_resource<S, B>(
    storage: Arc<S>,
    binary_storage: Arc<B>,
    tenant_id: String,
    repo_id: String,
    branch: String,
    auth_context: Option<AuthContext>,
) -> NodeAddResourceCallback
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    Arc::new(
        move |workspace: String, node_path: String, property_path: String, upload_data: Value| {
            let storage = storage.clone();
            let binary_storage = binary_storage.clone();
            let tenant = tenant_id.clone();
            let repo = repo_id.clone();
            let branch = branch.clone();
            // Use system context if no auth provided (for trigger/function execution)
            let auth = auth_context.clone().unwrap_or_else(AuthContext::system);

            Box::pin(async move {
                // Parse upload_data
                let base64_data = upload_data
                    .get("base64")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        raisin_error::Error::Validation(
                            "upload_data must contain 'base64' field".to_string(),
                        )
                    })?;

                let mime_type = upload_data
                    .get("mimeType")
                    .and_then(|v| v.as_str())
                    .unwrap_or("application/octet-stream");

                let name = upload_data
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&property_path);

                // Decode base64
                let binary_data = base64::engine::general_purpose::STANDARD
                    .decode(base64_data)
                    .map_err(|e| {
                        raisin_error::Error::Validation(format!("Invalid base64 data: {}", e))
                    })?;

                let size = binary_data.len() as i64;
                let uuid = uuid::Uuid::new_v4().to_string();

                // Extract extension from mime type for storage
                let extension = mime_type
                    .split('/')
                    .next_back()
                    .unwrap_or("bin")
                    .replace("jpeg", "jpg");

                // Upload to binary storage using put_bytes
                let stored = binary_storage
                    .put_bytes(
                        &binary_data,
                        Some(mime_type),
                        Some(&extension),
                        Some(name),
                        Some(&tenant),
                    )
                    .await
                    .map_err(|e| {
                        raisin_error::Error::Backend(format!("Failed to upload binary: {}", e))
                    })?;

                // Use the storage key from the stored object
                let storage_key = stored.key;

                // Create resource metadata with storage_key
                let mut resource_metadata = HashMap::new();
                resource_metadata.insert(
                    "storage_key".to_string(),
                    PropertyValue::String(storage_key.clone()),
                );

                // Get current time for timestamps
                let now = chrono::Utc::now();

                // Create Resource struct
                let resource = Resource {
                    uuid: uuid.clone(),
                    name: Some(name.to_string()),
                    size: Some(size),
                    mime_type: Some(mime_type.to_string()),
                    url: None,
                    metadata: Some(resource_metadata),
                    is_loaded: Some(true),
                    is_external: Some(false),
                    created_at: now.into(),
                    updated_at: now.into(),
                };

                // Create PropertyValue::Resource
                let resource_value = PropertyValue::Resource(resource);

                // Create NodeService for this workspace with auth context
                let svc = NodeService::new_with_context(storage, tenant, repo, branch, workspace)
                    .with_auth(auth);

                // Use NodeService to update property - provides audit logging and events
                svc.update_property_by_path(&node_path, &property_path, resource_value)
                    .await?;

                // Return resource metadata
                Ok(serde_json::json!({
                    "uuid": uuid,
                    "name": name,
                    "size": size,
                    "mimeType": mime_type,
                    "storageKey": storage_key,
                }))
            })
        },
    )
}

/// Create pdf_process_from_storage callback: Process PDF from storage key.
///
/// This callback is used by the JavaScript `resource.processDocument()` method
/// to process PDFs using storage keys (avoiding base64 overhead for large files).
///
/// # Arguments
/// - `storage_key`: The key to the PDF file (e.g., "uploads/tenant/abc123.pdf")
/// - `options`: Processing options as JSON (ocr, ocrLanguages, generateThumbnail, thumbnailWidth)
///
/// # Returns
/// - JSON with text, pageCount, isScanned, ocrUsed, thumbnail
pub fn create_pdf_process_from_storage<B>(
    binary_storage: Arc<B>,
) -> crate::api::PdfProcessFromStorageCallback
where
    B: BinaryStorage + 'static,
{
    use raisin_ai::pdf::storage_processor::{process_pdf_from_storage, StoragePdfOptions};

    Arc::new(move |storage_key: String, options: Value| {
        let storage = binary_storage.clone();

        Box::pin(async move {
            // Parse options from JSON
            let pdf_options: StoragePdfOptions =
                serde_json::from_value(options).unwrap_or_default();

            // Process PDF from storage (using pdf_oxide for markdown extraction)
            let result = process_pdf_from_storage(&*storage, &storage_key, pdf_options)
                .await
                .map_err(|e| {
                    raisin_error::Error::Backend(format!("PDF processing failed: {}", e))
                })?;

            // Convert result to JSON
            serde_json::to_value(result).map_err(|e| {
                raisin_error::Error::Backend(format!("Failed to serialize result: {}", e))
            })
        })
    })
}

// TODO: Re-enable tests when MemoryBinaryStorage is available
// These tests use a mock MemoryBinaryStorage that was never implemented.
// #[cfg(test)]
// mod tests {
//     use super::*;
//     use raisin_binary::MemoryBinaryStorage;
//
//     #[tokio::test]
//     async fn test_resource_get_binary() {
//         let storage = Arc::new(MemoryBinaryStorage::new());
//         let test_data = b"Hello, World!";
//         storage.put("test/file.txt", test_data).await.unwrap();
//         let callback = create_resource_get_binary(storage);
//         let result = callback("test/file.txt".to_string()).await;
//         assert!(result.is_ok());
//         let base64_data = result.unwrap();
//         let decoded = base64::engine::general_purpose::STANDARD
//             .decode(&base64_data)
//             .unwrap();
//         assert_eq!(decoded, test_data);
//     }
//
//     #[tokio::test]
//     async fn test_resource_get_binary_not_found() {
//         let storage = Arc::new(MemoryBinaryStorage::new());
//         let callback = create_resource_get_binary(storage);
//         let result = callback("nonexistent/file.txt".to_string()).await;
//         assert!(result.is_err());
//     }
// }
