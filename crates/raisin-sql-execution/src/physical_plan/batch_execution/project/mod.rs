//! Batch-Aware Projection Execution
//!
//! This module implements columnar projection that processes entire batches at once,
//! significantly reducing overhead for JSON-heavy queries with repeated property extractions.
//!
//! # Performance Impact
//!
//! For queries with complex JSON projections (e.g., `author.properties ->> 'username'` repeated
//! across many columns), this provides:
//! - 30-40% reduction in latency (15-18ms -> 8-10ms target)
//! - Better CPU cache locality (columnar access patterns)
//! - Reduced redundant JSON parsing (extract base columns once per batch)
//!
//! # Module Structure
//!
//! - `column_utils` - Literal broadcasting, text extraction, type conversions

mod column_utils;

pub(crate) use column_utils::{
    broadcast_literal, extract_text_from_property_value, property_values_to_column_array,
};

use super::BatchStream;
use crate::physical_plan::batch::ColumnArray;
use crate::physical_plan::eval::eval_expr_async;
use crate::physical_plan::executor::{ExecutionContext, ExecutionError};
use crate::physical_plan::types::to_property_value;
use async_stream::try_stream;
use futures::StreamExt;
use indexmap::IndexMap;
use raisin_error::Error;
use raisin_models::nodes::properties::PropertyValue;
use raisin_sql::analyzer::{Expr, Literal, TypedExpr};
use raisin_sql::logical_plan::ProjectionExpr;
use raisin_storage::Storage;
use std::collections::HashMap;

use column_utils::{broadcast_literal_impl, json_value_to_property_value};

/// Execute batch-aware projection operator
///
/// This processes entire batches at once for better performance, especially for
/// JSON-heavy queries with repeated property extractions.
///
/// # Arguments
///
/// * `input` - Input batch stream
/// * `exprs` - Projection expressions with aliases
/// * `ctx` - Execution context
///
/// # Returns
///
/// Stream of projected batches
pub async fn execute_project_batch<S: Storage + 'static>(
    input: BatchStream,
    exprs: Vec<ProjectionExpr>,
    ctx: ExecutionContext<S>,
) -> Result<BatchStream, ExecutionError> {
    tracing::debug!(num_exprs = exprs.len(), "project_batch started");

    Ok(Box::pin(try_stream! {
        let mut input_stream = input;

        while let Some(batch_result) = input_stream.next().await {
            let batch = batch_result?;
            let batch_start = std::time::Instant::now();

            tracing::debug!(
                num_rows = batch.num_rows(),
                num_exprs = exprs.len(),
                "Processing batch"
            );

            // Build output batch column by column
            let mut output_columns: IndexMap<String, ColumnArray> = IndexMap::new();

            // Cache for common base columns (e.g., "author.properties")
            // This avoids re-evaluating the same column expression multiple times
            let mut column_cache: HashMap<String, ColumnArray> = HashMap::new();

            // Evaluate each projection expression
            for proj_expr in &exprs {
                let expr_start = std::time::Instant::now();

                // Try columnar evaluation first (fast path)
                let result_column = match try_eval_columnar(&proj_expr.expr, &batch, &mut column_cache, &ctx).await {
                    Ok(col) => {
                        tracing::trace!(
                            alias = %proj_expr.alias,
                            elapsed_us = expr_start.elapsed().as_micros(),
                            path = "columnar",
                            "Expression evaluated"
                        );
                        col
                    },
                    Err(_) => {
                        // Fall back to row-by-row evaluation for unsupported expressions
                        let col = eval_row_by_row_fallback(&proj_expr.expr, &batch, &ctx).await?;
                        tracing::trace!(
                            alias = %proj_expr.alias,
                            elapsed_us = expr_start.elapsed().as_micros(),
                            path = "row_fallback",
                            "Expression evaluated (fallback)"
                        );
                        col
                    }
                };

                output_columns.insert(proj_expr.alias.clone(), result_column);
            }

            tracing::debug!(
                num_rows = batch.num_rows(),
                elapsed_us = batch_start.elapsed().as_micros(),
                cache_hits = column_cache.len(),
                "Batch projection completed"
            );

            // Convert columnar output to batch
            let output_batch = super::super::batch::Batch::from_columns(output_columns);
            yield output_batch;
        }
    }))
}

