//! Statement-type execution handlers
//!
//! Contains execution logic for EXPLAIN, SELECT, scalar queries, and user lookup.
//!
//! # Module Structure
//!
//! - `mutation` - DML, DDL, Transaction, and SHOW statement handlers
//! - `scalar` - SELECT without FROM

mod mutation;
mod scalar;

use super::helpers;
use super::QueryEngine;
use crate::physical_plan::eval::{set_function_context, FunctionContext};
use crate::physical_plan::executor::{execute_plan, ExecutionContext, Row, RowStream};
use crate::physical_plan::planner::PhysicalPlanner;
use crate::physical_plan::IndexCatalog;
use futures::stream;
use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_sql::analyzer::{AnalyzedQuery, AnalyzedStatement, ExplainStatement, TypedExpr};
use raisin_sql::logical_plan::PlanBuilder;
use raisin_sql::optimizer::Optimizer;
use raisin_storage::{
    BranchRepository, NodeRepository, PropertyIndexRepository, Storage, StorageScope,
};
use std::sync::Arc;

impl<S: Storage + raisin_storage::transactional::TransactionalStorage + 'static> QueryEngine<S> {
    /// Execute an EXPLAIN statement and return the query plan as a result stream
    pub(crate) async fn execute_explain(
        &self,
        explain_stmt: &ExplainStatement,
    ) -> Result<RowStream, Error> {
        tracing::debug!("Executing EXPLAIN query");

        match explain_stmt.target.as_ref() {
            AnalyzedStatement::Query(query) => {
                self.explain_query(query, explain_stmt.verbose).await
            }
            AnalyzedStatement::Update(update) => {
                self.explain_dml(
                    "UPDATE",
                    &update.target,
                    update.filter.as_ref(),
                    explain_stmt.verbose,
                )
                .await
            }
            AnalyzedStatement::Delete(delete) => {
                self.explain_dml(
                    "DELETE",
                    &delete.target,
                    delete.filter.as_ref(),
                    explain_stmt.verbose,
                )
                .await
            }
            _ => Err(Error::Validation(
                "EXPLAIN only supports SELECT, UPDATE, and DELETE statements".to_string(),
            )),
        }
    }

    /// EXPLAIN for a SELECT query.
    async fn explain_query(
        &self,
        query: &AnalyzedQuery,
        verbose: bool,
    ) -> Result<RowStream, Error> {
        // The same planner call execution makes (`physical_plan_for`), so
        // EXPLAIN shows the plan the query runs.
        let statement = AnalyzedStatement::Query(query.clone());
        let optimized_plan = super::prepared::logical_plan(&self.catalog, &statement)?;
        let physical_plan = self
            .physical_plan_for(&statement, Some(&optimized_plan))
            .await?;

        let mut explain_output = String::new();

        if verbose {
            let logical_plan = PlanBuilder::new(self.catalog.as_ref())
                .build(&statement)
                .map_err(|e| Error::Validation(format!("Plan error: {}", e)))?;
            explain_output.push_str("=== Logical Plan ===\n");
            explain_output.push_str(&logical_plan.explain());
            explain_output.push_str("\n\n");

            explain_output.push_str("=== Optimized Logical Plan ===\n");
            explain_output.push_str(&optimized_plan.explain());
            explain_output.push_str("\n\n");
        }

        explain_output.push_str("=== Physical Execution Plan ===\n");
        explain_output.push_str(&physical_plan.explain());

        Ok(Self::explain_row_stream(explain_output))
    }

    /// EXPLAIN for UPDATE / DELETE — shows the actual row-matching strategy the
    /// DML executor will use: the id/path point-lookup fast path, or the bulk
    /// path's `SELECT id` physical plan (with compound indexes loaded, exactly
    /// as `find_matching_node_ids` plans it).
    async fn explain_dml(
        &self,
        op: &str,
        target: &raisin_sql::analyzer::DmlTableTarget,
        filter: Option<&TypedExpr>,
        verbose: bool,
    ) -> Result<RowStream, Error> {
        use crate::physical_plan::dml_executor::node_helpers::extract_node_identifier_from_filter;
        use raisin_sql::analyzer::DmlTableTarget;

        let workspace = match target {
            DmlTableTarget::Workspace(name) => name.clone(),
            DmlTableTarget::SchemaTable(kind) => {
                let out = format!(
                    "=== {} Plan ===\nSchemaTableWrite: {} (direct schema mutation, no scan)",
                    op,
                    kind.table_name()
                );
                return Ok(Self::explain_row_stream(out));
            }
        };

        // Fast path: WHERE id = '...' / path = '...' → a single point lookup.
        let owned_filter = filter.cloned();
        if let Ok(ident) = extract_node_identifier_from_filter(&owned_filter) {
            let desc = match ident {
                crate::physical_plan::dml_executor::node_helpers::NodeIdentifier::Id(id) => {
                    format!("NodeIdLookup: id='{}' (O(1) point write)", id)
                }
                crate::physical_plan::dml_executor::node_helpers::NodeIdentifier::Path(p) => {
                    format!("PathIndexLookup: path='{}' (O(1) point write)", p)
                }
            };
            let out = format!(
                "=== {} Plan ===\nTarget workspace: {}\nStrategy: fast path\n{}",
                op, workspace, desc
            );
            return Ok(Self::explain_row_stream(out));
        }

        let Some(filter) = filter else {
            let out = format!(
                "=== {} Plan ===\nTarget workspace: {}\nStrategy: full workspace {} (no WHERE clause)",
                op, workspace, op
            );
            return Ok(Self::explain_row_stream(out));
        };

        // Bulk path: plan the same `SELECT id FROM ws WHERE <filter>` the DML
        // executor runs, with compound indexes loaded like find_matching_node_ids.
        let analyzed_query = AnalyzedQuery {
            ctes: vec![],
            projection: vec![(
                TypedExpr::column(
                    workspace.clone(),
                    "id".to_string(),
                    raisin_sql::analyzer::DataType::Text,
                ),
                None,
            )],
            from: vec![raisin_sql::analyzer::TableRef {
                table: workspace.clone(),
                alias: None,
                workspace: Some(workspace.clone()),
                table_function: None,
                subquery: None,
                lateral_function: None,
            }],
            joins: vec![],
            selection: Some(filter.clone()),
            group_by: vec![],
            aggregates: vec![],
            order_by: vec![],
            limit: None,
            offset: None,
            max_revision: None,
            branch_override: None,
            locales: vec![],
            distinct: None,
            having: None,
            set_operation: None,
        };

        let mut catalog = raisin_sql::StaticCatalog::default_nodes_schema();
        catalog.register_workspace(workspace.clone());
        let plan_builder = PlanBuilder::new(&catalog);
        let logical_plan = plan_builder
            .build(&AnalyzedStatement::Query(analyzed_query))
            .map_err(|e| Error::Validation(format!("Plan error: {}", e)))?;

        let optimizer = Optimizer::default();
        let optimized_plan = optimizer.optimize(logical_plan.clone());

        let mut physical_planner = PhysicalPlanner::with_context(
            self.tenant_id.clone(),
            self.repo_id.clone(),
            self.branch.clone(),
            workspace.clone(),
        );

        let compound = match helpers::extract_node_type_from_expr(filter) {
            Some(node_type_name) => {
                helpers::load_compound_indexes(
                    &*self.storage,
                    &self.tenant_id,
                    &self.repo_id,
                    &self.branch,
                    &node_type_name,
                )
                .await
            }
            // No `node_type =` in the WHERE clause. A hierarchy query is
            // usually written without one, so fall back to every compound
            // index on the branch rather than planning as if none existed.
            None => {
                helpers::load_all_compound_indexes(
                    &*self.storage,
                    &self.tenant_id,
                    &self.repo_id,
                    &self.branch,
                )
                .await
            }
        };
        if let Some(indexes) = compound {
            physical_planner.set_compound_indexes(indexes);
        }

        let physical_plan = physical_planner.plan(&optimized_plan)?;

        let mut out = format!(
            "=== {} Plan ===\nTarget workspace: {}\nStrategy: bulk (match ids via SELECT, then write per id)\n",
            op, workspace
        );
        if verbose {
            out.push_str("\n=== Matching Logical Plan ===\n");
            out.push_str(&logical_plan.explain());
            out.push('\n');
        }
        out.push_str("\n=== Matching Physical Plan ===\n");
        out.push_str(&physical_plan.explain());

        Ok(Self::explain_row_stream(out))
    }

    /// Wrap EXPLAIN text output into a single-row stream.
    fn explain_row_stream(explain_output: String) -> RowStream {
        let mut row = Row::new();
        row.columns.insert(
            "QUERY PLAN".to_string(),
            PropertyValue::String(explain_output),
        );
        Box::pin(stream::iter(vec![Ok(row)]))
    }

    /// Install the thread-local function context for `RAISIN_CURRENT_USER()`.
    ///
    /// The ONLY consumer of `FunctionContext` is `RaisinCurrentUserFunction`, and
    /// it reads `user_node` alone — so resolving the node is pure waste unless the
    /// statement actually calls that function. Resolving it costs a property-index
    /// scan, a node read and a full serialization, on every authenticated
    /// statement in the product.
    ///
    /// The gate is a substring test over the raw SQL: the parser cannot produce a
    /// call without the identifier appearing verbatim, so there are no false
    /// negatives. A false positive (the name inside a string literal) merely does
    /// the old work.
    ///
    /// Call this from EVERY path that sets the function context — `execute` and
    /// the batch path had verbatim copies of this logic before.
    pub(crate) async fn install_function_context(&self, sql: &str, branch: &str) {
        // Not a secret and not per user: every caller may learn the distance
        // its own vector queries are cut at (`EMBEDDING_MAX_DISTANCE()`).
        let default_max_distance = self.tenant_default_max_distance();
        let Some(auth) = self.auth_context.as_ref() else {
            set_function_context(FunctionContext {
                default_max_distance,
                ..FunctionContext::default()
            });
            return;
        };

        let user_node = match auth.user_id.as_ref() {
            Some(user_id) if sql_may_call_current_user(sql) => {
                self.lookup_user_node(user_id, branch).await
            }
            _ => None,
        };

        set_function_context(FunctionContext {
            user_id: auth.user_id.clone(),
            user_node,
            default_max_distance,
        });
    }

    /// Look up the user node from the property index for `RAISIN_CURRENT_USER()`
    pub(crate) async fn lookup_user_node(
        &self,
        user_id: &str,
        branch: &str,
    ) -> Option<serde_json::Value> {
        let workspace = "raisin:access_control";
        tracing::debug!(
            "[lookup_user_node] Looking up user: user_id={}, workspace={}, branch={}",
            user_id,
            workspace,
            branch
        );

        let property_value = PropertyValue::String(user_id.to_string());

        // 1. Query property index to find node_id by user_id
        let node_ids = match self
            .storage
            .property_index()
            .find_by_property(
                StorageScope::new(&self.tenant_id, &self.repo_id, branch, workspace),
                "user_id",
                &property_value,
                false,
                // HEAD: the caller's user node as it is now.
                None,
            )
            .await
        {
            Ok(ids) => {
                tracing::debug!(
                    "[lookup_user_node] Property index query returned {} nodes",
                    ids.len()
                );
                ids
            }
            Err(e) => {
                tracing::error!(
                    "[lookup_user_node] Property index query failed for user_id={}: {}",
                    user_id,
                    e
                );
                return None;
            }
        };

        let node_id = match node_ids.first() {
            Some(id) => {
                tracing::debug!("[lookup_user_node] Found node_id: {}", id);
                id
            }
            None => {
                // Expected for internal actors ("system", jobs) that have no
                // user node — don't spam ERROR for those (~50k/day on a busy
                // scheduler); a real user missing their node is still notable.
                if user_id == "system" {
                    tracing::debug!(
                        "[lookup_user_node] No nodes found with user_id={} in workspace={}",
                        user_id,
                        workspace
                    );
                } else {
                    tracing::warn!(
                        "[lookup_user_node] No nodes found with user_id={} in workspace={}",
                        user_id,
                        workspace
                    );
                }
                return None;
            }
        };

        // 2. Load the full node from storage
        let node = match self
            .storage
            .nodes()
            .get(
                StorageScope::new(&self.tenant_id, &self.repo_id, branch, workspace),
                node_id,
                None,
            )
            .await
        {
            Ok(Some(n)) => {
                tracing::debug!("[lookup_user_node] Successfully loaded user node");
                n
            }
            Ok(None) => {
                tracing::error!(
                    "[lookup_user_node] Node {} exists in index but not in storage",
                    node_id
                );
                return None;
            }
            Err(e) => {
                tracing::error!("[lookup_user_node] Failed to load node {}: {}", node_id, e);
                return None;
            }
        };

        match serde_json::to_value(&node) {
            Ok(value) => {
                tracing::debug!(
                    "[lookup_user_node] Successfully serialized user node, path={:?}",
                    value.get("path")
                );
                Some(value)
            }
            Err(e) => {
                tracing::error!("[lookup_user_node] Failed to serialize node: {}", e);
                None
            }
        }
    }
}

