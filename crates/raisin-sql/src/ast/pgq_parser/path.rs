//! Top-level path pattern parsing: path variable, selector, restrictor.
//!
//! ```text
//! top_level_path := [ path_variable "=" ] [ path_selector ] [ path_restrictor ]
//!                   [ path_variable "=" ] path_expr
//! ```
//!
//! The variable is accepted on either side of the selector/restrictor prefix,
//! because both spellings are in circulation — DuckPGQ writes
//! `p = ANY SHORTEST (...)`, the committee examples write
//! `ALL SHORTEST TRAIL p = (...)`. The selector/restrictor order itself is
//! fixed: selector first.

use nom::{
    character::complete::{char, digit1, multispace0, multispace1},
    combinator::opt,
    sequence::tuple,
    IResult, Parser,
};

use super::error::fail;
use super::keywords::{keyword, two_words};
use super::patterns::{parse_node_pattern, parse_relationship_pattern};
use super::primitives::parse_identifier;
use crate::ast::pgq::{PathPattern, PathRestrictor, PathSelector, PatternElement, SourceSpan};

/// Parse one comma-separated entry of a MATCH clause.
pub fn parse_top_level_path(input: &str) -> IResult<&str, PathPattern> {
    let start = input;

    let (input, leading_var) = opt(parse_path_variable).parse(input)?;
    let (input, _) = multispace0.parse(input)?;

    let (input, selector) = opt(parse_selector).parse(input)?;
    let (input, _) = multispace0.parse(input)?;

    let (input, restrictor) = opt(parse_restrictor).parse(input)?;
    let (input, _) = multispace0.parse(input)?;

    // A restrictor written before the selector is a common slip; name the fix.
    if let (Some(restrictor), None) = (restrictor, selector) {
        if let Ok((_, found)) = parse_selector(input) {
            return fail(
                input,
                format!(
                    "path restrictor {restrictor} must come after the path selector; write \
                     `{found} {restrictor}`, not `{restrictor} {found}`"
                ),
            );
        }
    }

    let (input, trailing_var) = opt(parse_path_variable).parse(input)?;
    let (input, _) = multispace0.parse(input)?;

    let variable = match (leading_var, trailing_var) {
        (Some(_), Some(second)) => {
            return fail(
                input,
                format!(
                    "path variable given twice; write it once, either before the selector \
                     (`{second} = ANY SHORTEST (...)`) or after the restrictor \
                     (`ANY SHORTEST TRAIL {second} = (...)`)"
                ),
            )
        }
        (Some(v), None) | (None, Some(v)) => Some(v),
        (None, None) => None,
    };

    let (input, elements) = parse_path_elements(input)?;

    let pattern = PathPattern {
        variable,
        selector,
        restrictor,
        elements,
        span: SourceSpan::empty(),
    };

    check_quantifier_scope(start, &pattern)?;
    check_cost(start, &pattern)?;
    Ok((input, pattern))
}

fn parse_path_elements(input: &str) -> IResult<&str, Vec<PatternElement>> {
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

    Ok((input, elements))
}

/// `p =`
fn parse_path_variable(input: &str) -> IResult<&str, String> {
    let (rest, name) = parse_identifier(input)?;
    let (rest, _) = multispace0.parse(rest)?;
    let (rest, _) = char('=').parse(rest)?;
    Ok((rest, name))
}

/// Parse a path selector.
///
/// Ordering is mandatory: the two-word forms must be tried before bare `ANY`,
/// or `ANY SHORTEST` is truncated to `ANY` and the parse then dies on the
/// leftover `SHORTEST`.
fn parse_selector(input: &str) -> IResult<&str, PathSelector> {
    reject_deferred_selector(input)?;

    if let Ok((rest, _)) = two_words(input, "ANY", "SHORTEST") {
        return Ok((rest, PathSelector::AnyShortest));
    }
    if let Ok((rest, _)) = two_words(input, "ANY", "CHEAPEST") {
        return Ok((rest, PathSelector::AnyCheapest));
    }
    if let Ok((rest, _)) = two_words(input, "ALL", "SHORTEST") {
        return Ok((rest, PathSelector::AllShortest));
    }
    let (rest, _) = keyword(input, "ANY")?;
    Ok((rest, PathSelector::Any))
}

