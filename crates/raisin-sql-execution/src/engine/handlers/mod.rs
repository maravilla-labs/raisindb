//! Statement-type execution handlers
//!
//! Contains execution logic for EXPLAIN, SELECT, scalar queries, and user lookup.
//!
//! # Module Structure
//!
//! - `mutation` - DML, DDL, Transaction, and SHOW statement handlers

mod mutation;

use super::helpers;
use super::QueryEngine;
use crate::physical_plan::eval::{
    eval_expr, eval_expr_async, set_function_context, FunctionContext,
};
use crate::physical_plan::executor::{execute_plan, ExecutionContext, Row, RowStream};
use crate::physical_plan::planner::PhysicalPlanner;
use crate::physical_plan::IndexCatalog;
use futures::stream;
use indexmap::IndexMap;
use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_sql::analyzer::{
    AnalyzedQuery, AnalyzedStatement, ExplainStatement, Expr, Literal, TypedExpr,
};
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
        tracing::info!("Executing EXPLAIN query");

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
        let plan_builder = PlanBuilder::new(self.catalog.as_ref());
        let logical_plan = plan_builder
            .build(&AnalyzedStatement::Query(query.clone()))
            .map_err(|e| Error::Validation(format!("Plan error: {}", e)))?;

        let optimizer = Optimizer::default();
        let optimized_plan = optimizer.optimize(logical_plan.clone());

        let workspace = query
            .from
            .first()
            .and_then(|t| t.workspace.clone())
            .unwrap_or_else(|| "default".to_string());

        // Attach the spatial index's build state. Without it the catalog answers
        // `NotBuilt` for every property and every ST_DWITHIN degrades to a
        // row-level filter on a full scan — correct, but never fast. With it, the
        // planner can tell "indexed" from "never indexed" and only then drop the
        // predicate.
        let index_catalog: Arc<dyn IndexCatalog> = Arc::new(
            crate::physical_plan::catalog::RocksDBIndexCatalog::new()
                .with_optional_spatial_state(self.storage.spatial_state()),
        );

        let mut physical_planner = PhysicalPlanner::with_catalog(
            self.tenant_id.clone(),
            self.repo_id.clone(),
            self.branch.clone(),
            workspace,
            index_catalog,
        );

        if let Some(ref selection) = query.selection {
            let compound = match helpers::extract_node_type_from_expr(selection) {
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
        }

        // Load schema statistics for data-driven selectivity estimation
        self.apply_schema_stats(&mut physical_planner, &self.branch)
            .await;

        let physical_plan = physical_planner.plan(&optimized_plan)?;

        let mut explain_output = String::new();

        if verbose {
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

    /// Execute a SELECT query (extracted from execute() for reuse in batch)
    pub(crate) async fn execute_query(
        &self,
        analyzed: &AnalyzedStatement,
    ) -> Result<RowStream, Error> {
        if let AnalyzedStatement::Query(ref q) = analyzed {
            if q.from.is_empty() {
                if query_has_invoke_functions(q) {
                    return self.execute_scalar_query_async(q).await;
                }
                return self.execute_scalar_query(q).await;
            }
        }

        let plan_builder = PlanBuilder::new(self.catalog.as_ref());
        let logical_plan = plan_builder
            .build(analyzed)
            .map_err(|e| Error::Validation(format!("Plan error: {}", e)))?;

        let optimizer = Optimizer::default();
        let optimized = optimizer.optimize(logical_plan);

        let workspace = if let AnalyzedStatement::Query(ref q) = analyzed {
            q.from
                .first()
                .and_then(|t| t.workspace.clone())
                .unwrap_or_else(|| "default".to_string())
        } else {
            "default".to_string()
        };

        // Attach the spatial index's build state. Without it the catalog answers
        // `NotBuilt` for every property and every ST_DWITHIN degrades to a
        // row-level filter on a full scan — correct, but never fast. With it, the
        // planner can tell "indexed" from "never indexed" and only then drop the
        // predicate.
        let index_catalog: Arc<dyn IndexCatalog> = Arc::new(
            crate::physical_plan::catalog::RocksDBIndexCatalog::new()
                .with_optional_spatial_state(self.storage.spatial_state()),
        );

        let mut physical_planner = PhysicalPlanner::with_catalog(
            self.tenant_id.clone(),
            self.repo_id.clone(),
            self.branch.clone(),
            workspace.clone(),
            index_catalog,
        );

        let compound = match helpers::extract_node_type_from_analyzed(analyzed) {
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

        // Load schema statistics for data-driven selectivity estimation
        self.apply_schema_stats(&mut physical_planner, &self.branch)
            .await;

        let physical_plan = physical_planner.plan(&optimized)?;

        let (max_revision, branch_override, locales) =
            if let AnalyzedStatement::Query(ref q) = analyzed {
                (q.max_revision, q.branch_override.clone(), q.locales.clone())
            } else {
                (None, None, Vec::new())
            };

        let branch = branch_override.unwrap_or_else(|| self.branch.clone());

        let max_revision = if max_revision.is_none() {
            let branch_opt = self
                .storage
                .branches()
                .get_branch(&self.tenant_id, &self.repo_id, &branch)
                .await?;

            Some(
                branch_opt
                    .map(|b| b.head)
                    .unwrap_or_else(|| raisin_hlc::HLC::new(0, 0)),
            )
        } else {
            max_revision
        };

        let mut ctx = ExecutionContext::new(
            self.storage.clone(),
            self.tenant_id.clone(),
            self.repo_id.clone(),
            branch.clone(),
            workspace.clone(),
        );

        ctx.default_language = Arc::from(self.default_language.as_str());
        ctx = ctx.with_max_revision(max_revision);
        ctx.locales = Arc::from(locales.as_slice());

        if let Some(ref engine) = self.indexing_engine {
            ctx = ctx.with_indexing_engine(engine.clone());
        }
        if let Some(ref engine) = self.hnsw_engine {
            ctx = ctx.with_hnsw_engine(engine.clone());
        }
        if let Some(ref provider) = self.embedding_provider {
            ctx = ctx.with_embedding_provider(provider.clone());
        }
        if let Some(ref storage) = self.embedding_storage {
            ctx = ctx.with_embedding_storage(storage.clone());
        }
        if let Some(ref config) = self.repository_config {
            ctx = ctx.with_repository_config(config.clone());
        }
        if let Some(ref auth) = self.auth_context {
            ctx = ctx.with_auth_context(auth.clone());
        }
        if let Some(ref cb) = self.function_invoke {
            ctx.function_invoke = Some(cb.clone());
        }
        if let Some(ref cb) = self.function_invoke_sync {
            ctx.function_invoke_sync = Some(cb.clone());
        }
        if let Some(ref mgr) = self.lock_manager {
            ctx.lock_manager = Some(mgr.clone());
        }

        execute_plan(&physical_plan, &ctx).await
    }

    /// Execute a scalar query (SELECT without FROM clause)
    async fn execute_scalar_query(&self, query: &AnalyzedQuery) -> Result<RowStream, Error> {
        let empty_row = Row::new();
        let mut result_columns = IndexMap::new();

        for (i, (expr, alias)) in query.projection.iter().enumerate() {
            let value = eval_expr(expr, &empty_row)?;
            let col_name = alias.clone().unwrap_or_else(|| format!("column{}", i + 1));
            result_columns.insert(col_name, literal_to_property_value(value));
        }

        let row = Row::from_map(result_columns);
        Ok(Box::pin(stream::once(async move { Ok(row) })))
    }

    /// Execute a scalar query with async evaluation (for INVOKE/INVOKE_SYNC functions)
    async fn execute_scalar_query_async(&self, query: &AnalyzedQuery) -> Result<RowStream, Error> {
        let branch = self.effective_branch().await;
        let mut ctx = ExecutionContext::new(
            self.storage.clone(),
            self.tenant_id.clone(),
            self.repo_id.clone(),
            branch,
            "default".to_string(),
        );
        if let Some(ref cb) = self.function_invoke {
            ctx.function_invoke = Some(cb.clone());
        }
        if let Some(ref cb) = self.function_invoke_sync {
            ctx.function_invoke_sync = Some(cb.clone());
        }
        if let Some(ref mgr) = self.lock_manager {
            ctx.lock_manager = Some(mgr.clone());
        }
        // Propagate auth so ACL-gated functions (e.g. RAISIN_TRY_ACQUIRE) see the caller.
        if let Some(ref auth) = self.auth_context {
            ctx.auth_context = Some(auth.clone());
        }

        let empty_row = Row::new();
        let mut result_columns = IndexMap::new();

        for (i, (expr, alias)) in query.projection.iter().enumerate() {
            let value = eval_expr_async(expr, &empty_row, &ctx).await?;
            let col_name = alias.clone().unwrap_or_else(|| format!("column{}", i + 1));
            result_columns.insert(col_name, literal_to_property_value(value));
        }

        let row = Row::from_map(result_columns);
        Ok(Box::pin(stream::once(async move { Ok(row) })))
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
        let Some(auth) = self.auth_context.as_ref() else {
            set_function_context(FunctionContext::default());
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

/// Check if a query's projections contain INVOKE or INVOKE_SYNC function calls.
fn query_has_invoke_functions(q: &AnalyzedQuery) -> bool {
    q.projection
        .iter()
        .any(|(expr, _)| expr_contains_invoke(expr))
}

/// Recursively check if an expression contains INVOKE/INVOKE_SYNC.
fn expr_contains_invoke(expr: &TypedExpr) -> bool {
    match &expr.expr {
        Expr::Function { name, args, .. } => {
            matches!(
                name.to_uppercase().as_str(),
                "INVOKE"
                    | "INVOKE_SYNC"
                    | "RAISIN_TRY_ACQUIRE"
                    | "RAISIN_RELEASE"
                    | "RAISIN_RENEW"
                    | "RAISIN_CLAIM"
                    | "RAISIN_RELEASE_CLAIM"
            ) || args.iter().any(|a| expr_contains_invoke(a))
        }
        Expr::BinaryOp { left, right, .. } => {
            expr_contains_invoke(left) || expr_contains_invoke(right)
        }
        Expr::UnaryOp { expr: inner, .. } => expr_contains_invoke(inner),
        _ => false,
    }
}

/// Convert a Literal to PropertyValue (shared between scalar and async scalar paths).
fn literal_to_property_value(value: Literal) -> PropertyValue {
    match value {
        Literal::Null => PropertyValue::Null,
        Literal::Boolean(b) => PropertyValue::Boolean(b),
        Literal::Int(n) => PropertyValue::Integer(n as i64),
        Literal::BigInt(n) => PropertyValue::Integer(n),
        Literal::Double(f) => PropertyValue::Float(f),
        Literal::Text(s) | Literal::Uuid(s) | Literal::Path(s) => PropertyValue::String(s),
        Literal::JsonB(json) => match json {
            serde_json::Value::Null => PropertyValue::Null,
            serde_json::Value::Bool(b) => PropertyValue::Boolean(b),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    PropertyValue::Integer(i)
                } else if let Some(f) = n.as_f64() {
                    PropertyValue::Float(f)
                } else {
                    PropertyValue::String(n.to_string())
                }
            }
            serde_json::Value::String(s) => PropertyValue::String(s),
            serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
                PropertyValue::String(json.to_string())
            }
        },
        Literal::Timestamp(t) => PropertyValue::Date(t.into()),
        Literal::Vector(v) => PropertyValue::Vector(v),
        Literal::Geometry(geojson) => match serde_json::from_value(geojson) {
            Ok(geo) => PropertyValue::Geometry(geo),
            Err(_) => PropertyValue::Null,
        },
        Literal::Interval(_) | Literal::Parameter(_) => PropertyValue::Null,
    }
}
