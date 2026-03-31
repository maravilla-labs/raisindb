//! TopN heap-based sorting
//!
//! Uses a BinaryHeap for O(N log K) performance when only the top K rows are
//! needed. Much faster than full sorting when K << N.

use super::comparison::compare_literals_vec;
use super::{eval_expr_async, ExecutionContext, ExecutionError, Row, RowStream};
use futures::stream::StreamExt;
use raisin_sql::analyzer::Literal;
use raisin_storage::Storage;
use std::cmp::Ordering;
use std::collections::BinaryHeap;

/// Wrapper for heap entries to enable custom Ord implementation
#[derive(Clone)]
pub(super) struct HeapEntry {
    /// The row data
    pub row: Row,
    /// Pre-evaluated sort expression values
    pub eval_values: Vec<Literal>,
}

impl HeapEntry {
    pub fn new(row: Row, eval_values: Vec<Literal>) -> Self {
        Self { row, eval_values }
    }
}

impl PartialEq for HeapEntry {
    fn eq(&self, other: &Self) -> bool {
        self.eval_values == other.eval_values
    }
}

impl Eq for HeapEntry {}

impl Ord for HeapEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        compare_literals_vec(&self.eval_values, &other.eval_values)
    }
}

impl PartialOrd for HeapEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Execute TopN using BinaryHeap for O(N log K) performance
///
/// Algorithm:
/// - ASCENDING (ORDER BY col ASC): Use max-heap, keep K smallest values
/// - DESCENDING (ORDER BY col DESC): Use min-heap (via Reverse), keep K largest values
pub(super) async fn execute_topn_with_heap<S: Storage>(
    mut input_stream: RowStream,
    sort_exprs: &[raisin_sql::logical_plan::SortExpr],
    limit: usize,
    ctx: &ExecutionContext<S>,
) -> Result<Vec<Row>, ExecutionError> {
    use std::cmp::Reverse;

    if limit == 0 {
        return Ok(vec![]);
    }

    let ascending = sort_exprs.first().map(|s| s.ascending).unwrap_or(true);

    tracing::info!(
        "   Using BinaryHeap TopN optimization: limit={}, direction={}",
        limit,
        if ascending { "ASC" } else { "DESC" }
    );

    if ascending {
        execute_topn_ascending(&mut input_stream, sort_exprs, limit, ctx).await
    } else {
        execute_topn_descending(&mut input_stream, sort_exprs, limit, ctx).await
    }
}

/// ASCENDING: Use max-heap to keep K smallest values
async fn execute_topn_ascending<S: Storage>(
    input_stream: &mut RowStream,
    sort_exprs: &[raisin_sql::logical_plan::SortExpr],
    limit: usize,
    ctx: &ExecutionContext<S>,
) -> Result<Vec<Row>, ExecutionError> {
    let mut heap: BinaryHeap<HeapEntry> = BinaryHeap::new();
    let mut row_count = 0;

    while let Some(row_result) = input_stream.next().await {
        let row = row_result?;
        row_count += 1;

        let mut eval_values = Vec::with_capacity(sort_exprs.len());
        for sort_expr in sort_exprs {
            let value = eval_expr_async(&sort_expr.expr, &row, ctx).await?;
            eval_values.push(value);
        }

        let entry = HeapEntry::new(row, eval_values);

        if heap.len() < limit {
            heap.push(entry);
        } else if let Some(max_entry) = heap.peek() {
            if compare_literals_vec(&entry.eval_values, &max_entry.eval_values) == Ordering::Less {
                heap.pop();
                heap.push(entry);
            }
        }
    }

    tracing::info!("   Processed {} rows, kept top {}", row_count, heap.len());

    let mut results: Vec<HeapEntry> = heap.into_iter().collect();
    results.sort_by(|a, b| compare_literals_vec(&a.eval_values, &b.eval_values));

    Ok(results.into_iter().map(|e| e.row).collect())
}

/// DESCENDING: Use min-heap (via Reverse) to keep K largest values
async fn execute_topn_descending<S: Storage>(
    input_stream: &mut RowStream,
    sort_exprs: &[raisin_sql::logical_plan::SortExpr],
    limit: usize,
    ctx: &ExecutionContext<S>,
) -> Result<Vec<Row>, ExecutionError> {
    use std::cmp::Reverse;

    let mut heap: BinaryHeap<Reverse<HeapEntry>> = BinaryHeap::new();
    let mut row_count = 0;

    while let Some(row_result) = input_stream.next().await {
        let row = row_result?;
        row_count += 1;

        let mut eval_values = Vec::with_capacity(sort_exprs.len());
        for sort_expr in sort_exprs {
            let value = eval_expr_async(&sort_expr.expr, &row, ctx).await?;
            eval_values.push(value);
        }

        let entry = HeapEntry::new(row, eval_values);

        if heap.len() < limit {
            heap.push(Reverse(entry));
        } else if let Some(Reverse(min_entry)) = heap.peek() {
            if compare_literals_vec(&entry.eval_values, &min_entry.eval_values) == Ordering::Greater
            {
                heap.pop();
                heap.push(Reverse(entry));
            }
        }
    }

    tracing::info!("   Processed {} rows, kept top {}", row_count, heap.len());

    let mut results: Vec<HeapEntry> = heap.into_iter().map(|Reverse(e)| e).collect();
    results.sort_by(|a, b| compare_literals_vec(&a.eval_values, &b.eval_values).reverse());

    Ok(results.into_iter().map(|e| e.row).collect())
}
