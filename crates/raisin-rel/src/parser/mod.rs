//! REL expression parser

mod common;
mod expr;
mod literal;

use crate::ast::Expr;
use crate::error::ParseError;
use common::{get_position, ws0, Span};

/// Parse an expression string into an AST
pub fn parse(input: &str) -> Result<Expr, ParseError> {
    let span = Span::new(input);

    // Skip leading whitespace
    let (span, _) = ws0(span).map_err(|_| ParseError::syntax_error(1, 1, "Invalid input"))?;

    // Parse the expression
    let (remaining, expr) = expr::expr(span).map_err(|e| match e {
        nom::Err::Error(err) | nom::Err::Failure(err) => {
            let (line, column) = get_position(&err.input);
            let found = err
                .input
                .fragment()
                .chars()
                .next()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "end of input".to_string());
            ParseError::syntax_error(line, column, format!("Unexpected: {}", found))
        }
        nom::Err::Incomplete(_) => ParseError::unexpected_eof(1, 1),
    })?;

    // Skip trailing whitespace
    let (remaining, _) =
        ws0(remaining).map_err(|_| ParseError::syntax_error(1, 1, "Invalid input"))?;

    // Check for remaining input
    if !remaining.fragment().is_empty() {
        let (line, column) = get_position(&remaining);
        return Err(ParseError::syntax_error(
            line,
            column,
            format!("Unexpected trailing input: {}", remaining.fragment()),
        ));
    }

    Ok(expr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{BinOp, Literal};

    #[test]
    fn test_parse_simple() {
        let expr = parse("42").unwrap();
        assert!(matches!(expr, Expr::Literal(Literal::Integer(42))));
    }

    #[test]
    fn test_parse_comparison() {
        let expr = parse("x > 10").unwrap();
        assert!(matches!(expr, Expr::BinaryOp { op: BinOp::Gt, .. }));
    }

    #[test]
    fn test_parse_complex() {
        let expr = parse("input.value > 10 && input.status == 'active'").unwrap();
        assert!(matches!(expr, Expr::BinaryOp { op: BinOp::And, .. }));
    }

    #[test]
    fn test_parse_error() {
        let err = parse("42 +").unwrap_err();
        assert!(matches!(err, ParseError::SyntaxError { .. }));
    }

    #[test]
    fn test_parse_whitespace() {
        let expr = parse("  42  ").unwrap();
        assert!(matches!(expr, Expr::Literal(Literal::Integer(42))));
    }

    #[test]
    fn test_parse_trailing_error() {
        let err = parse("42 garbage").unwrap_err();
        assert!(matches!(err, ParseError::SyntaxError { .. }));
    }
}
