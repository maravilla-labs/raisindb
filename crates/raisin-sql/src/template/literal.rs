//! The literal a bound value becomes: what the analyzer makes of the SQL text
//! the value is rendered to.

use crate::analyzer::literal_from_sql_value;
use crate::analyzer::Literal;
use sqlparser::ast::{Expr, Value};
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::parser::Parser;
use sqlparser::tokenizer::Token;

/// The literal `rendered` — one parameter value as the substitution renders it
/// (`'it''s'`, `42`, `1.5`, `true`, `NULL`, …) — analyzes to, or `None` when
/// the rendering is not a single literal token (a negative number is a unary
/// minus, a functions-runtime array is `ARRAY[…]`): such a value cannot be
/// bound into a template and the statement is planned from its text.
///
/// Exact by construction: the general path parses `rendered` with the
/// analyzer's dialect and converts the value with the analyzer's own rule.
/// The fast paths below are the cases whose answer that path provably gives
/// (PostgreSQL strings have no backslash escapes; a digit run is one number
/// token).
pub fn param_literal(rendered: &str) -> Option<Literal> {
    match rendered {
        "NULL" => return Some(Literal::Null),
        "true" => return Some(Literal::Boolean(true)),
        "false" => return Some(Literal::Boolean(false)),
        _ => {}
    }
    if let Some(inner) = rendered
        .strip_prefix('\'')
        .and_then(|r| r.strip_suffix('\''))
    {
        if !inner.replace("''", "").contains('\'') {
            return Some(Literal::Text(inner.replace("''", "'")));
        }
    }
    if !rendered.is_empty() && rendered.bytes().all(|b| b.is_ascii_digit()) {
        return literal_from_sql_value(&Value::Number(rendered.to_string(), false)).ok();
    }
    parsed_literal(rendered)
}

/// The general path: parse `rendered` as one expression and require a bare
/// value covering all of it.
fn parsed_literal(rendered: &str) -> Option<Literal> {
    let dialect = PostgreSqlDialect {};
    let mut parser = Parser::new(&dialect).try_with_sql(rendered).ok()?;
    let expr = parser.parse_expr().ok()?;
    if parser.peek_token().token != Token::EOF {
        return None;
    }
    match expr {
        Expr::Value(v) if !matches!(v.value, Value::Placeholder(_)) => {
            literal_from_sql_value(&v.value).ok()
        }
        _ => None,
    }
}
