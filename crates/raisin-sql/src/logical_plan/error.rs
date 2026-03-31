//! Logical planning errors

use thiserror::Error;

pub type Result<T> = std::result::Result<T, PlanError>;

#[derive(Debug, Error)]
pub enum PlanError {
    #[error("Table not found: {0}")]
    TableNotFound(String),

    #[error("Column not found: {0}")]
    ColumnNotFound(String),

    #[error("Invalid plan: {0}")]
    InvalidPlan(String),

    #[error("Unsupported operation: {0}")]
    UnsupportedOperation(String),

    #[error("Analysis error: {0}")]
    AnalysisError(#[from] crate::analyzer::AnalysisError),
}
