//! Execution support for table-valued functions

use indexmap::IndexMap;
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::sync::Arc;

use futures::StreamExt;
use raisin_hlc::HLC;
use raisin_models::auth::AuthContext;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::nodes::Node;
use raisin_models::permissions::PermissionScope;
use raisin_storage::{NodeRepository, Storage, StorageScope};

use super::executor::{ExecutionContext, ExecutionError, Row, RowStream};
use super::operators::PhysicalPlan;
use raisin_sql::analyzer::{types::DataType, Expr, Literal, TypedExpr};

fn extract_string_literal(
    expr: &TypedExpr,
    function: &str,
    position: usize,
) -> Result<String, ExecutionError> {
    match &expr.expr {
        Expr::Literal(Literal::Text(value)) | Expr::Literal(Literal::Path(value)) => {
            Ok(value.clone())
        }
        _ => Err(ExecutionError::Validation(format!(
            "Argument {} for table function {} must be a string literal",
            position + 1,
            function
        ))),
    }
}

fn extract_json_argument(
    expr: &TypedExpr,
    function: &str,
    position: usize,
) -> Result<Option<JsonValue>, ExecutionError> {
    match &expr.expr {
        Expr::Literal(literal) => match literal {
            Literal::JsonB(value) => Ok(Some(value.clone())),
            Literal::Text(text) => {
                if text.trim().is_empty() {
                    return Ok(Some(JsonValue::String(String::new())));
                }
                serde_json::from_str(text).map(Some).map_err(|e| {
                    ExecutionError::Validation(format!(
                        "Argument {} for table function {} must be valid JSON: {}",
                        position + 1,
                        function,
                        e
                    ))
                })
            }
            Literal::Null => Ok(None),
            _ => Err(ExecutionError::Validation(format!(
                "Argument {} for table function {} must be JSONB or TEXT literal",
                position + 1,
                function
            ))),
        },
        Expr::Cast {
            expr: inner,
            target_type,
        } if *target_type.base_type() == DataType::JsonB => {
            extract_json_argument(inner, function, position)
        }
        _ => Err(ExecutionError::Validation(format!(
            "Argument {} for table function {} must be JSONB literal",
            position + 1,
            function
        ))),
    }
}

fn json_to_params_map(
    value: JsonValue,
    function: &str,
) -> Result<HashMap<String, PropertyValue>, ExecutionError> {
    match value {
        JsonValue::Object(map) => map
            .into_iter()
            .map(|(key, val)| Ok((key, json_value_to_property_value(&val))))
            .collect(),
        JsonValue::Null => Ok(HashMap::new()),
        other => Err(ExecutionError::Validation(format!(
            "Argument 2 for table function {} must be a JSON object, got {}",
            function, other
        ))),
    }
}

/// Convert PGQ SqlValue to PropertyValue
fn sql_value_to_property_value(value: crate::physical_plan::pgq::SqlValue) -> PropertyValue {
    use crate::physical_plan::pgq::SqlValue;
    match value {
        SqlValue::Null => PropertyValue::Null,
        SqlValue::Boolean(b) => PropertyValue::Boolean(b),
        SqlValue::Integer(i) => PropertyValue::Integer(i),
        SqlValue::Float(f) => PropertyValue::Float(f),
        SqlValue::String(s) => PropertyValue::String(s),
        SqlValue::Array(arr) => {
            PropertyValue::Array(arr.into_iter().map(sql_value_to_property_value).collect())
        }
        SqlValue::Json(json) => json_value_to_property_value(&json),
    }
}

fn json_value_to_property_value(value: &JsonValue) -> PropertyValue {
    match value {
        JsonValue::Null => PropertyValue::Null,
        JsonValue::Bool(b) => PropertyValue::Boolean(*b),
        JsonValue::Number(num) => {
            if let Some(i) = num.as_i64() {
                PropertyValue::Integer(i)
            } else if let Some(u) = num.as_u64() {
                PropertyValue::Float(u as f64)
            } else if let Some(f) = num.as_f64() {
                PropertyValue::Float(f)
            } else {
                PropertyValue::Null
            }
        }
        JsonValue::String(s) => PropertyValue::String(s.clone()),
        JsonValue::Array(items) => {
            PropertyValue::Array(items.iter().map(json_value_to_property_value).collect())
        }
        JsonValue::Object(map) => {
            let mut obj = HashMap::new();
            for (key, val) in map.iter() {
                obj.insert(key.clone(), json_value_to_property_value(val));
            }
            PropertyValue::Object(obj)
        }
    }
}

