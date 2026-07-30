//! Node and relationship pattern parsing for PGQ MATCH clause
//!
//! Inline `WHERE` inside `(...)` or `[...]` is rejected here rather than
//! parsed. It used to land in a `filter` field that no execution path ever
//! read, so the predicate was silently dropped and the query returned
//! unfiltered rows — a wrong-results bug that looked like a working feature.

use nom::{
    branch::alt,
    bytes::complete::tag,
    character::complete::{char, multispace0},
    combinator::opt,
    multi::separated_list1,
    sequence::{pair, preceded, tuple},
    IResult, Parser,
};

use super::cost::parse_cost_clause;
use super::error::fail;
use super::primitives::parse_identifier;
use super::quantifier::{parse_legacy_quantifier, parse_standard_quantifier};
use crate::ast::pgq::{
    Direction, Expr, NodePattern, PathQuantifier, RelationshipPattern, SourceSpan,
};

/// Parse node pattern: `(n:Label)` / `(n:Label|Other)`
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

    if starts_with_where(input) {
        return fail(
            input,
            "inline WHERE inside a node pattern is not supported. Move the predicate to the \
             GRAPH_TABLE WHERE clause: MATCH (n:User) WHERE n.active = true COLUMNS (n.id)",
        );
    }

    let (input, _) = char(')').parse(input)?;

    Ok((
        input,
        NodePattern {
            variable,
            labels,
            span: SourceSpan::empty(),
        },
    ))
}

/// Parse relationship pattern including any canonical postfix quantifier.
///
/// `-[r:TYPE]->{1,3}` / `<-[r]-` / `-[r]-` / `-[r:TYPE*1..3]->` (deprecated)
pub fn parse_relationship_pattern(input: &str) -> IResult<&str, RelationshipPattern> {
    let (input, rel) = alt((
        parse_right_relationship,
        parse_left_relationship,
        parse_any_relationship,
    ))
    .parse(input)?;

    let (input, postfix) = opt(parse_standard_quantifier).parse(input)?;

    match (rel.quantifier, postfix) {
        (Some(_), Some(_)) => fail(
            input,
            "a relationship carries two quantifiers. Keep the canonical postfix form after the \
             arrow (->{1,3}) and drop the deprecated Cypher-style one inside the brackets (*1..3)",
        ),
        (_, Some(q)) => Ok((
            input,
            RelationshipPattern {
                quantifier: Some(q),
                ..rel
            },
        )),
        (_, None) => Ok((input, rel)),
    }
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

    let (input, (cost, quantifier)) = parse_edge_modifiers(input, variable.as_deref())?;

    if starts_with_where(input) {
        return fail(
            input,
            "inline WHERE inside a relationship pattern is not supported. Move the predicate to \
             the GRAPH_TABLE WHERE clause: MATCH (a)-[r:follows]->(b) WHERE r.since > 2020 \
             COLUMNS (a.id)",
        );
    }

    let (input, _) = char(']').parse(input)?;

    Ok((
        input,
        RelationshipPattern {
            variable,
            types,
            direction: Direction::Right,
            quantifier,
            cost,
            span: SourceSpan::empty(),
        },
    ))
}

/// Parse the `COST` clause and the deprecated in-bracket quantifier, in either
/// order, at most once each.
fn parse_edge_modifiers<'a>(
    input: &'a str,
    edge_variable: Option<&str>,
) -> IResult<&'a str, (Option<Box<Expr>>, Option<PathQuantifier>)> {
    let mut rest = input;
    let mut cost = None;
    let mut quantifier = None;

    loop {
        let (after_space, _) = multispace0.parse(rest)?;

        if quantifier.is_none() {
            if let Ok((next, q)) = parse_legacy_quantifier(after_space) {
                quantifier = Some(q);
                rest = next;
                continue;
            }
        }
        if cost.is_none() {
            match parse_cost_clause(after_space, edge_variable) {
                Ok((next, c)) => {
                    cost = Some(Box::new(c));
                    rest = next;
                    continue;
                }
                Err(err @ nom::Err::Failure(_)) => return Err(err),
                Err(_) => {}
            }
        }
        rest = after_space;
        break;
    }

    Ok((rest, (cost, quantifier)))
}

/// True when the remaining input begins with a `WHERE` keyword token.
fn starts_with_where(input: &str) -> bool {
    let mut chars = input.chars();
    let head: String = chars.by_ref().take(5).collect();
    if !head.eq_ignore_ascii_case("WHERE") {
        return false;
    }
    match chars.next() {
        None => true,
        Some(c) => c.is_whitespace() || c == '(',
    }
}
