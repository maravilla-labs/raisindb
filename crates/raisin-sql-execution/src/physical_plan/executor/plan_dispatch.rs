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

/// A boxed operator future.
type PlanFuture<'a> = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<RowStream, ExecutionError>> + Send + 'a>,
>;

/// Pick the operator for `plan` and return its future, boxed, WITHOUT polling it.
///
/// This is deliberately a plain function and not part of `execute_plan`'s async
/// block. Written inline, the match put every operator's future (and the
/// temporaries of building each one) into `execute_plan`'s own state machine,
/// whose poll frame is live for the whole execution of the operator below it.
/// In an unoptimised build that frame was ~154 KB, and a plan is a chain of
/// `execute_plan` frames (Sort > Project > Filter > scan is four of them): a few
/// operators were enough to overflow a 2 MiB worker stack, aborting the server.
/// Here the arm temporaries live only while this function runs; what stays on
/// the stack during execution is the small `execute_plan` frame plus the
/// operator's own.
fn dispatch_row<'a, S: Storage + raisin_storage::transactional::TransactionalStorage + 'static>(
    plan: &'a PhysicalPlan,
    ctx: &'a ExecutionContext<S>,
) -> PlanFuture<'a> {
    match plan {
        PhysicalPlan::TableScan { .. } => Box::pin(
            crate::physical_plan::scan_executors::execute_table_scan(plan, ctx),
        ),
        PhysicalPlan::CountScan { .. } => Box::pin(
            crate::physical_plan::scan_executors::execute_count_scan(plan, ctx),
        ),
        PhysicalPlan::PropertyIndexCountScan { .. } => Box::pin(
            crate::physical_plan::scan_executors::execute_property_index_count_scan(plan, ctx),
        ),
        PhysicalPlan::TableFunction { .. } => {
            Box::pin(crate::physical_plan::table_function::execute_table_function(plan, ctx))
        }
        PhysicalPlan::PrefixScan { .. } => Box::pin(
            crate::physical_plan::scan_executors::execute_prefix_scan(plan, ctx),
        ),
        PhysicalPlan::PropertyIndexScan { .. } => {
            Box::pin(crate::physical_plan::scan_executors::execute_property_index_scan(plan, ctx))
        }
        PhysicalPlan::PropertyOrderScan { .. } => {
            Box::pin(crate::physical_plan::scan_executors::execute_property_order_scan(plan, ctx))
        }
        PhysicalPlan::PropertyRangeScan { .. } => {
            Box::pin(crate::physical_plan::scan_executors::execute_property_range_scan(plan, ctx))
        }
        PhysicalPlan::PathIndexScan { .. } => {
            Box::pin(crate::physical_plan::scan_executors::execute_path_index_scan(plan, ctx))
        }
        PhysicalPlan::NodeIdScan { .. } => Box::pin(
            crate::physical_plan::scan_executors::execute_node_id_scan(plan, ctx),
        ),
        PhysicalPlan::LocalizedPathLookup { .. } => {
            Box::pin(crate::physical_plan::scan_executors::execute_localized_path_lookup(plan, ctx))
        }
        PhysicalPlan::FullTextScan { .. } => Box::pin(
            crate::physical_plan::fulltext::execute_fulltext_scan(plan, ctx),
        ),
        PhysicalPlan::NeighborsScan { .. } => {
            Box::pin(crate::physical_plan::scan_executors::execute_neighbors_scan(plan, ctx))
        }
        PhysicalPlan::NeighborhoodScan { .. } => {
            Box::pin(crate::physical_plan::scan_executors::execute_neighborhood_scan(plan, ctx))
        }
        PhysicalPlan::SpatialDistanceScan { .. } => {
            Box::pin(crate::physical_plan::scan_executors::execute_spatial_distance_scan(plan, ctx))
        }
        PhysicalPlan::SpatialKnnScan { .. } => {
            Box::pin(crate::physical_plan::scan_executors::execute_spatial_knn_scan(plan, ctx))
        }
        PhysicalPlan::SpatialAnnotate { .. } => {
            Box::pin(crate::physical_plan::spatial_annotate::execute_spatial_annotate(plan, ctx))
        }
        PhysicalPlan::ReferenceIndexScan { .. } => {
            Box::pin(crate::physical_plan::scan_executors::execute_reference_index_scan(plan, ctx))
        }
        PhysicalPlan::Filter { .. } => {
            Box::pin(crate::physical_plan::filter::execute_filter(plan, ctx))
        }
        PhysicalPlan::Project { .. } => {
            Box::pin(crate::physical_plan::project::execute_project(plan, ctx))
        }
        PhysicalPlan::Sort { .. } => Box::pin(crate::physical_plan::sort::execute_sort(plan, ctx)),
        PhysicalPlan::TopN { .. } =>
        // For now, implement TopN as Sort + Limit
        // TODO: Implement actual heap-based TopN for better performance
        {
            Box::pin(crate::physical_plan::sort::execute_sort(plan, ctx))
        }
        PhysicalPlan::Limit { .. } => {
            Box::pin(crate::physical_plan::limit::execute_limit(plan, ctx))
        }
        PhysicalPlan::NestedLoopJoin { .. } => {
            Box::pin(crate::physical_plan::nested_loop_join::execute_nested_loop_join(plan, ctx))
        }
        PhysicalPlan::HashJoin { .. } => Box::pin(
            crate::physical_plan::hash_join::execute_hash_join(plan, ctx),
        ),
        PhysicalPlan::HashSemiJoin { .. } => Box::pin(
            crate::physical_plan::semi_join::execute_hash_semi_join(plan, ctx),
        ),
        PhysicalPlan::IndexLookupJoin { .. } => {
            Box::pin(crate::physical_plan::index_lookup_join::execute_index_lookup_join(plan, ctx))
        }
        PhysicalPlan::HashAggregate { .. } => {
            Box::pin(crate::physical_plan::hash_aggregate::execute_hash_aggregate(plan, ctx))
        }
        PhysicalPlan::WithCTE { ctes, main_query } => {
            Box::pin(execute_with_cte(ctes, main_query, ctx))
        }
        PhysicalPlan::CTEScan { cte_name, .. } => Box::pin(execute_cte_scan(cte_name, ctx)),
        PhysicalPlan::VectorScan { .. } => Box::pin(
            crate::physical_plan::scan_executors::execute_vector_scan(plan, ctx),
        ),
        PhysicalPlan::Window { .. } => {
            Box::pin(crate::physical_plan::window::execute_window(plan, ctx))
        }
        PhysicalPlan::Distinct { .. } => {
            Box::pin(crate::physical_plan::distinct::execute_distinct(plan, ctx))
        }
        PhysicalPlan::Union { .. } => {
            Box::pin(crate::physical_plan::union::execute_union(plan, ctx))
        }
        PhysicalPlan::SetOperation { .. } => Box::pin(
            crate::physical_plan::set_operation::execute_set_operation(plan, ctx),
        ),
        PhysicalPlan::PhysicalInsert {
            target,
            columns,
            values,
            is_upsert,
            returning,
            ..
        } => Box::pin(crate::physical_plan::dml_executor::execute_insert(
            target,
            columns,
            values,
            *is_upsert,
            returning.as_deref(),
            ctx,
        )),
        PhysicalPlan::PhysicalUpdate {
            target,
            assignments,
            filter,
            returning,
            ..
        } => Box::pin(crate::physical_plan::dml_executor::execute_update(
            target,
            assignments,
            filter,
            returning.as_deref(),
            ctx,
        )),
        PhysicalPlan::PhysicalDelete {
            target,
            filter,
            returning,
            ..
        } => Box::pin(crate::physical_plan::dml_executor::execute_delete(
            target,
            filter,
            returning.as_deref(),
            ctx,
        )),
        PhysicalPlan::PhysicalOrder {
            source,
            target,
            position,
            workspace,
            branch_override,
        } => Box::pin(crate::physical_plan::dml_executor::execute_order(
            source,
            target,
            position,
            workspace,
            branch_override,
            ctx,
        )),
        PhysicalPlan::PhysicalMove {
            source,
            target_parent,
            workspace,
            branch_override,
        } => Box::pin(crate::physical_plan::dml_executor::execute_move(
            source,
            target_parent,
            workspace,
            branch_override,
            ctx,
        )),
        PhysicalPlan::PhysicalCopy {
            source,
            target_parent,
            new_name,
            recursive,
            workspace,
            branch_override,
        } => Box::pin(crate::physical_plan::dml_executor::execute_copy(
            source,
            target_parent,
            new_name,
            *recursive,
            workspace,
            branch_override,
            ctx,
        )),
        PhysicalPlan::PhysicalTranslate {
            locale,
            node_translations,
            filter,
            workspace,
            branch_override,
        } => Box::pin(crate::physical_plan::dml_executor::execute_translate(
            locale,
            node_translations,
            filter,
            workspace,
            branch_override,
            ctx,
        )),
        PhysicalPlan::PhysicalRelate {
            source,
            target,
            relation_type,
            weight,
            branch_override,
        } => Box::pin(crate::physical_plan::dml_executor::execute_relate(
            source,
            target,
            relation_type,
            *weight,
            branch_override,
            ctx,
        )),
        PhysicalPlan::PhysicalUnrelate {
            source,
            target,
            relation_type,
            branch_override,
        } => Box::pin(crate::physical_plan::dml_executor::execute_unrelate(
            source,
            target,
            relation_type,
            branch_override,
            ctx,
        )),
        PhysicalPlan::PhysicalRestore { .. } => Box::pin(async {
            // RESTORE is handled directly in QueryEngine::execute_restore()
            // This branch should never be reached
            Err(Error::Internal(
                "PhysicalRestore should be handled directly by engine".to_string(),
            ))
        }),
        PhysicalPlan::LateralMap { .. } => Box::pin(
            crate::physical_plan::lateral_map::execute_lateral_map(plan, ctx),
        ),
        PhysicalPlan::CompoundIndexScan { .. } => {
            Box::pin(crate::physical_plan::scan_executors::execute_compound_index_scan(plan, ctx))
        }
        PhysicalPlan::Empty => Box::pin(async {
            // Empty plan - return empty stream (DDL is handled directly in engine)
            let empty: RowStream = Box::pin(futures::stream::empty());
            Ok(empty)
        }),
    }
}

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

        let result = dispatch_row(plan, ctx).await;

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
                execute_project_batch(input_stream, exprs.to_vec(), ctx.clone()).await
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
