//! Error types for the REL parser and evaluator

use serde::{Deserialize, Serialize};

/// Position information in source code
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    /// Line number (1-based)
    pub line: usize,
    /// Column number (1-based)
    pub column: usize,
    /// Byte offset from start
    pub offset: usize,
}

impl Position {
    pub fn new(line: usize, column: usize, offset: usize) -> Self {
        Self {
            line,
            column,
            offset,
        }
    }
}

impl Default for Position {
    fn default() -> Self {
        Self {
            line: 1,
            column: 1,
            offset: 0,
        }
    }
}

/// Parse error with position information
#[derive(Debug, Clone, thiserror::Error, Serialize, Deserialize)]
pub enum ParseError {
    #[error("Syntax error at line {line}, column {column}: {message}")]
    SyntaxError {
        line: usize,
        column: usize,
        message: String,
    },

    #[error(
        "Unexpected token at line {line}, column {column}: expected {expected}, found {found}"
    )]
    UnexpectedToken {
        line: usize,
        column: usize,
        expected: String,
        found: String,
    },

    #[error("Unexpected end of input at line {line}, column {column}")]
    UnexpectedEof { line: usize, column: usize },

    #[error("Invalid number at line {line}, column {column}: {value}")]
    InvalidNumber {
        line: usize,
        column: usize,
        value: String,
    },

    #[error("Unterminated string at line {line}, column {column}")]
    UnterminatedString { line: usize, column: usize },

    #[error("Unknown function at line {line}, column {column}: {name}")]
    UnknownFunction {
        line: usize,
        column: usize,
        name: String,
    },
}

impl ParseError {
    pub fn syntax_error(line: usize, column: usize, message: impl Into<String>) -> Self {
        Self::SyntaxError {
            line,
            column,
            message: message.into(),
        }
    }

    pub fn unexpected_token(
        line: usize,
        column: usize,
        expected: impl Into<String>,
        found: impl Into<String>,
    ) -> Self {
        Self::UnexpectedToken {
            line,
            column,
            expected: expected.into(),
            found: found.into(),
        }
    }

    pub fn unexpected_eof(line: usize, column: usize) -> Self {
        Self::UnexpectedEof { line, column }
    }

    /// Get the line number of the error
    pub fn line(&self) -> usize {
        match self {
            Self::SyntaxError { line, .. }
            | Self::UnexpectedToken { line, .. }
            | Self::UnexpectedEof { line, .. }
            | Self::InvalidNumber { line, .. }
            | Self::UnterminatedString { line, .. }
            | Self::UnknownFunction { line, .. } => *line,
        }
    }

    /// Get the column number of the error
    pub fn column(&self) -> usize {
        match self {
            Self::SyntaxError { column, .. }
            | Self::UnexpectedToken { column, .. }
            | Self::UnexpectedEof { column, .. }
            | Self::InvalidNumber { column, .. }
            | Self::UnterminatedString { column, .. }
            | Self::UnknownFunction { column, .. } => *column,
        }
    }
}

/// Evaluation error
#[derive(Debug, Clone, thiserror::Error, Serialize, Deserialize)]
pub enum EvalError {
    #[error("Undefined variable: {0}")]
    UndefinedVariable(String),

    #[error("Graph traversal error: {0}")]
    GraphError(String),

    #[error("Property '{property}' not found on {value_type}")]
    PropertyNotFound {
        property: String,
        value_type: String,
    },

    #[error("Index {index} out of bounds for array of length {length}")]
    IndexOutOfBounds { index: i64, length: usize },

    #[error("Invalid index type: expected integer, got {0}")]
    InvalidIndexType(String),

    #[error("Type error in {operation}: expected {expected}, got {actual}")]
    TypeError {
        operation: String,
        expected: String,
        actual: String,
    },

    #[error("Unknown function: {0}")]
    UnknownFunction(String),

    #[error("Unknown method: {0}")]
    UnknownMethod(String),

    #[error("Wrong number of arguments for {function}: expected {expected}, got {actual}")]
    WrongArgCount {
        function: String,
        expected: usize,
        actual: usize,
    },

    #[error("Division by zero")]
    DivisionByZero,

    #[error("Cannot compare {left_type} with {right_type}")]
    IncomparableTypes {
        left_type: String,
        right_type: String,
    },
}

impl EvalError {
    pub fn undefined_variable(name: impl Into<String>) -> Self {
        Self::UndefinedVariable(name.into())
    }

    pub fn property_not_found(property: impl Into<String>, value_type: impl Into<String>) -> Self {
        Self::PropertyNotFound {
            property: property.into(),
            value_type: value_type.into(),
        }
    }

    pub fn index_out_of_bounds(index: i64, length: usize) -> Self {
        Self::IndexOutOfBounds { index, length }
    }

    pub fn type_error(
        operation: impl Into<String>,
        expected: impl Into<String>,
        actual: impl Into<String>,
    ) -> Self {
        Self::TypeError {
            operation: operation.into(),
            expected: expected.into(),
            actual: actual.into(),
        }
    }

    pub fn unknown_function(name: impl Into<String>) -> Self {
        Self::UnknownFunction(name.into())
    }

    pub fn unknown_method(name: impl Into<String>) -> Self {
        Self::UnknownMethod(name.into())
    }

    pub fn wrong_arg_count(function: impl Into<String>, expected: usize, actual: usize) -> Self {
        Self::WrongArgCount {
            function: function.into(),
            expected,
            actual,
        }
    }

    pub fn graph_error(message: impl Into<String>) -> Self {
        Self::GraphError(message.into())
    }
}

/// Combined error type for parse or eval failures
#[derive(Debug, Clone, thiserror::Error)]
pub enum RelError {
    #[error("Parse error: {0}")]
    Parse(#[from] ParseError),

    #[error("Evaluation error: {0}")]
    Eval(#[from] EvalError),
}
