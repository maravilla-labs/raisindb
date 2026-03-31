//! Ranking window functions
//!
//! Implements ROW_NUMBER, RANK, and DENSE_RANK window functions.
//! These functions assign ordinal positions to rows within partitions.

use super::compare::compare_literals;
use crate::physical_plan::eval::eval_expr;
use crate::physical_plan::executor::Row;
use raisin_sql::analyzer::Literal;
use raisin_sql::logical_plan::WindowExpr;
use std::cmp::Ordering;

/// State for tracking rank computations across rows
#[derive(Debug)]
pub(crate) struct RankState {
    current_rank: i64,
    current_dense_rank: i64,
}

impl RankState {
    pub(crate) fn new() -> Self {
        Self {
            current_rank: 1,
            current_dense_rank: 1,
        }
    }

    /// Compute RANK() for current row
    ///
    /// RANK gives the same rank to tied rows, with gaps in the sequence.
    /// Example: 1, 1, 3, 4, 4, 6
    pub(crate) fn compute_rank(
        &mut self,
        row_idx: usize,
        result_rows: &[Row],
        window_expr: &WindowExpr,
    ) -> Literal {
        if row_idx == 0 {
            // First row in partition always gets rank 1
            self.current_rank = 1;
            return Literal::BigInt(1);
        }

        // Check if ORDER BY values changed from previous row
        let order_changed = self.order_by_changed(row_idx, result_rows, &window_expr.order_by);

        if order_changed {
            // Values changed: increment rank by number of tied rows
            self.current_rank = (row_idx + 1) as i64;
        }
        // Values same as previous: keep same rank

        Literal::BigInt(self.current_rank)
    }

    /// Compute DENSE_RANK() for current row
    ///
    /// DENSE_RANK gives the same rank to tied rows, without gaps.
    /// Example: 1, 1, 2, 3, 3, 4
    pub(crate) fn compute_dense_rank(
        &mut self,
        row_idx: usize,
        result_rows: &[Row],
        window_expr: &WindowExpr,
    ) -> Literal {
        if row_idx == 0 {
            self.current_dense_rank = 1;
            return Literal::BigInt(1);
        }

        let order_changed = self.order_by_changed(row_idx, result_rows, &window_expr.order_by);

        if order_changed {
            // Values changed: increment dense rank by 1
            self.current_dense_rank += 1;
        }

        Literal::BigInt(self.current_dense_rank)
    }

    /// Check if ORDER BY values changed from previous row to current row
    fn order_by_changed(
        &self,
        row_idx: usize,
        result_rows: &[Row],
        order_by: &[(raisin_sql::analyzer::TypedExpr, bool)],
    ) -> bool {
        if row_idx == 0 || order_by.is_empty() {
            return true;
        }

        let prev_row = &result_rows[row_idx - 1];
        let curr_row = &result_rows[row_idx];

        for (expr, _) in order_by {
            let prev_val = eval_expr(expr, prev_row).unwrap_or(Literal::Null);
            let curr_val = eval_expr(expr, curr_row).unwrap_or(Literal::Null);

            if compare_literals(&prev_val, &curr_val) != Ordering::Equal {
                return true;
            }
        }

        false
    }
}
