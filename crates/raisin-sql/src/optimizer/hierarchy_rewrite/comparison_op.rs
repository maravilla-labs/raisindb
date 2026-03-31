//! Comparison operator type for range predicates

use crate::analyzer::BinaryOperator;

/// Comparison operators for range predicates
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparisonOp {
    Lt,   // <
    LtEq, // <=
    Gt,   // >
    GtEq, // >=
}

impl ComparisonOp {
    /// Convert from BinaryOperator to ComparisonOp
    pub fn from_binary_op(op: &BinaryOperator) -> Option<Self> {
        match op {
            BinaryOperator::Lt => Some(ComparisonOp::Lt),
            BinaryOperator::LtEq => Some(ComparisonOp::LtEq),
            BinaryOperator::Gt => Some(ComparisonOp::Gt),
            BinaryOperator::GtEq => Some(ComparisonOp::GtEq),
            _ => None,
        }
    }

    /// Convert to BinaryOperator
    pub fn to_binary_op(&self) -> BinaryOperator {
        match self {
            ComparisonOp::Lt => BinaryOperator::Lt,
            ComparisonOp::LtEq => BinaryOperator::LtEq,
            ComparisonOp::Gt => BinaryOperator::Gt,
            ComparisonOp::GtEq => BinaryOperator::GtEq,
        }
    }

    /// Check if this is an upper bound operator (< or <=)
    pub fn is_upper_bound(&self) -> bool {
        matches!(self, ComparisonOp::Lt | ComparisonOp::LtEq)
    }

    /// Check if this is a lower bound operator (> or >=)
    pub fn is_lower_bound(&self) -> bool {
        matches!(self, ComparisonOp::Gt | ComparisonOp::GtEq)
    }

    /// Check if the bound is inclusive (<= or >=)
    pub fn is_inclusive(&self) -> bool {
        matches!(self, ComparisonOp::LtEq | ComparisonOp::GtEq)
    }

    /// Reverse the operator (for when value OP column instead of column OP value)
    pub fn reverse(&self) -> Self {
        match self {
            ComparisonOp::Lt => ComparisonOp::Gt,
            ComparisonOp::LtEq => ComparisonOp::GtEq,
            ComparisonOp::Gt => ComparisonOp::Lt,
            ComparisonOp::GtEq => ComparisonOp::LtEq,
        }
    }
}
