//! Execution support for table-valued functions

use indexmap::IndexMap;
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::sync::Arc;

use futures::StreamExt;
use raisin_models::nodes::properties::PropertyValue;
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
            ..
        } => {
            let function_name = name.clone();
            let args = args.clone();
            let schema = Arc::clone(schema);
            let workspace_override = workspace.clone();
            let branch_override = branch_override.clone();
            let max_revision = *max_revision;

            if function_name.eq_ignore_ascii_case("CYPHER") {
                tracing::info!("🔵 CYPHER table function invoked");

                let query_expr = args.first().ok_or_else(|| {
                    ExecutionError::Validation(
                        "CYPHER table function expects at least one argument".to_string(),
                    )
                })?;

                let cypher_query = extract_string_literal(query_expr, &function_name, 0)?;
                tracing::debug!("   Cypher query: {}", cypher_query);

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

                let param_map = if let Some(params_expr) = args.get(1) {
                    let json_value = extract_json_argument(params_expr, &function_name, 1)?;
                    if let Some(json_value) = json_value {
                        json_to_params_map(json_value, &function_name)?
                    } else {
                        HashMap::new()
                    }
                } else {
                    HashMap::new()
                };

                let cypher_stream = crate::physical_plan::cypher::execute_cypher(
                    storage,
                    workspace_name,
                    tenant_id,
                    repo_id,
                    branch,
                    revision,
                    &cypher_query,
                    param_map,
                );

                let output_schema = Arc::clone(&schema);
                // Use alias if provided, otherwise use function name (e.g., "cypher")
                let table_name = alias
                    .clone()
                    .unwrap_or_else(|| function_name.to_lowercase());

                let row_stream = cypher_stream.map(move |row_result| {
                    row_result.map(|cypher_row| {
                        tracing::debug!(
                            "   📋 Converting CypherRow: {} columns, {} values",
                            cypher_row.columns.len(),
                            cypher_row.values.len()
                        );
                        tracing::debug!("      Columns: {:?}", cypher_row.columns);

                        let mut columns: IndexMap<String, PropertyValue> = IndexMap::new();

                        if !cypher_row.columns.is_empty() {
                            for (idx, column_name) in cypher_row.columns.iter().enumerate() {
                                if let Some(value) = cypher_row.values.get(idx) {
                                    // Use table-qualified name: "cypher.result"
                                    let qualified_name = format!("{}.{}", table_name, column_name);
                                    tracing::debug!(
                                        "      Inserting column '{}' with value type: {:?}",
                                        qualified_name,
                                        std::mem::discriminant(value)
                                    );
                                    columns.insert(qualified_name, value.clone());
                                }
                            }
                        } else {
                            for (idx, column_def) in output_schema.columns.iter().enumerate() {
                                if let Some(value) = cypher_row.values.get(idx) {
                                    let qualified_name =
                                        format!("{}.{}", table_name, column_def.name);
                                    columns.insert(qualified_name, value.clone());
                                }
                            }

                            if columns.is_empty()
                                && !output_schema.columns.is_empty()
                                && !cypher_row.values.is_empty()
                            {
                                let qualified_name =
                                    format!("{}.{}", table_name, output_schema.columns[0].name);
                                columns.insert(qualified_name, cypher_row.values[0].clone());
                            }
                        }

                        tracing::debug!(
                            "      Created Row with {} columns: {:?}",
                            columns.len(),
                            columns.keys().collect::<Vec<_>>()
                        );
                        Row::from_map(columns)
                    })
                });

                Ok(Box::pin(row_stream))
            } else if function_name.eq_ignore_ascii_case("GRAPH_TABLE") {
                tracing::info!("🔵 GRAPH_TABLE table function invoked");

                // Extract PGQ query string argument (required)
                let query_expr = args.first().ok_or_else(|| {
                    ExecutionError::Validation(
                        "GRAPH_TABLE table function expects at least one argument".to_string(),
                    )
                })?;

                let pgq_query_str = extract_string_literal(query_expr, &function_name, 0)?;
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
            } else if function_name.eq_ignore_ascii_case("FULLTEXT_SEARCH") {
                tracing::info!("🔍 FULLTEXT_SEARCH table function invoked");

                // Extract query argument (required)
                let query_expr = args.first().ok_or_else(|| {
                    ExecutionError::Validation(
                        "FULLTEXT_SEARCH requires at least one argument (query)".to_string(),
                    )
                })?;
                let query = extract_string_literal(query_expr, &function_name, 0)?;

                // Extract language argument (required)
                let language_expr = args.get(1).ok_or_else(|| {
                    ExecutionError::Validation(
                        "FULLTEXT_SEARCH requires two arguments (query, language)".to_string(),
                    )
                })?;
                let language = extract_string_literal(language_expr, &function_name, 1)?;

                tracing::debug!("   Query: '{}', Language: '{}'", query, language);

                // Use alias if provided, otherwise use function name
                let table_name = alias
                    .clone()
                    .unwrap_or_else(|| function_name.to_lowercase());

                // Check if indexing engine is available
                let indexing_engine = ctx
                    .indexing_engine
                    .as_ref()
                    .ok_or_else(|| {
                        ExecutionError::Validation(
                            "Full-text search requires an indexing engine".to_string(),
                        )
                    })?
                    .clone();

                let storage = ctx.storage.clone();
                let tenant_id = ctx.tenant_id.to_string();
                let repo_id = ctx.repo_id.to_string();
                let branch = ctx.branch.to_string();
                let max_revision = ctx.max_revision;

                // Convert PostgreSQL query syntax to Tantivy if needed
                let tantivy_query = crate::physical_plan::fulltext::convert_postgres_query(&query);

                // Build search query for cross-workspace search
                let search_query = raisin_storage::fulltext::FullTextSearchQuery {
                    tenant_id: tenant_id.clone(),
                    repo_id: repo_id.clone(),
                    workspace_ids: None, // Cross-workspace by default
                    branch: branch.clone(),
                    language,
                    query: tantivy_query,
                    limit: 1000, // Default limit for table function
                    revision: max_revision,
                };

                use async_stream::try_stream;
                use raisin_storage::IndexingEngine;

                let row_stream = try_stream! {
                    // Execute search
                    let results = indexing_engine.search(&search_query)?;

                    // For each result, fetch the full node and create a row
                    for result in results {
                        // Fetch the node from storage
                        if let Some(node) = storage
                            .nodes()
                            .get(StorageScope::new(&tenant_id, &repo_id, &branch, &result.workspace_id), &result.node_id, max_revision.as_ref())
                            .await?
                        {
                            // Skip root nodes
                            if node.path == "/" {
                                continue;
                            }

                            // Create row with individual columns (qualified with table name/alias)
                            let mut columns = IndexMap::new();
                            columns.insert(format!("{}.node_id", table_name), PropertyValue::String(result.node_id.clone()));
                            columns.insert(format!("{}.workspace_id", table_name), PropertyValue::String(result.workspace_id.clone()));
                            columns.insert(format!("{}.name", table_name), PropertyValue::String(node.name.clone()));
                            columns.insert(format!("{}.path", table_name), PropertyValue::String(node.path.clone()));
                            columns.insert(format!("{}.node_type", table_name), PropertyValue::String(node.node_type.clone()));
                            columns.insert(format!("{}.score", table_name), PropertyValue::Float(result.score as f64));
                            columns.insert(format!("{}.revision", table_name), PropertyValue::Integer(node.version as i64));
                            columns.insert(format!("{}.properties", table_name), PropertyValue::Object(node.properties.clone()));

                            // Optional timestamp columns
                            if let Some(created) = node.created_at {
                                columns.insert(format!("{}.created_at", table_name), PropertyValue::String(created.to_rfc3339()));
                            }
                            if let Some(updated) = node.updated_at {
                                columns.insert(format!("{}.updated_at", table_name), PropertyValue::String(updated.to_rfc3339()));
                            }

                            yield Row { columns };
                        }
                    }
                };

                Ok(Box::pin(row_stream))
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
