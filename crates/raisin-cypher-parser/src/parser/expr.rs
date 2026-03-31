// SPDX-License-Identifier: BSL-1.1

//! Expression parsing with proper operator precedence
//!
//! Operator precedence (lowest to highest):
//! 1. OR
//! 2. XOR
//! 3. AND
//! 4. NOT
//! 5. Comparison (=, <>, <, <=, >, >=)
//! 6. String ops (STARTS WITH, ENDS WITH, CONTAINS, =~)
//! 7. IN
//! 8. Addition/Subtraction (+, -)
//! 9. Multiplication/Division/Modulo (*, /, %)
//! 10. Power (^)
//! 11. Unary (+, -, IS NULL, IS NOT NULL)
//! 12. Property access (.)
//! 13. Function calls, atoms (literals, variables, parameters)

use super::common::{any_identifier, comma_sep0, keyword, parameter, ws0, ws_token, PResult, Span};
use super::literal::literal;
use crate::ast::{BinOp, Expr, UnOp};
use nom::{
    branch::alt,
    character::complete::char,
    combinator::{map, opt},
    sequence::{delimited, pair, preceded, separated_pair},
    Parser,
};

/// Parse an expression
pub fn expr(input: Span) -> PResult<Expr> {
    or_expr(input)
}

/// OR expression (lowest precedence)
fn or_expr(input: Span) -> PResult<Expr> {
    let (input, first) = xor_expr(input)?;
    let (input, rest) =
        nom::multi::many0(preceded(ws_token(keyword("OR")), xor_expr)).parse(input)?;

    Ok((
        input,
        rest.into_iter()
            .fold(first, |acc, e| Expr::binary(acc, BinOp::Or, e)),
    ))
}

/// XOR expression
fn xor_expr(input: Span) -> PResult<Expr> {
    let (input, first) = and_expr(input)?;
    let (input, rest) =
        nom::multi::many0(preceded(ws_token(keyword("XOR")), and_expr)).parse(input)?;

    Ok((
        input,
        rest.into_iter()
            .fold(first, |acc, e| Expr::binary(acc, BinOp::Xor, e)),
    ))
}

/// AND expression
fn and_expr(input: Span) -> PResult<Expr> {
    let (input, first) = not_expr(input)?;
    let (input, rest) =
        nom::multi::many0(preceded(ws_token(keyword("AND")), not_expr)).parse(input)?;

    Ok((
        input,
        rest.into_iter()
            .fold(first, |acc, e| Expr::binary(acc, BinOp::And, e)),
    ))
}

/// NOT expression
fn not_expr(input: Span) -> PResult<Expr> {
    alt((
        map(preceded(ws_token(keyword("NOT")), not_expr), |e| {
            Expr::unary(UnOp::Not, e)
        }),
        comparison_expr,
    ))
    .parse(input)
}

/// Comparison expression (=, <>, <, <=, >, >=)
fn comparison_expr(input: Span) -> PResult<Expr> {
    let (input, first) = string_op_expr(input)?;

    // Try to parse comparison operator
    let mut comp_op = alt((
        map(ws_token(nom::bytes::complete::tag("<>")), |_| BinOp::Neq),
        map(ws_token(nom::bytes::complete::tag("!=")), |_| BinOp::Neq),
        map(ws_token(nom::bytes::complete::tag("<=")), |_| BinOp::Lte),
        map(ws_token(nom::bytes::complete::tag(">=")), |_| BinOp::Gte),
        map(ws_token(char('<')), |_| BinOp::Lt),
        map(ws_token(char('>')), |_| BinOp::Gt),
        map(ws_token(char('=')), |_| BinOp::Eq),
    ));

    if let Ok((input, op)) = comp_op.parse(input) {
        let (input, second) = string_op_expr(input)?;
        Ok((input, Expr::binary(first, op, second)))
    } else {
        Ok((input, first))
    }
}

/// String operations (STARTS WITH, ENDS WITH, CONTAINS, =~)
fn string_op_expr(input: Span) -> PResult<Expr> {
    let (input, first) = in_expr(input)?;

    // Try string operators
    let mut string_op = alt((
        map(
            (ws_token(keyword("STARTS")), ws_token(keyword("WITH"))),
            |_| BinOp::StartsWith,
        ),
        map(
            (ws_token(keyword("ENDS")), ws_token(keyword("WITH"))),
            |_| BinOp::EndsWith,
        ),
        map(ws_token(keyword("CONTAINS")), |_| BinOp::Contains),
        map(ws_token(nom::bytes::complete::tag("=~")), |_| {
            BinOp::RegexMatch
        }),
    ));

    if let Ok((input, op)) = string_op.parse(input) {
        let (input, second) = in_expr(input)?;
        Ok((input, Expr::binary(first, op, second)))
    } else {
        Ok((input, first))
    }
}

