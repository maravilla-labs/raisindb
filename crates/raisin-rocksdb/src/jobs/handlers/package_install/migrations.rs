// SPDX-License-Identifier: BSL-1.1

//! Declarative package migrations.

use raisin_error::{Error, Result};
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::value::PropertyValue;
use raisin_storage::jobs::JobId;
use raisin_storage::scope::StorageScope;
use raisin_storage::transactional::{TransactionalContext, TransactionalStorage};
use raisin_storage::{ListOptions, NodeRepository, Storage};
use std::collections::HashMap;
use std::io::{Cursor, Read};
use zip::ZipArchive;

use super::content_types::{compute_content_hash, InstallStats};
use super::handler::PackageInstallHandler;
use super::manifest::{
    CollisionMode, DeleteNodeMigration, MigrationOperation, MoveNodeMigration, PackageMigration,
    PatchNodesMigration, ReplaceNodeTypeMigration,
};
use super::types::InstallMode;

const MIGRATIONS_DIR: &str = "migrations/";

#[derive(Debug)]
struct MigrationFile {
    zip_path: String,
    hash: String,
    migration: PackageMigration,
}

impl<S: Storage + TransactionalStorage> PackageInstallHandler<S> {
    pub(super) async fn apply_package_migrations(
        &self,
        archive: &mut ZipArchive<Cursor<&Vec<u8>>>,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        package_node_id: &str,
        install_mode: InstallMode,
        job_id: &JobId,
        stats: &mut InstallStats,
    ) -> Result<()> {
        let migrations = collect_migration_files(archive)?;
        if migrations.is_empty() {
            return Ok(());
        }

        let package_node = self
            .storage
            .nodes()
            .get(
                StorageScope::new(tenant_id, repo_id, branch, "packages"),
                package_node_id,
                None,
            )
            .await?
            .ok_or_else(|| {
                Error::NotFound(format!("package node '{package_node_id}' not found"))
            })?;

        let mut applied = read_applied_migrations(&package_node.properties);

        for file in migrations {
            match applied.get(&file.migration.id) {
                Some(existing) if existing == &file.hash => {
                    let tx = self.storage.begin_context().await?;
                    tx.set_tenant_repo(tenant_id, repo_id)?;
                    tx.set_branch(branch)?;
                    tx.set_actor("package-migration")?;
                    tx.set_message(&format!(
                        "Package migration already applied: {}",
                        file.migration.id
                    ))?;
                    tx.set_auth_context(AuthContext::system())?;

                    let mut package_node = tx
                        .get_node("packages", package_node_id)
                        .await?
                        .ok_or_else(|| {
                            Error::NotFound(format!("package node '{package_node_id}' not found"))
                        })?;
                    write_migration_summary(&mut package_node.properties, &file, "applied");
                    tx.put_node("packages", &package_node).await?;
                    tx.commit().await?;

                    stats.migrations_skipped += 1;
                    continue;
                }
                Some(existing) => {
                    return Err(Error::Validation(format!(
                        "migration '{}' changed after it was applied (stored hash {}, package hash {})",
                        file.migration.id, existing, file.hash
                    )));
                }
                None => {}
            }

            let tx = self.storage.begin_context().await?;
            tx.set_tenant_repo(tenant_id, repo_id)?;
            tx.set_branch(branch)?;
            tx.set_actor("package-migration")?;
            tx.set_message(&format!("Package migration: {}", file.migration.id))?;
            tx.set_auth_context(AuthContext::system())?;

            for operation in &file.migration.operations {
                self.apply_migration_operation(
                    tx.as_ref(),
                    tenant_id,
                    repo_id,
                    branch,
                    operation,
                    install_mode,
                )
                .await?;
            }

            let mut package_node =
                tx.get_node("packages", package_node_id)
                    .await?
                    .ok_or_else(|| {
                        Error::NotFound(format!("package node '{package_node_id}' not found"))
                    })?;
            applied.insert(file.migration.id.clone(), file.hash.clone());
            write_applied_migrations(&mut package_node.properties, &applied);
            write_migration_summary(&mut package_node.properties, &file, "applied");
            tx.put_node("packages", &package_node).await?;
            tx.commit().await?;

            tracing::info!(
                job_id = %job_id,
                migration = %file.migration.id,
                path = %file.zip_path,
                "Applied package migration"
            );
            stats.migrations_applied += 1;
        }

        Ok(())
    }

