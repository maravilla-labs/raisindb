//! `COST` clause parsing for PGQ relationship patterns.
//!
//! `COST` is a RaisinDB extension used only under the `ANY CHEAPEST` selector;
//! neither GQL nor SQL/PGQ standardises weighted path search. The spelling
//! follows Google Spanner Graph.

use nom::{
    bytes::complete::tag_no_case,
    character::complete::{char, digit1, multispace1},
    combinator::{opt, recognize},
    sequence::tuple,
    IResult, Parser,
};

use super::error::fail;
use super::primitives::parse_identifier;
use crate::ast::pgq::{Expr, Literal, SourceSpan};

/// Parse `COST <edge>.weight` (or a positive finite numeric literal).
///
/// A RaisinDB relation has no property map — `RelationRef` carries only target,
/// workspace, target_node_type, relation_type and weight — so `weight` is the
/// only numeric thing an edge has, and anything else is rejected rather than
/// silently ignored.
///
/// Only those two shapes are *parsed*, deliberately. Handing the general
/// expression parser `COST t.weight*1..3` makes it read `t.weight * 1` as a
/// multiplication and swallow the deprecated quantifier that follows.
pub fn parse_cost_clause<'a>(
    input: &'a str,
    edge_variable: Option<&str>,
) -> IResult<&'a str, Expr> {
    let (rest, _) = tag_no_case("COST").parse(input)?;
    let (rest, _) = multispace1.parse(rest)?;

    if let Ok((rest, literal)) = parse_cost_number(rest) {
        let positive = match &literal {
            Literal::Integer(n) => *n > 0,
            Literal::Float(f) => f.is_finite() && *f > 0.0,
            _ => false,
        };
        return if positive {
            Ok((rest, Expr::Literal(literal)))
        } else {
            fail(
                input,
                "COST literal must be a positive finite number. COST 1 is legal and is equivalent \
                 to ANY SHORTEST.",
            )
        };
    }

    let Ok((rest, (variable, property))) = parse_qualified_name(rest) else {
        return fail(
            input,
            "COST must reference the edge weight; a RaisinDB relation has no property map (only \
             target, workspace, target_node_type, relation_type, weight). Write COST <edge>.weight.",
        );
    };

    if !property.eq_ignore_ascii_case("weight") {
        return fail(
            input,
            format!(
                "COST references '{variable}.{property}'; a RaisinDB relation has no property map \
                 (only target, workspace, target_node_type, relation_type, weight). Write \
                 COST {variable}.weight."
            ),
        );
    }
    match edge_variable {
        None => fail(
            input,
            "COST requires a bound edge variable: write -[t:Transfers COST t.weight]-> rather \
             than -[:Transfers COST ...]->",
        ),
        Some(bound) if bound != variable => fail(
            input,
            format!(
                "COST references '{variable}' but this edge is bound to '{bound}'. Write \
                 COST {bound}.weight."
            ),
        ),
        Some(_) => Ok((
            rest,
            Expr::PropertyAccess {
                variable,
                properties: vec![property],
                span: SourceSpan::empty(),
            },
        )),
    }
}

/// `123` or `1.5` — never consumes the `..` of a legacy quantifier.
fn parse_cost_number(input: &str) -> IResult<&str, Literal> {
    let (rest, text) = recognize(tuple((
        opt(char('-')),
        digit1,
        opt(tuple((char('.'), digit1))),
    )))
    .parse(input)?;

    if text.contains('.') {
        Ok((rest, Literal::Float(text.parse().unwrap_or(0.0))))
    } else {
        Ok((rest, Literal::Integer(text.parse().unwrap_or(0))))
    }
}

/// `edge.property`
fn parse_qualified_name(input: &str) -> IResult<&str, (String, String)> {
    let (rest, variable) = parse_identifier(input)?;
    let (rest, _) = char('.').parse(rest)?;
    let (rest, property) = parse_identifier(rest)?;
    Ok((rest, (variable, property)))
}
