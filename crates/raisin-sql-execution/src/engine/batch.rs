//! Batch SQL execution with async routing.
//!
//! Supports executing multiple SQL statements in sequence,
//! with automatic routing of complex WHERE clauses to background jobs.

use super::QueryEngine;
use crate::physical_plan::dml_executor::{classify_filter, FilterComplexity};
use crate::physical_plan::executor::{Row, RowStream};
use futures::stream;
use indexmap::IndexMap;
use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_sql::analyzer::AnalyzedStatement;
use raisin_storage::Storage;

impl<S: Storage + raisin_storage::transactional::TransactionalStorage + 'static> QueryEngine<S> {
    /// Execute multiple SQL statements in a batch with automatic async routing
    ///
    /// If a job registrar is configured and the batch contains complex WHERE clauses,
    /// the batch is routed to async job execution and returns a single row with
    /// `job_id`, `status`, `message` columns.
    pub async fn execute_batch(&self, sql: &str) -> Result<RowStream, Error> {
        tracing::debug!("SQL Query Engine starting batch execution");

        // 1. Analyze all statements (a one-statement batch from the
        // prepared-statement cache when it can; see `prepared.rs`).
        let statements = super::prepared::prepare_batch(&self.catalog, sql)?;
        self.execute_batch_prepared(sql, statements).await
    }

    /// [`Self::execute_batch`] with `$1`, `$2`, … bound to `params`, rendered
    /// by `format` — see [`Self::execute_with_params`]. The HTTP, WS and
    /// pgwire SQL endpoints call this, so a parameterized statement is
    /// planned once for all its values.
    pub async fn execute_batch_with_params(
        &self,
        sql: &str,
        params: &[serde_json::Value],
        format: &super::ParamFormat,
    ) -> Result<RowStream, Error> {
        let bound =
            super::prepared_params::prepare_with_params(&self.catalog, true, sql, params, format)?;
        self.execute_batch_prepared(&bound.sql, bound.statements)
            .await
    }

    /// Steps 2-3 of [`Self::execute_batch`]: async routing, then execution.
    async fn execute_batch_prepared(
        &self,
        sql: &str,
        statements: Vec<std::sync::Arc<super::prepared::Prepared>>,
    ) -> Result<RowStream, Error> {
        if statements.is_empty() {
            return Err(Error::Validation(
                "No valid statements to execute".to_string(),
            ));
        }

        // 2. Check if async routing is needed
        if let Some(ref registrar) = self.job_registrar {
            if requires_async(statements.iter().map(|p| &p.analyzed)) {
                tracing::debug!("Batch requires async execution (complex WHERE clause detected)");

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
                        "Bulk operation started. Poll GET /management/jobs/{job_id} for status."
                            .to_string(),
                    ),
                );

                let row = Row { columns };
                return Ok(Box::pin(stream::iter(vec![Ok(row)])));
            }
        }

        tracing::debug!("Executing {} statements in batch (sync)", statements.len());

        // 3. Execute each statement sequentially (sync path)
        self.execute_batch_sync_internal(sql, &statements).await
    }

    /// Execute a batch synchronously (force sync, no async routing)
    pub async fn execute_batch_sync(&self, sql: &str) -> Result<RowStream, Error> {
        self.execute_batch_sync_traced(sql)
            .await
            .map(|(stream, _)| stream)
    }

    /// [`execute_batch_sync`](Self::execute_batch_sync), also saying whether
    /// the batch was answered from the prepared-statement cache — for tests.
    #[doc(hidden)]
    pub async fn execute_batch_sync_traced(&self, sql: &str) -> Result<(RowStream, bool), Error> {
        tracing::debug!("SQL Query Engine starting batch execution (forced sync)");

        let (statements, cached) = super::prepared::prepare_batch_traced(&self.catalog, sql)?;

        if statements.is_empty() {
            return Err(Error::Validation(
                "No valid statements to execute".to_string(),
            ));
        }

        let stream = self.execute_batch_sync_internal(sql, &statements).await?;
        Ok((stream, cached))
    }

    /// Internal sync execution path for analyzed statements
    async fn execute_batch_sync_internal(
        &self,
        sql: &str,
        statements: &[std::sync::Arc<super::prepared::Prepared>],
    ) -> Result<RowStream, Error> {
        // Determine branch for user node lookup (check for branch_override in any Query statement)
        let branch_for_lookup = statements
            .iter()
            .find_map(|stmt| {
                if let AnalyzedStatement::Query(q) = &stmt.analyzed {
                    q.branch_override.clone()
                } else {
                    None
                }
            })
            .unwrap_or_else(|| self.branch.clone());

        // Set function context for system functions (RAISIN_CURRENT_USER).
        // Resolves the user node only when the SQL can actually call it.
        self.install_function_context(sql, &branch_for_lookup).await;

        let mut last_result: Option<RowStream> = None;

        for (idx, prepared) in statements.iter().enumerate() {
            let analyzed = &prepared.analyzed;
            tracing::debug!(
                "   Executing statement {}/{}: {:?}",
                idx + 1,
                statements.len(),
                statement_type_name(analyzed)
            );

            let result = if super::subquery_bind::statement_needs_binding(analyzed) {
                let bound = self.bind_subqueries(analyzed.clone()).await?;
                self.execute_analyzed_statement(&bound).await?
            } else if let AnalyzedStatement::Query(_) = analyzed {
                // The prepared logical (and physical) plan, when the
                // statement came from the cache; planned here otherwise.
                self.execute_query(analyzed, Some(prepared.as_ref()))
                    .await?
            } else {
                self.execute_analyzed_statement(analyzed).await?
            };
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
            AnalyzedStatement::AIConfig(ref stmt) => self.execute_ai_config(stmt).await,
            AnalyzedStatement::SpatialAdmin(ref stmt) => self.execute_spatial_admin(stmt).await,
            AnalyzedStatement::Query(_) => self.execute_query(analyzed, None).await,
        }
    }
}

/// Check if a batch of statements requires async execution
///
/// Returns `true` if any UPDATE or DELETE has a complex WHERE clause.
pub fn batch_requires_async(statements: &[AnalyzedStatement]) -> bool {
    requires_async(statements)
}

fn requires_async<'a>(statements: impl IntoIterator<Item = &'a AnalyzedStatement>) -> bool {
    for stmt in statements {
        match stmt {
            // A SCHEMA-TABLE write names ONE definition (`WHERE name = '…'`),
            // which the filter classifier counts as complex because it is not
            // an id or a path. Run as a job, the caller got "accepted" instead
            // of the write's own answer — including a refusal — and a refused
            // write was retried three times.
            AnalyzedStatement::Update(update)
                if matches!(
                    update.target,
                    raisin_sql::analyzer::DmlTableTarget::SchemaTable(_)
                ) => {}
            AnalyzedStatement::Delete(delete)
                if matches!(
                    delete.target,
                    raisin_sql::analyzer::DmlTableTarget::SchemaTable(_)
                ) => {}
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
        AnalyzedStatement::SpatialAdmin(_) => "SPATIAL ADMIN",
    }
}
