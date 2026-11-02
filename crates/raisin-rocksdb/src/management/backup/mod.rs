//! Repository-level backup and restore
//!
//! This module provides backup and restore operations scoped to repositories:
//! - Backup a specific repository (all branches, workspaces, revisions)
//! - Restore a repository from backup
//! - Backup all repositories for a tenant
//!
//! Backup format is JSON-based for portability and human-readability:
//! - metadata.json: Repository metadata
//! - nodes.jsonl: All nodes (JSON Lines format for streaming)
//! - branches.json: Branch metadata
//! - workspaces.json: Workspace metadata
//! - revisions.json: Revision history
//! - trees.json: Content-addressed trees
//! - nodetypes.json: NodeType schemas
//! - relations.jsonl: Graph relations live at each branch HEAD (see
//!   [`relations`]; absent in backups of older releases)
//! - retired_user_keys.jsonl: the tenant's retired user keys ledger (keys and
//!   times only, see [`crate::retired_keys`]); a restore merges it (union)
//!   and leaves out every user home whose key is retired

mod export;
mod helpers;
mod import;
mod relations;

/// File name of the retired user keys section of a repository backup.
const RETIRED_KEYS_FILE: &str = "retired_user_keys.jsonl";

/// Options of a restore.
#[derive(Debug, Clone, Default)]
pub struct RestoreOptions {
    /// Explicit admin override: restore user homes whose key is retired
    /// (deleted persons). Off by default; deliberate disaster recovery only,
    /// audit-logged with `actor`.
    pub allow_retired_user_keys: bool,
    /// Who asked for the override (for the audit log).
    pub actor: Option<String>,
}

/// What a restore left out or added.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RestoreReport {
    /// Keys of user homes not restored because the key is retired (keys only).
    pub skipped_retired_keys: Vec<String>,
    /// Nodes left out with them (the homes and everything below them).
    pub nodes_skipped_retired: usize,
    /// Relations left out because an endpoint was left out.
    pub relations_skipped_retired: usize,
    /// Ledger entries of the backup that were new to the tenant ledger.
    pub retired_keys_merged: usize,
}

use crate::RocksDBStorage;
use raisin_error::Result;
use raisin_storage::BackupInfo;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::Path;

/// Repository metadata for backup
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct RepositoryMetadata {
    pub tenant_id: String,
    pub repo_id: String,
    pub backed_up_at: chrono::DateTime<chrono::Utc>,
    pub version: String,
}

