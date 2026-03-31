//! DML, DDL, Transaction, and SHOW statement handlers
//!
//! Executes mutations (INSERT, UPDATE, DELETE), schema changes (CREATE NODETYPE),
//! transaction control (BEGIN, COMMIT, SET), and SHOW variable queries.

use super::super::helpers;
use super::super::QueryEngine;
use crate::physical_plan::executor::{execute_plan, ExecutionContext, Row, RowStream};
use crate::physical_plan::planner::PhysicalPlanner;
use crate::physical_plan::IndexCatalog;
use futures::stream;
use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_sql::analyzer::AnalyzedStatement;
use raisin_sql::logical_plan::PlanBuilder;
use raisin_storage::Storage;
use std::sync::Arc;

impl<S: Storage + raisin_storage::transactional::TransactionalStorage + 'static> QueryEngine<S> {
    /// Execute a DML statement (INSERT, UPDATE, DELETE, etc.)
    pub(crate) async fn execute_dml(
        &self,
        analyzed: &AnalyzedStatement,
    ) -> Result<RowStream, Error> {
        tracing::info!("Executing DML statement");

        let (workspace, branch) = extract_dml_workspace_branch(analyzed, &self.branch);

        let plan_builder = PlanBuilder::new(self.catalog.as_ref());
        let logical_plan = plan_builder
            .build(analyzed)
            .map_err(|e| Error::Validation(format!("DML plan error: {}", e)))?;

        let index_catalog: Arc<dyn IndexCatalog> =
            Arc::new(crate::physical_plan::catalog::RocksDBIndexCatalog::new());

        let physical_planner = PhysicalPlanner::with_catalog(
            self.tenant_id.clone(),
            self.repo_id.clone(),
            branch.clone(),
            workspace.clone(),
            index_catalog,
        );

        let physical_plan = physical_planner.plan(&logical_plan)?;

        let mut ctx = ExecutionContext::new(
            self.storage.clone(),
            self.tenant_id.clone(),
            self.repo_id.clone(),
            branch,
            workspace,
        );

        ctx.transaction_context = self.transaction_context.clone();

        if let Some(ref auth) = self.auth_context {
            ctx = ctx.with_auth_context(auth.clone());
        }
        if let Some(ref cb) = self.function_invoke {
            ctx.function_invoke = Some(cb.clone());
        }
        if let Some(ref cb) = self.function_invoke_sync {
            ctx.function_invoke_sync = Some(cb.clone());
        }

        let stream = execute_plan(&physical_plan, &ctx).await?;
        Ok(stream)
    }

    /// Execute a DDL statement (CREATE/ALTER/DROP NODETYPE/ARCHETYPE/ELEMENTTYPE)
    pub(crate) async fn execute_ddl(
        &self,
        ddl: &raisin_sql::ast::ddl::DdlStatement,
    ) -> Result<RowStream, Error> {
        tracing::info!("Executing DDL statement");

        let stream = crate::physical_plan::ddl_executor::execute_ddl(
            ddl,
            self.storage.clone(),
            &self.tenant_id,
            &self.repo_id,
            &self.branch,
        )
        .await?;

        Ok(stream)
    }

    /// Execute a transaction statement (BEGIN, COMMIT, SET)
    pub(crate) async fn execute_transaction(
        &self,
        txn_stmt: &raisin_sql::ast::transaction::TransactionStatement,
    ) -> Result<RowStream, Error> {
        use raisin_sql::ast::transaction::TransactionStatement;
        use raisin_storage::transactional::TransactionalContext;

        match txn_stmt {
            TransactionStatement::Begin => self.handle_begin().await,
            TransactionStatement::Commit { message, actor } => {
                self.handle_commit(message.as_deref(), actor.as_deref())
                    .await
            }
            TransactionStatement::Set { variable, value } => self.handle_set(variable, value).await,
        }
    }

    async fn handle_begin(&self) -> Result<RowStream, Error> {
        use raisin_storage::transactional::TransactionalContext;

        {
            let tx_lock = self.transaction_context.read().await;
            if tx_lock.is_some() {
                return Err(Error::InvalidState(
                    "Transaction already in progress. Use COMMIT or ROLLBACK before starting a new transaction."
                        .to_string(),
                ));
            }
        }

        let ctx = self.storage.begin_context().await?;
        ctx.set_tenant_repo(&self.tenant_id, &self.repo_id)?;
        ctx.set_branch(&self.branch)?;

        if let Some(ref auth) = self.auth_context {
            ctx.set_auth_context(auth.clone())?;
        }

        {
            let mut tx_lock = self.transaction_context.write().await;
            *tx_lock = Some(ctx);
        }

        let mut result_row = Row::new();
        result_row.insert(
            "message".to_string(),
            PropertyValue::String("Transaction started".to_string()),
        );
        Ok(Box::pin(stream::once(async move { Ok(result_row) })))
    }

    async fn handle_commit(
        &self,
        message: Option<&str>,
        actor: Option<&str>,
    ) -> Result<RowStream, Error> {
        use raisin_storage::transactional::TransactionalContext;

        let ctx = {
            let mut tx_lock = self.transaction_context.write().await;
            tx_lock.take()
        };

        if let Some(ctx) = ctx {
            ctx.set_message(message.unwrap_or("SQL transaction"))?;
            ctx.set_actor(actor.unwrap_or("sql-client"))?;
            ctx.commit().await?;

            let mut result_row = Row::new();
            result_row.insert(
                "message".to_string(),
                PropertyValue::String("Transaction committed".to_string()),
            );
            Ok(Box::pin(stream::once(async move { Ok(result_row) })))
        } else {
            Err(Error::InvalidState(
                "No active transaction to commit. Use BEGIN to start a transaction first."
                    .to_string(),
            ))
        }
    }

    async fn handle_set(&self, variable: &str, value: &str) -> Result<RowStream, Error> {
        match variable.to_lowercase().as_str() {
            "validate_schema" => {
                let enabled = match value.to_lowercase().as_str() {
                    "true" | "on" | "1" | "yes" => true,
                    "false" | "off" | "0" | "no" => false,
                    _ => {
                        return Err(Error::Validation(format!(
                            "Invalid value for validate_schema: '{}'. Expected true/false, on/off, 1/0, or yes/no.",
                            value
                        )));
                    }
                };

                {
                    let tx_lock = self.transaction_context.read().await;
                    if let Some(ref ctx) = *tx_lock {
                        use raisin_storage::transactional::TransactionalContext;
                        ctx.set_validate_schema(enabled)?;
                    }
                }

                let variable = variable.to_string();
                let value = value.to_string();
                let mut result_row = Row::new();
                result_row.insert(
                    "command".to_string(),
                    PropertyValue::String("SET".to_string()),
                );
                result_row.insert("variable".to_string(), PropertyValue::String(variable));
                result_row.insert("value".to_string(), PropertyValue::String(value));
                Ok(Box::pin(stream::once(async move { Ok(result_row) })))
            }
            _ => Err(Error::Validation(format!(
                "Unknown session variable: '{}'. Supported variables: validate_schema",
                variable
            ))),
        }
    }

    /// Execute a SHOW statement returning PostgreSQL configuration values
    pub(crate) async fn execute_show(
        &self,
        show_stmt: &raisin_sql::AnalyzedShow,
    ) -> Result<RowStream, Error> {
        let value = match show_stmt.variable.as_str() {
            "server_version" => "14.0",
            "server_encoding" => "UTF8",
            "client_encoding" => "UTF8",
            "is_superuser" => "off",
            "session_authorization" => "raisin",
            "timezone" => "UTC",
            "integer_datetimes" => "on",
            "standard_conforming_strings" => "on",
            "transaction isolation level"
            | "transaction_isolation"
            | "default_transaction_isolation" => "read committed",
            "max_identifier_length" => "63",
            "datestyle" => "ISO, MDY",
            "intervalstyle" => "postgres",
            "extra_float_digits" => "3",
            "application_name" => "",
            "search_path" => "public",
            _ => {
                return Err(Error::Validation(format!(
                    "unrecognized configuration parameter \"{}\"",
                    show_stmt.variable
                )));
            }
        };

        let variable = show_stmt.variable.clone();
        let mut result_row = Row::new();
        result_row.insert(variable, PropertyValue::String(value.to_string()));
        Ok(Box::pin(stream::once(async move { Ok(result_row) })))
    }
}

