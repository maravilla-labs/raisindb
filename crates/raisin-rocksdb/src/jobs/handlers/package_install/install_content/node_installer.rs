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

//! Batch installation of sorted content entries
//!
//! Handles Phase 3 of content installation: iterating the sorted
//! [`ContentEntry`] list and persisting YAML-defined nodes and binary
//! asset files inside transactional batches.

use raisin_error::Result;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::value::PropertyValue;
use raisin_models::nodes::Node;
use raisin_storage::jobs::JobId;
use raisin_storage::transactional::{TransactionalContext, TransactionalStorage};
use raisin_storage::Storage;
use std::collections::HashMap;

use crate::jobs::handlers::package_install::content_types::{
    compute_content_hash, derive_content_path, AssetFileDef, ContentEntry, InstallStats,
    CONTENT_BATCH_SIZE,
};
use crate::jobs::handlers::package_install::handler::PackageInstallHandler;
use crate::jobs::handlers::package_install::translation::derive_node_name_from_base_path;
use crate::jobs::handlers::package_install::types::InstallMode;

use super::resolve_folder_type;

impl<S: Storage + TransactionalStorage> PackageInstallHandler<S> {
    /// Install sorted content entries in batches
    pub(in crate::jobs::handlers::package_install) async fn install_sorted_entries(
        &self,
        entries: Vec<ContentEntry>,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        job_id: &JobId,
        install_mode: InstallMode,
        folder_type_map: &HashMap<String, String>,
        binary_store: Option<&super::super::types::BinaryStorageCallback>,
        stats: &mut InstallStats,
    ) -> Result<()> {
        let total_entries = entries.len();
        let mut processed = 0;

        for batch in entries.chunks(CONTENT_BATCH_SIZE) {
            let tx = self.storage.begin_context().await?;
            tx.set_tenant_repo(tenant_id, repo_id)?;
            tx.set_branch(branch)?;
            tx.set_actor("package-install")?;
            tx.set_message(&format!("Package install batch ({} items)", batch.len()))?;
            tx.set_auth_context(AuthContext::system())?;

            for entry in batch {
                match entry {
                    ContentEntry::NodeDef {
                        workspace, node, ..
                    } => {
                        let folder_type = resolve_folder_type(folder_type_map, workspace);
                        self.install_content_node(
                            tx.as_ref(),
                            workspace,
                            node,
                            job_id,
                            install_mode,
                            folder_type,
                            stats,
                        )
                        .await?;
                    }
                    ContentEntry::BinaryFile {
                        workspace,
                        zip_path,
                        filename,
                        parent_path,
                        data,
                        metadata,
                        content_hash,
                    } => {
                        let folder_type = resolve_folder_type(folder_type_map, workspace);
                        self.install_binary_file(
                            tx.as_ref(),
                            workspace,
                            zip_path,
                            filename,
                            parent_path,
                            data,
                            metadata.as_ref(),
                            content_hash,
                            job_id,
                            install_mode,
                            folder_type,
                            binary_store,
                            stats,
                        )
                        .await?;
                    }
                    ContentEntry::TranslationFile {
                        workspace,
                        base_node_yaml_path,
                        locale,
                        overlay,
                    } => {
                        self.install_translation(
                            tx.as_ref(),
                            workspace,
                            base_node_yaml_path,
                            locale,
                            overlay,
                            job_id,
                            stats,
                        )
                        .await?;
                    }
                }
            }

            // Commit the batch transaction
            tx.commit().await?;
            processed += batch.len();

            tracing::debug!(
                job_id = %job_id,
                processed = processed,
                total = total_entries,
                mode = ?install_mode,
                "Committed content batch"
            );
        }

        Ok(())
    }

