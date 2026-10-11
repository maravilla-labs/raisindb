//! The legacy `packages:/<name>` registration (`register_package: true`).
//!
//! A build for Studio passes `register_package: false` and writes nothing
//! under `packages:` (E4). The legacy path still registers, but never over an
//! installed or shipped package.

use super::PackageCreateFromSelectionHandler;
use raisin_core::package_registration;
use raisin_error::{Error, Result};
use raisin_packages::Manifest;
use raisin_storage::jobs::JobContext;
use raisin_storage::{CreateNodeOptions, NodeRepository, Storage, StorageScope, UpdateNodeOptions};

impl PackageCreateFromSelectionHandler {
    /// Create or update the package node in the packages workspace
    /// (legacy `register_package: true` only).
    pub(super) async fn create_or_update_package_node(
        &self,
        context: &JobContext,
        package_name: &str,
        package_version: &str,
        blob_key: &str,
        download_url: &str,
        package_size: usize,
    ) -> Result<()> {
        let node_repo = self.storage.nodes();
        let scope = StorageScope::new(
            &context.tenant_id,
            &context.repo_id,
            &context.branch,
            package_registration::PACKAGES_WORKSPACE,
        );
        let package_path = package_registration::package_node_path(package_name);

        // Probe by PATH and reuse whatever id is already there.
        let existing = node_repo
            .get_by_path(scope, &package_path, None)
            .await
            .ok()
            .flatten();
        if let Some(existing) = &existing {
            if super::submit::name_is_reserved(existing) {
                return Err(Error::Validation(format!(
                    "name_invalid: '{package_name}' is the name of an installed package"
                )));
            }
        }

        let manifest = Manifest {
            name: package_name.to_string(),
            version: package_version.to_string(),
            description: Some("Package created from selected content".to_string()),
            ..Default::default()
        };
        let package_node = package_registration::build_package_node(
            &package_registration::PackageRegistration {
                manifest: &manifest,
                resource: package_registration::PackageResource::new(
                    blob_key,
                    download_url,
                    package_size,
                ),
                origin: package_registration::PackageOrigin::Generated,
            },
            existing.as_ref(),
        );

        let create_options = CreateNodeOptions {
            validate_schema: false,
            validate_parent_allows_child: false,
            validate_workspace_allows_type: false,
            operation_meta: None,
        };

        match node_repo
            .create_deep_node(
                scope,
                &package_path,
                package_node.clone(),
                "raisin:Folder",
                create_options.clone(),
            )
            .await
        {
            Ok(_) => {}
            Err(e) if e.to_string().contains("already exists") => {
                node_repo
                    .update(
                        scope,
                        package_node,
                        UpdateNodeOptions {
                            validate_schema: false,
                            allow_type_change: false,
                            operation_meta: None,
                        },
                    )
                    .await
                    .map_err(|e| Error::storage(format!("Failed to update package node: {e}")))?;
            }
            Err(e) => {
                return Err(Error::storage(format!(
                    "Failed to create package node: {e}"
                )));
            }
        }

        Ok(())
    }
}
