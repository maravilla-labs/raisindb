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

//! Builtin package initialization event handler.
//!
//! This handler subscribes to `RepositoryCreated` events and automatically
//! installs builtin packages (like raisin-auth) for new repositories.
//!
//! Builtin packages are embedded at compile time and installed as if they
//! were manually uploaded, making them visible in the packages workspace.

use anyhow::Result;
use chrono::Utc;
use include_dir::Dir;
use raisin_binary::BinaryStorage;
use raisin_core::package_init::load_builtin_packages_with_hashes;
use raisin_events::{Event, EventHandler, RepositoryEventKind};
use raisin_hlc::HLC;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::value::PropertyValue;
use raisin_models::nodes::Node;
use raisin_packages::Manifest;
use raisin_rocksdb::{JobDataStore, SystemUpdateRepositoryImpl};
use raisin_storage::jobs::{JobContext, JobRegistry, JobType};
use raisin_storage::system_updates::{AppliedDefinition, ResourceType, SystemUpdateRepository};
use raisin_storage::transactional::TransactionalStorage;
use raisin_storage::{RepositoryManagementRepository, Storage};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{error, info};

/// Event handler that installs builtin packages when a repository is created
pub struct BuiltinPackageInitHandler<S, B>
where
    S: Storage + TransactionalStorage,
    B: BinaryStorage,
{
    storage: Arc<S>,
    binary_storage: Arc<B>,
    job_registry: Arc<JobRegistry>,
    job_data_store: Arc<JobDataStore>,
    system_update_repo: SystemUpdateRepositoryImpl,
}

