//! Physical plan dispatch for row and batch execution.
//!
//! Contains the top-level `execute_plan` and `execute_plan_batch` functions
//! that dispatch to the appropriate operator executor based on the plan variant.
//!
//! NOTE: File intentionally exceeds 300 lines - the dispatch match is a single
//! cohesive unit that cannot be meaningfully split further.

use super::context::ExecutionContext;
use super::cte::{execute_cte_scan, execute_with_cte};
use super::row::{ExecutionError, RowStream};
use crate::physical_plan::operators::PhysicalPlan;
use raisin_error::Error;
use raisin_storage::Storage;

/// Execute a physical plan and return a stream of rows
///
/// This is the main entry point for query execution. It dispatches to
/// the appropriate operator implementation based on the plan type.
///
/// # Example
///
/// ```rust,ignore
/// let ctx = ExecutionContext::new(storage, "tenant1", "repo1", "main", "workspace1");
/// let mut stream = execute_plan(&physical_plan, &ctx).await?;
///
/// while let Some(row) = stream.next().await {
///     println!("{:?}", row?);
/// }
/// ```
pub fn execute_plan<
    'a,
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
>(
    plan: &'a PhysicalPlan,
    ctx: &'a ExecutionContext<S>,
) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<RowStream, ExecutionError>> + Send + 'a>,
> {
    Box::pin(async move {
        tracing::debug!(
            operator = %plan.describe(),
            mode = "row",
            "execute_plan started"
        );
        let start = std::time::Instant::now();

        let result = match plan {
            PhysicalPlan::TableScan { .. } => {
                crate::physical_plan::scan_executors::execute_table_scan(plan, ctx).await
            }
            PhysicalPlan::CountScan { .. } => {
                crate::physical_plan::scan_executors::execute_count_scan(plan, ctx).await
            }
            PhysicalPlan::PropertyIndexCountScan { .. } => {
                crate::physical_plan::scan_executors::execute_property_index_count_scan(plan, ctx)
                    .await
            }
            PhysicalPlan::TableFunction { .. } => {
                crate::physical_plan::table_function::execute_table_function(plan, ctx).await
            }
            PhysicalPlan::PrefixScan { .. } => {
                crate::physical_plan::scan_executors::execute_prefix_scan(plan, ctx).await
            }
            PhysicalPlan::PropertyIndexScan { .. } => {
                crate::physical_plan::scan_executors::execute_property_index_scan(plan, ctx).await
            }
            PhysicalPlan::PropertyOrderScan { .. } => {
                crate::physical_plan::scan_executors::execute_property_order_scan(plan, ctx).await
            }
            PhysicalPlan::PropertyRangeScan { .. } => {
                crate::physical_plan::scan_executors::execute_property_range_scan(plan, ctx).await
            }
            PhysicalPlan::PathIndexScan { .. } => {
                crate::physical_plan::scan_executors::execute_path_index_scan(plan, ctx).await
            }
            PhysicalPlan::NodeIdScan { .. } => {
                crate::physical_plan::scan_executors::execute_node_id_scan(plan, ctx).await
            }
            PhysicalPlan::FullTextScan { .. } => {
                crate::physical_plan::fulltext::execute_fulltext_scan(plan, ctx).await
            }
            PhysicalPlan::NeighborsScan { .. } => {
                crate::physical_plan::scan_executors::execute_neighbors_scan(plan, ctx).await
            }
            PhysicalPlan::SpatialDistanceScan { .. } => {
                crate::physical_plan::scan_executors::execute_spatial_distance_scan(plan, ctx).await
            }
            PhysicalPlan::SpatialKnnScan { .. } => {
                crate::physical_plan::scan_executors::execute_spatial_knn_scan(plan, ctx).await
            }
            PhysicalPlan::ReferenceIndexScan { .. } => {
                crate::physical_plan::scan_executors::execute_reference_index_scan(plan, ctx).await
            }
            PhysicalPlan::Filter { .. } => {
                crate::physical_plan::filter::execute_filter(plan, ctx).await
            }
            PhysicalPlan::Project { .. } => {
                crate::physical_plan::project::execute_project(plan, ctx).await
            }
            PhysicalPlan::Sort { .. } => crate::physical_plan::sort::execute_sort(plan, ctx).await,
            PhysicalPlan::TopN { .. } => {
                // For now, implement TopN as Sort + Limit
                // TODO: Implement actual heap-based TopN for better performance
                crate::physical_plan::sort::execute_sort(plan, ctx).await
            }
            PhysicalPlan::Limit { .. } => {
                crate::physical_plan::limit::execute_limit(plan, ctx).await
            }
            PhysicalPlan::NestedLoopJoin { .. } => {
                crate::physical_plan::nested_loop_join::execute_nested_loop_join(plan, ctx).await
            }
            PhysicalPlan::HashJoin { .. } => {
                crate::physical_plan::hash_join::execute_hash_join(plan, ctx).await
            }
            PhysicalPlan::HashSemiJoin { .. } => {
                crate::physical_plan::semi_join::execute_hash_semi_join(plan, ctx).await
            }
            PhysicalPlan::IndexLookupJoin { .. } => {
                crate::physical_plan::index_lookup_join::execute_index_lookup_join(plan, ctx).await
            }
            PhysicalPlan::HashAggregate { .. } => {
                crate::physical_plan::hash_aggregate::execute_hash_aggregate(plan, ctx).await
            }
            PhysicalPlan::WithCTE { ctes, main_query } => {
                execute_with_cte(ctes, main_query, ctx).await
            }
            PhysicalPlan::CTEScan { cte_name, .. } => execute_cte_scan(cte_name, ctx).await,
            PhysicalPlan::VectorScan { .. } => {
                crate::physical_plan::scan_executors::execute_vector_scan(plan, ctx).await
            }
            PhysicalPlan::Window { .. } => {
                crate::physical_plan::window::execute_window(plan, ctx).await
            }
            PhysicalPlan::Distinct { .. } => {
                crate::physical_plan::distinct::execute_distinct(plan, ctx).await
            }
            PhysicalPlan::PhysicalInsert {
                target,
                columns,
                values,
                is_upsert,
                ..
            } => {
                crate::physical_plan::dml_executor::execute_insert(
                    target, columns, values, *is_upsert, ctx,
                )
                .await
            }
            PhysicalPlan::PhysicalUpdate {
                target,
                assignments,
                filter,
                ..
            } => {
                crate::physical_plan::dml_executor::execute_update(target, assignments, filter, ctx)
                    .await
            }
            PhysicalPlan::PhysicalDelete { target, filter, .. } => {
                crate::physical_plan::dml_executor::execute_delete(target, filter, ctx).await
            }
            PhysicalPlan::PhysicalOrder {
                source,
                target,
                position,
                workspace,
                branch_override,
            } => {
                crate::physical_plan::dml_executor::execute_order(
                    source,
                    target,
                    position,
                    workspace,
                    branch_override,
                    ctx,
                )
                .await
            }
            PhysicalPlan::PhysicalMove {
                source,
                target_parent,
                workspace,
                branch_override,
            } => {
                crate::physical_plan::dml_executor::execute_move(
                    source,
                    target_parent,
                    workspace,
                    branch_override,
                    ctx,
                )
                .await
            }
            PhysicalPlan::PhysicalCopy {
                source,
                target_parent,
                new_name,
                recursive,
                workspace,
                branch_override,
            } => {
                crate::physical_plan::dml_executor::execute_copy(
                    source,
                    target_parent,
                    new_name,
                    *recursive,
                    workspace,
                    branch_override,
                    ctx,
                )
                .await
            }
            PhysicalPlan::PhysicalTranslate {
                locale,
                node_translations,
                block_translations,
                filter,
                workspace,
                branch_override,
            } => {
                crate::physical_plan::dml_executor::execute_translate(
                    locale,
                    node_translations,
                    block_translations,
                    filter,
                    workspace,
                    branch_override,
                    ctx,
                )
                .await
            }
            PhysicalPlan::PhysicalRelate {
                source,
                target,
                relation_type,
                weight,
                branch_override,
            } => {
                crate::physical_plan::dml_executor::execute_relate(
                    source,
                    target,
                    relation_type,
                    *weight,
                    branch_override,
                    ctx,
                )
                .await
            }
            PhysicalPlan::PhysicalUnrelate {
                source,
                target,
                relation_type,
                branch_override,
            } => {
                crate::physical_plan::dml_executor::execute_unrelate(
                    source,
                    target,
                    relation_type,
                    branch_override,
                    ctx,
                )
                .await
            }
            PhysicalPlan::PhysicalRestore { .. } => {
                // RESTORE is handled directly in QueryEngine::execute_restore()
                // This branch should never be reached
                Err(Error::Internal(
                    "PhysicalRestore should be handled directly by engine".to_string(),
                ))
            }
            PhysicalPlan::LateralMap { .. } => {
                crate::physical_plan::lateral_map::execute_lateral_map(plan, ctx).await
            }
            PhysicalPlan::CompoundIndexScan { .. } => {
                crate::physical_plan::scan_executors::execute_compound_index_scan(plan, ctx).await
            }
            PhysicalPlan::Empty => {
                // Empty plan - return empty stream (DDL is handled directly in engine)
                let empty: RowStream = Box::pin(futures::stream::empty());
                Ok(empty)
            }
        };

        tracing::debug!(
            operator = %plan.describe(),
            elapsed_us = start.elapsed().as_micros(),
            "Operator completed"
        );

        result
    })
}

