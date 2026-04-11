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

        let plan_builder = PlanBuilder::new(self.catalog.as_ref());
        let logical_plan = plan_builder
            .build(&AnalyzedStatement::Query((*explain_stmt.query).clone()))
            .map_err(|e| Error::Validation(format!("Plan error: {}", e)))?;

        let optimizer = Optimizer::default();
        let optimized_plan = optimizer.optimize(logical_plan.clone());

        let workspace = explain_stmt
            .query
            .from
            .first()
            .and_then(|t| t.workspace.clone())
            .unwrap_or_else(|| "default".to_string());

        let index_catalog: Arc<dyn IndexCatalog> =
            Arc::new(crate::physical_plan::catalog::RocksDBIndexCatalog::new());

        let mut physical_planner = PhysicalPlanner::with_catalog(
            self.tenant_id.clone(),
            self.repo_id.clone(),
            self.branch.clone(),
            workspace,
            index_catalog,
        );

        if let Some(ref selection) = explain_stmt.query.selection {
            if let Some(node_type_name) = helpers::extract_node_type_from_expr(selection) {
                if let Some(indexes) = helpers::load_compound_indexes(
                    &*self.storage,
                    &self.tenant_id,
                    &self.repo_id,
                    &self.branch,
                    &node_type_name,
                )
                .await
                {
                    physical_planner.set_compound_indexes(indexes);
                }
            }
        }

        // Load schema statistics for data-driven selectivity estimation
        self.apply_schema_stats(&mut physical_planner, &self.branch)
            .await;

        let physical_plan = physical_planner.plan(&optimized_plan)?;

        let mut explain_output = String::new();

        if explain_stmt.verbose {
            explain_output.push_str("=== Logical Plan ===\n");
            explain_output.push_str(&logical_plan.explain());
            explain_output.push_str("\n\n");

            explain_output.push_str("=== Optimized Logical Plan ===\n");
            explain_output.push_str(&optimized_plan.explain());
            explain_output.push_str("\n\n");
        }

        explain_output.push_str("=== Physical Execution Plan ===\n");
        explain_output.push_str(&physical_plan.explain());

        let mut row = Row::new();
        row.columns.insert(
            "QUERY PLAN".to_string(),
            PropertyValue::String(explain_output),
        );

        Ok(Box::pin(stream::iter(vec![Ok(row)])))
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

        let index_catalog: Arc<dyn IndexCatalog> =
            Arc::new(crate::physical_plan::catalog::RocksDBIndexCatalog::new());

        let mut physical_planner = PhysicalPlanner::with_catalog(
            self.tenant_id.clone(),
            self.repo_id.clone(),
            self.branch.clone(),
            workspace.clone(),
            index_catalog,
        );

        if let Some(node_type_name) = helpers::extract_node_type_from_analyzed(analyzed) {
            if let Some(indexes) = helpers::load_compound_indexes(
                &*self.storage,
                &self.tenant_id,
                &self.repo_id,
                &self.branch,
                &node_type_name,
            )
            .await
            {
                physical_planner.set_compound_indexes(indexes);
            }
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

    /// Look up the user node from the property index for CURRENT_USER()
    pub(crate) async fn lookup_user_node(
        &self,
        user_id: &str,
        branch: &str,
    ) -> Option<serde_json::Value> {
        let workspace = "raisin:access_control";
        tracing::info!(
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
                tracing::info!(
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
                tracing::info!("[lookup_user_node] Found node_id: {}", id);
                id
            }
            None => {
                tracing::error!(
                    "[lookup_user_node] No nodes found with user_id={} in workspace={}",
                    user_id,
                    workspace
                );
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
                tracing::info!("[lookup_user_node] Successfully loaded user node");
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
                tracing::info!(
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
            matches!(name.to_uppercase().as_str(), "INVOKE" | "INVOKE_SYNC")
                || args.iter().any(|a| expr_contains_invoke(a))
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
