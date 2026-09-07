//! UNION / INTERSECT / EXCEPT execution.
//!
//! Output columns are the LEFT side's names; right-side rows are renamed to
//! them by position (SQL semantics: `SELECT name FROM a UNION SELECT title
//! FROM b` yields a `name` column). Duplicate detection hashes the row's
//! values in column order, so two rows are "the same" when their values
//! agree position by position — what the standard says, and independent of
//! the column names each side happened to produce.

use super::distinct::row_values_hash_key;
use super::executor::{execute_plan, ExecutionContext, ExecutionError, Row, RowStream};
use super::operators::PhysicalPlan;
use futures::stream::{self, StreamExt};
use raisin_error::Error;
use raisin_sql::analyzer::SetOperationKind;
use raisin_storage::Storage;
use std::collections::{HashMap, HashSet};

/// Rename a row's columns positionally to `columns`. Extra input columns are
/// dropped; missing ones are NOT invented (the analyzer already enforced
/// equal column counts).
fn align_row(row: Row, columns: &[String]) -> Row {
    let mut out = Row::new();
    for (name, (_, value)) in columns.iter().zip(row.columns.into_iter()) {
        out.insert(name.clone(), value);
    }
    out
}

async fn collect_aligned(
    stream: &mut RowStream,
    columns: &[String],
) -> Result<Vec<Row>, ExecutionError> {
    let mut rows = Vec::new();
    while let Some(row) = stream.next().await {
        rows.push(align_row(row?, columns));
    }
    Ok(rows)
}

pub async fn execute_set_operation<
    S: Storage + raisin_storage::transactional::TransactionalStorage + 'static,
>(
    plan: &PhysicalPlan,
    ctx: &ExecutionContext<S>,
) -> Result<RowStream, ExecutionError> {
    let (left, right, kind, all, columns) = match plan {
        PhysicalPlan::SetOperation {
            left,
            right,
            kind,
            all,
            columns,
        } => (left, right, *kind, *all, columns),
        _ => {
            return Err(Error::Validation(
                "Invalid plan for set operation".to_string(),
            ))
        }
    };

    let mut left_stream = execute_plan(left, ctx).await?;
    let mut right_stream = execute_plan(right, ctx).await?;

    let output: Vec<Row> = match kind {
        SetOperationKind::Union => {
            let mut out = collect_aligned(&mut left_stream, columns).await?;
            out.extend(collect_aligned(&mut right_stream, columns).await?);
            if all {
                out
            } else {
                let mut seen = HashSet::new();
                out.into_iter()
                    .filter(|r| seen.insert(row_values_hash_key(r)))
                    .collect()
            }
        }
        SetOperationKind::Intersect => {
            // Multiset semantics for ALL: a row appears min(l, r) times.
            let right_rows = collect_aligned(&mut right_stream, columns).await?;
            let mut right_counts: HashMap<String, usize> = HashMap::new();
            for r in &right_rows {
                *right_counts.entry(row_values_hash_key(r)).or_insert(0) += 1;
            }
            let left_rows = collect_aligned(&mut left_stream, columns).await?;
            let mut out = Vec::new();
            for row in left_rows {
                let key = row_values_hash_key(&row);
                match right_counts.get_mut(&key) {
                    Some(n) if *n > 0 => {
                        if all {
                            *n -= 1;
                        } else {
                            *n = 0;
                        }
                        out.push(row);
                    }
                    _ => {}
                }
            }
            out
        }
        SetOperationKind::Except => {
            // ALL: each right occurrence cancels ONE left occurrence.
            let right_rows = collect_aligned(&mut right_stream, columns).await?;
            let mut right_counts: HashMap<String, usize> = HashMap::new();
            for r in &right_rows {
                *right_counts.entry(row_values_hash_key(r)).or_insert(0) += 1;
            }
            let left_rows = collect_aligned(&mut left_stream, columns).await?;
            let mut out = Vec::new();
            let mut emitted = HashSet::new();
            for row in left_rows {
                let key = row_values_hash_key(&row);
                if let Some(n) = right_counts.get_mut(&key) {
                    if *n > 0 {
                        if all {
                            *n -= 1;
                        }
                        continue;
                    }
                }
                if all || emitted.insert(key) {
                    out.push(row);
                }
            }
            out
        }
    };

    tracing::debug!(
        "{}{}: {} row(s)",
        kind.keyword(),
        if all { " ALL" } else { "" },
        output.len()
    );

    Ok(Box::pin(stream::iter(output.into_iter().map(Ok))))
}
