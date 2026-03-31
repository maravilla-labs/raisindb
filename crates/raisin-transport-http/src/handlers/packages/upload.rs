// SPDX-License-Identifier: BSL-1.1

//! Package upload handler.
//!
//! Handles uploading `.rap` package files via multipart form,
//! extracting manifests, and creating `raisin:Package` nodes.

use axum::{
    body::Body,
    extract::{Extension, Path, State},
    Json,
};
use multer::Multipart;
use raisin_binary::BinaryStorage;
use raisin_models as models;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::value::PropertyValue;
use std::collections::HashMap;
use std::io::{Cursor, Read};
use zip::ZipArchive;

use crate::{error::ApiError, state::AppState};

use super::types::{PackageManifest, UploadResponse};

/// Upload a .rap package file (multipart form)
///
/// Creates a raisin:Package node in the packages workspace with:
/// - Manifest properties extracted from manifest.yaml
/// - ZIP binary stored in the `resource` property
///
/// POST /api/repos/{repo}/packages/upload
///
/// # Deprecated
/// This endpoint is deprecated. Use the unified repository upload endpoint instead:
/// ```
/// POST /api/repository/{repo}/main/head/packages/{package-name}?node_type=raisin:Package
/// ```
#[deprecated(
    since = "0.1.0",
    note = "Use POST /api/repository/{repo}/main/head/packages/{name}?node_type=raisin:Package instead"
)]
pub async fn upload_package(
    State(state): State<AppState>,
    Path(repo): Path<String>,
    auth: Option<Extension<AuthContext>>,
    axum_extra::TypedHeader(content_type): axum_extra::TypedHeader<
        axum_extra::headers::ContentType,
    >,
    body: axum::body::Bytes,
) -> Result<Json<UploadResponse>, ApiError> {
    let auth_context = auth.map(|Extension(ctx)| ctx);
    let tenant_id = "default";
    let branch = "main";
    let workspace = "packages";

    let content_type_str = content_type.to_string();
    let boundary = multer::parse_boundary(&content_type_str)
        .map_err(|_| ApiError::validation_failed("Invalid multipart boundary"))?;

    let body_stream = Body::from(body);
    let mut multipart = Multipart::new(body_stream.into_data_stream(), boundary);

    let field = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::validation_failed(format!("Invalid multipart field: {}", e)))?
        .ok_or_else(|| ApiError::validation_failed("No file field found in multipart request"))?;

    let file_name = field
        .file_name()
        .ok_or_else(|| ApiError::validation_failed("Missing filename"))?
        .to_string();

    if !file_name.ends_with(".rap") {
        return Err(ApiError::validation_failed("File must have .rap extension"));
    }

    let file_data = field
        .bytes()
        .await
        .map_err(|e| ApiError::validation_failed(format!("Failed to read file data: {}", e)))?;

    let manifest = extract_manifest(&file_data)?;

    let ext = Some("rap");
    let stored = state
        .bin
        .put_bytes(
            &file_data,
            Some("application/zip"),
            ext,
            Some(&file_name),
            Some(tenant_id),
        )
        .await
        .map_err(|e| ApiError::storage_error(format!("Failed to store package file: {}", e)))?;

    let node_service =
        state.node_service_for_context(tenant_id, &repo, branch, workspace, auth_context);

    let properties = build_manifest_properties(&manifest, &stored, file_data.len());

    let node_id = format!("package-{}", manifest.name);
    let node = models::nodes::Node {
        id: node_id.clone(),
        node_type: "raisin:Package".to_string(),
        name: manifest.name.clone(),
        path: format!("/{}", manifest.name),
        workspace: Some(workspace.to_string()),
        properties,
        ..Default::default()
    };

    node_service
        .upsert(node)
        .await
        .map_err(|e| ApiError::storage_error(format!("Failed to create package node: {}", e)))?;

    Ok(Json(UploadResponse {
        package_name: manifest.name,
        version: manifest.version,
        node_id,
    }))
}

/// Extract manifest.yaml from a .rap ZIP file.
pub fn extract_manifest(zip_data: &[u8]) -> Result<PackageManifest, ApiError> {
    let cursor = Cursor::new(zip_data);
    let mut archive = ZipArchive::new(cursor)
        .map_err(|e| ApiError::validation_failed(format!("Invalid ZIP file: {}", e)))?;

    let mut manifest_file = archive
        .by_name("manifest.yaml")
        .map_err(|_| ApiError::validation_failed("Package must contain manifest.yaml"))?;

    let mut manifest_content = String::new();
    manifest_file
        .read_to_string(&mut manifest_content)
        .map_err(|e| ApiError::validation_failed(format!("Failed to read manifest.yaml: {}", e)))?;

    serde_yaml::from_str(&manifest_content)
        .map_err(|e| ApiError::validation_failed(format!("Invalid manifest.yaml format: {}", e)))
}