    async fn apply_migration_operation(
        &self,
        tx: &dyn TransactionalContext,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        operation: &MigrationOperation,
        install_mode: InstallMode,
    ) -> Result<()> {
        match operation {
            MigrationOperation::ReplaceNodeType(op) => {
                self.replace_node_type(tx, tenant_id, repo_id, branch, op)
                    .await
            }
            MigrationOperation::PatchNodes(op) => {
                self.patch_nodes(tx, tenant_id, repo_id, branch, op).await
            }
            MigrationOperation::MoveNode(op) => self.move_node(tx, op).await,
            MigrationOperation::DeleteNode(op) => self.delete_node(tx, op, install_mode).await,
        }
    }

    async fn replace_node_type(
        &self,
        tx: &dyn TransactionalContext,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        op: &ReplaceNodeTypeMigration,
    ) -> Result<()> {
        let nodes = self
            .storage
            .nodes()
            .list_all(
                StorageScope::new(tenant_id, repo_id, branch, &op.workspace),
                ListOptions::default(),
            )
            .await?;

        for mut node in nodes {
            if node.node_type != op.from {
                continue;
            }
            node.node_type = op.to.clone();
            if op
                .archetype_from
                .as_ref()
                .is_some_and(|from| node.archetype.as_ref() == Some(from))
            {
                node.archetype = op.archetype_to.clone();
            }
            tx.put_node(&op.workspace, &node).await?;
        }

        Ok(())
    }

    async fn patch_nodes(
        &self,
        tx: &dyn TransactionalContext,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        op: &PatchNodesMigration,
    ) -> Result<()> {
        let mut nodes = Vec::new();
        if let Some(path) = &op.path {
            if let Some(node) = tx.get_node_by_path(&op.workspace, path).await? {
                nodes.push(node);
            }
        } else {
            nodes = self
                .storage
                .nodes()
                .list_all(
                    StorageScope::new(tenant_id, repo_id, branch, &op.workspace),
                    ListOptions::default(),
                )
                .await?;
        }

        for mut node in nodes {
            if op
                .node_type
                .as_ref()
                .is_some_and(|node_type| &node.node_type != node_type)
            {
                continue;
            }
            merge_properties(&mut node.properties, &op.properties);
            tx.put_node(&op.workspace, &node).await?;
        }

        Ok(())
    }

    async fn move_node(&self, tx: &dyn TransactionalContext, op: &MoveNodeMigration) -> Result<()> {
        let Some(node) = tx.get_node_by_path(&op.workspace, &op.from).await? else {
            return Ok(());
        };
        if tx.get_node_by_path(&op.workspace, &op.to).await?.is_some() {
            return match op.on_collision {
                CollisionMode::Skip => Ok(()),
                CollisionMode::Fail => Err(Error::AlreadyExists(format!(
                    "{}:{} already exists",
                    op.workspace, op.to
                ))),
            };
        }
        tx.move_node_tree(&op.workspace, &node.id, &op.to).await
    }

    async fn delete_node(
        &self,
        tx: &dyn TransactionalContext,
        op: &DeleteNodeMigration,
        install_mode: InstallMode,
    ) -> Result<()> {
        let Some(node) = tx.get_node_by_path(&op.workspace, &op.path).await? else {
            return Ok(());
        };
        let children = tx.list_children(&op.workspace, &op.path).await?;
        if !children.is_empty() && op.if_empty {
            return Err(Error::Conflict(format!(
                "{}:{} is not empty",
                op.workspace, op.path
            )));
        }
        if !children.is_empty() && install_mode != InstallMode::Overwrite {
            return Err(Error::Conflict(format!(
                "{}:{} recursive delete requires overwrite mode",
                op.workspace, op.path
            )));
        }
        for child in children {
            delete_tree(tx, &op.workspace, &child).await?;
        }
        tx.delete_node(&op.workspace, &node.id).await
    }
}

fn delete_tree<'a>(
    tx: &'a dyn TransactionalContext,
    workspace: &'a str,
    node: &'a raisin_models::nodes::Node,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
    Box::pin(async move {
        for child in tx.list_children(workspace, &node.path).await? {
            delete_tree(tx, workspace, &child).await?;
        }
        tx.delete_node(workspace, &node.id).await
    })
}

