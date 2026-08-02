// SPDX-License-Identifier: BSL-1.1

//! Package upload handler.
//!
//! Handles uploading `.rap` package files via multipart form,
//! extracting manifests, and registering `raisin:Package` nodes.

use async_trait::async_trait;
use axum::{
    body::Body,
    extract::{Extension, Path, State},
    Json,
};
use multer::Multipart;
use raisin_core::package_registration::{
    self, PackageNodeStore, PackageOrigin, PackageRegistration, PackageResource, PACKAGES_WORKSPACE,
};
use raisin_core::services::node_service::NodeService;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::Node;
use raisin_packages::Manifest;
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::Storage;
use std::io::{Cursor, Read};
use zip::ZipArchive;

use crate::{error::ApiError, middleware::TenantInfo, state::AppState};

use super::types::UploadResponse;

/// [`PackageNodeStore`] over a `NodeService`.
///
/// The upload path already has a workspace-scoped service with the caller's
/// auth context; only the write mechanism lives here, every decision above it
/// is [`raisin_core::package_registration`]'s.
struct NodeServicePackageStore<S: Storage + TransactionalStorage> {
    node_service: NodeService<S>,
}

#[async_trait]
impl<S> PackageNodeStore for NodeServicePackageStore<S>
where
    S: Storage + TransactionalStorage + Send + Sync + 'static,
{
    async fn get_package_node_by_path(&self, path: &str) -> anyhow::Result<Option<Node>> {
        match self.node_service.get_by_path(path).await {
            Ok(found) => Ok(found),
            // A missing package is not an error here — it is a first upload.
            Err(_) => Ok(None),
        }
    }

    async fn write_package_node(&self, node: Node, _existing: Option<&Node>) -> anyhow::Result<()> {
        self.node_service
            .upsert(node)
            .await
            .map(|_| ())
            .map_err(|e| anyhow::anyhow!("{}", e))
    }
}

#[deprecated(
    since = "0.1.0",
    note = "Use POST /api/repository/{repo}/main/head/packages/{name}?node_type=raisin:Package instead"
)]
pub async fn upload_package(
    State(state): State<AppState>,
    Path(repo): Path<String>,
    Extension(tenant_info): Extension<TenantInfo>,
    auth: Option<Extension<AuthContext>>,
    axum_extra::TypedHeader(content_type): axum_extra::TypedHeader<
        axum_extra::headers::ContentType,
    >,
    body: axum::body::Bytes,
) -> Result<Json<UploadResponse>, ApiError> {
    let auth_context = auth.map(|Extension(ctx)| ctx);
    let tenant_id = tenant_info.tenant_id.as_str();
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

    // The `.rap` blob is always tenant-scoped: the tenant prefix is part of
    // the generated key, so an unscoped store leaves it outside the tenant's
    // namespace.
    let stored = package_registration::store_package_blob(
        state.bin.as_ref(),
        tenant_id,
        &manifest,
        &file_data,
    )
    .await
    .map_err(|e| ApiError::storage_error(format!("Failed to store package file: {}", e)))?;

    let store = NodeServicePackageStore {
        node_service: state.node_service_for_context(
            tenant_id,
            &repo,
            branch,
            PACKAGES_WORKSPACE,
            auth_context,
        ),
    };

    let registration = PackageRegistration {
        manifest: &manifest,
        resource: PackageResource::from_stored(&stored, file_data.len()),
        origin: PackageOrigin::Uploaded,
    };

    let registered = package_registration::register_package_node(&store, &registration)
        .await
        .map_err(|e| ApiError::storage_error(format!("Failed to create package node: {}", e)))?;

    Ok(Json(UploadResponse {
        package_name: manifest.name,
        version: manifest.version,
        node_id: registered.node_id,
    }))
}

/// Extract `manifest.yaml` from a `.rap` ZIP file.
///
/// Parses into `raisin_packages::Manifest`, the canonical manifest type the
/// package installer and the definition stack already use — a second,
/// narrower manifest struct here is how the uploaded and builtin package nodes
/// drifted apart in the first place.
pub fn extract_manifest(zip_data: &[u8]) -> Result<Manifest, ApiError> {
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

    Manifest::from_yaml(&manifest_content)
        .map_err(|e| ApiError::validation_failed(format!("Invalid manifest.yaml format: {}", e)))
}
