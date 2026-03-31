//! RESTORE statement execution.
//!
//! Restores a node (and optionally its descendants) to its state at a previous
//! revision. Supports HEAD~N, branch~N, and direct HLC timestamp references.

use super::QueryEngine;
use crate::physical_plan::executor::{Row, RowStream};
use futures::stream;
use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::{BranchRepository, RevisionRepository, Storage};

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
        let workspace = "default";

        tracing::info!(
            "Executing RESTORE: {:?} TO REVISION {:?} on branch '{}'",
            restore_stmt.node,
            restore_stmt.revision,
            branch
        );

        // Step 1: Create NodeService
        let node_service = NodeService::new_with_context(
            self.storage.clone(),
            self.tenant_id.clone(),
            self.repo_id.clone(),
            branch.clone(),
            workspace.to_string(),
        );

        // Step 2: Resolve node reference
        let (node_path, node_id) =
            match &restore_stmt.node {
                NodeReference::Path(path) => {
                    let node = node_service.get_by_path(path).await?.ok_or_else(|| {
                        Error::NotFound(format!("Node at path '{}' not found", path))
                    })?;
                    (path.clone(), node.id)
                }
                NodeReference::Id(id) => {
                    let node = node_service.get(id).await?.ok_or_else(|| {
                        Error::NotFound(format!("Node with id '{}' not found", id))
                    })?;
                    (node.path.clone(), id.clone())
                }
            };

        // Step 3: Resolve revision reference to HLC
        let revision_hlc = match &restore_stmt.revision {
            RevisionRef::HeadRelative(offset) => {
                let revisions = self
                    .storage
                    .revisions()
                    .get_node_revisions(
                        &self.tenant_id,
                        &self.repo_id,
                        &node_id,
                        (*offset as usize) + 1,
                    )
                    .await?;

                if revisions.is_empty() {
                    return Err(Error::NotFound(format!(
                        "Node '{}' has no revision history",
                        node_path
                    )));
                }

                if *offset as usize >= revisions.len() {
                    return Err(Error::NotFound(format!(
                        "Node '{}' only has {} revisions, cannot go back {} revisions (HEAD~{})",
                        node_path,
                        revisions.len(),
                        offset,
                        offset
                    )));
                }

                revisions[*offset as usize]
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
        let historical_service = NodeService::new_with_context(
            self.storage.clone(),
            self.tenant_id.clone(),
            self.repo_id.clone(),
            branch.clone(),
            workspace.to_string(),
        )
        .at_revision(revision_hlc);

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
