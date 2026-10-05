//! Which templates may be bound: every placeholder where its value steers
//! nothing before physical planning (module docs, point 2).

use crate::analyzer::{AnalyzedStatement, BinaryOperator, Expr, Literal, TypedExpr};
use sqlparser::dialect::PostgreSqlDialect;
use sqlparser::tokenizer::{Token, Tokenizer};

/// `Ok` when `stmt` is a template binding can serve; otherwise the reason it
/// is value-dependent (it is then planned per value).
pub fn check(stmt: &AnalyzedStatement) -> Result<(), String> {
    let AnalyzedStatement::Query(q) = stmt else {
        return Err("not a query".into());
    };
    if q.set_operation.is_some() || !q.ctes.is_empty() {
        return Err("set operations and CTEs are planned per value".into());
    }
    let admitted = match &q.selection {
        Some(selection) => admitted_in(selection, false)?,
        None => 0,
    };
    // Every placeholder anywhere in the statement must be one admitted in the
    // WHERE clause. Counted over the `Debug` rendering, which reaches every
    // field (projection, ORDER BY, joins, table-function arguments, …)
    // without a second enumeration to keep in step; it runs once per
    // template. A string cannot forge the pattern: `Debug` escapes its quotes.
    let everywhere = format!("{q:?}").matches("Parameter(\"").count();
    if everywhere != admitted {
        return Err(format!(
            "{} parameter(s) outside an admitted WHERE position",
            everywhere - admitted.min(everywhere)
        ));
    }
    Ok(())
}

/// The placeholders admitted in `expr`, or the reason one is not.
/// `allowed`: `expr` itself is an admitted operand position.
fn admitted_in(expr: &TypedExpr, allowed: bool) -> Result<usize, String> {
    if is_param(expr) {
        return if allowed {
            Ok(1)
        } else {
            Err("a parameter where its value steers analysis".into())
        };
    }
    match &expr.expr {
        Expr::BinaryOp { left, op, right } if is_comparison(op) => {
            let left_ok = is_param(left) && column_like(right);
            let right_ok = is_param(right) && column_like(left);
            Ok(admitted_in(left, left_ok)? + admitted_in(right, right_ok)?)
        }
        Expr::InList {
            expr: subject,
            list,
            ..
        } => {
            let ok = column_like(subject);
            let mut n = admitted_in(subject, false)?;
            for item in list {
                n += admitted_in(item, ok)?;
            }
            Ok(n)
        }
        Expr::Between {
            expr: subject,
            low,
            high,
        } => {
            let ok = column_like(subject);
            Ok(admitted_in(subject, false)? + admitted_in(low, ok)? + admitted_in(high, ok)?)
        }
        Expr::Function { name, args, .. } => {
            let path_arg = match name.to_ascii_uppercase().as_str() {
                "CHILD_OF" | "DESCENDANT_OF" | "REFERENCES" => Some(0),
                "PATH_STARTS_WITH" => Some(1),
                _ => None,
            };
            let mut n = 0;
            for (i, arg) in args.iter().enumerate() {
                n += admitted_in(arg, path_arg == Some(i))?;
            }
            Ok(n)
        }
        _ => {
            let mut n = 0;
            let mut err = None;
            expr.for_each_child(&mut |child| match admitted_in(child, false) {
                Ok(k) => n += k,
                Err(e) => err = Some(e),
            });
            err.map_or(Ok(n), Err)
        }
    }
}

fn is_param(expr: &TypedExpr) -> bool {
    matches!(expr.expr, Expr::Literal(Literal::Parameter(_)))
}

fn is_comparison(op: &BinaryOperator) -> bool {
    matches!(
        op,
        BinaryOperator::Eq
            | BinaryOperator::NotEq
            | BinaryOperator::Lt
            | BinaryOperator::LtEq
            | BinaryOperator::Gt
            | BinaryOperator::GtEq
    )
}

/// A column, or a JSON extraction of one by a literal key, optionally cast.
/// `__revision`, `__branch` and `locale` are excluded: the analyzer extracts
/// a literal compared to them into the statement (a parameter `locale` is
/// extracted into a marker before this check ever sees it).
fn column_like(expr: &TypedExpr) -> bool {
    match &expr.expr {
        Expr::Column { column, .. } => {
            !matches!(column.as_str(), "__revision" | "__branch" | "locale")
        }
        Expr::Cast { expr: inner, .. } => column_like(inner),
        Expr::JsonExtract { object, key } | Expr::JsonExtractText { object, key } => {
            matches!(object.expr, Expr::Column { .. }) && literal_key(key)
        }
        _ => false,
    }
}

fn literal_key(key: &TypedExpr) -> bool {
    match &key.expr {
        Expr::Literal(lit) => !matches!(lit, Literal::Parameter(_)),
        Expr::Cast { expr, .. } => literal_key(expr),
        _ => false,
    }
}

/// Whether every `$n` the substitution would replace is a placeholder TOKEN
/// of the template. The substitution rewrites `$n` anywhere in the text —
/// inside a string literal, a comment, a `GRAPH_TABLE` body — where the
/// parsed template sees no placeholder; such a template is value-dependent.
pub fn placeholders_are_tokens(sql: &str) -> bool {
    let bytes = sql.as_bytes();
    let scanned = bytes
        .iter()
        .enumerate()
        .filter(|(i, b)| **b == b'$' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit))
        .count();
    let dialect = PostgreSqlDialect {};
    let Ok(tokens) = Tokenizer::new(&dialect, sql).tokenize() else {
        return false;
    };
    let tokens = tokens
        .iter()
        .filter(|t| match t {
            Token::Placeholder(p) => p
                .strip_prefix('$')
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())),
            _ => false,
        })
        .count();
    scanned == tokens
}
