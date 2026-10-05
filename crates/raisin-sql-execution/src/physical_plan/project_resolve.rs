//! RESOLVE over a chunk of rows: one frontier walk for many rows.
//!
//! RESOLVE's memo already reads a target shared by every row once per
//! statement. What it cannot do row by row is batch the targets rows do NOT
//! share — a listing of 50 articles, each with its own author and image, made
//! 50 small batched reads. The projection therefore buffers rows and resolves
//! a top-level `RESOLVE(...)` for the whole buffer at once
//! (`eval_resolve_rows` → `ReferenceResolver::resolve_json_many`), so each
//! level is ONE batched read across the chunk.
//!
//! The buffer grows 1, 2, 4, … up to [`RESOLVE_CHUNK`] rows. The projection
//! cannot see a `LIMIT` above it, and a fixed 64-row buffer under `LIMIT 1`
//! would resolve 63 rows nobody reads; growing from one row keeps a point
//! query at exactly one row and any limited query at most ~2× the rows it
//! consumes, while a long listing reaches full chunks after six.
//!
//! Only a projection expression that IS a RESOLVE call is chunked. One nested
//! in another expression (`RESOLVE(...)->>'k'`) is evaluated per row as
//! before — its frontier is still read one batch per level.

use super::batch::ColumnArray;
use super::batch_execution::property_values_to_column_array;
use super::eval::eval_resolve_rows;
use super::executor::{ExecutionContext, ExecutionError, Row, RowStream};
use super::project::project_row;
use async_stream::try_stream;
use futures::StreamExt;
use raisin_models::nodes::properties::PropertyValue;
use raisin_sql::analyzer::{Expr, TypedExpr};
use raisin_sql::logical_plan::ProjectionExpr;
use raisin_storage::Storage;

/// The most rows one RESOLVE chunk holds.
pub(crate) const RESOLVE_CHUNK: usize = 64;

/// The arguments of `expr` when it is a RESOLVE call.
pub(crate) fn resolve_args(expr: &TypedExpr) -> Option<&[TypedExpr]> {
    match &expr.expr {
        Expr::Function { name, args, .. } if name.eq_ignore_ascii_case("RESOLVE") => Some(args),
        _ => None,
    }
}

/// Positions of the projection expressions that are a RESOLVE call.
pub(crate) fn chunked_resolve_exprs(exprs: &[ProjectionExpr]) -> Vec<usize> {
    exprs
        .iter()
        .enumerate()
        .filter(|(_, e)| resolve_args(&e.expr).is_some())
        .map(|(i, _)| i)
        .collect()
}

/// The next buffer size: double, up to [`RESOLVE_CHUNK`].
pub(crate) fn next_chunk(current: usize) -> usize {
    (current.max(1) * 2).min(RESOLVE_CHUNK)
}

/// The projection, reading its input a chunk at a time and evaluating the
/// RESOLVE expressions at `resolve_at` once per chunk.
pub(crate) fn project_chunked<S: Storage + 'static>(
    mut input: RowStream,
    exprs: std::sync::Arc<[ProjectionExpr]>,
    resolve_at: Vec<usize>,
    ctx: ExecutionContext<S>,
) -> RowStream {
    Box::pin(try_stream! {
        let mut chunk = 1usize;
        let mut exhausted = false;
        while !exhausted {
            let mut rows: Vec<Row> = Vec::with_capacity(chunk);
            while rows.len() < chunk {
                match input.next().await {
                    Some(row) => rows.push(row?),
                    None => {
                        exhausted = true;
                        break;
                    }
                }
            }
            if rows.is_empty() {
                break;
            }

            let mut precomputed: Vec<Vec<Option<PropertyValue>>> = vec![vec![None; exprs.len()]; rows.len()];
            for &at in &resolve_at {
                let Some(args) = resolve_args(&exprs[at].expr) else {
                    continue;
                };
                let values = eval_resolve_rows(args, &rows, &ctx).await?;
                for (slots, value) in precomputed.iter_mut().zip(values) {
                    slots[at] = Some(value);
                }
            }
            for (row, mut slots) in rows.iter().zip(precomputed) {
                yield project_row(&exprs, row, &ctx, &mut slots).await?;
            }
            chunk = next_chunk(chunk);
        }
    })
}

/// A RESOLVE projection over a whole batch (the columnar projection), in
/// chunks of [`RESOLVE_CHUNK`] rows.
pub(crate) async fn resolve_column<S: Storage>(
    args: &[TypedExpr],
    rows: &[Row],
    ctx: &ExecutionContext<S>,
) -> Result<ColumnArray, ExecutionError> {
    let mut results = Vec::with_capacity(rows.len());
    for chunk in rows.chunks(RESOLVE_CHUNK) {
        results.extend(
            eval_resolve_rows(args, chunk, ctx)
                .await?
                .into_iter()
                .map(Some),
        );
    }
    property_values_to_column_array(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_grow_from_one_row() {
        let mut sizes = vec![1];
        while *sizes.last().unwrap() < RESOLVE_CHUNK {
            sizes.push(next_chunk(*sizes.last().unwrap()));
        }
        assert_eq!(sizes, vec![1, 2, 4, 8, 16, 32, 64]);
        assert_eq!(next_chunk(RESOLVE_CHUNK), RESOLVE_CHUNK);
    }
}
