//! Batch SQL execution with async routing.
//!
//! Supports executing multiple SQL statements in sequence,
//! with automatic routing of complex WHERE clauses to background jobs.

use super::QueryEngine;
use crate::physical_plan::dml_executor::{classify_filter, FilterComplexity};
use crate::physical_plan::eval::{set_function_context, FunctionContext};
use crate::physical_plan::executor::{Row, RowStream};
use futures::stream;
use indexmap::IndexMap;
use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_sql::analyzer::{AnalyzedStatement, Analyzer};
use raisin_storage::Storage;

impl<S: Storage + raisin_storage::transactional::TransactionalStorage + 'static> QueryEngine<S> {
    /// Execute multiple SQL statements in a batch with automatic async routing
    ///
    /// If a job registrar is configured and the batch contains complex WHERE clauses,
    /// the batch is routed to async job execution and returns a single row with
    /// `job_id`, `status`, `message` columns.
    pub async fn execute_batch(&self, sql: &str) -> Result<RowStream, Error> {
        tracing::info!("SQL Query Engine starting batch execution");

        // 1. Analyze all statements
        let analyzer = Analyzer::with_catalog(self.catalog.clone_box());
        let statements = analyzer
            .analyze_batch(sql)
            .map_err(|e| Error::Validation(format!("Batch analysis error: {}", e)))?;

        if statements.is_empty() {
            return Err(Error::Validation(
                "No valid statements to execute".to_string(),
            ));
        }

        // 2. Check if async routing is needed
        if let Some(ref registrar) = self.job_registrar {
            if batch_requires_async(&statements) {
                tracing::info!("Batch requires async execution (complex WHERE clause detected)");

                let job_id = registrar(sql.to_string(), self.default_actor.clone()).await?;

                let mut columns = IndexMap::new();
                columns.insert("job_id".to_string(), PropertyValue::String(job_id));
                columns.insert(
                    "status".to_string(),
                    PropertyValue::String("accepted".to_string()),
                );
                columns.insert(
                    "message".to_string(),
                    PropertyValue::String(
                        "Bulk operation started. Poll /api/jobs/{job_id} for status.".to_string(),
                    ),
                );

                let row = Row { columns };
                return Ok(Box::pin(stream::iter(vec![Ok(row)])));
            }
        }

        tracing::info!("Executing {} statements in batch (sync)", statements.len());

        // 3. Execute each statement sequentially (sync path)
        self.execute_batch_sync_internal(&statements).await
    }

    /// Execute a batch synchronously (force sync, no async routing)
    pub async fn execute_batch_sync(&self, sql: &str) -> Result<RowStream, Error> {
        tracing::info!("SQL Query Engine starting batch execution (forced sync)");

        let analyzer = Analyzer::with_catalog(self.catalog.clone_box());
        let statements = analyzer
            .analyze_batch(sql)
            .map_err(|e| Error::Validation(format!("Batch analysis error: {}", e)))?;

        if statements.is_empty() {
            return Err(Error::Validation(
                "No valid statements to execute".to_string(),
            ));
        }

        self.execute_batch_sync_internal(&statements).await
    }

    /// Internal sync execution path for analyzed statements
    async fn execute_batch_sync_internal(
        &self,
        statements: &[AnalyzedStatement],
    ) -> Result<RowStream, Error> {
        // Determine branch for user node lookup (check for branch_override in any Query statement)
        let branch_for_lookup = statements
            .iter()
            .find_map(|stmt| {
                if let AnalyzedStatement::Query(q) = stmt {
                    q.branch_override.clone()
                } else {
                    None
                }
            })
            .unwrap_or_else(|| self.branch.clone());

        // Set function context for system functions (RAISIN_CURRENT_USER)
        if let Some(ref auth) = self.auth_context {
            tracing::info!(
                "[execute_batch_sync_internal] Setting up function context: user_id={:?}",
                auth.user_id
            );

            let user_node = if let Some(ref user_id) = auth.user_id {
                let node = self.lookup_user_node(user_id, &branch_for_lookup).await;
                tracing::info!(
                    "[execute_batch_sync_internal] lookup_user_node result for user_id={}: found={}",
                    user_id,
                    node.is_some()
                );
                node
            } else {
                tracing::warn!(
                    "[execute_batch_sync_internal] auth_context exists but user_id is None"
                );
                None
            };

            set_function_context(FunctionContext {
                user_id: auth.user_id.clone(),
                user_node: user_node.clone(),
            });

            tracing::info!(
                "[execute_batch_sync_internal] Function context set: user_id={:?}, has_node={}",
                auth.user_id,
                user_node.is_some()
            );
        } else {
            tracing::info!(
                "[execute_batch_sync_internal] No auth_context available, using default"
            );
            set_function_context(FunctionContext::default());
        }

        let mut last_result: Option<RowStream> = None;

        for (idx, analyzed) in statements.iter().enumerate() {
            tracing::debug!(
                "   Executing statement {}/{}: {:?}",
                idx + 1,
                statements.len(),
                statement_type_name(analyzed)
            );

            let result = self.execute_analyzed_statement(analyzed).await?;
            last_result = Some(result);
        }

        last_result.ok_or_else(|| Error::Validation("No statements executed".to_string()))
    }

    /// Execute an already-analyzed statement (dispatch to type-specific handlers)
    pub(crate) async fn execute_analyzed_statement(
        &self,
        analyzed: &AnalyzedStatement,
    ) -> Result<RowStream, Error> {
        match analyzed {
            AnalyzedStatement::Explain(ref explain_stmt) => {
                self.execute_explain(explain_stmt).await
            }
            AnalyzedStatement::Insert(_)
            | AnalyzedStatement::Update(_)
            | AnalyzedStatement::Delete(_)
            | AnalyzedStatement::Order(_)
            | AnalyzedStatement::Move(_)
            | AnalyzedStatement::Copy(_)
            | AnalyzedStatement::Translate(_)
            | AnalyzedStatement::Relate(_)
            | AnalyzedStatement::Unrelate(_) => self.execute_dml(analyzed).await,
            AnalyzedStatement::Restore(ref restore_stmt) => {
                self.execute_restore(restore_stmt).await
            }
            AnalyzedStatement::Ddl(ref ddl_stmt) => self.execute_ddl(ddl_stmt).await,
            AnalyzedStatement::Transaction(ref txn_stmt) => {
                self.execute_transaction(txn_stmt).await
            }
            AnalyzedStatement::Show(ref show_stmt) => self.execute_show(show_stmt).await,
            AnalyzedStatement::Branch(ref branch_stmt) => {
                self.execute_branch_statement(branch_stmt).await
            }
            AnalyzedStatement::Acl(ref acl_stmt) => self.execute_acl(acl_stmt).await,
            AnalyzedStatement::AIConfig(_) => {
                Err(Error::Validation(
                    "AI config statements are not yet supported in execution engine".to_string(),
                ))
            }
            AnalyzedStatement::Query(_) => self.execute_query(analyzed).await,
        }
    }
}