    /// Install a single content node within a transaction
    async fn install_content_node(
        &self,
        tx: &dyn TransactionalContext,
        workspace: &str,
        node: &Node,
        job_id: &JobId,
        install_mode: InstallMode,
        folder_type: &str,
        stats: &mut InstallStats,
    ) -> Result<()> {
        let node_path = node.path.clone();

        // Check if node exists at this path
        let existing = tx.get_node_by_path(workspace, &node_path).await?;

        tracing::info!(
            job_id = %job_id,
            workspace = %workspace,
            path = %node_path,
            node_id = %node.id,
            existing_id = existing.as_ref().map(|n| n.id.as_str()).unwrap_or("none"),
            mode = ?install_mode,
            "Package install: processing content node"
        );

        match install_mode {
            InstallMode::Skip => {
                if existing.is_some() {
                    tracing::debug!(
                        job_id = %job_id,
                        workspace = %workspace,
                        path = %node_path,
                        "Content node already exists at path, skipping (skip mode)"
                    );
                    stats.content_nodes_skipped += 1;
                    return Ok(());
                }
                tx.upsert_deep_node(workspace, node, folder_type).await?;
                stats.content_nodes_created += 1;
            }
            InstallMode::Overwrite | InstallMode::Sync => {
                tx.upsert_deep_node(workspace, node, folder_type).await?;
                if existing.is_some() {
                    tracing::debug!(
                        job_id = %job_id,
                        workspace = %workspace,
                        path = %node_path,
                        mode = ?install_mode,
                        "Updated existing content node"
                    );
                    stats.content_nodes_synced += 1;
                } else {
                    tracing::debug!(
                        job_id = %job_id,
                        workspace = %workspace,
                        path = %node_path,
                        mode = ?install_mode,
                        "Created new content node"
                    );
                    stats.content_nodes_created += 1;
                }
            }
        }

        Ok(())
    }

    /// Install a single binary file as a raisin:Asset node within a transaction
    #[allow(clippy::too_many_arguments)]
    async fn install_binary_file(
        &self,
        tx: &dyn TransactionalContext,
        workspace: &str,
        zip_path: &str,
        filename: &str,
        parent_path: &str,
        data: &[u8],
        metadata: &Option<AssetFileDef>,
        content_hash: &str,
        job_id: &JobId,
        install_mode: InstallMode,
        folder_type: &str,
        binary_store: Option<&super::super::types::BinaryStorageCallback>,
        stats: &mut InstallStats,
    ) -> Result<()> {
        // Skip binary files if no storage callback configured
        let Some(binary_store) = binary_store else {
            tracing::warn!(
                job_id = %job_id,
                file = %zip_path,
                "Skipping binary file: no binary storage callback configured"
            );
            return Ok(());
        };

        // Build asset path: /{parent_path}/{filename}
        let asset_path = if parent_path.is_empty() {
            format!("/{}", filename)
        } else {
            format!("/{}/{}", parent_path, filename)
        };

        // Check if asset already exists
        let existing = tx.get_node_by_path(workspace, &asset_path).await?;

        // Hash-based change detection
        let should_update = match (&existing, install_mode) {
            (None, _) => true,                         // New file, always create
            (Some(_), InstallMode::Overwrite) => true, // Always replace
            (Some(ex), InstallMode::Skip) | (Some(ex), InstallMode::Sync) => {
                // Check if content has changed
                let existing_hash = ex.properties.get("content_hash").and_then(|v| match v {
                    PropertyValue::String(s) => Some(s.as_str()),
                    _ => None,
                });
                existing_hash != Some(content_hash)
            }
        };

        if !should_update {
            tracing::debug!(
                job_id = %job_id,
                workspace = %workspace,
                path = %asset_path,
                "Binary file unchanged (hash match), skipping"
            );
            stats.content_nodes_skipped += 1;
            return Ok(());
        }

        // Detect MIME type
        let mime_type = mime_guess::from_path(filename)
            .first()
            .map(|m| m.to_string());

        // Get file extension
        let ext = std::path::Path::new(filename)
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string());

