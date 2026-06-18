//! RESTORE statement execution.
//!
//! Restores a node (and optionally its descendants) to its state at a previous
//! revision. Supports HEAD~N, branch~N, and direct HLC timestamp references.

use super::QueryEngine;
use crate::physical_plan::executor::{Row, RowStream};
use futures::stream;
use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::{
    BranchRepository, NodeRepository, RepoScope, Storage, StorageScope, WorkspaceRepository,
};

impl<S: Storage + raisin_storage::transactional::TransactionalStorage + 'static> QueryEngine<S> {
    /// Execute a RESTORE statement
    ///
    /// Restores a node (and optionally its descendants) to its state at a previous revision.
    /// The node stays at its current path -- this is an in-place restore, not a copy.
    pub(crate) async fn execute_restore(
        &self,
        restore_stmt: &raisin_sql::analyzer::AnalyzedRestore,
    ) -> Result<RowStream, Error> {
        use raisin_core::NodeService;
        use raisin_sql::ast::branch::RevisionRef;
        use raisin_sql::ast::order::NodeReference;

        let branch = self.effective_branch().await;

        tracing::info!(
            "Executing RESTORE: {:?} TO REVISION {:?} on branch '{}'",
            restore_stmt.node,
            restore_stmt.revision,
            branch
        );

        // Step 1: Resolve which workspace the node lives in. RESTORE carries no
        // workspace (no FROM clause, no session workspace), so we locate the node
        // by scanning the repo's workspaces. We try the node path's first segment
        // first as a fast path (matches the common convention), then fall back to
        // every workspace, so the lookup is correct regardless of naming.
        let mut candidates: Vec<String> = self
            .storage
            .workspaces()
            .list(RepoScope::new(&self.tenant_id, &self.repo_id))
            .await
            .map(|wss| wss.into_iter().map(|w| w.name).collect())
            .unwrap_or_default();
        if candidates.is_empty() {
            candidates.push("default".to_string());
        }
        // Prioritise the path's first segment if it names a known workspace.
        if let NodeReference::Path(path) = &restore_stmt.node {
            if let Some(seg) = path.trim_start_matches('/').split('/').next() {
                if !seg.is_empty() {
                    if let Some(pos) = candidates.iter().position(|w| w == seg) {
                        candidates.swap(0, pos);
                    }
                }
            }
        }

        let build_service = |workspace: String| {
            let svc = NodeService::new_with_context(
                self.storage.clone(),
                self.tenant_id.clone(),
                self.repo_id.clone(),
                branch.clone(),
                workspace,
            );
            // Propagate the engine's auth context so reads/writes pass RLS the
            // same way the rest of the SQL pipeline does. Without this, get_by_path
            // is RLS-filtered to empty and the node looks "not found".
            match self.auth_context.clone() {
                Some(auth) => svc.with_auth(auth),
                None => svc,
            }
        };

        // Step 2: Resolve node reference within the first workspace that has it.
        let mut resolved: Option<(String, String, String)> = None; // (workspace, path, id)
        for ws in &candidates {
            let svc = build_service(ws.clone());
            match &restore_stmt.node {
                NodeReference::Path(path) => {
                    if let Some(node) = svc.get_by_path(path).await? {
                        resolved = Some((ws.clone(), path.clone(), node.id));
                        break;
                    }
                }
                NodeReference::Id(id) => {
                    if let Some(node) = svc.get(id).await? {
                        resolved = Some((ws.clone(), node.path.clone(), id.clone()));
                        break;
                    }
                }
            }
        }
        let (workspace, node_path, node_id) = resolved.ok_or_else(|| {
            Error::NotFound(format!(
                "Node {:?} not found in any workspace of {}/{} on branch '{}'",
                restore_stmt.node, self.tenant_id, self.repo_id, branch
            ))
        })?;

        let node_service = build_service(workspace.clone());

        // Step 3: Resolve revision reference to HLC
        let revision_hlc = match &restore_stmt.revision {
            RevisionRef::HeadRelative(offset) => {
                // Resolve HEAD~N from the MVCC node history (newest-first), which
                // is populated for EVERY write path — node API, SQL DML, and
                // functions — because they all commit through the transaction
                // layer. (The older `revisions().get_node_revisions` reverse index
                // is only written by the repository CRUD path, so it was empty for
                // SQL/function writes and HEAD~N silently failed.)
                let history = self
                    .storage
                    .nodes()
                    .get_node_history(
                        StorageScope::new(&self.tenant_id, &self.repo_id, &branch, &workspace),
                        &node_id,
                        Some((*offset as usize) + 1),
                    )
                    .await?;

                if history.is_empty() {
                    return Err(Error::NotFound(format!(
                        "Node '{}' has no revision history",
                        node_path
                    )));
                }

                if *offset as usize >= history.len() {
                    return Err(Error::NotFound(format!(
                        "Node '{}' only has {} revisions, cannot go back {} revisions (HEAD~{})",
                        node_path,
                        history.len(),
                        offset,
                        offset
                    )));
                }

                history[*offset as usize].revision
            }
            RevisionRef::BranchRelative {
                branch: source_branch,
                offset,
            } => {
                if *offset == 0 {
                    self.storage
                        .branches()
                        .get_head(&self.tenant_id, &self.repo_id, source_branch)
                        .await?
                } else {
                    return Err(Error::Validation(format!(
                        "{}~{} resolution not yet implemented. Use {}~0 or an HLC timestamp directly.",
                        source_branch, offset, source_branch
                    )));
                }
            }
            RevisionRef::Hlc(hlc_str) => {
                let normalized = hlc_str.replace('_', "-");
                normalized.parse::<raisin_hlc::HLC>().map_err(|e| {
                    Error::Validation(format!("Invalid HLC timestamp '{}': {}", hlc_str, e))
                })?
            }
        };

        // Step 4: Get the node at the historical revision
        let historical_service = build_service(workspace.clone()).at_revision(revision_hlc);

        let historical_node = historical_service
            .get_by_path(&node_path)
            .await?
            .ok_or_else(|| {
                Error::NotFound(format!(
                    "Node at path '{}' not found at revision {}",
                    node_path, revision_hlc
                ))
            })?;

        // Step 5: Handle recursive restore (RESTORE TREE NODE)
        if restore_stmt.recursive {
            if let Some(ref registrar) = self.restore_tree_registrar {
                let job_id = registrar(
                    node_id.clone(),
                    node_path.clone(),
                    revision_hlc.to_string(),
                    restore_stmt.translations.clone(),
                    self.default_actor.clone(),
                )
                .await?;

                let mut row = Row::new();
                row.insert(
                    "result".to_string(),
                    PropertyValue::String(format!(
                        "RestoreTree job queued for '{}' to revision {}",
                        node_path, revision_hlc
                    )),
                );
                row.insert("job_id".to_string(), PropertyValue::String(job_id));
                row.insert(
                    "status".to_string(),
                    PropertyValue::String("queued".to_string()),
                );
                row.insert("path".to_string(), PropertyValue::String(node_path));
                row.insert(
                    "revision".to_string(),
                    PropertyValue::String(revision_hlc.to_string()),
                );

                return Ok(Box::pin(stream::once(async move { Ok(row) })));
            } else {
                return Err(Error::Validation(
                    "RESTORE TREE NODE requires background job support. Job registrar not configured.".to_string()
                ));
            }
        }

        // Step 6: For single node restore with TRANSLATIONS clause, merge translations
        let current_node = node_service
            .get_by_path(&node_path)
            .await?
            .ok_or_else(|| Error::NotFound(format!("Node at path '{}' not found", node_path)))?;

        let restored_node = if let Some(ref translations) = restore_stmt.translations {
            let mut merged_node = current_node.clone();
            let historical_translations = historical_node.translations.as_ref();

            for locale in translations {
                if let Some(hist_trans) = historical_translations {
                    if let Some(historical_value) = hist_trans.get(locale) {
                        let merged_translations = merged_node
                            .translations
                            .get_or_insert_with(std::collections::HashMap::new);
                        merged_translations.insert(locale.clone(), historical_value.clone());
                    }
                }
            }

            merged_node
        } else {
            let mut restored = historical_node.clone();
            restored.path = current_node.path.clone();
            restored.id = current_node.id.clone();
            restored
        };

        // Step 7: Perform the restore by updating the node
        node_service
            .update_node(restored_node.clone())
            .await
            .map_err(|e| Error::Backend(format!("Failed to restore node: {}", e)))?;

        let translations_info = if let Some(ref translations) = restore_stmt.translations {
            format!(" (translations: {:?})", translations)
        } else {
            String::new()
        };

        tracing::info!(
            "Restored node '{}' to revision {}{}",
            node_path,
            revision_hlc,
            translations_info
        );

        let mut row = Row::new();
        row.insert(
            "result".to_string(),
            PropertyValue::String(format!(
                "Node '{}' restored to revision {}{}",
                node_path, revision_hlc, translations_info
            )),
        );
        row.insert("affected_rows".to_string(), PropertyValue::Integer(1));
        row.insert("path".to_string(), PropertyValue::String(node_path));
        row.insert(
            "revision".to_string(),
            PropertyValue::String(revision_hlc.to_string()),
        );

        Ok(Box::pin(stream::once(async move { Ok(row) })))
    }
}