/// Try to evaluate expression in columnar mode
///
/// This is the fast path for supported expression types. If an expression
/// cannot be efficiently evaluated columnar, this returns an error and
/// the caller should fall back to row-by-row evaluation.
fn try_eval_columnar<'a, S: Storage + 'static>(
    expr: &'a TypedExpr,
    batch: &'a super::super::batch::Batch,
    column_cache: &'a mut HashMap<String, ColumnArray>,
    ctx: &'a ExecutionContext<S>,
) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<ColumnArray, ExecutionError>> + Send + 'a>,
> {
    Box::pin(async move {
        match &expr.expr {
            // Direct column reference - zero copy!
            Expr::Column { table, column } => {
                let col_key = format!("{}.{}", table, column);

                // Check cache first
                if let Some(cached) = column_cache.get(&col_key) {
                    return Ok(cached.clone());
                }

                // Get from batch
                let col = batch
                    .column(&col_key)
                    .ok_or_else(|| Error::Validation(format!("Column not found: {}", col_key)))?;

                // Cache for future use
                column_cache.insert(col_key, col.clone());

                Ok(col.clone())
            }

            // JSON text extraction: properties ->> 'key'
            Expr::JsonExtractText { object, key } => {
                eval_json_extract_columnar(object, key, batch, column_cache, ctx).await
            }

            // Literal - broadcast to all rows
            Expr::Literal(lit) => {
                let num_rows = batch.num_rows();
                Ok(broadcast_literal_impl(lit, num_rows))
            }

            // COALESCE - evaluate each argument columnar and merge
            Expr::Function {
                name,
                args,
                signature: _,
                filter: _,
            } if name.to_uppercase() == "COALESCE" => {
                eval_coalesce_columnar(args, batch, column_cache, ctx).await
            }

            // All other expressions fall back to row-by-row
            _ => Err(Error::Validation(
                "Unsupported columnar operation".to_string(),
            )),
        }
    })
}

/// Extract JSON field from entire column (CRITICAL OPTIMIZATION)
///
/// This is the key optimization for JSON-heavy queries. Instead of parsing
/// each JSON object multiple times per row, we extract the field from the
/// entire column at once.
async fn eval_json_extract_columnar<S: Storage + 'static>(
    object_expr: &TypedExpr,
    key_expr: &TypedExpr,
    batch: &super::super::batch::Batch,
    column_cache: &mut HashMap<String, ColumnArray>,
    ctx: &ExecutionContext<S>,
) -> Result<ColumnArray, ExecutionError> {
    // Step 1: Get base JSON column (may be cached)
    let json_column = try_eval_columnar(object_expr, batch, column_cache, ctx).await?;

    // Step 2: Extract key (must be a constant literal for columnar evaluation)
    let key = match &key_expr.expr {
        Expr::Literal(Literal::Text(k)) => k,
        _ => {
            // Dynamic keys require row-by-row fallback
            return Err(Error::Validation(
                "Dynamic JSON keys not supported in columnar mode".to_string(),
            ));
        }
    };

    // Step 3: Extract field from every JSON object in column
    let mut result = Vec::with_capacity(json_column.len());

    match json_column {
        ColumnArray::Object(jsons) => {
            for json_opt in jsons {
                let value = json_opt
                    .as_ref()
                    .and_then(|json| json.get(key))
                    .and_then(extract_text_from_property_value);
                result.push(value);
            }
        }
        _ => {
            return Err(Error::Validation(
                "JSON extraction requires Object column".to_string(),
            ))
        }
    }

    Ok(ColumnArray::String(result))
}

/// Evaluate COALESCE function in columnar mode
///
/// COALESCE returns the first non-null value from its arguments.
/// In columnar mode, we evaluate each argument column and merge them.
async fn eval_coalesce_columnar<S: Storage + 'static>(
    args: &[TypedExpr],
    batch: &super::super::batch::Batch,
    column_cache: &mut HashMap<String, ColumnArray>,
    ctx: &ExecutionContext<S>,
) -> Result<ColumnArray, ExecutionError> {
    if args.is_empty() {
        return Err(Error::Validation(
            "COALESCE requires at least 1 argument".to_string(),
        ));
    }

    // Evaluate all argument columns
    let mut arg_columns = Vec::with_capacity(args.len());
    for arg in args {
        let col: super::super::batch::ColumnArray =
            try_eval_columnar(arg, batch, column_cache, ctx).await?;
        arg_columns.push(col);
    }

    // Merge columns: for each row, return first non-null value
    let num_rows = batch.num_rows();
    let mut result = Vec::with_capacity(num_rows);

    for row_idx in 0..num_rows {
        let mut found = None;
        for col in &arg_columns {
            if let Some(val) = col.get(row_idx) {
                found = Some(val);
                break;
            }
        }
        result.push(found);
    }

    // Convert to appropriate column type (use type of first argument)
    property_values_to_column_array(result)
}

/// Fall back to row-by-row evaluation for unsupported expressions
///
/// This is used when an expression cannot be efficiently evaluated in columnar
/// mode (e.g., complex functions, dynamic operations).
async fn eval_row_by_row_fallback<S: Storage + 'static>(
    expr: &TypedExpr,
    batch: &super::super::batch::Batch,
    ctx: &ExecutionContext<S>,
) -> Result<ColumnArray, ExecutionError> {
    let mut results = Vec::with_capacity(batch.num_rows());

    // Evaluate expression for each row
    for row in batch.iter() {
        let value = eval_expr_async(expr, &row, ctx).await?;

        // Convert literal to PropertyValue
        let prop_value = match to_property_value(&value) {
            Ok(pv) => Some(pv),
            Err(_) if matches!(value, Literal::Null) => None,
            Err(e) => {
                return Err(Error::Validation(format!(
                    "Failed to convert expression result: {}",
                    e
                )))
            }
        };

        results.push(prop_value);
    }

    // Convert to column array
    property_values_to_column_array(results)
}