/// Execute a physical plan and return a batch stream
///
/// This is the batch-aware execution path that returns a stream of batches instead
/// of individual rows. It's optimized for OLAP-style queries that process many rows.
///
/// # When to Use
///
/// Use batch execution when:
/// - Processing large result sets (>1000 rows)
/// - Performing analytical queries (aggregations, scans)
/// - Throughput is more important than latency
///
/// Use row execution when:
/// - Processing small result sets (<100 rows)
/// - Performing point queries or index lookups
/// - Low latency is critical
///
/// # Example
///
/// ```rust,ignore
/// use raisin_sql::physical_plan::batch::BatchConfig;
///
/// let batch_config = BatchConfig::default();
/// let ctx = ExecutionContext::new(storage, "tenant1", "repo1", "main", "workspace1");
/// let mut batch_stream = execute_plan_batch(&physical_plan, &ctx, &batch_config).await?;
///
/// while let Some(batch) = batch_stream.next().await {
///     let batch = batch?;
///     // Process entire batch at once
///     for row in batch.iter() {
///         // ...
///     }
/// }
/// ```
pub fn execute_plan_batch<
    'a,
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
>(
    plan: &'a PhysicalPlan,
    ctx: &'a ExecutionContext<S>,
    batch_config: &'a crate::physical_plan::batch::BatchConfig,
) -> std::pin::Pin<
    Box<
        dyn std::future::Future<
                Output = Result<crate::physical_plan::batch_execution::BatchStream, ExecutionError>,
            > + Send
            + 'a,
    >,
