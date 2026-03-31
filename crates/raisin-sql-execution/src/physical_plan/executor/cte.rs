//! CTE (Common Table Expression) execution helpers.
//!
//! Handles materialization of CTEs into the execution context and
//! scanning from previously materialized CTE result sets.

use super::context::ExecutionContext;
use super::plan_dispatch::execute_plan;
use super::row::{ExecutionError, RowStream};
use crate::physical_plan::cte_storage::MaterializedCTE;
use crate::physical_plan::operators::PhysicalPlan;
use raisin_storage::Storage;

/// Execute a WITH CTE (Common Table Expression) query
///
/// This function materializes each CTE in order (allowing dependencies between CTEs),
/// stores them in the execution context's CTE storage, and then executes the main query.
///
/// # CTE Materialization Process
///
/// 1. For each CTE in order:
///    - Execute the CTE's physical plan
///    - Collect all results into memory or spill to disk (based on size)
///    - Store in `ctx.cte_storage` for later reference
///    - Track temp file paths in `ctx.temp_files` for cleanup
///
/// 2. Execute the main query with all CTEs available
///
/// # Arguments
///
/// * `ctes` - List of (name, plan) pairs for each CTE to materialize
/// * `main_query` - The main query plan that may reference the CTEs
/// * `ctx` - Execution context with storage and CTE configuration
///
/// # Returns
///
/// A stream of rows from the main query execution
///
/// # Errors
///
/// Returns `ExecutionError` if:
/// - Any CTE materialization fails
/// - Disk spillage I/O fails
/// - Main query execution fails
pub(super) async fn execute_with_cte<
    'a,
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
>(
    ctes: &'a [(String, Box<PhysicalPlan>)],
    main_query: &'a PhysicalPlan,
    ctx: &'a ExecutionContext<S>,
) -> Result<RowStream, ExecutionError> {
    use tracing::{debug, info};

    // Materialize each CTE in order
    for (cte_name, cte_plan) in ctes {
        debug!("Materializing CTE: {}", cte_name);

        // Execute and materialize the CTE
        let materialized_cte =
            MaterializedCTE::materialize(cte_plan.as_ref(), ctx, &ctx.cte_config).await?;

        // Log materialization details
        if materialized_cte.is_in_memory() {
            info!(
                "CTE '{}' materialized in memory: {} rows, {} bytes",
                cte_name,
                materialized_cte.row_count(),
                materialized_cte.size_bytes()
            );
        } else {
            info!(
                "CTE '{}' spilled to disk: {} rows, {} bytes",
                cte_name,
                materialized_cte.row_count(),
                materialized_cte.size_bytes()
            );

            // Track temp file for cleanup
            if let MaterializedCTE::OnDisk { file_path, .. } = &materialized_cte {
                ctx.temp_files.write().await.push(file_path.clone());
            }
        }

        // Store the materialized CTE for later reference
        ctx.cte_storage
            .write()
            .await
            .insert(cte_name.clone(), materialized_cte);
    }

    debug!("All CTEs materialized, executing main query");

    // Execute the main query with all CTEs available
    execute_plan(main_query, ctx).await
}

/// Execute a CTE scan operation
///
/// This function reads from a previously materialized CTE stored in the execution
/// context. It works transparently whether the CTE is stored in memory or spilled
/// to disk.
///
/// # Arguments
///
/// * `cte_name` - Name of the CTE to scan
/// * `ctx` - Execution context containing CTE storage
///
/// # Returns
///
/// A stream of rows from the materialized CTE
///
/// # Errors
///
/// Returns `ExecutionError::Validation` if:
/// - The CTE name is not found in storage (indicates a planner bug)
/// - The CTE was not materialized before being scanned
///
/// Returns `ExecutionError::Backend` if:
/// - Disk-spilled CTE file cannot be read
/// - Deserialization from disk fails
pub(super) async fn execute_cte_scan<
    'a,
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
>(
    cte_name: &'a str,
    ctx: &'a ExecutionContext<S>,
) -> Result<RowStream, ExecutionError> {
    use futures::stream;
    use futures::StreamExt;
    use tracing::debug;

    debug!("Scanning CTE: {}", cte_name);

    // Look up the CTE in storage and get an iterator
    // We need to hold the lock only while getting the iterator, not during iteration
    let cte_iter = {
        let cte_storage = ctx.cte_storage.read().await;

        let materialized_cte = cte_storage.get(cte_name).ok_or_else(|| {
            ExecutionError::Validation(format!(
                "CTE '{}' not found in storage. This indicates a planner bug - \
                 CTEs must be materialized before being scanned.",
                cte_name
            ))
        })?;

        debug!(
            "CTEScan: Found CTE '{}' with {} rows, {} bytes, in_memory={}",
            cte_name,
            materialized_cte.row_count(),
            materialized_cte.size_bytes(),
            materialized_cte.is_in_memory()
        );

        // Get an iterator over the CTE results
        materialized_cte.iter()?
    }; // Lock is released here

    // Convert the iterator to an async stream
    // This works for both in-memory and disk-spilled CTEs
    let cte_name_owned = cte_name.to_string();
    let row_stream = stream::iter(cte_iter).map(move |row_result| {
        let cte_name = cte_name_owned.clone();

        row_result.map(|mut row| {
            debug!(
                "CTEScan: Processing row from '{}' with {} columns: {:?}",
                cte_name,
                row.columns.len(),
                row.columns.keys().collect::<Vec<_>>()
            );
            // Re-qualify columns for CTE access and provide unqualified aliases for downstream operators.
            let original_keys: Vec<String> = row.columns.keys().cloned().collect();

            for key in original_keys {
                let value = match row.columns.get(&key).cloned() {
                    Some(v) => v,
                    None => continue,
                };

                if let Some((_, column)) = key.split_once('.') {
                    let cte_key = format!("{}.{}", cte_name, column);
                    if !row.columns.contains_key(&cte_key) {
                        row.columns.insert(cte_key, value.clone());
                    }

                    if !row.columns.contains_key(column) {
                        row.columns.insert(column.to_string(), value.clone());
                    }
                } else {
                    let cte_key = format!("{}.{}", cte_name, key);
                    if !row.columns.contains_key(&cte_key) {
                        row.columns.insert(cte_key, value.clone());
                    }
                }
            }

            row
        })
    });

    Ok(Box::pin(row_stream))
}