/// Check if a batch of statements requires async execution
///
/// Returns `true` if any UPDATE or DELETE has a complex WHERE clause.
pub fn batch_requires_async(statements: &[AnalyzedStatement]) -> bool {
    for stmt in statements {
        match stmt {
            AnalyzedStatement::Update(update) => {
                if matches!(classify_filter(&update.filter), FilterComplexity::Complex) {
                    tracing::debug!(
                        "Batch requires async: UPDATE with complex WHERE clause detected"
                    );
                    return true;
                }
            }
            AnalyzedStatement::Delete(delete) => {
                if matches!(classify_filter(&delete.filter), FilterComplexity::Complex) {
                    tracing::debug!(
                        "Batch requires async: DELETE with complex WHERE clause detected"
                    );
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// Helper to get a descriptive name for a statement type
fn statement_type_name(stmt: &AnalyzedStatement) -> &'static str {
    match stmt {
        AnalyzedStatement::Query(_) => "SELECT",
        AnalyzedStatement::Insert(_) => "INSERT",
        AnalyzedStatement::Update(_) => "UPDATE",
        AnalyzedStatement::Delete(_) => "DELETE",
        AnalyzedStatement::Order(_) => "ORDER",
        AnalyzedStatement::Move(_) => "MOVE",
        AnalyzedStatement::Copy(_) => "COPY",
        AnalyzedStatement::Translate(_) => "TRANSLATE",
        AnalyzedStatement::Relate(_) => "RELATE",
        AnalyzedStatement::Unrelate(_) => "UNRELATE",
        AnalyzedStatement::Explain(_) => "EXPLAIN",
        AnalyzedStatement::Ddl(_) => "DDL",
        AnalyzedStatement::Transaction(_) => "TRANSACTION",
        AnalyzedStatement::Show(_) => "SHOW",
        AnalyzedStatement::Branch(_) => "BRANCH",
        AnalyzedStatement::Restore(_) => "RESTORE",
        AnalyzedStatement::Acl(_) => "ACL",
        AnalyzedStatement::AIConfig(_) => "AI CONFIG",
    }
}