/// IN expression
fn in_expr(input: Span) -> PResult<Expr> {
    let (input, first) = add_expr(input)?;

    if let Ok((input, _)) = ws_token(keyword("IN")).parse(input) {
        let (input, second) = add_expr(input)?;
        Ok((input, Expr::binary(first, BinOp::In, second)))
    } else {
        Ok((input, first))
    }
}

/// Addition/Subtraction
fn add_expr(input: Span) -> PResult<Expr> {
    let (input, first) = mul_expr(input)?;
    let (input, rest) = nom::multi::many0(pair(
        alt((
            map(ws_token(char('+')), |_| BinOp::Add),
            map(ws_token(char('-')), |_| BinOp::Sub),
        )),
        mul_expr,
    ))
    .parse(input)?;

    Ok((
        input,
        rest.into_iter()
            .fold(first, |acc, (op, e)| Expr::binary(acc, op, e)),
    ))
}

/// Multiplication/Division/Modulo
fn mul_expr(input: Span) -> PResult<Expr> {
    let (input, first) = pow_expr(input)?;
    let (input, rest) = nom::multi::many0(pair(
        alt((
            map(ws_token(char('*')), |_| BinOp::Mul),
            map(ws_token(char('/')), |_| BinOp::Div),
            map(ws_token(char('%')), |_| BinOp::Mod),
        )),
        pow_expr,
    ))
    .parse(input)?;

    Ok((
        input,
        rest.into_iter()
            .fold(first, |acc, (op, e)| Expr::binary(acc, op, e)),
    ))
}

/// Power expression
fn pow_expr(input: Span) -> PResult<Expr> {
    let (input, first) = unary_expr(input)?;

    if let Ok((input, _)) = ws_token(char('^')).parse(input) {
        // Right-associative
        let (input, second) = pow_expr(input)?;
        Ok((input, Expr::binary(first, BinOp::Pow, second)))
    } else {
        Ok((input, first))
    }
}

/// Unary expression (+, -, IS NULL, IS NOT NULL)
fn unary_expr(input: Span) -> PResult<Expr> {
    alt((
        map(preceded(ws_token(char('+')), unary_expr), |e| {
            Expr::unary(UnOp::Plus, e)
        }),
        map(preceded(ws_token(char('-')), unary_expr), |e| {
            Expr::unary(UnOp::Minus, e)
        }),
        postfix_expr,
    ))
    .parse(input)
}

/// Postfix expression (property access, IS NULL, IS NOT NULL)
fn postfix_expr(input: Span) -> PResult<Expr> {
    let (input, mut expr) = atom_expr(input)?;

    // Parse postfix operators
    let mut current = input;
    loop {
        // Try property access
        if let Ok((rest, _)) = ws_token(char('.')).parse(current) {
            if let Ok((rest, prop)) = any_identifier(rest) {
                expr = Expr::property(expr, prop);
                current = rest;
                continue;
            }
        }

        // Try IS NOT NULL
        if let Ok((rest, _)) = (
            ws_token(keyword("IS")),
            ws_token(keyword("NOT")),
            ws_token(keyword("NULL")),
        )
            .parse(current)
        {
            expr = Expr::unary(UnOp::IsNotNull, expr);
            current = rest;
            continue;
        }

        // Try IS NULL
        if let Ok((rest, _)) = (ws_token(keyword("IS")), ws_token(keyword("NULL"))).parse(current) {
            expr = Expr::unary(UnOp::IsNull, expr);
            current = rest;
            continue;
        }

        break;
    }

    Ok((current, expr))
}

/// Atom expression (literals, variables, parameters, function calls, lists, maps, parenthesized)
fn atom_expr(input: Span) -> PResult<Expr> {
    alt((
        map(literal, Expr::Literal),
        map(parameter, Expr::Parameter),
        function_call,
        list_expr,
        map_expr,
        case_expr,
        delimited(ws_token(char('(')), expr, ws_token(char(')'))),
        map(any_identifier, Expr::Variable),
    ))
    .parse(input)
}

/// Function call: name(args) or name(DISTINCT args)
fn function_call(input: Span) -> PResult<Expr> {
    let (input, name) = any_identifier(input)?;
    let (input, _) = ws0(input)?;

    // Check if followed by '(' - if not, it's just a variable
    let lookahead = opt(char('(')).parse(input)?;
    if lookahead.1.is_none() {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Char,
        )));
    }

    let (input, _) = char('(').parse(input)?;
    let (input, _) = ws0(input)?;

    // Check for DISTINCT
    let (input, distinct) = opt(keyword("DISTINCT")).parse(input)?;
    let distinct = distinct.is_some();

    if distinct {
        let (_input, _) = ws0(input)?;
    }

    // Parse arguments
    let (input, args) = comma_sep0(expr).parse(input)?;
    let (input, _) = ws0(input)?;
    let (input, _) = char(')').parse(input)?;

    Ok((
        input,
        Expr::FunctionCall {
            name,
            distinct,
            args,
        },
    ))
}