        // Upload binary to storage
        let stored = binary_store(
            data.to_vec(),
            mime_type.clone(),
            ext,
            Some(filename.to_string()),
            None,
        )
        .await?;

        // Build Resource property
        use raisin_models::nodes::properties::value::Resource;
        let mut resource_metadata = HashMap::new();
        resource_metadata.insert(
            "storage_key".to_string(),
            PropertyValue::String(stored.key.clone()),
        );

        let resource = Resource {
            uuid: nanoid::nanoid!(),
            name: stored.name.clone(),
            size: Some(stored.size),
            mime_type: mime_type.clone(),
            url: Some(stored.url.clone()),
            metadata: Some(resource_metadata),
            is_loaded: Some(true),
            is_external: Some(false),
            created_at: stored.created_at.into(),
            updated_at: stored.updated_at.into(),
        };

        // Build node properties
        let meta = metadata.clone().unwrap_or_default();
        let mut properties = meta.properties.unwrap_or_default();
        properties.insert(
            "title".to_string(),
            PropertyValue::String(meta.title.unwrap_or_else(|| filename.to_string())),
        );
        properties.insert("file".to_string(), PropertyValue::Resource(resource));
        properties.insert(
            "file_type".to_string(),
            PropertyValue::String(
                mime_type.unwrap_or_else(|| "application/octet-stream".to_string()),
            ),
        );
        properties.insert("file_size".to_string(), PropertyValue::Integer(stored.size));
        properties.insert(
            "content_hash".to_string(),
            PropertyValue::String(content_hash.to_string()),
        );

        if let Some(desc) = &meta.description {
            properties.insert(
                "description".to_string(),
                PropertyValue::String(desc.clone()),
            );
        }

        // Create or update asset node
        let asset_node = Node {
            id: existing
                .as_ref()
                .map(|n| n.id.clone())
                .unwrap_or_else(|| nanoid::nanoid!()),
            node_type: meta.node_type,
            name: filename.to_string(),
            path: asset_path.clone(),
            workspace: Some(workspace.to_string()),
            properties,
            ..Default::default()
        };

        // Always use upsert to handle potential duplicates
        tx.upsert_deep_node(workspace, &asset_node, folder_type)
            .await?;
        if existing.is_some() {
            tracing::debug!(
                job_id = %job_id,
                workspace = %workspace,
                path = %asset_path,
                mode = ?install_mode,
                "Updated binary asset node"
            );
            stats.content_nodes_synced += 1;
        } else {
            tracing::debug!(
                job_id = %job_id,
                workspace = %workspace,
                path = %asset_path,
                "Created binary asset node"
            );
            stats.content_nodes_created += 1;
        }
        stats.binary_files_installed += 1;

        Ok(())
    }

    /// Install a translation for a content node within a transaction
    async fn install_translation(
        &self,
        tx: &dyn TransactionalContext,
        workspace: &str,
        base_node_yaml_path: &str,
        locale: &str,
        overlay: &raisin_models::translations::LocaleOverlay,
        job_id: &JobId,
        stats: &mut InstallStats,
    ) -> Result<()> {
        let node_name = derive_node_name_from_base_path(base_node_yaml_path);
        let node_path = derive_content_path(base_node_yaml_path, &node_name);

        match tx.get_node_by_path(workspace, &node_path).await? {
            Some(node) => {
                tx.store_translation(workspace, &node.id, locale, overlay.clone())
                    .await?;
                tracing::debug!(
                    job_id = %job_id, workspace = %workspace,
                    node_path = %node_path, node_id = %node.id, locale = %locale,
                    "Applied translation to content node"
                );
                stats.translations_applied += 1;
            }
            None => {
                tracing::warn!(
                    job_id = %job_id, workspace = %workspace,
                    node_path = %node_path, locale = %locale,
                    "Translation target node not found, skipping"
                );
                stats.translations_skipped += 1;
            }
        }
        Ok(())
    }
}
