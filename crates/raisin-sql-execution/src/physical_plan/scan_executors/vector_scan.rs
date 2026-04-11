// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Vector scan executor.
//!
//! Performs k-nearest neighbor (k-NN) search using the HNSW index.
//! This is the optimized path for vector similarity queries.

use super::helpers::{get_locales_to_use, resolve_node_for_locale};
use super::node_to_row::node_to_row;
use crate::physical_plan::executor::{ExecutionContext, ExecutionError, RowStream};
use crate::physical_plan::operators::PhysicalPlan;
use raisin_core::services::rls_filter;
use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_models::permissions::PermissionScope;
use raisin_storage::{NodeRepository, Storage, StorageScope};

/// Execute a VectorScan operator.
///
/// Performs k-nearest neighbor (k-NN) search using the HNSW index.
///
/// # Algorithm
/// 1. Evaluate query_vector expression (may call EMBEDDING() function)
/// 2. Call HNSW engine with appropriate distance metric
/// 3. Fetch full node data for result node_ids
/// 4. Apply projection and return row stream
///
/// # Performance
/// - HNSW search: O(log n) approximate nearest neighbor
/// - Currently only Cosine distance is fully optimized via HNSW
pub async fn execute_vector_scan<S: Storage + 'static>(
    plan: &PhysicalPlan,
    ctx: &ExecutionContext<S>,
) -> Result<RowStream, ExecutionError> {
    let (
        tenant_id,
        repo_id,
        branch,
        workspace,
        table,
        alias,
        query_vector,
        distance_metric,
        _vector_column,
        k,
        max_distance,
        projection,
        distance_alias,
    ) = match plan {
        PhysicalPlan::VectorScan {
            tenant_id,
            repo_id,
            branch,
            workspace,
            table,
            alias,
            query_vector,
            distance_metric,
            vector_column,
            k,
            max_distance,
            projection,
            distance_alias,
        } => (
            tenant_id.clone(),
            repo_id.clone(),
            branch.clone(),
            workspace.clone(),
            table.clone(),
            alias.clone(),
            query_vector.clone(),
            *distance_metric,
            vector_column.clone(),
            *k,
            *max_distance,
            projection.clone(),
            distance_alias.clone(),
        ),
        _ => {
            return Err(Error::Validation(
                "Invalid plan for vector scan".to_string(),
            ))
        }
    };

    tracing::info!(
        "   VectorScan: tenant={}, repo={}, branch={}, workspace={}, metric={}, k={}, threshold={:?}",
        tenant_id, repo_id, branch, workspace, distance_metric, k, max_distance
    );

    let hnsw_engine = ctx.hnsw_engine.as_ref().ok_or_else(|| {
        Error::Validation("HNSW engine not configured in execution context".to_string())
    })?;

    // Step 1: Evaluate query_vector expression to get Vec<f32>
    // Conditionally normalize: only for metrics that require normalized vectors
    let query_vec = if distance_metric.to_hnsw_metric().requires_normalization() {
        let raw = evaluate_query_vector(&query_vector, ctx).await?;
        raisin_hnsw::normalize_vector(&raw)
    } else {
        evaluate_query_vector(&query_vector, ctx).await?
    };

    // Step 2: Call HNSW search with distance threshold (the index uses the metric it was created with)
    let search_results = hnsw_engine
        .search_with_threshold(
            &tenant_id,
            &repo_id,
            &branch,
            Some(&workspace),
            &query_vec,
            k,
            max_distance,
        )
        .map_err(|e| Error::Backend(format!("HNSW search failed: {}", e)))?;

    tracing::debug!("   HNSW returned {} results", search_results.len());

    // Step 3: Fetch nodes and build result rows
    let storage = ctx.storage.clone();
    let max_revision = ctx.max_revision;
    let qualifier = alias.clone().unwrap_or_else(|| table.clone());
    let ctx_clone = ctx.clone();

    let stream = async_stream::stream! {
        let locales_to_use = get_locales_to_use(&ctx_clone);

        for result in search_results {
            let node_opt = storage
                .nodes()
                .get(
                    StorageScope::new(&tenant_id, &repo_id, &branch, &result.workspace_id),
                    &result.node_id,
                    max_revision.as_ref(),
                )
                .await;

            match node_opt {
                Ok(Some(node)) => {
                    if node.path == "/" { continue; }

                    let node = if let Some(ref auth) = ctx_clone.auth_context {
                        let scope = PermissionScope::new(&workspace, &branch);
                        match rls_filter::filter_node(node, auth, &scope) {
                            Some(n) => n,
                            None => continue,
                        }
                    } else {
                        node
                    };

                    for locale in &locales_to_use {
                        let translated_node = match resolve_node_for_locale(node.clone(), &ctx_clone, locale).await {
                            Ok(Some(n)) => n,
                            Ok(None) => continue,
                            Err(e) => {
                                yield Err(e);
                                return;
                            }
                        };

                        let mut row = match node_to_row(&translated_node, &qualifier, &result.workspace_id, &projection, &ctx_clone, locale).await {
                            Ok(r) => r,
                            Err(e) => {
                                yield Err(e);
                                return;
                            }
                        };

                        let distance_column_name = if let Some(ref alias_name) = distance_alias {
                            format!("{}.{}", qualifier, alias_name)
                        } else {
                            format!("{}.distance", qualifier)
                        };

                        row.insert(
                            distance_column_name,
                            PropertyValue::Float(result.distance as f64),
                        );

                        yield Ok(row);
                    }
                }
                Ok(None) => {
                    tracing::warn!("Node not found: {} (workspace: {})", result.node_id, result.workspace_id);
                    continue;
                }
                Err(e) => {
                    yield Err(Error::Backend(format!("Failed to fetch node {}: {}", result.node_id, e)));
                    return;
                }
            }
        }
    };

    Ok(Box::pin(stream))
}

/// Evaluate the query vector expression to produce a Vec<f32>.
async fn evaluate_query_vector<S: Storage + 'static>(
    query_vector: &raisin_sql::analyzer::TypedExpr,
    ctx: &ExecutionContext<S>,
) -> Result<Vec<f32>, Error> {
    use raisin_sql::analyzer::Expr;

    match &query_vector.expr {
        Expr::Literal(raisin_sql::analyzer::Literal::Vector(vec)) => Ok(vec.clone()),
        Expr::Function { name, args, .. } if name.to_uppercase() == "EMBEDDING" => {
            if args.len() != 1 {
                return Err(Error::Validation(format!(
                    "EMBEDDING() expects 1 argument (text), got {}",
                    args.len()
                )));
            }

            let text = match &args[0].expr {
                Expr::Literal(raisin_sql::analyzer::Literal::Text(s)) => s.clone(),
                _ => {
                    return Err(Error::Validation(format!(
                        "EMBEDDING() argument must be a text literal, got: {:?}",
                        args[0].expr
                    )));
                }
            };

            tracing::info!(
                text = %text,
                "Generating embedding for EMBEDDING() function in VectorScan"
            );

            let embedding = crate::physical_plan::eval::generate_embedding_cached(&text, ctx)
                .await
                .map_err(|e| Error::Backend(format!("Failed to generate embedding: {}", e)))?;

            // Note: normalization is now handled by the caller based on the distance metric
            tracing::info!(
                text = %text,
                dimensions = embedding.len(),
                "Successfully generated embedding"
            );

            Ok(embedding)
        }
        _ => Err(Error::Validation(format!(
            "Unsupported query vector expression: {:?}",
            query_vector.expr
        ))),
    }
}