/// Build the properties map from a parsed manifest and stored binary.
fn build_manifest_properties(
    manifest: &PackageManifest,
    stored: &raisin_binary::StoredObject,
    file_size: usize,
) -> HashMap<String, PropertyValue> {
    let mut properties = HashMap::new();
    properties.insert(
        "name".to_string(),
        PropertyValue::String(manifest.name.clone()),
    );
    properties.insert(
        "version".to_string(),
        PropertyValue::String(manifest.version.clone()),
    );

    if let Some(title) = &manifest.title {
        properties.insert("title".to_string(), PropertyValue::String(title.clone()));
    }
    if let Some(description) = &manifest.description {
        properties.insert(
            "description".to_string(),
            PropertyValue::String(description.clone()),
        );
    }
    if let Some(author) = &manifest.author {
        properties.insert("author".to_string(), PropertyValue::String(author.clone()));
    }
    if let Some(license) = &manifest.license {
        properties.insert(
            "license".to_string(),
            PropertyValue::String(license.clone()),
        );
    }
    if let Some(icon) = &manifest.icon {
        properties.insert("icon".to_string(), PropertyValue::String(icon.clone()));
    }
    if let Some(color) = &manifest.color {
        properties.insert("color".to_string(), PropertyValue::String(color.clone()));
    }
    if let Some(keywords) = &manifest.keywords {
        properties.insert(
            "keywords".to_string(),
            PropertyValue::Array(
                keywords
                    .iter()
                    .map(|k| PropertyValue::String(k.clone()))
                    .collect(),
            ),
        );
    }
    if let Some(category) = &manifest.category {
        properties.insert(
            "category".to_string(),
            PropertyValue::String(category.clone()),
        );
    }
    if let Some(dependencies) = &manifest.dependencies {
        let mut deps_map = HashMap::new();
        for (i, dep) in dependencies.iter().enumerate() {
            let mut dep_obj = HashMap::new();
            dep_obj.insert("name".to_string(), PropertyValue::String(dep.name.clone()));
            dep_obj.insert(
                "version".to_string(),
                PropertyValue::String(dep.version.clone()),
            );
            deps_map.insert(i.to_string(), PropertyValue::Object(dep_obj));
        }
        properties.insert("dependencies".to_string(), PropertyValue::Object(deps_map));
    }
    if let Some(provides) = &manifest.provides {
        let mut provides_obj = HashMap::new();
        if let Some(nodetypes) = &provides.nodetypes {
            provides_obj.insert(
                "nodetypes".to_string(),
                PropertyValue::Array(
                    nodetypes
                        .iter()
                        .map(|nt| PropertyValue::String(nt.clone()))
                        .collect(),
                ),
            );
        }
        if let Some(workspaces) = &provides.workspaces {
            provides_obj.insert(
                "workspaces".to_string(),
                PropertyValue::Array(
                    workspaces
                        .iter()
                        .map(|ws| PropertyValue::String(ws.clone()))
                        .collect(),
                ),
            );
        }
        if let Some(content) = &provides.content {
            provides_obj.insert(
                "content".to_string(),
                PropertyValue::Array(
                    content
                        .iter()
                        .map(|c| PropertyValue::String(c.clone()))
                        .collect(),
                ),
            );
        }
        properties.insert("provides".to_string(), PropertyValue::Object(provides_obj));
    }
    if let Some(workspace_patches) = &manifest.workspace_patches {
        let mut patches_obj = HashMap::new();
        for (ws_name, patch) in workspace_patches {
            let mut patch_map = HashMap::new();
            if let Some(allowed_node_types) = &patch.allowed_node_types {
                let mut ant_map = HashMap::new();
                if let Some(add) = &allowed_node_types.add {
                    ant_map.insert(
                        "add".to_string(),
                        PropertyValue::Array(
                            add.iter()
                                .map(|nt| PropertyValue::String(nt.clone()))
                                .collect(),
                        ),
                    );
                }
                patch_map.insert(
                    "allowed_node_types".to_string(),
                    PropertyValue::Object(ant_map),
                );
            }
            patches_obj.insert(ws_name.clone(), PropertyValue::Object(patch_map));
        }
        properties.insert(
            "workspace_patches".to_string(),
            PropertyValue::Object(patches_obj),
        );
    }

    properties.insert("installed".to_string(), PropertyValue::Boolean(false));

    let mut resource_obj = HashMap::new();
    resource_obj.insert("key".to_string(), PropertyValue::String(stored.key.clone()));
    resource_obj.insert("url".to_string(), PropertyValue::String(stored.url.clone()));
    resource_obj.insert(
        "mime_type".to_string(),
        PropertyValue::String("application/zip".to_string()),
    );
    resource_obj.insert("size".to_string(), PropertyValue::Integer(file_size as i64));
    properties.insert("resource".to_string(), PropertyValue::Object(resource_obj));

    properties
}