/// Deferred selector spellings get a named error, never silent acceptance.
fn reject_deferred_selector(input: &str) -> IResult<&str, ()> {
    if let Ok((rest, _)) = keyword(input, "SHORTEST") {
        if let Ok((rest, _)) = spaced_digits(rest) {
            let grouped = spaced_keyword(rest, "GROUP").is_ok();
            let written = if grouped {
                "SHORTEST k GROUP"
            } else {
                "SHORTEST k"
            };
            return fail(
                input,
                format!(
                    "`{written}` is not supported yet. Use `ANY SHORTEST` for one minimum-hop \
                     path, or `ALL SHORTEST` for every minimum-hop path."
                ),
            );
        }
    }
    if let Ok((rest, _)) = keyword(input, "ANY") {
        if spaced_digits(rest).is_ok() {
            return fail(
                input,
                "`ANY k` is not supported yet. Use `ANY` for one arbitrary path, or \
                 `ANY SHORTEST` for a minimum-hop path.",
            );
        }
    }
    Ok((input, ()))
}

fn parse_restrictor(input: &str) -> IResult<&str, PathRestrictor> {
    if keyword(input, "SIMPLE").is_ok() {
        return fail(
            input,
            "`SIMPLE` is not supported yet. It differs from `ACYCLIC` only by permitting a closed \
             walk (first node == last node), and shipping a subtly-wrong SIMPLE is worse than not \
             shipping it. Use `ACYCLIC` (node-distinct) or `TRAIL` (edge-distinct).",
        );
    }
    if let Ok((rest, _)) = keyword(input, "WALK") {
        return Ok((rest, PathRestrictor::Walk));
    }
    if let Ok((rest, _)) = keyword(input, "TRAIL") {
        return Ok((rest, PathRestrictor::Trail));
    }
    let (rest, _) = keyword(input, "ACYCLIC")?;
    Ok((rest, PathRestrictor::Acyclic))
}

fn spaced_digits(input: &str) -> IResult<&str, &str> {
    let (rest, _) = multispace1.parse(input)?;
    digit1.parse(rest)
}

fn spaced_keyword<'a>(input: &'a str, word: &str) -> IResult<&'a str, &'a str> {
    let (rest, _) = multispace1.parse(input)?;
    keyword(rest, word)
}

/// Rule Q-SCOPE: an unbounded quantifier in the canonical form must sit under
/// an explicit selector or restrictor.
///
/// The legacy form is exempt — it predates the rule and is capped at
/// [`crate::ast::pgq::PathQuantifier::DEFAULT_MAX`] instead.
fn check_quantifier_scope<'a>(
    start: &'a str,
    pattern: &PathPattern,
) -> Result<(), nom::Err<nom::error::Error<&'a str>>> {
    if pattern.selector.is_some() || pattern.restrictor.is_some() {
        return Ok(());
    }
    let unscoped = pattern
        .relationships()
        .filter_map(|r| r.quantifier)
        .any(|q| q.is_unbounded() && !q.is_legacy());
    if !unscoped {
        return Ok(());
    }
    let failed: IResult<&'a str, ()> = fail(
        start,
        "an unbounded quantifier (`*`, `+`, `{m,}`) must be contained in the scope of a path \
         selector or a path restrictor. Add a selector — `MATCH ANY SHORTEST p = (a)-[:t]->*(b)` \
         — or a restrictor — `MATCH TRAIL p = (a)-[:t]->*(b)`. A bounded quantifier such as \
         `->{1,3}` needs neither.",
    );
    Err(failed.unwrap_err())
}

/// `COST` and `ANY CHEAPEST` imply each other.
///
/// Neither may appear alone: a weighted query that silently answers by hop
/// count, or a `COST` that is silently ignored, is the wrong-results class this
/// grammar exists to avoid.
fn check_cost<'a>(
    start: &'a str,
    pattern: &PathPattern,
) -> Result<(), nom::Err<nom::error::Error<&'a str>>> {
    let has_cost = pattern.relationships().any(|r| r.cost.is_some());
    let cheapest = pattern.selector == Some(PathSelector::AnyCheapest);

    let message = match (cheapest, has_cost) {
        (true, false) => {
            "ANY CHEAPEST requires a COST clause on at least one edge of the path; write \
             -[t:Transfers COST t.weight]->{1,3}. It must not fall back to hop count — use \
             ANY SHORTEST if hop count is what you want."
        }
        (false, true) => {
            "COST is only meaningful under ANY CHEAPEST. Add the selector \
             (`MATCH ANY CHEAPEST p = ...`) or drop the COST clause."
        }
        _ => return Ok(()),
    };

    let failed: IResult<&'a str, ()> = fail(start, message);
    Err(failed.unwrap_err())
}
