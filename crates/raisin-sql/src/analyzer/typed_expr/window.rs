//! Window function types and frame specifications
//!
//! Defines window functions (ROW_NUMBER, RANK, aggregate-as-window) and
//! window frame specifications (ROWS BETWEEN, RANGE BETWEEN).

use super::expressions::TypedExpr;
use crate::analyzer::types::DataType;

/// Window function types
#[derive(Debug, Clone)]
pub enum WindowFunction {
    // Ranking functions
    RowNumber,
    Rank,
    DenseRank,

    // Aggregate functions used as window functions
    Count,
    Sum(Box<TypedExpr>),
    Avg(Box<TypedExpr>),
    Min(Box<TypedExpr>),
    Max(Box<TypedExpr>),
}

impl WindowFunction {
    /// Get the return type for this window function
    pub fn return_type(&self) -> DataType {
        match self {
            WindowFunction::RowNumber => DataType::BigInt,
            WindowFunction::Rank => DataType::BigInt,
            WindowFunction::DenseRank => DataType::BigInt,
            WindowFunction::Count => DataType::BigInt,
            WindowFunction::Sum(expr) => {
                // Sum returns the same type as input, or BigInt for integers
                match expr.data_type.base_type() {
                    DataType::Int | DataType::BigInt => DataType::BigInt,
                    DataType::Double => DataType::Double,
                    _ => DataType::BigInt,
                }
            }
            WindowFunction::Avg(_) => DataType::Double, // Average always returns double
            WindowFunction::Min(expr) => expr.data_type.clone(),
            WindowFunction::Max(expr) => expr.data_type.clone(),
        }
    }
}

/// Window frame specification (ROWS BETWEEN / RANGE BETWEEN)
#[derive(Debug, Clone, PartialEq)]
pub struct WindowFrame {
    pub mode: FrameMode,
    pub start: FrameBound,
    pub end: Option<FrameBound>,
}

impl WindowFrame {
    /// Validate that the frame specification is valid
    pub fn validate(&self) -> Result<(), String> {
        // If end is specified, start must be <= end
        if let Some(end) = &self.end {
            if !self.start.is_before_or_equal(end) {
                return Err(format!(
                    "Invalid frame: start {:?} is after end {:?}",
                    self.start, end
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameMode {
    Rows,  // ROWS BETWEEN
    Range, // RANGE BETWEEN
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameBound {
    UnboundedPreceding,
    Preceding(usize),
    CurrentRow,
    Following(usize),
    UnboundedFollowing,
}

impl FrameBound {
    /// Check if this bound is before or equal to another bound
    fn is_before_or_equal(&self, other: &FrameBound) -> bool {
        use FrameBound::*;
        match (self, other) {
            (UnboundedPreceding, _) => true,
            (_, UnboundedFollowing) => true,
            (Preceding(_), CurrentRow | Following(_) | Preceding(_)) => true,
            (CurrentRow, CurrentRow | Following(_)) => true,
            (Following(a), Following(b)) => a <= b,
            _ => false,
        }
    }
}
