// SPDX-License-Identifier: BSL-1.1

//! Error types for Cypher parser

/// Parser error with position information
#[derive(Debug, thiserror::Error, Clone, PartialEq)]
pub enum ParseError {
    /// Syntax error with position
    #[error("Syntax error at line {line}, column {column}: {message}")]
    SyntaxError {
        line: usize,
        column: usize,
        message: String,
    },

    /// Unexpected token
    #[error(
        "Unexpected token at line {line}, column {column}: expected {expected}, found {found}"
    )]
    UnexpectedToken {
        line: usize,
        column: usize,
        expected: String,
        found: String,
    },

    /// Invalid syntax
    #[error("Invalid {what} at line {line}, column {column}: {message}")]
    InvalidSyntax {
        line: usize,
        column: usize,
        what: String,
        message: String,
    },

    /// Unexpected end of input
    #[error("Unexpected end of input at line {line}, column {column}: expected {expected}")]
    UnexpectedEof {
        line: usize,
        column: usize,
        expected: String,
    },

    /// Incomplete parsing
    #[error("Failed to parse complete input: {0}")]
    Incomplete(String),
}

impl ParseError {
    /// Create a syntax error with position
    pub fn syntax(line: usize, column: usize, message: impl Into<String>) -> Self {
        Self::SyntaxError {
            line,
            column,
            message: message.into(),
        }
    }

    /// Create an unexpected token error
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

    /// Create an invalid syntax error
    pub fn invalid(
        line: usize,
        column: usize,
        what: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::InvalidSyntax {
            line,
            column,
            what: what.into(),
            message: message.into(),
        }
    }

    /// Create an unexpected EOF error
    pub fn unexpected_eof(line: usize, column: usize, expected: impl Into<String>) -> Self {
        Self::UnexpectedEof {
            line,
            column,
            expected: expected.into(),
        }
    }
}

/// Result type for parser operations
pub type Result<T> = std::result::Result<T, ParseError>;