> {
    Box::pin(async move {
        tracing::debug!(
            operator = %plan.describe(),
            mode = "batch",
            batch_size = batch_config.default_batch_size,
            "execute_plan_batch started"
        );
        let start = std::time::Instant::now();

        use crate::physical_plan::batch_execution::{
            convert_row_stream_to_batch_stream, execute_prefix_scan_batch, execute_project_batch,
            execute_property_index_scan_batch, execute_table_scan_batch,
        };

        let result = match plan {
            // Batch-aware scan operators
            PhysicalPlan::TableScan { .. } => {
                execute_table_scan_batch(plan, ctx, batch_config).await
            }
            PhysicalPlan::PrefixScan { .. } => {
                execute_prefix_scan_batch(plan, ctx, batch_config).await
            }
            PhysicalPlan::PropertyIndexScan { .. } => {
                execute_property_index_scan_batch(plan, ctx, batch_config).await
            }

            // Batch-aware projection operator
            PhysicalPlan::Project { input, exprs } => {
                let input_stream = execute_plan_batch(input.as_ref(), ctx, batch_config).await?;
                execute_project_batch(input_stream, exprs.clone(), ctx.clone()).await
            }

            // For all other operators, fall back to row execution and convert to batches
            _ => {
                let row_stream = execute_plan(plan, ctx).await?;
                Ok(convert_row_stream_to_batch_stream(row_stream, batch_config))
            }
        };

        tracing::debug!(
            operator = %plan.describe(),
            elapsed_us = start.elapsed().as_micros(),
            batch_size = batch_config.default_batch_size,
            "Batch operator completed"
        );

        result
    })
}