/// Backup a specific repository (all branches, workspaces, revisions)
pub async fn backup_repository(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    dest: &Path,
) -> Result<BackupInfo> {
    let start = std::time::Instant::now();
    let created_at = chrono::Utc::now();

    tracing::info!("Starting backup of repository {}/{}", tenant_id, repo_id);

    // Create backup directory structure
    let repo_backup_dir = dest.join(tenant_id).join(repo_id);
    std::fs::create_dir_all(&repo_backup_dir).map_err(|e| {
        raisin_error::Error::storage(format!("Failed to create backup directory: {}", e))
    })?;

    // 1. Export repository metadata
    let metadata = RepositoryMetadata {
        tenant_id: tenant_id.to_string(),
        repo_id: repo_id.to_string(),
        backed_up_at: created_at,
        version: env!("CARGO_PKG_VERSION").to_string(),
    };

    let metadata_file = repo_backup_dir.join("metadata.json");
    std::fs::write(
        &metadata_file,
        rmp_serde::to_vec(&metadata).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to serialize metadata: {}", e))
        })?,
    )
    .map_err(|e| raisin_error::Error::storage(format!("Failed to write metadata: {}", e)))?;

    // 2. Export all nodes as JSON Lines
    tracing::info!("Exporting nodes...");
    let nodes = export::export_all_repository_nodes(storage, tenant_id, repo_id).await?;
    let node_count = nodes.len() as u64;

    let nodes_file = repo_backup_dir.join("nodes.jsonl");
    let mut nodes_writer = std::fs::File::create(&nodes_file)
        .map_err(|e| raisin_error::Error::storage(format!("Failed to create nodes file: {}", e)))?;

    for node in &nodes {
        serde_json::to_writer(&mut nodes_writer, node)
            .map_err(|e| raisin_error::Error::storage(format!("Failed to write node: {}", e)))?;
        writeln!(&mut nodes_writer)
            .map_err(|e| raisin_error::Error::storage(format!("Failed to write newline: {}", e)))?;
    }

    // 2b. Export graph relations (RELATION_INDEX is keyed by node id and is
    // not part of the node records).
    tracing::info!("Exporting relations...");
    let rels = relations::export_relations(storage, tenant_id, repo_id).await?;
    relations::write_relations(&repo_backup_dir.join(relations::RELATIONS_FILE), &rels)?;
    tracing::info!("Exported {} relations", rels.len());

    // 2c. Export the tenant's retired user keys ledger (keys and times only).
    let ledger = crate::retired_keys::list(storage.db(), tenant_id)?;
    {
        let mut w =
            std::fs::File::create(repo_backup_dir.join(RETIRED_KEYS_FILE)).map_err(|e| {
                raisin_error::Error::storage(format!("Failed to create ledger file: {}", e))
            })?;
        for entry in &ledger {
            serde_json::to_writer(&mut w, entry).map_err(|e| {
                raisin_error::Error::storage(format!("Failed to write ledger entry: {}", e))
            })?;
            writeln!(&mut w).map_err(|e| {
                raisin_error::Error::storage(format!("Failed to write newline: {}", e))
            })?;
        }
    }

    // 3. Export branches
    tracing::info!("Exporting branches...");
    let branches = export::export_branches(storage, tenant_id, repo_id).await?;
    let branches_file = repo_backup_dir.join("branches.json");
    std::fs::write(
        &branches_file,
        rmp_serde::to_vec(&branches).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to serialize branches: {}", e))
        })?,
    )
    .map_err(|e| raisin_error::Error::storage(format!("Failed to write branches: {}", e)))?;

    // 4. Export workspaces
    tracing::info!("Exporting workspaces...");
    let workspaces = export::export_workspaces(storage, tenant_id, repo_id).await?;
    let workspaces_file = repo_backup_dir.join("workspaces.json");
    std::fs::write(
        &workspaces_file,
        rmp_serde::to_vec_named(&workspaces).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to serialize workspaces: {}", e))
        })?,
    )
    .map_err(|e| raisin_error::Error::storage(format!("Failed to write workspaces: {}", e)))?;

    // 5. Export revisions
    tracing::info!("Exporting revisions...");
    let revisions = export::export_revisions(storage, tenant_id, repo_id).await?;
    let revisions_file = repo_backup_dir.join("revisions.json");
    std::fs::write(
        &revisions_file,
        rmp_serde::to_vec(&revisions).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to serialize revisions: {}", e))
        })?,
    )
    .map_err(|e| raisin_error::Error::storage(format!("Failed to write revisions: {}", e)))?;

    // 6. Export NodeTypes
    tracing::info!("Exporting NodeTypes...");
    let nodetypes = export::export_nodetypes(storage, tenant_id, repo_id).await?;
    let nodetypes_file = repo_backup_dir.join("nodetypes.json");
    std::fs::write(
        &nodetypes_file,
        rmp_serde::to_vec(&nodetypes).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to serialize nodetypes: {}", e))
        })?,
    )
    .map_err(|e| raisin_error::Error::storage(format!("Failed to write nodetypes: {}", e)))?;

    // 7. Export trees (content-addressed storage)
    tracing::info!("Exporting trees...");
    let trees = export::export_trees(storage, tenant_id, repo_id).await?;
    let trees_file = repo_backup_dir.join("trees.jsonl");
    let mut trees_writer = std::fs::File::create(&trees_file)
        .map_err(|e| raisin_error::Error::storage(format!("Failed to create trees file: {}", e)))?;

    for (tree_id, entries) in &trees {
        let tree_data = export::TreeBackupEntry {
            tree_id_hex: hex::encode(tree_id),
            entries: entries.clone(),
        };
        serde_json::to_writer(&mut trees_writer, &tree_data)
            .map_err(|e| raisin_error::Error::storage(format!("Failed to write tree: {}", e)))?;
        writeln!(&mut trees_writer)
            .map_err(|e| raisin_error::Error::storage(format!("Failed to write newline: {}", e)))?;
    }

    // 8. Calculate total backup size
    let size_bytes = helpers::calculate_directory_size(&repo_backup_dir)?;
    let duration_ms = start.elapsed().as_millis() as u64;

    tracing::info!(
        "Backup complete: {} nodes, {} bytes in {}ms",
        node_count,
        size_bytes,
        duration_ms
    );

    Ok(BackupInfo {
        tenant: format!("{}/{}", tenant_id, repo_id),
        path: repo_backup_dir,
        size_bytes,
        created_at,
        duration_ms,
        node_count,
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

/// Restore a repository from backup (no ledger override; the report is
/// logged).
pub async fn restore_repository(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    src: &Path,
) -> Result<()> {
    let report = restore_repository_with_options(
        storage,
        tenant_id,
        repo_id,
        src,
        &RestoreOptions::default(),
    )
    .await?;
    if !report.skipped_retired_keys.is_empty() {
        tracing::warn!(
            skipped_retired_keys = report.skipped_retired_keys.len(),
            keys = ?report.skipped_retired_keys,
            nodes = report.nodes_skipped_retired,
            relations = report.relations_skipped_retired,
            "Restore left out user homes whose key is retired (deleted persons)"
        );
    }
    Ok(())
}

/// Restore a repository from backup.
///
/// The backup's retired keys are MERGED into the tenant ledger first (union,
/// never a replacement), then every user home whose key the ledger knows as
/// retired is left out with everything below it and its relations, unless
/// `options.allow_retired_user_keys` (an administrator's explicit override,
/// audit-logged).
pub async fn restore_repository_with_options(
    storage: &RocksDBStorage,
    tenant_id: &str,
    repo_id: &str,
    src: &Path,
    options: &RestoreOptions,
) -> Result<RestoreReport> {
    let mut report = RestoreReport::default();
    let start = std::time::Instant::now();

    tracing::info!("Starting restore of repository {}/{}", tenant_id, repo_id);

    let repo_backup_dir = src.join(tenant_id).join(repo_id);

    if !repo_backup_dir.exists() {
        return Err(raisin_error::Error::storage(format!(
            "Backup directory not found: {}",
            repo_backup_dir.display()
        )));
    }

    // 1. Verify and read metadata
    let metadata_file = repo_backup_dir.join("metadata.json");
    let metadata: RepositoryMetadata =
        rmp_serde::from_slice(&std::fs::read(&metadata_file).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to read metadata: {}", e))
        })?)
        .map_err(|e| raisin_error::Error::storage(format!("Failed to parse metadata: {}", e)))?;

    tracing::info!("Restoring backup from {}", metadata.backed_up_at);

    // 2. Import NodeTypes first (nodes may reference them)
    tracing::info!("Importing NodeTypes...");
    let nodetypes_file = repo_backup_dir.join("nodetypes.json");
    if nodetypes_file.exists() {
        import::import_nodetypes(storage, tenant_id, repo_id, &nodetypes_file).await?;
    }

    // 3. Import branches
    tracing::info!("Importing branches...");
    let branches_file = repo_backup_dir.join("branches.json");
    if branches_file.exists() {
        import::import_branches(storage, tenant_id, repo_id, &branches_file).await?;
    }

    // 4. Import workspaces
    tracing::info!("Importing workspaces...");
    let workspaces_file = repo_backup_dir.join("workspaces.json");
    if workspaces_file.exists() {
        import::import_workspaces(storage, tenant_id, repo_id, &workspaces_file).await?;
    }

    // 5. Import revisions
    tracing::info!("Importing revisions...");
    let revisions_file = repo_backup_dir.join("revisions.json");
    if revisions_file.exists() {
        import::import_revisions(storage, tenant_id, repo_id, &revisions_file).await?;
    }

    // 6. Import trees
    tracing::info!("Importing trees...");
    let trees_file = repo_backup_dir.join("trees.jsonl");
    if trees_file.exists() {
        import::import_trees(storage, tenant_id, repo_id, &trees_file).await?;
    }

    // 6b. Merge the backup's retired keys into the tenant ledger (union).
    let ledger_file = repo_backup_dir.join(RETIRED_KEYS_FILE);
    if ledger_file.exists() {
        let text = std::fs::read_to_string(&ledger_file).map_err(|e| {
            raisin_error::Error::storage(format!("Failed to read ledger file: {}", e))
        })?;
        let mut entries = Vec::new();
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let entry: crate::retired_keys::RetiredUserKey =
                serde_json::from_str(line).map_err(|e| {
                    raisin_error::Error::storage(format!("Failed to parse ledger: {}", e))
                })?;
            if entry.v > crate::retired_keys::RETIRED_KEY_LINE_VERSION {
                return Err(raisin_error::Error::Validation(format!(
                    "restore refused: {} has format v{}",
                    RETIRED_KEYS_FILE, entry.v
                )));
            }
            entries.push(entry);
        }
        report.retired_keys_merged = crate::retired_keys::merge(storage.db(), tenant_id, &entries)?;
    }

    // 7. Import nodes (last, as they may reference other data), leaving out
    // the homes of deleted persons (retired keys) with their subtrees.
    tracing::info!("Importing nodes...");
    let nodes_file = repo_backup_dir.join("nodes.jsonl");
    let db = storage.db().clone();
    let skipped_keys = std::sync::Mutex::new(std::collections::BTreeSet::<String>::new());
    let restored_keys = std::sync::Mutex::new(std::collections::BTreeSet::<String>::new());
    let skip = |n: &raisin_models::nodes::Node| -> bool {
        if n.workspace.as_deref() != Some(raisin_models::auth::user_key::USER_WORKSPACE) {
            return false;
        }
        let Some(key) = raisin_models::auth::user_key::user_key_of_path(&n.path) else {
            return false;
        };
        // A ledger read error is a refusal: fail closed.
        let retired =
            crate::retired_keys::retired_and_gone(&db, tenant_id, repo_id, key).unwrap_or(true);
        if !retired {
            return false;
        }
        if options.allow_retired_user_keys {
            restored_keys
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(key.to_string());
            return false;
        }
        skipped_keys
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key.to_string());
        true
    };
    let skipped_ids =
        import::import_nodes_from_jsonl(storage, tenant_id, repo_id, &nodes_file, &skip).await?;
    report.nodes_skipped_retired = skipped_ids.len();
    report.skipped_retired_keys = skipped_keys
        .into_inner()
        .unwrap_or_else(|e| e.into_inner())
        .into_iter()
        .collect();
    let restored = restored_keys
        .into_inner()
        .unwrap_or_else(|e| e.into_inner());
    if !restored.is_empty() {
        crate::retired_keys::audit_override(
            tenant_id,
            "backup restore",
            options.actor.as_deref().unwrap_or("unknown"),
            &restored.into_iter().collect::<Vec<_>>(),
        );
    }

    // 8. Import graph relations (node ids are preserved, so the endpoints are
    // written back unchanged). Older backups have no relations file.
    let relations_file = repo_backup_dir.join(relations::RELATIONS_FILE);
    if relations_file.exists() {
        let mut rels = relations::read_relations(&relations_file)?;
        let before = rels.len();
        rels.retain(|r| {
            !skipped_ids.contains(&r.relation.source_id)
                && !skipped_ids.contains(&r.relation.target_id)
        });
        report.relations_skipped_retired = before - rels.len();
        let n = relations::import_relations(storage, tenant_id, repo_id, &rels)?;
        tracing::info!("Imported {} relations", n);
    }

    let duration_ms = start.elapsed().as_millis() as u64;
    tracing::info!("Restore complete in {}ms", duration_ms);

    Ok(report)
}

/// Backup all repositories for a tenant
pub async fn backup_tenant(
    storage: &RocksDBStorage,
    tenant_id: &str,
    dest: &Path,
) -> Result<Vec<BackupInfo>> {
    let repos = helpers::list_repositories(storage, tenant_id).await?;
    let mut infos = Vec::new();

    tracing::info!(
        "Backing up {} repositories for tenant {}",
        repos.len(),
        tenant_id
    );

    for repo_id in repos {
        let info = backup_repository(storage, tenant_id, &repo_id, dest).await?;
        infos.push(info);
    }

    Ok(infos)
}
