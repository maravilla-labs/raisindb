// SPDX-License-Identifier: BSL-1.1

//! Internal parser implementation using nom.
//!
//! This module contains the actual nom parser combinators that implement
//! the openCypher grammar. It's intentionally kept private - users should
//! use the public API in lib.rs instead.
//!
//! The implementation is organized into sub-modules for maintainability.

mod clause;
mod common;
mod expr;
mod literal;
mod pattern;

// Re-export the main parsing functions from the mod module
pub(crate) use self::clause::{
    query as parse_query_internal, statement as parse_statement_internal,
};
pub(crate) use self::expr::expr as parse_expr_internal;
pub(crate) use self::pattern::{
    graph_pattern as parse_pattern_internal, path_pattern as parse_path_internal,
};

use crate::ast::{Expr, GraphPattern, PathPattern, Query, Statement};
use crate::error::{ParseError, Result};
use common::Span;

/// Parse a complete Cypher query
pub(crate) fn parse_query(input: &str) -> Result<Query> {
    let span = Span::new(input);

    match parse_query_internal(span) {
        Ok((remaining, query)) => {
            // Check if there's unparsed input
            let remaining_str = remaining.fragment().trim();
            if !remaining_str.is_empty() {
                return Err(ParseError::syntax(
                    remaining.location_line() as usize,
                    remaining.get_column(),
                    format!("Unexpected input after query: '{}'", remaining_str),
                ));
            }
            Ok(query)
        }
        Err(e) => convert_nom_error(input, e),
    }
}

/// Parse a Cypher statement
pub(crate) fn parse_statement(input: &str) -> Result<Statement> {
    let span = Span::new(input);

    match parse_statement_internal(span) {
        Ok((remaining, stmt)) => {
            // Check if there's unparsed input
            let remaining_str = remaining.fragment().trim();
            if !remaining_str.is_empty() {
                return Err(ParseError::syntax(
                    remaining.location_line() as usize,
                    remaining.get_column(),
                    format!("Unexpected input after statement: '{}'", remaining_str),
                ));
            }
            Ok(stmt)
        }
        Err(e) => convert_nom_error(input, e),
    }
}

/// Parse a Cypher expression
pub(crate) fn parse_expr(input: &str) -> Result<Expr> {
    let span = Span::new(input);

    match parse_expr_internal(span) {
        Ok((remaining, e)) => {
            // Check if there's unparsed input
            let remaining_str = remaining.fragment().trim();
            if !remaining_str.is_empty() {
                return Err(ParseError::syntax(
                    remaining.location_line() as usize,
                    remaining.get_column(),
                    format!("Unexpected input after expression: '{}'", remaining_str),
                ));
            }
            Ok(e)
        }
        Err(e) => convert_nom_error(input, e),
    }
}

/// Parse a graph pattern
pub(crate) fn parse_pattern(input: &str) -> Result<GraphPattern> {
    let span = Span::new(input);

    match parse_pattern_internal(span) {
        Ok((remaining, p)) => {
            // Check if there's unparsed input
            let remaining_str = remaining.fragment().trim();
            if !remaining_str.is_empty() {
                return Err(ParseError::syntax(
                    remaining.location_line() as usize,
                    remaining.get_column(),
                    format!("Unexpected input after pattern: '{}'", remaining_str),
                ));
            }
            Ok(p)
        }
        Err(e) => convert_nom_error(input, e),
    }
}

/// Parse a single path pattern
pub(crate) fn parse_path(input: &str) -> Result<PathPattern> {
    let span = Span::new(input);

    match parse_path_internal(span) {
        Ok((remaining, p)) => {
            // Check if there's unparsed input
            let remaining_str = remaining.fragment().trim();
            if !remaining_str.is_empty() {
                return Err(ParseError::syntax(
                    remaining.location_line() as usize,
                    remaining.get_column(),
                    format!("Unexpected input after path: '{}'", remaining_str),
                ));
            }
            Ok(p)
        }
        Err(e) => convert_nom_error(input, e),
    }
}

/// Convert nom error to our ParseError type
fn convert_nom_error<T>(_input: &str, error: nom::Err<nom::error::Error<Span>>) -> Result<T> {
    match error {
        nom::Err::Error(e) | nom::Err::Failure(e) => {
            let line = e.input.location_line() as usize;
            let column = e.input.get_column();
            let fragment = e.input.fragment();

            // Try to provide a helpful message based on the error kind
            let message = match e.code {
                nom::error::ErrorKind::Tag => {
                    if fragment.is_empty() {
                        "unexpected end of input".to_string()
                    } else {
                        format!(
                            "unexpected token '{}'",
                            fragment.chars().take(20).collect::<String>()
                        )
                    }
                }
                nom::error::ErrorKind::Char => {
                    if fragment.is_empty() {
                        "unexpected end of input".to_string()
                    } else {
                        format!(
                            "expected different character, found '{}'",
                            fragment.chars().next().unwrap()
                        )
                    }
                }
                nom::error::ErrorKind::Eof => "unexpected end of input".to_string(),
                nom::error::ErrorKind::Many1 => "expected at least one element".to_string(),
                _ => {
                    format!("parse error: {:?}", e.code)
                }
            };

            Err(ParseError::syntax(line, column, message))
        }
        nom::Err::Incomplete(_) => Err(ParseError::Incomplete(
            "Parser requires more input (this should not happen with complete parsers)".to_string(),
        )),
    }
}