/// Could this SQL text possibly call `RAISIN_CURRENT_USER()`?
///
/// Conservative by construction: the parser cannot emit that call unless the
/// identifier appears verbatim in the source, so a `false` here is proof the
/// function is unreachable. A `true` on a mere mention (a string literal, a
/// column comment) just falls back to the old behaviour.
pub(crate) fn sql_may_call_current_user(sql: &str) -> bool {
    const NEEDLE: &[u8] = b"RAISIN_CURRENT_USER";

    // Case-insensitive substring search without allocating an uppercased copy of
    // the whole statement — this runs on every query.
    sql.as_bytes()
        .windows(NEEDLE.len())
        .any(|w| w.eq_ignore_ascii_case(NEEDLE))
}

#[cfg(test)]
mod gate_tests {
    use super::sql_may_call_current_user;

    #[test]
    fn detects_the_call_in_any_case() {
        assert!(sql_may_call_current_user("SELECT RAISIN_CURRENT_USER()"));
        assert!(sql_may_call_current_user("select raisin_current_user()"));
        assert!(sql_may_call_current_user("SELECT Raisin_Current_User()"));
        assert!(sql_may_call_current_user(
            "SELECT RAISIN_CURRENT_USER()->>'path' AS p FROM 'ws'"
        ));
    }

    #[test]
    fn skips_ordinary_statements() {
        assert!(!sql_may_call_current_user("SELECT * FROM 'ws'"));
        assert!(!sql_may_call_current_user(
            "UPDATE 'ws' SET properties = $1::jsonb WHERE path = $2"
        ));
        // CURRENT_USER is a different, unrelated function — it reads no context.
        assert!(!sql_may_call_current_user("SELECT CURRENT_USER"));
        // Near-misses must not trip the gate.
        assert!(!sql_may_call_current_user("SELECT raisin_current_use()"));
    }

    #[test]
    fn errs_toward_doing_the_work() {
        // A mere mention is a false positive: wasteful, never wrong.
        assert!(sql_may_call_current_user(
            "SELECT 'RAISIN_CURRENT_USER' AS note"
        ));
    }

    #[test]
    fn handles_input_shorter_than_the_needle() {
        assert!(!sql_may_call_current_user(""));
        assert!(!sql_may_call_current_user("SELECT 1"));
    }
}
