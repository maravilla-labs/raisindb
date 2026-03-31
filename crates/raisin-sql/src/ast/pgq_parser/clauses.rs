//! WHERE and COLUMNS clause parsing for PGQ

use nom::{
    bytes::complete::tag_no_case,
    character::complete::{char, multispace0, multispace1},
    combinator::opt,
    multi::separated_list1,
    sequence::{preceded, tuple},
    IResult, Parser,
};

use super::expression::parse_expression;
use super::primitives::parse_identifier;
use crate::ast::pgq::{ColumnExpr, ColumnsClause, SourceSpan, WhereClause};

/// Parse WHERE clause
pub fn parse_where_clause(input: &str) -> IResult<&str, WhereClause> {
    let (input, _) = tag_no_case("WHERE").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, expression) = parse_expression(input)?;

    Ok((
        input,
        WhereClause {
            expression,
            span: SourceSpan::empty(),
        },
    ))
}

/// Parse COLUMNS clause
pub fn parse_columns_clause(input: &str) -> IResult<&str, ColumnsClause> {
    let (input, _) = tag_no_case("COLUMNS").parse(input)?;
    let (input, _) = multispace0.parse(input)?;
    let (input, _) = char('(').parse(input)?;
    let (input, _) = multispace0.parse(input)?;

    let (input, columns) = separated_list1(
        tuple((multispace0, char(','), multispace0)),
        parse_column_expr,
    )
    .parse(input)?;

    let (input, _) = multispace0.parse(input)?;
    let (input, _) = char(')').parse(input)?;

    Ok((
        input,
        ColumnsClause {
            columns,
            span: SourceSpan::empty(),
        },
    ))
}

fn parse_column_expr(input: &str) -> IResult<&str, ColumnExpr> {
    let (input, expr) = parse_expression(input)?;
    let (input, _) = multispace0.parse(input)?;

    let (input, alias) = opt(preceded(
        tuple((tag_no_case("AS"), multispace1)),
        parse_identifier,
    ))
    .parse(input)?;

    Ok((
        input,
        ColumnExpr {
            expr,
            alias,
            span: SourceSpan::empty(),
        },
    ))
}