fn collect_migration_files(
    archive: &mut ZipArchive<Cursor<&Vec<u8>>>,
) -> Result<Vec<MigrationFile>> {
    let mut files = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| Error::Validation(format!("Failed to read package migration: {}", e)))?;
        let name = entry.name().to_string();
        if !name.starts_with(MIGRATIONS_DIR) || !name.ends_with(".yaml") || entry.is_dir() {
            continue;
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).map_err(|e| {
            Error::storage(format!(
                "Failed to read package migration '{}': {}",
                name, e
            ))
        })?;
        let migration: PackageMigration = serde_yaml::from_slice(&bytes)
            .map_err(|e| Error::Validation(format!("Invalid migration '{}': {}", name, e)))?;
        if migration.id.trim().is_empty() {
            return Err(Error::Validation(format!(
                "Invalid migration '{}': id must not be empty",
                name
            )));
        }
        files.push(MigrationFile {
            zip_path: name,
            hash: compute_content_hash(&bytes),
            migration,
        });
    }
    files.sort_by(|a, b| a.zip_path.cmp(&b.zip_path));
    Ok(files)
}

fn read_applied_migrations(properties: &HashMap<String, PropertyValue>) -> HashMap<String, String> {
    let Some(PropertyValue::Array(items)) = properties.get("applied_migrations") else {
        return HashMap::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let PropertyValue::Object(obj) = item else {
                return None;
            };
            let id = match obj.get("id") {
                Some(PropertyValue::String(s)) => s.clone(),
                _ => return None,
            };
            let hash = match obj.get("hash") {
                Some(PropertyValue::String(s)) => s.clone(),
                _ => return None,
            };
            Some((id, hash))
        })
        .collect()
}

fn write_applied_migrations(
    properties: &mut HashMap<String, PropertyValue>,
    applied: &HashMap<String, String>,
) {
    let mut entries: Vec<_> = applied.iter().collect();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    properties.insert(
        "applied_migrations".to_string(),
        PropertyValue::Array(
            entries
                .into_iter()
                .map(|(id, hash)| {
                    PropertyValue::Object(HashMap::from([
                        ("id".to_string(), PropertyValue::String(id.clone())),
                        ("hash".to_string(), PropertyValue::String(hash.clone())),
                        (
                            "status".to_string(),
                            PropertyValue::String("applied".to_string()),
                        ),
                    ]))
                })
                .collect(),
        ),
    );
}

fn write_migration_summary(
    properties: &mut HashMap<String, PropertyValue>,
    file: &MigrationFile,
    status: &str,
) {
    let mut summaries = match properties.remove("migrations") {
        Some(PropertyValue::Array(items)) => items,
        _ => Vec::new(),
    };
    summaries.retain(|item| match item {
        PropertyValue::Object(obj) => {
            obj.get("id") != Some(&PropertyValue::String(file.migration.id.clone()))
        }
        _ => true,
    });
    summaries.push(PropertyValue::Object(HashMap::from([
        (
            "id".to_string(),
            PropertyValue::String(file.migration.id.clone()),
        ),
        ("hash".to_string(), PropertyValue::String(file.hash.clone())),
        (
            "path".to_string(),
            PropertyValue::String(file.zip_path.clone()),
        ),
        (
            "status".to_string(),
            PropertyValue::String(status.to_string()),
        ),
        (
            "operations".to_string(),
            PropertyValue::Integer(file.migration.operations.len() as i64),
        ),
    ])));
    properties.insert("migrations".to_string(), PropertyValue::Array(summaries));
}

fn merge_properties(
    target: &mut HashMap<String, PropertyValue>,
    patch: &HashMap<String, PropertyValue>,
) {
    for (key, value) in patch {
        match (target.get_mut(key), value) {
            (Some(PropertyValue::Object(current)), PropertyValue::Object(next)) => {
                merge_properties(current, next);
            }
            (Some(PropertyValue::Array(current)), PropertyValue::Object(next)) => {
                if let Some(PropertyValue::Array(add)) = next.get("add") {
                    for item in add {
                        if !current.contains(item) {
                            current.push(item.clone());
                        }
                    }
                }
                if let Some(PropertyValue::Array(remove)) = next.get("remove") {
                    current.retain(|item| !remove.contains(item));
                }
            }
            _ => {
                target.insert(key.clone(), value.clone());
            }
        }
    }
}
