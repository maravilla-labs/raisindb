//! Branch management statement execution.
//!
//! Handles all BRANCH statement variants: USE, SHOW, CREATE, DROP, ALTER,
//! MERGE, DESCRIBE, SHOW DIVERGENCE, and SHOW CONFLICTS.

use super::QueryEngine;
use crate::physical_plan::executor::{Row, RowStream};
use futures::stream;
use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_sql::ast::branch::{
    BranchAlteration, BranchScope, BranchStatement, MergeStrategy, SqlResolutionType,
};
use raisin_storage::{BranchRepository, Storage};

impl<S: Storage + raisin_storage::transactional::TransactionalStorage + 'static> QueryEngine<S> {
    /// Execute a BRANCH statement
    pub(crate) async fn execute_branch_statement(
        &self,
        branch_stmt: &BranchStatement,
    ) -> Result<RowStream, Error> {
        tracing::info!("Executing BRANCH statement: {}", branch_stmt.operation());

        match branch_stmt {
            BranchStatement::UseBranch { name, scope } => {
                self.execute_use_branch(name, scope).await
            }
            BranchStatement::ShowCurrentBranch => self.execute_show_current_branch().await,
            BranchStatement::ShowBranches => self.execute_show_branches().await,
            BranchStatement::DescribeBranch(name) => self.execute_describe_branch(name).await,
            BranchStatement::ShowDivergence { branch, from } => {
                self.execute_show_divergence(branch, from).await
            }
            BranchStatement::ShowConflicts { source, target } => {
                self.execute_show_conflicts(source, target).await
            }
            BranchStatement::Create(create) => self.execute_create_branch(create).await,
            BranchStatement::Drop(drop) => self.execute_drop_branch(drop).await,
            BranchStatement::Alter(alter) => self.execute_alter_branch(alter).await,
            BranchStatement::Merge(merge) => self.execute_merge_branch(merge).await,
        }
    }

    async fn execute_use_branch(
        &self,
        name: &str,
        scope: &BranchScope,
    ) -> Result<RowStream, Error> {
        match scope {
            BranchScope::Session => {
                self.set_session_branch(name.to_string()).await;
            }
            BranchScope::Local => {
                self.set_local_branch(name.to_string()).await;
            }
        }

        let mut result_row = Row::new();
        result_row.insert(
            "command".to_string(),
            PropertyValue::String("SET".to_string()),
        );
        result_row.insert(
            "branch".to_string(),
            PropertyValue::String(name.to_string()),
        );
        result_row.insert(
            "scope".to_string(),
            PropertyValue::String(scope.to_string()),
        );
        Ok(Box::pin(stream::once(async move { Ok(result_row) })))
    }

    async fn execute_show_current_branch(&self) -> Result<RowStream, Error> {
        let branch = self.effective_branch().await;
        let mut result_row = Row::new();
        result_row.insert("branch".to_string(), PropertyValue::String(branch));
        Ok(Box::pin(stream::once(async move { Ok(result_row) })))
    }

    async fn execute_show_branches(&self) -> Result<RowStream, Error> {
        let branches = self
            .storage
            .branches()
            .list_branches(&self.tenant_id, &self.repo_id)
            .await?;

        let rows: Vec<Result<Row, Error>> = branches
            .into_iter()
            .map(|b| {
                let mut row = Row::new();
                row.insert("name".to_string(), PropertyValue::String(b.name));
                row.insert(
                    "head".to_string(),
                    PropertyValue::String(b.head.to_string()),
                );
                row.insert("protected".to_string(), PropertyValue::Boolean(b.protected));
                row.insert(
                    "upstream".to_string(),
                    b.upstream_branch
                        .map(PropertyValue::String)
                        .unwrap_or(PropertyValue::Null),
                );
                row.insert(
                    "created_at".to_string(),
                    PropertyValue::String(b.created_at.to_rfc3339()),
                );
                row.insert(
                    "created_by".to_string(),
                    PropertyValue::String(b.created_by),
                );
                Ok(row)
            })
            .collect();

        Ok(Box::pin(stream::iter(rows)))
    }

    async fn execute_describe_branch(&self, name: &str) -> Result<RowStream, Error> {
        let branch = self
            .storage
            .branches()
            .get_branch(&self.tenant_id, &self.repo_id, name)
            .await?
            .ok_or_else(|| Error::NotFound(format!("Branch '{}' not found", name)))?;

        let mut row = Row::new();
        row.insert("name".to_string(), PropertyValue::String(branch.name));
        row.insert(
            "head".to_string(),
            PropertyValue::String(branch.head.to_string()),
        );
        row.insert(
            "protected".to_string(),
            PropertyValue::Boolean(branch.protected),
        );
        row.insert(
            "upstream".to_string(),
            branch
                .upstream_branch
                .map(PropertyValue::String)
                .unwrap_or(PropertyValue::Null),
        );
        row.insert(
            "created_at".to_string(),
            PropertyValue::String(branch.created_at.to_rfc3339()),
        );
        row.insert(
            "created_by".to_string(),
            PropertyValue::String(branch.created_by),
        );
        row.insert(
            "created_from".to_string(),
            branch
                .created_from
                .map(|hlc| PropertyValue::String(hlc.to_string()))
                .unwrap_or(PropertyValue::Null),
        );
        row.insert(
            "description".to_string(),
            branch
                .description
                .map(PropertyValue::String)
                .unwrap_or(PropertyValue::Null),
        );

        Ok(Box::pin(stream::once(async move { Ok(row) })))
    }

    async fn execute_show_divergence(&self, branch: &str, from: &str) -> Result<RowStream, Error> {
        let divergence = self
            .storage
            .branches()
            .calculate_divergence(&self.tenant_id, &self.repo_id, branch, from)
            .await?;

        let mut row = Row::new();
        row.insert(
            "branch".to_string(),
            PropertyValue::String(branch.to_string()),
        );
        row.insert("base".to_string(), PropertyValue::String(from.to_string()));
        row.insert(
            "ahead".to_string(),
            PropertyValue::Integer(divergence.ahead as i64),
        );
        row.insert(
            "behind".to_string(),
            PropertyValue::Integer(divergence.behind as i64),
        );
        row.insert(
            "common_ancestor".to_string(),
            PropertyValue::String(divergence.common_ancestor.to_string()),
        );

        Ok(Box::pin(stream::once(async move { Ok(row) })))
    }

    async fn execute_show_conflicts(&self, source: &str, target: &str) -> Result<RowStream, Error> {
        let conflicts = self
            .storage
            .branches()
            .find_merge_conflicts(&self.tenant_id, &self.repo_id, target, source)
            .await?;

        if conflicts.is_empty() {
            let mut row = Row::new();
            row.insert(
                "result".to_string(),
                PropertyValue::String("No conflicts detected".to_string()),
            );
            return Ok(Box::pin(stream::once(async move { Ok(row) })));
        }

        let rows: Vec<Result<Row, Error>> = conflicts
            .into_iter()
            .map(|c| {
                let mut row = Row::new();
                row.insert("node_id".to_string(), PropertyValue::String(c.node_id));
                row.insert("path".to_string(), PropertyValue::String(c.path));
                row.insert(
                    "conflict_type".to_string(),
                    PropertyValue::String(format!("{:?}", c.conflict_type)),
                );
                row.insert(
                    "base_properties".to_string(),
                    c.base_properties
                        .as_ref()
                        .map(|p| {
                            PropertyValue::String(serde_json::to_string(p).unwrap_or_default())
                        })
                        .unwrap_or(PropertyValue::Null),
                );
                row.insert(
                    "target_properties".to_string(),
                    c.target_properties
                        .as_ref()
                        .map(|p| {
                            PropertyValue::String(serde_json::to_string(p).unwrap_or_default())
                        })
                        .unwrap_or(PropertyValue::Null),
                );
                row.insert(
                    "source_properties".to_string(),
                    c.source_properties
                        .as_ref()
                        .map(|p| {
                            PropertyValue::String(serde_json::to_string(p).unwrap_or_default())
                        })
                        .unwrap_or(PropertyValue::Null),
                );
                if let Some(locale) = c.translation_locale {
                    row.insert(
                        "translation_locale".to_string(),
                        PropertyValue::String(locale),
                    );
                }
                Ok(row)
            })
            .collect();

        Ok(Box::pin(stream::iter(rows)))
    }

    async fn execute_create_branch(
        &self,
        create: &raisin_sql::ast::branch::CreateBranch,
    ) -> Result<RowStream, Error> {
        let from_revision = if let Some(ref source) = create.from_branch {
            let head = self
                .storage
                .branches()
                .get_head(&self.tenant_id, &self.repo_id, source)
                .await?;
            Some(head)
        } else {
            None
        };

        self.storage
            .branches()
            .create_branch(
                &self.tenant_id,
                &self.repo_id,
                &create.name,
                "system",
                from_revision,
                create.upstream.clone(),
                create.protected,
                create.with_history,
            )
            .await?;

        let name = create.name.clone();
        let mut row = Row::new();
        row.insert(
            "result".to_string(),
            PropertyValue::String(format!("Branch '{}' created", name)),
        );
        Ok(Box::pin(stream::once(async move { Ok(row) })))
    }

    async fn execute_drop_branch(
        &self,
        drop: &raisin_sql::ast::branch::DropBranch,
    ) -> Result<RowStream, Error> {
        let deleted = self
            .storage
            .branches()
            .delete_branch(&self.tenant_id, &self.repo_id, &drop.name)
            .await?;

        if !deleted && !drop.if_exists {
            return Err(Error::NotFound(format!("Branch '{}' not found", drop.name)));
        }

        let name = drop.name.clone();
        let mut row = Row::new();
        row.insert(
            "result".to_string(),
            PropertyValue::String(format!("Branch '{}' dropped", name)),
        );
        Ok(Box::pin(stream::once(async move { Ok(row) })))
    }

    async fn execute_alter_branch(
        &self,
        alter: &raisin_sql::ast::branch::AlterBranch,
    ) -> Result<RowStream, Error> {
        match &alter.alteration {
            BranchAlteration::SetUpstream(upstream) => {
                self.storage
                    .branches()
                    .set_upstream_branch(
                        &self.tenant_id,
                        &self.repo_id,
                        &alter.name,
                        Some(upstream.clone()),
                    )
                    .await?;
            }
            BranchAlteration::UnsetUpstream => {
                self.storage
                    .branches()
                    .set_upstream_branch(&self.tenant_id, &self.repo_id, &alter.name, None)
                    .await?;
            }
            BranchAlteration::SetProtected(protected) => {
                self.storage
                    .branches()
                    .set_protected(&self.tenant_id, &self.repo_id, &alter.name, *protected)
                    .await?;
            }
            BranchAlteration::SetDescription(desc) => {
                self.storage
                    .branches()
                    .set_description(
                        &self.tenant_id,
                        &self.repo_id,
                        &alter.name,
                        Some(desc.clone()),
                    )
                    .await?;
            }
            BranchAlteration::RenameTo(_) => {
                return Err(Error::Validation("RENAME TO is not supported".to_string()));
            }
        }

        let name = alter.name.clone();
        let mut row = Row::new();
        row.insert(
            "result".to_string(),
            PropertyValue::String(format!("Branch '{}' altered", name)),
        );
        Ok(Box::pin(stream::once(async move { Ok(row) })))
    }

    async fn execute_merge_branch(
        &self,
        merge: &raisin_sql::ast::branch::MergeBranch,
    ) -> Result<RowStream, Error> {
        let message = merge.message.as_deref().unwrap_or("SQL merge");

        let result = if !merge.resolutions.is_empty() {
            let resolutions: Vec<raisin_context::ConflictResolution> = merge
                .resolutions
                .iter()
                .map(|r| {
                    let (resolution_type, properties) = match &r.resolution {
                        SqlResolutionType::KeepOurs => (
                            raisin_context::ResolutionType::KeepOurs,
                            serde_json::Value::Null,
                        ),
                        SqlResolutionType::KeepTheirs => (
                            raisin_context::ResolutionType::KeepTheirs,
                            serde_json::Value::Null,
                        ),
                        SqlResolutionType::Delete => (
                            raisin_context::ResolutionType::Manual,
                            serde_json::Value::Null,
                        ),
                        SqlResolutionType::UseValue(v) => {
                            (raisin_context::ResolutionType::Manual, v.clone())
                        }
                    };
                    raisin_context::ConflictResolution {
                        node_id: r.node_id.clone(),
                        resolution_type,
                        resolved_properties: properties,
                        translation_locale: r.translation_locale.clone(),
                    }
                })
                .collect();

            self.storage
                .branches()
                .resolve_merge_with_resolutions(
                    &self.tenant_id,
                    &self.repo_id,
                    &merge.target_branch,
                    &merge.source_branch,
                    resolutions,
                    message,
                    &self.default_actor,
                )
                .await?
        } else {
            let strategy = match merge.strategy {
                Some(MergeStrategy::FastForward) => raisin_context::MergeStrategy::FastForward,
                Some(MergeStrategy::ThreeWay) | None => raisin_context::MergeStrategy::ThreeWay,
            };

            self.storage
                .branches()
                .merge_branches(
                    &self.tenant_id,
                    &self.repo_id,
                    &merge.target_branch,
                    &merge.source_branch,
                    strategy,
                    message,
                    &self.default_actor,
                )
                .await?
        };

        if result.success {
            let mut row = Row::new();
            row.insert(
                "result".to_string(),
                PropertyValue::String("Merge completed".to_string()),
            );
            if let Some(rev) = result.revision {
                row.insert("revision".to_string(), PropertyValue::Integer(rev as i64));
            }
            row.insert(
                "fast_forward".to_string(),
                PropertyValue::Boolean(result.fast_forward),
            );
            row.insert(
                "nodes_changed".to_string(),
                PropertyValue::Integer(result.nodes_changed as i64),
            );
            Ok(Box::pin(stream::once(async move { Ok(row) })))
        } else {
            let conflicts_json =
                serde_json::to_string(&result.conflicts).unwrap_or_else(|_| "[]".to_string());

            Err(Error::Validation(format!(
                "Merge has {} conflict(s). Use SHOW CONFLICTS FOR MERGE '{}' INTO '{}' to view details, then use MERGE ... RESOLVE CONFLICTS (...) to resolve. Conflicts: {}",
                result.conflicts.len(),
                merge.source_branch,
                merge.target_branch,
                conflicts_json
            )))
        }
    }
}
