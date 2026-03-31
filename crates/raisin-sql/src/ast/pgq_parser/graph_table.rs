//! GRAPH_TABLE and MATCH clause parsing

use nom::{
    bytes::complete::tag_no_case,
    character::complete::{char, multispace0},
    combinator::opt,
    multi::separated_list1,
    sequence::tuple,
    IResult, Parser,
};

use super::clauses::{parse_columns_clause, parse_where_clause};
use super::patterns::{parse_node_pattern, parse_relationship_pattern};
use super::primitives::parse_identifier;
use crate::ast::pgq::{GraphTableQuery, MatchClause, PathPattern, PatternElement, SourceSpan};

/// Parse complete GRAPH_TABLE expression
pub fn parse_graph_table_internal(input: &str) -> IResult<&str, GraphTableQuery> {
    let (input, _) = tag_no_case("GRAPH_TABLE").parse(input)?;
    let (input, _) = multispace0.parse(input)?;
    let (input, _) = char('(').parse(input)?;
    let (input, _) = multispace0.parse(input)?;

    let (input, graph_name) = opt(parse_graph_name).parse(input)?;
    let (input, _) = multispace0.parse(input)?;

    let (input, match_clause) = parse_match_clause(input)?;
    let (input, _) = multispace0.parse(input)?;

    let (input, where_clause) = opt(parse_where_clause).parse(input)?;
    let (input, _) = multispace0.parse(input)?;

    let (input, columns_clause) = parse_columns_clause(input)?;
    let (input, _) = multispace0.parse(input)?;

    let (input, _) = char(')').parse(input)?;

    Ok((
        input,
        GraphTableQuery {
            graph_name,
            match_clause,
            where_clause,
            columns_clause,
            span: SourceSpan::empty(),
        },
    ))
}

fn parse_graph_name(input: &str) -> IResult<&str, String> {
    let test_input = input.trim_start();
    if test_input.to_uppercase().starts_with("MATCH") {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Tag,
        )));
    }

    let (input, name) = parse_identifier(input)?;
    Ok((input, name))
}

fn parse_match_clause(input: &str) -> IResult<&str, MatchClause> {
    let (input, _) = tag_no_case("MATCH").parse(input)?;
    let (input, _) = multispace0.parse(input)?;
    let (input, patterns) = separated_list1(
        tuple((multispace0, char(','), multispace0)),
        parse_path_pattern,
    )
    .parse(input)?;

    Ok((
        input,
        MatchClause {
            patterns,
            span: SourceSpan::empty(),
        },
    ))
}

fn parse_path_pattern(input: &str) -> IResult<&str, PathPattern> {
    let (input, first_node) = parse_node_pattern(input)?;
    let (input, _) = multispace0.parse(input)?;

    let mut elements = vec![PatternElement::Node(first_node)];

    let (input, pairs) = nom::multi::many0(tuple((
        multispace0,
        parse_relationship_pattern,
        multispace0,
        parse_node_pattern,
    )))
    .parse(input)?;

    for (_, rel, _, node) in pairs {
        elements.push(PatternElement::Relationship(rel));
        elements.push(PatternElement::Node(node));
    }

    Ok((
        input,
        PathPattern {
            elements,
            span: SourceSpan::empty(),
        },
    ))
}
