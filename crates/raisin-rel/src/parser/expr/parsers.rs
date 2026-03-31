//! Expression parsing with proper operator precedence
//!
//! Operator precedence (lowest to highest):
//! 1. || (OR)
//! 2. && (AND)
//! 3. ==, !=, <, >, <=, >= (comparison), RELATES
//! 4. +, - (additive)
//! 5. *, /, % (multiplicative)
//! 6. !, - (unary NOT, unary minus)
//! 7. . and [...] (property/index access), method calls (.method())
//! 8. Atoms (literals, variables, parentheses)

use super::super::common::{
    add_op, and_op, close_bracket, close_paren, comma, comparison_op, div_op, dot, dotdot,
    identifier, kw_any, kw_depth, kw_direction, kw_incoming, kw_outgoing, kw_relates, kw_via,
    mod_op, mul_op, neg_op, not_op, open_bracket, open_paren, or_op, sub_op, ws, ws0, ws1,
    ws_before, PResult, Span,
};
use super::super::literal::literal;
use crate::ast::{BinOp, Expr, RelDirection, UnOp};
use nom::{
    branch::alt,
    character::complete::{char, i64 as parse_i64},
    combinator::map,
    multi::{many0, separated_list0, separated_list1},
    sequence::{delimited, preceded},
    Parser,
};

/// Parse a complete expression
pub fn expr(input: Span) -> PResult<Expr> {
    or_expr(input)
}

/// Parse OR expression (lowest precedence)
fn or_expr(input: Span) -> PResult<Expr> {
    let (input, first) = and_expr(input)?;
    let (input, rest) = many0(preceded(ws(or_op), and_expr)).parse(input)?;

    Ok((
        input,
        rest.into_iter()
            .fold(first, |acc, e| Expr::binary(acc, BinOp::Or, e)),
    ))
}

/// Parse AND expression
fn and_expr(input: Span) -> PResult<Expr> {
    let (input, first) = comparison_expr(input)?;
    let (input, rest) = many0(preceded(ws(and_op), comparison_expr)).parse(input)?;

    Ok((
        input,
        rest.into_iter()
            .fold(first, |acc, e| Expr::binary(acc, BinOp::And, e)),
    ))
}

/// Parse comparison expression (==, !=, <, >, <=, >=) and RELATES
fn comparison_expr(input: Span) -> PResult<Expr> {
    let (input, first) = additive_expr(input)?;

    // Try to parse RELATES keyword
    if let Ok((input, _)) = ws1.and(kw_relates).parse(input) {
        return parse_relates_continuation(input, first);
    }

    // Try to parse comparison operator
    if let Ok((input, op_str)) = ws(comparison_op).parse(input) {
        let (input, second) = additive_expr(input)?;
        let op = match op_str {
            "==" => BinOp::Eq,
            "!=" => BinOp::Neq,
            "<" => BinOp::Lt,
            ">" => BinOp::Gt,
            "<=" => BinOp::Lte,
            ">=" => BinOp::Gte,
            _ => unreachable!(),
        };
        Ok((input, Expr::binary(first, op, second)))
    } else {
        Ok((input, first))
    }
}

/// Parse additive expression (+, -)
fn additive_expr(input: Span) -> PResult<Expr> {
    let (input, first) = multiplicative_expr(input)?;
    let (input, rest) = many0(|input| {
        let (input, op) = ws(alt((
            map(add_op, |_| BinOp::Add),
            map(sub_op, |_| BinOp::Sub),
        )))
        .parse(input)?;
        let (input, e) = multiplicative_expr(input)?;
        Ok((input, (op, e)))
    })
    .parse(input)?;

    Ok((
        input,
        rest.into_iter()
            .fold(first, |acc, (op, e)| Expr::binary(acc, op, e)),
    ))
}

/// Parse multiplicative expression (*, /, %)
fn multiplicative_expr(input: Span) -> PResult<Expr> {
    let (input, first) = unary_expr(input)?;
    let (input, rest) = many0(|input| {
        let (input, op) = ws(alt((
            map(mul_op, |_| BinOp::Mul),
            map(div_op, |_| BinOp::Div),
            map(mod_op, |_| BinOp::Mod),
        )))
        .parse(input)?;
        let (input, e) = unary_expr(input)?;
        Ok((input, (op, e)))
    })
    .parse(input)?;

    Ok((
        input,
        rest.into_iter()
            .fold(first, |acc, (op, e)| Expr::binary(acc, op, e)),
    ))
}