/// Extract workspace and branch from a DML analyzed statement
fn extract_dml_workspace_branch(
    analyzed: &AnalyzedStatement,
    default_branch: &str,
) -> (String, String) {
    match analyzed {
        AnalyzedStatement::Insert(insert) => (
            match &insert.target {
                raisin_sql::analyzer::DmlTableTarget::Workspace(ws) => ws.clone(),
                _ => "default".to_string(),
            },
            default_branch.to_string(),
        ),
        AnalyzedStatement::Update(update) => (
            match &update.target {
                raisin_sql::analyzer::DmlTableTarget::Workspace(ws) => ws.clone(),
                _ => "default".to_string(),
            },
            update
                .branch_override
                .clone()
                .unwrap_or_else(|| default_branch.to_string()),
        ),
        AnalyzedStatement::Delete(delete) => (
            match &delete.target {
                raisin_sql::analyzer::DmlTableTarget::Workspace(ws) => ws.clone(),
                _ => "default".to_string(),
            },
            delete
                .branch_override
                .clone()
                .unwrap_or_else(|| default_branch.to_string()),
        ),
        AnalyzedStatement::Order(order) => (
            order.workspace.clone(),
            order
                .branch_override
                .clone()
                .unwrap_or_else(|| default_branch.to_string()),
        ),
        AnalyzedStatement::Move(mv) => (
            mv.workspace.clone(),
            mv.branch_override
                .clone()
                .unwrap_or_else(|| default_branch.to_string()),
        ),
        AnalyzedStatement::Copy(cp) => (
            cp.workspace.clone(),
            cp.branch_override
                .clone()
                .unwrap_or_else(|| default_branch.to_string()),
        ),
        AnalyzedStatement::Translate(tr) => (
            tr.workspace.clone(),
            tr.branch_override
                .clone()
                .unwrap_or_else(|| default_branch.to_string()),
        ),
        AnalyzedStatement::Relate(rel) => (
            rel.source.workspace.clone(),
            rel.branch_override
                .clone()
                .unwrap_or_else(|| default_branch.to_string()),
        ),
        AnalyzedStatement::Unrelate(unrel) => (
            unrel.source.workspace.clone(),
            unrel
                .branch_override
                .clone()
                .unwrap_or_else(|| default_branch.to_string()),
        ),
        _ => ("default".to_string(), default_branch.to_string()),
    }
}
