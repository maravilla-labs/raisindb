//! Path quantifier parsing for PGQ relationship patterns.
//!
//! Two spellings are accepted, in **disjoint syntactic slots**, so nothing is
//! ambiguous:
//!
//! | slot | form | example | meaning |
//! |---|---|---|---|
//! | after the arrow | canonical | `-[:t]->{1,3}` | 1..3 hops |
//! | inside the brackets | deprecated | `-[:t*1..3]->` | 1..3 hops |
//!
//! The one place they disagree is bare `*`: standard `*` is `{0,}`, legacy `*`
//! is `{1,}`. Because the slots differ this is never ambiguous, but it is why
//! [`crate::ast::pgq::PathQuantifier::deprecation_note`] must be surfaced.

use nom::{
    branch::alt,
    bytes::complete::tag,
    character::complete::{char, digit1, multispace0},
    combinator::{map, opt, value},
    sequence::{preceded, tuple},
    IResult, Parser,
};

use super::error::fail;
use crate::ast::pgq::{PathQuantifier, QuantifierSyntax};

/// Parse the canonical postfix quantifier that follows an arrow.
///
/// `{m,n}` | `{m,}` | `{m}` | `*` | `+` | `?`
pub fn parse_standard_quantifier(input: &str) -> IResult<&str, PathQuantifier> {
    alt((
        parse_brace_quantifier,
        value(bounds(0, None), char('*')),
        value(bounds(1, None), char('+')),
        value(bounds(0, Some(1)), char('?')),
    ))
    .parse(input)
}

fn parse_brace_quantifier(input: &str) -> IResult<&str, PathQuantifier> {
    let (rest, _) = char('{').parse(input)?;
    let (rest, _) = multispace0.parse(rest)?;
    let (rest, min) = uint(rest)?;
    let (rest, _) = multispace0.parse(rest)?;

    // `{m}` (exact) vs `{m,n}` / `{m,}`
    let (rest, comma) = opt(char(',')).parse(rest)?;
    let (rest, _) = multispace0.parse(rest)?;
    let (rest, max) = match comma {
        None => (rest, Some(min)),
        Some(_) => {
            let (rest, max) = opt(uint).parse(rest)?;
            (rest, max)
        }
    };
    let (rest, _) = multispace0.parse(rest)?;
    let (rest, _) = char('}').parse(rest)?;

    if let Some(max) = max {
        if max < min {
            return fail(
                input,
                format!(
                    "quantifier {{{min},{max}}} is empty: the maximum hop count is below the \
                     minimum. Write {{{max},{min}}} if the bounds were swapped."
                ),
            );
        }
    }
    Ok((rest, bounds(min, max)))
}

/// Parse the deprecated Cypher-style quantifier written inside the brackets.
///
/// `*` | `*n` | `*n..m` | `*n..` | `*..m`
pub fn parse_legacy_quantifier(input: &str) -> IResult<&str, PathQuantifier> {
    let (input, _) = char('*').parse(input)?;
    let (input, _) = multispace0.parse(input)?;

    alt((
        map(tuple((uint, tag(".."), opt(uint))), |(min, _, max)| {
            PathQuantifier {
                min,
                max,
                syntax: QuantifierSyntax::LegacyStar,
            }
        }),
        map(preceded(tag(".."), uint), |max| PathQuantifier {
            min: 1,
            max: Some(max),
            syntax: QuantifierSyntax::LegacyStar,
        }),
        map(uint, |n| PathQuantifier {
            min: n,
            max: Some(n),
            syntax: QuantifierSyntax::LegacyStar,
        }),
        value(PathQuantifier::unbounded(), multispace0),
    ))
    .parse(input)
}

fn uint(input: &str) -> IResult<&str, u32> {
    map(digit1, |s: &str| s.parse::<u32>().unwrap_or(0)).parse(input)
}

fn bounds(min: u32, max: Option<u32>) -> PathQuantifier {
    PathQuantifier {
        min,
        max,
        syntax: QuantifierSyntax::Standard,
    }
}