/// Parse the continuation of a RELATES expression after the keyword
fn parse_relates_continuation(input: Span, source: Expr) -> PResult<Expr> {
    // Parse target expression
    let (input, _) = ws1(input)?;
    let (input, target) = unary_expr(input)?;

    // Parse VIA keyword and relation types
    let (input, _) = ws1(input)?;
    let (input, _) = kw_via(input)?;
    let (input, _) = ws1(input)?;
    let (input, relation_types) = parse_relation_types(input)?;

    // Parse optional DEPTH clause
    let (input, (min_depth, max_depth)) = if let Ok((input, _)) = ws1.and(kw_depth).parse(input) {
        let (input, _) = ws1(input)?;
        parse_depth_range(input)?
    } else {
        (input, (1, 1))
    };

    // Parse optional DIRECTION clause
    let (input, direction) = if let Ok((input, _)) = ws1.and(kw_direction).parse(input) {
        let (input, _) = ws1(input)?;
        parse_direction(input)?
    } else {
        (input, RelDirection::default())
    };

    Ok((
        input,
        Expr::relates(
            source,
            target,
            relation_types,
            min_depth,
            max_depth,
            direction,
        ),
    ))
}

/// Parse relation types: 'TYPE' or ['TYPE1', 'TYPE2', ...]
fn parse_relation_types(input: Span) -> PResult<Vec<String>> {
    use super::super::literal::string_literal;

    alt((
        // Array of quoted strings - try this first
        delimited(
            preceded(ws0, open_bracket),
            separated_list1(
                comma,
                map(ws(string_literal), |lit| {
                    if let crate::ast::Literal::String(s) = lit {
                        s
                    } else {
                        String::new()
                    }
                }),
            ),
            preceded(ws0, close_bracket),
        ),
        // Single quoted string
        map(string_literal, |lit| {
            if let crate::ast::Literal::String(s) = lit {
                vec![s]
            } else {
                vec![]
            }
        }),
    ))
    .parse(input)
}

/// Parse depth range: min..max
fn parse_depth_range(input: Span) -> PResult<(u32, u32)> {
    let (input, min) = parse_i64(input)?;
    let (input, _) = dotdot(input)?;
    let (input, max) = parse_i64(input)?;

    Ok((input, (min as u32, max as u32)))
}

/// Parse direction: OUTGOING | INCOMING | ANY
fn parse_direction(input: Span) -> PResult<RelDirection> {
    alt((
        map(kw_outgoing, |_| RelDirection::Outgoing),
        map(kw_incoming, |_| RelDirection::Incoming),
        map(kw_any, |_| RelDirection::Any),
    ))
    .parse(input)
}

/// Parse unary expression (!, -)
fn unary_expr(input: Span) -> PResult<Expr> {
    alt((
        // NOT operator
        map(preceded(ws_before(not_op), unary_expr), |e| {
            Expr::unary(UnOp::Not, e)
        }),
        // Unary minus - but be careful not to confuse with negative number literals
        map(preceded(ws_before(neg_op), unary_expr), |e| {
            Expr::unary(UnOp::Neg, e)
        }),
        postfix_expr,
    ))
    .parse(input)
}

/// Parse postfix expression (property access . and index access [])
fn postfix_expr(input: Span) -> PResult<Expr> {
    let (input, base) = atom_expr(input)?;
    postfix_chain(input, base)
}

/// Parse chain of postfix operators
fn postfix_chain(input: Span, base: Expr) -> PResult<Expr> {
    // Try property access or method call: .property or .method()
    if let Ok((input, _)) = dot(input) {
        let (input, name) = property_name(input)?;

        // Check if followed by ( - it's a method call
        if let Ok((input, _)) = open_paren(input) {
            let (input, _) = ws0(input)?;
            let (input, args) = separated_list0(comma, ws(expr)).parse(input)?;
            let (input, _) = ws0(input)?;
            let (input, _) = close_paren(input)?;
            return postfix_chain(input, Expr::method_call(base, name, args));
        }

        // Otherwise it's property access
        return postfix_chain(input, Expr::property_access(base, name));
    }

    // Try index access: [index]
    if let Ok((input, _)) = open_bracket(input) {
        let (input, _) = ws0(input)?;
        let (input, index) = expr(input)?;
        let (input, _) = ws0(input)?;
        let (input, _) = close_bracket(input)?;
        return postfix_chain(input, Expr::index_access(base, index));
    }

    // No more postfix operators
    Ok((input, base))
}

/// Parse a property name (allows keywords as property names)
fn property_name(input: Span) -> PResult<String> {
    let (input, name) =
        nom::bytes::complete::take_while1(|c: char| c.is_ascii_alphanumeric() || c == '_')(input)?;
    Ok((input, name.fragment().to_string()))
}

/// Parse atomic expression (highest precedence)
fn atom_expr(input: Span) -> PResult<Expr> {
    let (input, _) = ws0(input)?;

    alt((
        // Parenthesized expression
        map(
            delimited(char('('), delimited(ws0, expr, ws0), char(')')),
            Expr::grouped,
        ),
        // Literal value
        map(literal, Expr::literal),
        // Variable reference
        map(identifier, Expr::variable),
    ))
    .parse(input)
}
