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
                    shape_type: None,
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
            } else if function_name.eq_ignore_ascii_case("HYBRID_SEARCH") {
                tracing::info!("HYBRID_SEARCH table function invoked");

                // Extract query argument (required)
                let query_expr = args.first().ok_or_else(|| {
                    ExecutionError::Validation(
                        "HYBRID_SEARCH requires at least one argument (query)".to_string(),
                    )
                })?;
                let query = extract_string_literal(query_expr, &function_name, 0)?;

                // Extract optional limit argument (default 10)
                let limit = args
                    .get(1)
                    .and_then(|e| match &e.expr {
                        Expr::Literal(Literal::Int(n)) => Some(*n as usize),
                        Expr::Literal(Literal::BigInt(n)) => Some(*n as usize),
                        _ => None,
                    })
                    .unwrap_or(10);

                let table_name = alias
                    .clone()
                    .unwrap_or_else(|| function_name.to_lowercase());

                let storage = ctx.storage.clone();
                let tenant_id = ctx.tenant_id.to_string();
                let repo_id = ctx.repo_id.to_string();
                let branch = ctx.branch.to_string();
                let max_revision = ctx.max_revision;

                // Get fulltext and vector engines
                let indexing_engine = ctx.indexing_engine.clone();
                let hnsw_engine = ctx.hnsw_engine.clone();
                let embedding_provider = ctx.embedding_provider.clone();

                use async_stream::try_stream;
                use std::collections::HashMap;

                let row_stream = try_stream! {
                    // 1. Fulltext search (if indexing engine available)
                    let mut fulltext_rankings: HashMap<String, usize> = HashMap::new();
                    if let Some(ref engine) = indexing_engine {
                        use raisin_storage::IndexingEngine;
                        let search_query = raisin_storage::fulltext::FullTextSearchQuery {
                            tenant_id: tenant_id.clone(),
                            repo_id: repo_id.clone(),
                            workspace_ids: None,
                            branch: branch.clone(),
                            language: "en".to_string(),
                            query: crate::physical_plan::fulltext::convert_postgres_query(&query),
                            limit: limit * 2,
                            revision: max_revision,
                            shape_type: None,
                        };
                        if let Ok(results) = engine.search(&search_query) {
                            for (rank, result) in results.iter().enumerate() {
                                fulltext_rankings.insert(result.node_id.clone(), rank + 1);
                            }
                        }
                    }

                    // 2. Vector search (if HNSW engine + embedding provider available)
                    let mut vector_rankings: HashMap<String, (usize, f32)> = HashMap::new();
                    if let (Some(ref hnsw), Some(ref provider)) = (&hnsw_engine, &embedding_provider) {
                        let query_embedding = provider.generate_embedding(&query).await
                            .map_err(|e| raisin_error::Error::Backend(format!("Failed to generate embedding: {}", e)))?;
                        let normalized = raisin_hnsw::normalize_vector(&query_embedding);

                        if let Ok(results) = hnsw.search(
                            &tenant_id, &repo_id, &branch, None, &normalized, limit * 2,
                        ) {
                            for (rank, result) in results.iter().enumerate() {
                                vector_rankings.insert(result.node_id.clone(), (rank + 1, result.distance));
                            }
                        }
                    }

                    // 3. RRF merge: score = sum of 1/(k + rank) across lists
                    const RRF_K: f64 = 60.0;
                    let mut all_node_ids: Vec<String> = fulltext_rankings
                        .keys()
                        .chain(vector_rankings.keys())
                        .cloned()
                        .collect();
                    all_node_ids.sort();
                    all_node_ids.dedup();

                    let mut scored: Vec<(String, f64, Option<usize>, Option<usize>, Option<f32>)> = all_node_ids
                        .into_iter()
                        .map(|node_id| {
                            let mut score = 0.0;
                            let ft_rank = fulltext_rankings.get(&node_id).copied();
                            let (vec_rank, vec_dist) = vector_rankings
                                .get(&node_id)
                                .map(|(r, d)| (Some(*r), Some(*d)))
                                .unwrap_or((None, None));

                            if let Some(rank) = ft_rank {
                                score += 1.0 / (RRF_K + rank as f64);
                            }
                            if let Some(rank) = vec_rank {
                                score += 1.0 / (RRF_K + rank as f64);
                            }

                            (node_id, score, ft_rank, vec_rank, vec_dist)
                        })
                        .collect();

                    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                    scored.truncate(limit);

                    // 4. Fetch nodes and build rows
                    for (node_id, score, ft_rank, vec_rank, vec_dist) in scored {
                        // Determine workspace from vector results or use default
                        let workspace_id = "default";
                        if let Some(node) = storage
                            .nodes()
                            .get(StorageScope::new(&tenant_id, &repo_id, &branch, workspace_id), &node_id, max_revision.as_ref())
                            .await?
                        {
                            if node.path == "/" { continue; }

                            let mut columns = IndexMap::new();
                            columns.insert(format!("{}.node_id", table_name), PropertyValue::String(node_id));
                            columns.insert(format!("{}.workspace_id", table_name), PropertyValue::String(workspace_id.to_string()));
                            columns.insert(format!("{}.name", table_name), PropertyValue::String(node.name.clone()));
                            columns.insert(format!("{}.path", table_name), PropertyValue::String(node.path.clone()));
                            columns.insert(format!("{}.node_type", table_name), PropertyValue::String(node.node_type.clone()));
                            columns.insert(format!("{}.score", table_name), PropertyValue::Float(score));
                            columns.insert(format!("{}.fulltext_rank", table_name),
                                ft_rank.map(|r| PropertyValue::Integer(r as i64)).unwrap_or(PropertyValue::Null));
                            columns.insert(format!("{}.vector_rank", table_name),
                                vec_rank.map(|r| PropertyValue::Integer(r as i64)).unwrap_or(PropertyValue::Null));
                            columns.insert(format!("{}.vector_distance", table_name),
                                vec_dist.map(|d| PropertyValue::Float(d as f64)).unwrap_or(PropertyValue::Null));
                            columns.insert(format!("{}.revision", table_name), PropertyValue::Integer(node.version as i64));
                            columns.insert(format!("{}.properties", table_name), PropertyValue::Object(node.properties.clone()));

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