/// List expression: [expr, ...]
fn list_expr(input: Span) -> PResult<Expr> {
    delimited(
        ws_token(char('[')),
        map(comma_sep0(expr), Expr::List),
        ws_token(char(']')),
    )
    .parse(input)
}

/// Map expression: {key: value, ...}
fn map_expr(input: Span) -> PResult<Expr> {
    delimited(
        ws_token(char('{')),
        map(
            comma_sep0(separated_pair(any_identifier, ws_token(char(':')), expr)),
            Expr::Map,
        ),
        ws_token(char('}')),
    )
    .parse(input)
}

/// CASE expression
fn case_expr(input: Span) -> PResult<Expr> {
    let (input, _) = ws_token(keyword("CASE")).parse(input)?;

    // Optional operand for simple CASE
    let (input, operand) = opt(expr).parse(input)?;
    let (input, _) = ws0(input)?;

    // WHEN branches
    let (input, when_branches) = nom::multi::many1((
        preceded(ws_token(keyword("WHEN")), expr),
        preceded(ws_token(keyword("THEN")), expr),
    ))
    .parse(input)?;

    // Optional ELSE
    let (input, else_branch) = opt(preceded(ws_token(keyword("ELSE")), expr)).parse(input)?;

    let (input, _) = ws_token(keyword("END")).parse(input)?;

    Ok((
        input,
        Expr::Case {
            operand: operand.map(Box::new),
            when_branches,
            else_branch: else_branch.map(Box::new),
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Literal;

    fn span(s: &str) -> Span {
        Span::new(s)
    }

    #[test]
    fn test_literal() {
        let result = expr(span("123")).unwrap();
        assert!(matches!(result.1, Expr::Literal(Literal::Integer(123))));
    }

    #[test]
    fn test_variable() {
        let result = expr(span("name")).unwrap();
        assert!(matches!(result.1, Expr::Variable(s) if s == "name"));
    }

    #[test]
    fn test_parameter() {
        let result = expr(span("$param")).unwrap();
        assert!(matches!(result.1, Expr::Parameter(s) if s == "param"));
    }

    #[test]
    fn test_property_access() {
        let result = expr(span("person.name")).unwrap();
        match result.1 {
            Expr::Property { expr, property } => {
                assert!(matches!(*expr, Expr::Variable(s) if s == "person"));
                assert_eq!(property, "name");
            }
            _ => panic!("Expected property access"),
        }
    }

    #[test]
    fn test_binary_op() {
        let result = expr(span("a + b")).unwrap();
        match result.1 {
            Expr::BinaryOp { left, op, right } => {
                assert!(matches!(*left, Expr::Variable(s) if s == "a"));
                assert_eq!(op, BinOp::Add);
                assert!(matches!(*right, Expr::Variable(s) if s == "b"));
            }
            _ => panic!("Expected binary op"),
        }
    }

    #[test]
    fn test_comparison() {
        let result = expr(span("age > 18")).unwrap();
        match result.1 {
            Expr::BinaryOp { left, op, right } => {
                assert!(matches!(*left, Expr::Variable(s) if s == "age"));
                assert_eq!(op, BinOp::Gt);
                assert!(matches!(*right, Expr::Literal(Literal::Integer(18))));
            }
            _ => panic!("Expected comparison"),
        }
    }

    #[test]
    fn test_function_call() {
        let result = expr(span("toUpper(name)")).unwrap();
        match result.1 {
            Expr::FunctionCall { name, args, .. } => {
                assert_eq!(name, "toUpper");
                assert_eq!(args.len(), 1);
            }
            _ => panic!("Expected function call"),
        }
    }

    #[test]
    fn test_list() {
        let result = expr(span("[1, 2, 3]")).unwrap();
        match result.1 {
            Expr::List(items) => {
                assert_eq!(items.len(), 3);
            }
            _ => panic!("Expected list"),
        }
    }

    #[test]
    fn test_precedence() {
        // 1 + 2 * 3 should be 1 + (2 * 3)
        let result = expr(span("1 + 2 * 3")).unwrap();
        match result.1 {
            Expr::BinaryOp { left, op, right } => {
                assert_eq!(op, BinOp::Add);
                assert!(matches!(*left, Expr::Literal(Literal::Integer(1))));
                assert!(matches!(*right, Expr::BinaryOp { op: BinOp::Mul, .. }));
            }
            _ => panic!("Expected correct precedence"),
        }
    }
}
