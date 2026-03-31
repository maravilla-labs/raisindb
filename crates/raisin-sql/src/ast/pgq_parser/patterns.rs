//! Node and relationship pattern parsing for PGQ MATCH clause

use nom::{
    branch::alt,
    bytes::complete::{tag, tag_no_case},
    character::complete::{char, digit1, multispace0, multispace1},
    combinator::{map, opt, value},
    multi::separated_list1,
    sequence::{pair, preceded, tuple},
    IResult, Parser,
};

use super::expression::parse_expression;
use super::primitives::parse_identifier;
use crate::ast::pgq::{Direction, NodePattern, PathQuantifier, RelationshipPattern, SourceSpan};

/// Parse node pattern: (n:Label WHERE ...)
pub fn parse_node_pattern(input: &str) -> IResult<&str, NodePattern> {
    let (input, _) = char('(').parse(input)?;
    let (input, _) = multispace0.parse(input)?;

    let (input, variable) = opt(parse_identifier).parse(input)?;
    let (input, _) = multispace0.parse(input)?;

    let (input, labels) = opt(preceded(
        pair(char(':'), multispace0),
        separated_list1(pair(multispace0, char('|')), parse_identifier),
    ))
    .parse(input)?;
    let labels = labels.unwrap_or_default();
    let (input, _) = multispace0.parse(input)?;

    let (input, filter) = opt(preceded(
        pair(tag_no_case("WHERE"), multispace1),
        parse_expression,
    ))
    .parse(input)?;
    let (input, _) = multispace0.parse(input)?;

    let (input, _) = char(')').parse(input)?;

    Ok((
        input,
        NodePattern {
            variable,
            labels,
            filter: filter.map(Box::new),
            span: SourceSpan::empty(),
        },
    ))
}

/// Parse relationship pattern: -[r:TYPE*1..3]->
pub fn parse_relationship_pattern(input: &str) -> IResult<&str, RelationshipPattern> {
    alt((
        parse_right_relationship,
        parse_left_relationship,
        parse_any_relationship,
    ))
    .parse(input)
}

fn parse_right_relationship(input: &str) -> IResult<&str, RelationshipPattern> {
    let (input, _) = char('-').parse(input)?;
    let (input, inner) = parse_relationship_inner(input)?;
    let (input, _) = tag("->").parse(input)?;

    Ok((
        input,
        RelationshipPattern {
            direction: Direction::Right,
            ..inner
        },
    ))
}

fn parse_left_relationship(input: &str) -> IResult<&str, RelationshipPattern> {
    let (input, _) = tag("<-").parse(input)?;
    let (input, inner) = parse_relationship_inner(input)?;
    let (input, _) = char('-').parse(input)?;

    Ok((
        input,
        RelationshipPattern {
            direction: Direction::Left,
            ..inner
        },
    ))
}

fn parse_any_relationship(input: &str) -> IResult<&str, RelationshipPattern> {
    let (input, _) = char('-').parse(input)?;
    let (input, inner) = parse_relationship_inner(input)?;
    let (input, _) = char('-').parse(input)?;

    Ok((
        input,
        RelationshipPattern {
            direction: Direction::Any,
            ..inner
        },
    ))
}

fn parse_relationship_inner(input: &str) -> IResult<&str, RelationshipPattern> {
    let (input, _) = char('[').parse(input)?;
    let (input, _) = multispace0.parse(input)?;

    let (input, variable) = opt(parse_identifier).parse(input)?;
    let (input, _) = multispace0.parse(input)?;

    let (input, types) = opt(preceded(
        pair(char(':'), multispace0),
        separated_list1(
            tuple((multispace0, char('|'), multispace0)),
            parse_identifier,
        ),
    ))
    .parse(input)?;
    let types = types.unwrap_or_default();
    let (input, _) = multispace0.parse(input)?;

    let (input, quantifier) = opt(parse_quantifier).parse(input)?;
    let (input, _) = multispace0.parse(input)?;

    let (input, filter) = opt(preceded(
        pair(tag_no_case("WHERE"), multispace1),
        parse_expression,
    ))
    .parse(input)?;
    let (input, _) = multispace0.parse(input)?;

    let (input, _) = char(']').parse(input)?;

    Ok((
        input,
        RelationshipPattern {
            variable,
            types,
            direction: Direction::Right,
            quantifier,
            filter: filter.map(Box::new),
            span: SourceSpan::empty(),
        },
    ))
}

fn parse_quantifier(input: &str) -> IResult<&str, PathQuantifier> {
    let (input, _) = char('*').parse(input)?;
    let (input, _) = multispace0.parse(input)?;

    alt((
        map(
            tuple((
                map(digit1, |s: &str| s.parse::<u32>().unwrap_or(1)),
                tag(".."),
                opt(map(digit1, |s: &str| s.parse::<u32>().unwrap_or(10))),
            )),
            |(min, _, max)| PathQuantifier { min, max },
        ),
        map(
            preceded(
                tag(".."),
                map(digit1, |s: &str| s.parse::<u32>().unwrap_or(10)),
            ),
            |max| PathQuantifier {
                min: 1,
                max: Some(max),
            },
        ),
        map(map(digit1, |s: &str| s.parse::<u32>().unwrap_or(1)), |n| {
            PathQuantifier {
                min: n,
                max: Some(n),
            }
        }),
        value(PathQuantifier::unbounded(), multispace0),
    ))
    .parse(input)
}
