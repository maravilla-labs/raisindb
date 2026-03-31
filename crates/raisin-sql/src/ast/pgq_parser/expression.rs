//! Expression parser for PGQ WHERE clauses and COLUMNS expressions

use nom::{
    branch::alt,
    bytes::complete::{tag, tag_no_case, take_until},
    character::complete::{char, multispace0, multispace1},
    combinator::{map, opt, value},
    multi::{many0, separated_list0},
    sequence::{delimited, pair, preceded, tuple},
    IResult, Parser,
};

use super::primitives::{
    parse_identifier, parse_identifier_or_star, parse_identifier_str, parse_number_literal,
    parse_string_literal,
};
use crate::ast::pgq::{BinaryOperator, Expr, Literal, SourceSpan, UnaryOperator};

/// Parse expression
pub fn parse_expression(input: &str) -> IResult<&str, Expr> {
    parse_or_expression(input)
}

fn parse_or_expression(input: &str) -> IResult<&str, Expr> {
    let (input, first) = parse_and_expression(input)?;
    let (input, rest) = many0(preceded(
        tuple((multispace0, tag_no_case("OR"), multispace1)),
        parse_and_expression,
    ))
    .parse(input)?;

    let result = rest.into_iter().fold(first, |left, right| Expr::BinaryOp {
        left: Box::new(left),
        op: BinaryOperator::Or,
        right: Box::new(right),
        span: SourceSpan::empty(),
    });

    Ok((input, result))
}

fn parse_and_expression(input: &str) -> IResult<&str, Expr> {
    let (input, first) = parse_comparison_expression(input)?;
    let (input, rest) = many0(preceded(
        tuple((multispace0, tag_no_case("AND"), multispace1)),
        parse_comparison_expression,
    ))
    .parse(input)?;

    let result = rest.into_iter().fold(first, |left, right| Expr::BinaryOp {
        left: Box::new(left),
        op: BinaryOperator::And,
        right: Box::new(right),
        span: SourceSpan::empty(),
    });

    Ok((input, result))
}

fn parse_comparison_expression(input: &str) -> IResult<&str, Expr> {
    let (input, left) = parse_additive_expression(input)?;
    let (input, _) = multispace0.parse(input)?;

    let (input, op_and_right) = opt(tuple((
        parse_comparison_op,
        multispace0,
        parse_additive_expression,
    )))
    .parse(input)?;

    match op_and_right {
        Some((op, _, right)) => Ok((
            input,
            Expr::BinaryOp {
                left: Box::new(left),
                op,
                right: Box::new(right),
                span: SourceSpan::empty(),
            },
        )),
        None => Ok((input, left)),
    }
}

fn parse_comparison_op(input: &str) -> IResult<&str, BinaryOperator> {
    alt((
        value(BinaryOperator::LtEq, tag("<=")),
        value(BinaryOperator::GtEq, tag(">=")),
        value(BinaryOperator::NotEq, tag("<>")),
        value(BinaryOperator::NotEq, tag("!=")),
        value(BinaryOperator::Eq, tag("=")),
        value(BinaryOperator::Lt, tag("<")),
        value(BinaryOperator::Gt, tag(">")),
    ))
    .parse(input)
}

fn parse_additive_expression(input: &str) -> IResult<&str, Expr> {
    let (input, first) = parse_multiplicative_expression(input)?;
    let (input, rest) = many0(tuple((
        multispace0,
        alt((
            value(BinaryOperator::Plus, char('+')),
            value(BinaryOperator::Minus, char('-')),
            value(BinaryOperator::Concat, tag("||")),
        )),
        multispace0,
        parse_multiplicative_expression,
    )))
    .parse(input)?;

    let result = rest
        .into_iter()
        .fold(first, |left, (_, op, _, right)| Expr::BinaryOp {
            left: Box::new(left),
            op,
            right: Box::new(right),
            span: SourceSpan::empty(),
        });

    Ok((input, result))
}

fn parse_multiplicative_expression(input: &str) -> IResult<&str, Expr> {
    let (input, first) = parse_unary_expression(input)?;
    let (input, rest) = many0(tuple((
        multispace0,
        alt((
            value(BinaryOperator::Multiply, char('*')),
            value(BinaryOperator::Divide, char('/')),
            value(BinaryOperator::Modulo, char('%')),
        )),
        multispace0,
        parse_unary_expression,
    )))
    .parse(input)?;

    let result = rest
        .into_iter()
        .fold(first, |left, (_, op, _, right)| Expr::BinaryOp {
            left: Box::new(left),
            op,
            right: Box::new(right),
            span: SourceSpan::empty(),
        });

    Ok((input, result))
}

fn parse_unary_expression(input: &str) -> IResult<&str, Expr> {
    alt((
        map(
            preceded(
                pair(tag_no_case("NOT"), multispace1),
                parse_unary_expression,
            ),
            |expr| Expr::UnaryOp {
                op: UnaryOperator::Not,
                expr: Box::new(expr),
                span: SourceSpan::empty(),
            },
        ),
        map(
            preceded(pair(char('-'), multispace0), parse_unary_expression),
            |expr| Expr::UnaryOp {
                op: UnaryOperator::Minus,
                expr: Box::new(expr),
                span: SourceSpan::empty(),
            },
        ),
        parse_json_access_expression,
    ))
    .parse(input)
}