impl<S, B> BuiltinPackageInitHandler<S, B>
where
    S: Storage + TransactionalStorage,
    B: BinaryStorage,
{
    /// Create a new builtin package initialization handler
    pub fn new(
        storage: Arc<S>,
        binary_storage: Arc<B>,
        job_registry: Arc<JobRegistry>,
        job_data_store: Arc<JobDataStore>,
        system_update_repo: SystemUpdateRepositoryImpl,
    ) -> Self {
        info!("BuiltinPackageInitHandler created and ready to handle events");
        Self {
            storage,
            binary_storage,
            job_registry,
            job_data_store,
            system_update_repo,
        }
    }

    /// Scan all existing repositories and ensure builtin packages are installed.
    ///
    /// This should be called on server startup to handle:
    /// - Repositories created before new builtin packages were added
    /// - Repositories where package installation failed or was corrupted
    /// - "Zombie" packages where the binary was deleted
    pub async fn scan_existing_repositories(&self) -> Result<()> {
        info!("Scanning existing repositories for missing/broken builtin packages");

        let repos = self
            .storage
            .repository_management()
            .list_repositories()
            .await?;
        let repo_count = repos.len();

        info!(
            repository_count = repo_count,
            "Found repositories to scan for builtin packages"
        );

        for repo_info in repos {
            if let Err(e) = self
                .install_builtin_packages(&repo_info.tenant_id, &repo_info.repo_id)
                .await
            {
                error!(
                    tenant_id = %repo_info.tenant_id,
                    repo_id = %repo_info.repo_id,
                    error = %e,
                    "Failed to scan/install builtin packages for repository"
                );
                // Continue with other repos even if one fails
            }
        }

        info!("Finished scanning repositories for builtin packages");
        Ok(())
    }

    /// Install all builtin packages for a repository
    async fn install_builtin_packages(&self, tenant_id: &str, repo_id: &str) -> Result<()> {
        info!(
            tenant_id = tenant_id,
            repo_id = repo_id,
            "Installing builtin packages"
        );

        // Get all builtin packages with their content hashes
        let builtin_packages = load_builtin_packages_with_hashes();

        for package_info in builtin_packages {
            // Get the embedded directory for this package
            if let Some(package_dir) =
                raisin_core::package_init::get_builtin_package_dir(&package_info.manifest.name)
            {
                match self
                    .install_package(package_dir, tenant_id, repo_id, &package_info.content_hash)
                    .await
                {
                    Ok(()) => {
                        // Record the applied hash for system updates tracking
                        if let Err(e) = self
                            .system_update_repo
                            .set_applied(
                                tenant_id,
                                repo_id,
                                ResourceType::Package,
                                &package_info.manifest.name,
                                AppliedDefinition {
                                    content_hash: package_info.content_hash.clone(),
                                    applied_version: None, // Package version is string, not i32
                                    applied_at: Utc::now(),
                                    applied_by: "system".to_string(),
                                },
                            )
                            .await
                        {
                            error!(
                                tenant_id = tenant_id,
                                repo_id = repo_id,
                                package = %package_info.manifest.name,
                                error = %e,
                                "Failed to record applied package hash"
                            );
                        } else {
                            info!(
                                tenant_id = tenant_id,
                                repo_id = repo_id,
                                package = %package_info.manifest.name,
                                hash = %&package_info.content_hash[..8],
                                "Recorded package hash for system updates tracking"
                            );
                        }
                    }
                    Err(e) => {
                        error!(
                            tenant_id = tenant_id,
                            repo_id = repo_id,
                            package = %package_info.manifest.name,
                            error = %e,
                            "Failed to install builtin package"
                        );
                        // Don't fail the entire operation - log and continue
                    }
                }
            } else {
                error!(
                    tenant_id = tenant_id,
                    repo_id = repo_id,
                    package = %package_info.manifest.name,
                    "Could not find embedded directory for builtin package"
                );
            }
        }

        Ok(())
    }

    /// Install a single embedded package
    async fn install_package(
        &self,
        package_dir: &Dir<'static>,
        tenant_id: &str,
        repo_id: &str,
        content_hash: &str,
    ) -> Result<()> {
        // Parse manifest (with fallback for include_dir path resolution issues)
        let manifest_file = package_dir
            .get_file("manifest.yaml")
            .or_else(|| {
                // Fallback: search through all files by filename
                package_dir.files().find(|f| {
                    f.path()
                        .file_name()
                        .map(|n| n == "manifest.yaml")
                        .unwrap_or(false)
                })
            })
            .ok_or_else(|| anyhow::anyhow!("Missing manifest.yaml in package"))?;

        let manifest: Manifest = Manifest::from_bytes(manifest_file.contents())?;

        info!(
            package_name = %manifest.name,
            package_version = %manifest.version,
            tenant_id = tenant_id,
            repo_id = repo_id,
            "Installing builtin package"
        );

        // Create transaction with system auth context FIRST - all node operations must go through it
        let tx = self.storage.begin_context().await?;
        tx.set_tenant_repo(tenant_id, repo_id)?;
        tx.set_branch("main")?;
        tx.set_actor("builtin-package-init")?;
        tx.set_auth_context(AuthContext::system())?;

        // Check if package already exists and determine if we need to update
        let node_id = format!("package-{}", manifest.name);
        let existing = tx.get_node("packages", &node_id).await?;

        // Determine if this is an update vs new installation
        let is_update = if let Some(ref existing_node) = existing {
            // Check if package installation is incomplete (installed=false means job failed)
            let is_installed = existing_node
                .properties
                .get("installed")
                .and_then(|v| match v {
                    PropertyValue::Boolean(b) => Some(*b),
                    _ => None,
                })
                .unwrap_or(false);

            if !is_installed {
                // Package node exists but install never completed - broken!
                info!(
                    package_name = %manifest.name,
                    "Package exists but not installed (broken), reinstalling"
                );
                true
            } else {
                // Package is installed, check if content hash has changed
                let applied = self
                    .system_update_repo
                    .get_applied(tenant_id, repo_id, ResourceType::Package, &manifest.name)
                    .await?;

                match applied {
                    Some(applied) if applied.content_hash == content_hash => {
                        // Hash matches, but verify binary actually exists (zombie detection)
                        let resource_key = existing_node
                            .properties
                            .get("resource")
                            .and_then(|v| match v {
                                PropertyValue::Object(obj) => obj.get("key"),
                                _ => None,
                            })
                            .and_then(|v| match v {
                                PropertyValue::String(s) => Some(s.clone()),
                                _ => None,
                            });

                        if let Some(key) = resource_key {
                            // Try to fetch the binary - if it fails, the file was deleted
                            if self.binary_storage.get(&key).await.is_err() {
                                info!(
                                    package_name = %manifest.name,
                                    resource_key = %key,
                                    "Package binary missing from storage (zombie), reinstalling"
                                );
                                true // force reinstall
                            } else {
                                // Binary exists, truly skip
                                info!(package_name = %manifest.name, "Package unchanged and binary exists, skipping");
                                return Ok(());
                            }
                        } else {
                            // No resource key - corrupted node, reinstall
                            info!(
                                package_name = %manifest.name,
                                "Package has no resource key (corrupted), reinstalling"
                            );
                            true
                        }
                    }
                    Some(_) => {
                        info!(
                            package_name = %manifest.name,
                            "Package content updated, reinstalling"
                        );
                        true
                    }
                    None => {
                        // No hash recorded but installed=true - legacy state, reinstall to be safe
                        info!(
                            package_name = %manifest.name,
                            "Package has no hash record, reinstalling"
                        );
                        true
                    }
                }
            }
        } else {
            false // new installation
        };

        // Create ZIP from embedded files using shared function
        let zip_data = raisin_core::package_init::create_package_zip(package_dir)?;

        // Store the binary
        let filename = format!("{}-{}.rap", manifest.name, manifest.version);
        let stored = self
            .binary_storage
            .put_bytes(
                &zip_data,
                Some("application/zip"),
                Some("rap"),
                Some(&filename),
                Some(tenant_id),
            )
            .await?;

        // Create the raisin:Package node
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
        // icon and color have defaults in Manifest
        properties.insert(
            "icon".to_string(),
            PropertyValue::String(manifest.icon.clone()),
        );
        properties.insert(
            "color".to_string(),
            PropertyValue::String(manifest.color.clone()),
        );
        // keywords is Vec<String> (not Option)
        if !manifest.keywords.is_empty() {
            properties.insert(
                "keywords".to_string(),
                PropertyValue::Array(
                    manifest
                        .keywords
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

        // Mark as builtin
        properties.insert("builtin".to_string(), PropertyValue::Boolean(true));

        // Set installed to false initially (will be set to true after job completes)
        properties.insert("installed".to_string(), PropertyValue::Boolean(false));

        // Add resource reference
        let mut resource_obj = HashMap::new();
        resource_obj.insert("key".to_string(), PropertyValue::String(stored.key.clone()));
        resource_obj.insert("url".to_string(), PropertyValue::String(stored.url.clone()));
        resource_obj.insert(
            "mime_type".to_string(),
            PropertyValue::String("application/zip".to_string()),
        );
        resource_obj.insert(
            "size".to_string(),
            PropertyValue::Integer(zip_data.len() as i64),
        );
        properties.insert("resource".to_string(), PropertyValue::Object(resource_obj));

        let node = Node {
            id: node_id.clone(),
            node_type: "raisin:Package".to_string(),
            name: manifest.name.clone(),
            path: format!("/{}", manifest.name),
            workspace: Some("packages".to_string()),
            properties,
            ..Default::default()
        };

        // Create or update the package node (transaction already created above)
        if is_update {
            tx.set_message(&format!("Update builtin package node: {}", manifest.name))?;
            tx.upsert_node("packages", &node).await?;
            tx.commit().await?;
            info!(
                package_name = %manifest.name,
                node_id = %node_id,
                "Updated package node, triggering reinstallation job"
            );
        } else {
            tx.set_message(&format!("Create builtin package node: {}", manifest.name))?;
            tx.add_node("packages", &node).await?;
            tx.commit().await?;
            info!(
                package_name = %manifest.name,
                node_id = %node_id,
                "Created package node, triggering installation job"
            );
        }

        // Create installation job type
        let job_type = JobType::PackageInstall {
            package_name: manifest.name.clone(),
            package_version: manifest.version.clone(),
            package_node_id: node_id.clone(),
        };

        // Create job context with metadata
        // Use "sync" mode for updates to update existing content, "skip" for new installations
        let install_mode = if is_update { "sync" } else { "skip" };
        let mut metadata = HashMap::new();
        metadata.insert(
            "resource_key".to_string(),
            serde_json::Value::String(stored.key.clone()),
        );
        metadata.insert(
            "install_mode".to_string(),
            serde_json::Value::String(install_mode.to_string()),
        );

        let job_context = JobContext {
            tenant_id: tenant_id.to_string(),
            repo_id: repo_id.to_string(),
            branch: "main".to_string(),
            workspace_id: "packages".to_string(),
            revision: HLC::now(),
            metadata,
        };

        // Register the job
        let job_id = self
            .job_registry
            .register_job(job_type, Some(tenant_id.to_string()), None, None, None)
            .await?;

        // Store job context
        self.job_data_store.put(&job_id, &job_context)?;

        info!(
            package_name = %manifest.name,
            job_id = %job_id,
            "Package installation job created"
        );

        Ok(())
    }
}

impl<S, B> EventHandler for BuiltinPackageInitHandler<S, B>
where
    S: Storage + TransactionalStorage + 'static,
    B: BinaryStorage + 'static,
{
    fn handle<'a>(
        &'a self,
        event: &'a Event,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            // Only handle RepositoryCreated events
            if let Event::Repository(repo_event) = event {
                if matches!(repo_event.kind, RepositoryEventKind::Created) {
                    info!(
                        tenant_id = %repo_event.tenant_id,
                        repo_id = %repo_event.repository_id,
                        "Processing RepositoryCreated event for builtin packages"
                    );

                    self.install_builtin_packages(&repo_event.tenant_id, &repo_event.repository_id)
                        .await?;
                }
            }

            Ok(())
        })
    }

    fn name(&self) -> &str {
        "BuiltinPackageInitHandler"
    }
}