/// Execute a table-valued function and return a row stream
pub async fn execute_table_function<S: Storage + 'static>(
    plan: &PhysicalPlan,
    ctx: &ExecutionContext<S>,
) -> Result<RowStream, ExecutionError> {
    match plan {
        PhysicalPlan::TableFunction {
            name,
            alias,
            args,
            schema,
            workspace,
            branch_override,
            max_revision,
            filter,
            ..
        } => {
            let function_name = name.clone();
            let args = args.clone();
            let schema = Arc::clone(schema);
            let workspace_override = workspace.clone();
            let branch_override = branch_override.clone();
            let max_revision = *max_revision;

            if function_name.eq_ignore_ascii_case("CYPHER") {
                // Withdrawn from SQL.
                //
                // The Cypher executor applies NO row-level security — it has no
                // auth context at all — so `SELECT * FROM CYPHER('...')` was a
                // complete RLS bypass for any caller who could issue SQL. It is
                // also already deprecated in favour of GRAPH_TABLE, which is
                // strictly more capable, so there is nothing to preserve at the
                // SQL surface.
                //
                // The `physical_plan::cypher` module deliberately stays: its
                // graph algorithms and matching code are reused elsewhere and
                // are wanted for future work. Only this entrypoint is closed, so
                // GRAPH_TABLE is the one graph query language reachable from SQL
                // and there is exactly ONE surface to enforce RLS on.
                return Err(ExecutionError::Validation(
                    "CYPHER(...) is no longer available from SQL. Use GRAPH_TABLE, \
                     which supports the same patterns and enforces row-level \
                     security. See the raisindb-sql skill, § GRAPH_TABLE."
                        .to_string(),
                ));
            }

            if function_name.eq_ignore_ascii_case("GRAPH_TABLE") {
                tracing::info!("🔵 GRAPH_TABLE table function invoked");

                // Extract PGQ query string argument (required)
                let query_expr = args.first().ok_or_else(|| {
                    ExecutionError::Validation(
                        "GRAPH_TABLE table function expects at least one argument".to_string(),
                    )
                })?;

                let pgq_query_str = extract_string_literal(&query_expr.value, &function_name, 0)?;
                tracing::debug!("   PGQ query string: {}", pgq_query_str);

                // Parse the PGQ query from the string argument
                // The preprocessing step wrapped the original GRAPH_TABLE content as a string literal
                let pgq_query = raisin_sql::ast::pgq_parser::parse_graph_table(&pgq_query_str)
                    .map_err(|e| {
                        ExecutionError::Validation(format!(
                            "Failed to parse GRAPH_TABLE query: {}",
                            e
                        ))
                    })?;

                let workspace_name =
                    workspace_override.unwrap_or_else(|| ctx.workspace.to_string());
                let branch = branch_override.unwrap_or_else(|| ctx.branch.to_string());
                let revision = max_revision.or(ctx.max_revision);

                tracing::debug!(
                    "   Context: workspace={}, branch={}, revision={:?}",
                    workspace_name,
                    branch,
                    revision
                );

                let storage = ctx.storage.clone();
                let tenant_id = ctx.tenant_id.to_string();
                let repo_id = ctx.repo_id.to_string();

                // Execute the PGQ query
                let results = crate::physical_plan::pgq::execute_graph_table(
                    storage,
                    workspace_name,
                    tenant_id,
                    repo_id,
                    branch,
                    revision,
                    // Row-level security: the projected endpoints are filtered
                    // with the caller's identity. `None` (system caller) stays
                    // unfiltered, matching the scan executors.
                    ctx.auth_context.clone(),
                    pgq_query,
                )
                .await?;

                // Use alias if provided, otherwise use "graph_table"
                let table_name = alias.clone().unwrap_or_else(|| "graph_table".to_string());

                // Convert PgqRow results to Row stream
                let rows: Vec<Result<Row, ExecutionError>> = results
                    .into_iter()
                    .map(|pgq_row| {
                        let mut columns: IndexMap<String, PropertyValue> = IndexMap::new();

                        for (col_name, sql_value) in pgq_row.iter() {
                            let qualified_name = format!("{}.{}", table_name, col_name);
                            let property_value = sql_value_to_property_value(sql_value.clone());
                            columns.insert(qualified_name, property_value);
                        }

                        Ok(Row::from_map(columns))
                    })
                    .collect();

                let row_stream = futures::stream::iter(rows);
                Ok(Box::pin(row_stream))
            } else if let Some(search_function) =
                crate::physical_plan::search::args::SearchFunction::from_name(&function_name)
            {
                // FULLTEXT_SEARCH, HYBRID_SEARCH and KNN are ONE implementation.
                //
                // They used to be two hand-written fetch loops over two index
                // legs plus a KNN arm that did not exist at all (every
                // `SELECT * FROM KNN(...)` died with "Unsupported table
                // function" while the keyword help shipped a worked example).
                // The two that did exist had already drifted twice: the hybrid
                // leg hard-coded `language: "en"` while the standalone function
                // took the language as an argument, and the hybrid leg fetched
                // its hits from a hard-coded workspace. Both are the same bug
                // shape, so there is now one loop, one scope resolver, one RLS
                // pass and one column set.
                let table_name = alias
                    .clone()
                    .unwrap_or_else(|| function_name.to_lowercase());
                crate::physical_plan::search::emit::execute_search(
                    search_function,
                    &args,
                    // ADVISORY. The authoritative `Filter` above this node is
                    // what makes the query correct; this copy exists so the
                    // legs can be asked for a narrower candidate pool and so a
                    // row dropped by a predicate counts against `limit` in the
                    // same loop that counts a row dropped by permissions.
                    filter.as_ref(),
                    table_name,
                    ctx,
                )
                .await
            } else {
                Err(ExecutionError::Validation(format!(
                    "Unsupported table function: {}",
                    function_name
                )))
            }
        }
        _ => Err(ExecutionError::Validation(
            "Invalid plan variant for table function execution".to_string(),
        )),
    }
}
