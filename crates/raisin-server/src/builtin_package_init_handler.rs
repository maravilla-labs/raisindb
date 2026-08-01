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
//! Builtin packages come from the resolved definition stack
//! (`raisin_core::definitions`): embedded in the binary by default, or from the
//! filesystem overlay when one shadows an embedded package by name. Either way
//! they are installed as if manually uploaded, making them visible in the
//! packages workspace.

use anyhow::Result;
use chrono::Utc;
use raisin_binary::BinaryStorage;
use raisin_core::definitions::{DefinitionResolver, PackageSource};
use raisin_events::{Event, EventHandler, RepositoryEventKind};
use raisin_hlc::HLC;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::value::PropertyValue;
use raisin_models::nodes::Node;
use raisin_packages::Manifest;
use raisin_rocksdb::{JobDataStore, SystemUpdateRepositoryImpl};
use raisin_storage::jobs::{JobContext, JobId, JobRegistry, JobType};
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
    /// Resolved definition stack the builtin packages are read from.
    definitions: Arc<DefinitionResolver>,
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
        definitions: Arc<DefinitionResolver>,
    ) -> Self {
        info!("BuiltinPackageInitHandler created and ready to handle events");
        Self {
            storage,
            binary_storage,
            job_registry,
            job_data_store,
            system_update_repo,
            definitions,
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

        // Resolved builtin packages with their content hashes — an overlay copy
        // of a package shadows the embedded one by name.
        let builtin_packages = self.definitions.packages();

        for package in builtin_packages {
            let package_info = package.info;
            // Opt-out packages (e.g. the heavy provider connector adapters) are
            // still REGISTERED on boot — so they appear in the Integrations
            // gallery and can be installed on demand — but their content is NOT
            // written into every repo automatically. Auto-installing their
            // content into repos that can't yet accept their node types produced
            // endlessly retrying, concurrent PackageInstall jobs that starved the
            // worker pool. Only `auto_install: true` packages (the lightweight
            // base infra, e.g. raisin-integrations) get their content installed
            // automatically; registration is a single inline write, serialized by
            // this loop, so it cannot stampede.
            let install_content = package_info.manifest.auto_install;

            {
                match self
                    .install_package(
                        &package.source,
                        tenant_id,
                        repo_id,
                        &package_info.content_hash,
                        install_content,
                    )
                    .await
                {
                    // Only record the applied hash when content was actually
                    // installed. Register-only packages are not "applied" yet —
                    // recording them would hide a later on-demand install from
                    // system-updates tracking.
                    Ok(()) if install_content => {
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
                    // Register-only success (auto_install: false): the package is
                    // now available for on-demand install; nothing else to do.
                    Ok(()) => {}
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
            }
        }

        Ok(())
    }

    /// Register (and optionally install) a single builtin package, from
    /// whichever definition layer supplied it.
    ///
    /// Both modes create/update the `raisin:Package` node and store the package
    /// `.rap` bytes so the package is listable and installable on demand. When
    /// `install_content` is `true` an install job is also enqueued to write the
    /// package's content into the repo; when `false` the package is left
    /// registered but uninstalled (`installed: false`) — the operator installs it
    /// later from the Integrations gallery, which resolves the very node this
    /// registration creates.
    async fn install_package(
        &self,
        package_source: &PackageSource,
        tenant_id: &str,
        repo_id: &str,
        content_hash: &str,
        install_content: bool,
    ) -> Result<()> {
        let manifest: Manifest = Manifest::from_bytes(&package_source.manifest_bytes()?)?;

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

        // Register-only idempotency: for opt-out packages we only need the
        // package node + its `.rap` bytes to exist so on-demand install works.
        // If the node is already registered with a live binary, skip — otherwise
        // every boot would needlessly re-zip and re-commit for each opt-out
        // package in every repo (a self-inflicted write storm).
        if !install_content {
            if let Some(ref existing_node) = existing {
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
                // Compare CONTENT, not just presence. This used to skip whenever
                // the node existed and its binary was fetchable, with no regard
                // for whether the embedded package had changed — so a rebuilt
                // opt-out package (every provider connector) never re-registered
                // and its stale `.rap` stayed in the repo forever. A connector
                // fix could ship to a server and still not run, which is exactly
                // what happened to the Microsoft Graph adapter.
                let registered_hash =
                    existing_node
                        .properties
                        .get("content_hash")
                        .and_then(|v| match v {
                            PropertyValue::String(s) => Some(s.as_str()),
                            _ => None,
                        });
                let content_unchanged = registered_hash == Some(content_hash);
                if let Some(key) = resource_key {
                    if content_unchanged && self.binary_storage.get(&key).await.is_ok() {
                        info!(
                            package_name = %manifest.name,
                            "Builtin package already registered and unchanged, skipping"
                        );
                        return Ok(());
                    }
                    if !content_unchanged {
                        info!(
                            package_name = %manifest.name,
                            registered = registered_hash.unwrap_or("none"),
                            current = %&content_hash[..8.min(content_hash.len())],
                            "Builtin package content changed, re-registering"
                        );
                    }
                }
            }
        }

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
        let zip_data = package_source.to_zip()?;

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
        // What the skip check above compares against on the next boot.
        properties.insert(
            "content_hash".to_string(),
            PropertyValue::String(content_hash.to_string()),
        );

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
            // upsert, not add. The `existing` probe above and `add_node`'s own
            // uniqueness check do not always agree — observed in production as
            // `get_node` returning None for `package-{name}` while `add_node`
            // rejected the very same id with "Node with id 'package-X' already
            // exists", on every boot, for several packages and tenants. Whatever
            // makes the two disagree (a tombstoned id still held in the index is
            // the leading suspect), the retry could never succeed, so builtin
            // package content stopped reaching those repos entirely — which is
            // how an adapter fix ships to a server and still does not run.
            //
            // The write is idempotent either way: this node is system-owned and
            // rebuilt from the embedded manifest on every pass, so replacing an
            // unexpected occupant is the intended outcome rather than a loss.
            tx.upsert_node("packages", &node).await?;
            tx.commit().await?;
            info!(
                package_name = %manifest.name,
                node_id = %node_id,
                "Created package node, triggering installation job"
            );
        }

        // Register-only: the package node + `.rap` bytes are now persisted, so it
        // is listable and installable on demand, but we do NOT enqueue a content
        // install job (that is the opt-out that keeps boot from stampeding).
        if !install_content {
            info!(
                package_name = %manifest.name,
                node_id = %node_id,
                "Registered builtin package for on-demand install (auto-install opt-out)"
            );
            return Ok(());
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

        // Store job context BEFORE registering: registration makes the job
        // visible to dispatch immediately, and a worker that claims it
        // without a stored context fails the job ("Missing job context").
        let job_id = JobId::new();
        self.job_data_store.put(&job_id, &job_context)?;

        // Register the job under the pre-generated ID
        self.job_registry
            .register_job_with_id(
                job_id.clone(),
                job_type,
                tenant_id.to_string(),
                None,
                None,
                None,
            )
            .await?;

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

#[cfg(all(test, feature = "storage-rocksdb"))]
mod tests {
    use super::*;
    use raisin_binary::FilesystemBinaryStorage;
    use raisin_rocksdb::{PackageInstallHandler, RocksDBStorage};
    use raisin_storage::jobs::JobInfo;

    const TENANT: &str = "default";
    const REPO: &str = "test-repo";
    const AI_TOOLS_INDEX_PATH: &str = "/lib/raisin/ai/agent-handler/index.js";

    struct TestEnv {
        _db_dir: tempfile::TempDir,
        _bin_dir: tempfile::TempDir,
        storage: Arc<RocksDBStorage>,
        bin: Arc<FilesystemBinaryStorage>,
        handler: BuiltinPackageInitHandler<RocksDBStorage, FilesystemBinaryStorage>,
        system_update_repo: SystemUpdateRepositoryImpl,
    }

    async fn setup() -> TestEnv {
        let _ = tracing_subscriber::fmt()
            .with_env_filter("info")
            .with_test_writer()
            .try_init();
        let db_dir = tempfile::tempdir().unwrap();
        let bin_dir = tempfile::tempdir().unwrap();

        let storage = Arc::new(RocksDBStorage::new(db_dir.path()).unwrap());
        let bin = Arc::new(FilesystemBinaryStorage::new(bin_dir.path(), None));
        let system_update_repo = SystemUpdateRepositoryImpl::new(storage.db().clone());

        let handler = BuiltinPackageInitHandler::new(
            storage.clone(),
            bin.clone(),
            storage.job_registry().clone(),
            storage.job_data_store().clone(),
            system_update_repo.clone(),
            // Embedded-only stack: these tests assert on the packages compiled
            // into the binary, independent of any overlay on the test machine.
            Arc::new(raisin_core::definitions::DefinitionResolver::embedded_only()),
        );

        storage
            .repository_management()
            .create_repository(TENANT, REPO, raisin_context::RepositoryConfig::default())
            .await
            .unwrap();

        // Storage-level create_repository does not create the default
        // branch or workspaces (the core service does that in production).
        use raisin_storage::BranchRepository;
        storage
            .branches()
            .create_branch(TENANT, REPO, "main", "test", None, None, false, false)
            .await
            .unwrap();

        // Register the built-in system NodeTypes (raisin:Package etc.),
        // normally done by NodeTypeInitHandler on RepositoryCreated.
        raisin_core::nodetype_init::init_repository_nodetypes(
            storage.clone(),
            TENANT,
            REPO,
            "main",
        )
        .await
        .unwrap();

        // Register the global system workspaces (packages, functions, ...),
        // normally done during repository initialization.
        use raisin_storage::{RepoScope, WorkspaceRepository};
        for ws in raisin_core::workspace_init::load_global_workspaces() {
            storage
                .workspaces()
                .put(RepoScope::new(TENANT, REPO), ws)
                .await
                .unwrap();
        }

        TestEnv {
            _db_dir: db_dir,
            _bin_dir: bin_dir,
            storage,
            bin,
            handler,
            system_update_repo,
        }
    }

    /// Find Scheduled PackageInstall jobs for the given package name.
    async fn find_install_jobs(env: &TestEnv, package: &str) -> Vec<JobInfo> {
        env.storage
            .job_registry()
            .list_jobs()
            .await
            .into_iter()
            .filter(|j| {
                matches!(
                    &j.job_type,
                    JobType::PackageInstall { package_name, .. } if package_name == package
                ) && matches!(j.status, raisin_storage::jobs::JobStatus::Scheduled)
            })
            .collect()
    }

    /// Execute a PackageInstall job the same way a worker would: load the
    /// context from the JobDataStore (asserting it is already present, which
    /// is the register-before-put regression) and run the install handler.
    async fn run_install_job(env: &TestEnv, job: &JobInfo) -> JobContext {
        let context = env
            .storage
            .job_data_store()
            .get(TENANT, &job.id)
            .unwrap()
            .expect(
                "JobContext must be stored before the job is visible to dispatch \
                 (register-before-put regression)",
            );

        let bin_for_get = env.bin.clone();
        let retrieval: raisin_rocksdb::BinaryRetrievalCallback = Arc::new(move |key: String| {
            let bin = bin_for_get.clone();
            Box::pin(async move {
                bin.get(&key)
                    .await
                    .map(|b| b.to_vec())
                    .map_err(|e| raisin_error::Error::storage(e.to_string()))
            })
        });
        let store_cb = crate::startup::jobs::create_binary_storage_callback(env.bin.clone());

        let install_handler =
            PackageInstallHandler::new(env.storage.clone(), env.storage.job_registry().clone())
                .with_binary_callback(retrieval)
                .with_binary_store_callback(store_cb);

        install_handler
            .handle(job, &context)
            .await
            .expect("package install job must succeed");

        // Mirror the worker lifecycle so the job leaves the Scheduled state.
        env.storage
            .job_registry()
            .mark_running(&job.id)
            .await
            .unwrap();
        env.storage
            .job_registry()
            .mark_completed(&job.id)
            .await
            .unwrap();

        context
    }

    fn install_mode_of(context: &JobContext) -> String {
        context
            .metadata
            .get("install_mode")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string()
    }

    /// Size of the embedded (current) agent-handler/index.js.
    fn embedded_index_js_len() -> i64 {
        let dir = raisin_core::package_init::get_builtin_package_dir("ai-tools")
            .expect("ai-tools builtin package must be embedded");
        let mut stack: Vec<&include_dir::Dir<'static>> = vec![dir];
        while let Some(d) = stack.pop() {
            for f in d.files() {
                if f.path().ends_with("agent-handler/index.js") {
                    return f.contents().len() as i64;
                }
            }
            stack.extend(d.dirs());
        }
        panic!("embedded ai-tools must contain agent-handler/index.js");
    }

    async fn get_index_js_node(env: &TestEnv) -> Option<Node> {
        let tx = env.storage.begin_context().await.unwrap();
        tx.set_tenant_repo(TENANT, REPO).unwrap();
        tx.set_branch("main").unwrap();
        tx.set_auth_context(AuthContext::system()).unwrap();
        tx.get_node_by_path("functions", AI_TOOLS_INDEX_PATH)
            .await
            .unwrap()
    }

    /// End-to-end regression test for the builtin-package auto-update:
    ///
    /// 1. Fresh repo: install_builtin_packages registers an install job whose
    ///    context is readable at dispatch time; running it installs content.
    /// 2. Simulate a binary upgrade that ships changed package content by
    ///    rewriting the recorded content hash to a stale value and tampering
    ///    the installed node. Re-running install_builtin_packages must detect
    ///    the change, enqueue a "sync" install job (again with context
    ///    available), and running that job must bring the content node back
    ///    to the embedded version.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_builtin_package_update_applied_to_existing_repo() {
        let env = setup().await;

        // ---- 1. Initial install into the (existing) repo ----
        env.handler
            .install_builtin_packages(TENANT, REPO)
            .await
            .unwrap();

        let jobs = find_install_jobs(&env, "ai-tools").await;
        assert_eq!(jobs.len(), 1, "fresh install must enqueue one ai-tools job");
        let context = run_install_job(&env, &jobs[0]).await;
        assert_eq!(
            install_mode_of(&context),
            "skip",
            "fresh install uses skip mode"
        );

        let node = get_index_js_node(&env)
            .await
            .expect("agent-handler/index.js must be installed");
        let expected_len = embedded_index_js_len();
        assert_eq!(
            node.properties.get("file_size"),
            Some(&PropertyValue::Integer(expected_len)),
            "installed index.js must match the embedded size"
        );

        // The package node must be marked installed for the hash-compare
        // branch to run on the next scan. The real PackageInstall job sets
        // this on completion; do it explicitly here since we invoke the
        // handler directly.
        {
            let tx = env.storage.begin_context().await.unwrap();
            tx.set_tenant_repo(TENANT, REPO).unwrap();
            tx.set_branch("main").unwrap();
            tx.set_actor("test").unwrap();
            tx.set_auth_context(AuthContext::system()).unwrap();
            tx.set_message("mark installed").unwrap();
            let mut pkg = tx
                .get_node("packages", "package-ai-tools")
                .await
                .unwrap()
                .expect("package node must exist");
            pkg.properties
                .insert("installed".to_string(), PropertyValue::Boolean(true));
            tx.upsert_node("packages", &pkg).await.unwrap();
            tx.commit().await.unwrap();
        }

        // ---- 2. Simulate a content change shipped in a new binary ----
        // The recorded hash no longer matches the embedded package hash.
        env.system_update_repo
            .set_applied(
                TENANT,
                REPO,
                ResourceType::Package,
                "ai-tools",
                AppliedDefinition {
                    content_hash: "stale-hash-from-old-binary".to_string(),
                    applied_version: None,
                    applied_at: Utc::now(),
                    applied_by: "test".to_string(),
                },
            )
            .await
            .unwrap();

        // Tamper the installed content node so we can observe the sync
        // rewriting it back to the embedded content.
        {
            let tx = env.storage.begin_context().await.unwrap();
            tx.set_tenant_repo(TENANT, REPO).unwrap();
            tx.set_branch("main").unwrap();
            tx.set_actor("test").unwrap();
            tx.set_auth_context(AuthContext::system()).unwrap();
            tx.set_message("tamper index.js").unwrap();
            let mut node = tx
                .get_node_by_path("functions", AI_TOOLS_INDEX_PATH)
                .await
                .unwrap()
                .unwrap();
            node.properties
                .insert("file_size".to_string(), PropertyValue::Integer(1));
            node.properties.insert(
                "content_hash".to_string(),
                PropertyValue::String("stale".to_string()),
            );
            tx.upsert_node("functions", &node).await.unwrap();
            tx.commit().await.unwrap();
        }

        // ---- 3. Startup scan must detect the update and reinstall ----
        env.handler
            .install_builtin_packages(TENANT, REPO)
            .await
            .unwrap();

        let jobs = find_install_jobs(&env, "ai-tools").await;
        assert_eq!(
            jobs.len(),
            1,
            "content change must enqueue exactly one new ai-tools install job"
        );
        let context = run_install_job(&env, &jobs[0]).await;
        assert_eq!(
            install_mode_of(&context),
            "sync",
            "update of an existing repo must use sync mode"
        );

        let node = get_index_js_node(&env)
            .await
            .expect("agent-handler/index.js must still exist after update");
        assert_eq!(
            node.properties.get("file_size"),
            Some(&PropertyValue::Integer(expected_len)),
            "sync install must restore index.js to the embedded version"
        );
        match node.properties.get("content_hash") {
            Some(PropertyValue::String(h)) => {
                assert_ne!(h, "stale", "content hash must be rewritten by the sync")
            }
            other => panic!("content_hash property missing or wrong type: {:?}", other),
        }
    }
}
