//! Error types for RaisinSQL parser

use thiserror::Error;

pub type Result<T> = std::result::Result<T, ParseError>;

#[derive(Error, Debug)]
pub enum ParseError {
    #[error("SQL parser error: {0}")]
    SqlParserError(String),

    #[error("Unsupported statement type: {0}")]
    UnsupportedStatement(String),

    #[error("Invalid table for {operation}: got '{table}', expected '{expected}'")]
    InvalidTable {
        operation: String,
        table: String,
        expected: String,
    },

    #[error("Invalid RaisinDB function: {0}")]
    InvalidFunction(String),

    #[error("Function '{function}' requires {expected} argument(s), got {actual}")]
    InvalidFunctionArity {
        function: String,
        expected: String,
        actual: usize,
    },

    #[error("Unsupported function in RaisinSQL: {0}")]
    UnsupportedFunction(String),

    #[error("Transaction statement: {0}")]
    TransactionStatement(super::transaction::TransactionStatement),

    #[error("DDL statement: {0}")]
    DdlStatement(String),
}