fn parse_json_access_expression(input: &str) -> IResult<&str, Expr> {
    let (input, base) = parse_primary_expression(input)?;

    let (input, accesses) = many0(tuple((
        multispace0,
        alt((value(true, tag("->>")), value(false, tag("->")))),
        multispace0,
        alt((
            map(
                delimited(char('\''), take_until("'"), char('\'')),
                |s: &str| s.to_string(),
            ),
            map(
                delimited(char('"'), take_until("\""), char('"')),
                |s: &str| s.to_string(),
            ),
            map(parse_identifier_str, |s| s.to_string()),
        )),
    )))
    .parse(input)?;

    let result = accesses
        .into_iter()
        .fold(base, |expr, (_, as_text, _, key)| Expr::JsonAccess {
            expr: Box::new(expr),
            key,
            as_text,
            span: SourceSpan::empty(),
        });

    Ok((input, result))
}

fn parse_primary_expression(input: &str) -> IResult<&str, Expr> {
    alt((
        parse_jsonpath_access,
        parse_parenthesized,
        parse_function_call,
        parse_literal,
        parse_property_access_or_wildcard,
    ))
    .parse(input)
}

fn parse_jsonpath_access(input: &str) -> IResult<&str, Expr> {
    let (input, _) = tag("$.").parse(input)?;
    let (input, first) = parse_identifier_str(input)?;
    let (input, rest) = many0(preceded(char('.'), parse_identifier_str)).parse(input)?;

    let variable = first.to_string();
    let path: Vec<String> = rest.into_iter().map(|s| s.to_string()).collect();

    Ok((
        input,
        Expr::JsonPathAccess {
            variable,
            path,
            span: SourceSpan::empty(),
        },
    ))
}

fn parse_parenthesized(input: &str) -> IResult<&str, Expr> {
    let (input, _) = char('(').parse(input)?;
    let (input, _) = multispace0.parse(input)?;
    let (input, expr) = parse_expression(input)?;
    let (input, _) = multispace0.parse(input)?;
    let (input, _) = char(')').parse(input)?;

    Ok((input, Expr::Nested(Box::new(expr))))
}

fn parse_function_call(input: &str) -> IResult<&str, Expr> {
    let (input, name) = parse_identifier(input)?;
    let (input, _) = multispace0.parse(input)?;
    let (input, _) = char('(').parse(input)?;
    let (input, _) = multispace0.parse(input)?;

    let (input, distinct) = opt(preceded(tag_no_case("DISTINCT"), multispace1)).parse(input)?;

    let (input, args) = alt((
        map(char('*'), |_| {
            vec![Expr::Wildcard {
                qualifier: None,
                span: SourceSpan::empty(),
            }]
        }),
        separated_list0(
            tuple((multispace0, char(','), multispace0)),
            parse_expression,
        ),
    ))
    .parse(input)?;

    let (input, _) = multispace0.parse(input)?;
    let (input, _) = char(')').parse(input)?;

    Ok((
        input,
        Expr::FunctionCall {
            name,
            args,
            distinct: distinct.is_some(),
            span: SourceSpan::empty(),
        },
    ))
}

fn parse_literal(input: &str) -> IResult<&str, Expr> {
    alt((
        map(parse_string_literal, |s| Expr::Literal(Literal::String(s))),
        map(parse_number_literal, |n| {
            if n.fract() == 0.0 {
                Expr::Literal(Literal::Integer(n as i64))
            } else {
                Expr::Literal(Literal::Float(n))
            }
        }),
        map(tag_no_case("true"), |_| {
            Expr::Literal(Literal::Boolean(true))
        }),
        map(tag_no_case("false"), |_| {
            Expr::Literal(Literal::Boolean(false))
        }),
        map(tag_no_case("null"), |_| Expr::Literal(Literal::Null)),
    ))
    .parse(input)
}

fn parse_property_access_or_wildcard(input: &str) -> IResult<&str, Expr> {
    if input.starts_with('*') {
        let (input, _) = char('*').parse(input)?;
        return Ok((
            input,
            Expr::Wildcard {
                qualifier: None,
                span: SourceSpan::empty(),
            },
        ));
    }

    let (input, first) = parse_identifier(input)?;
    let (input, rest) = many0(preceded(char('.'), parse_identifier_or_star)).parse(input)?;

    if let Some(last) = rest.last() {
        if *last == "*" {
            let properties: Vec<_> = rest[..rest.len() - 1]
                .iter()
                .map(|s| s.to_string())
                .collect();
            if properties.is_empty() {
                return Ok((
                    input,
                    Expr::Wildcard {
                        qualifier: Some(first),
                        span: SourceSpan::empty(),
                    },
                ));
            }
        }
    }

    Ok((
        input,
        Expr::PropertyAccess {
            variable: first,
            properties: rest.into_iter().map(|s| s.to_string()).collect(),
            span: SourceSpan::empty(),
        },
    ))
}
