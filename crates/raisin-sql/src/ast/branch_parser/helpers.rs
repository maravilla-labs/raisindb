//\! Shared helper parsers for BRANCH statement parsing
//\!
//\! Contains reusable nom combinators for branch names, quoted strings,
//\! identifiers, revision references, and boolean values.

use nom::{
    branch::alt,
    bytes::complete::{tag_no_case, take_until, take_while1},
    character::complete::{char, digit1},
    combinator::{map, value},
    sequence::{preceded, tuple},
    IResult, Parser,
};

use super::super::branch::RevisionRef;

/// Parse a branch name: quoted string or unquoted identifier
pub(crate) fn branch_name(input: &str) -> IResult<&str, String> {
    alt((
        map(quoted_string, |s| s.to_string()),
        map(identifier, |s| s.to_string()),
    ))
    .parse(input)
}

/// Parse a quoted string: 'content' or "content"
pub(crate) fn quoted_string(input: &str) -> IResult<&str, &str> {
    alt((
        nom::sequence::delimited(char('\''), take_until("'"), char('\'')),
        nom::sequence::delimited(char('"'), take_until("\""), char('"')),
    ))
    .parse(input)
}

/// Parse an identifier: alphanumeric + underscore, must start with letter or underscore
pub(crate) fn identifier(input: &str) -> IResult<&str, &str> {
    take_while1(|c: char| c.is_alphanumeric() || c == '_').parse(input)
}

/// Parse a revision reference: HLC (1234_56) or HEAD~N or branch~N
pub(crate) fn revision_ref(input: &str) -> IResult<&str, RevisionRef> {
    alt((
        // HEAD~N
        map(
            preceded(
                (tag_no_case("HEAD"), char('~')),
                map(digit1, |s: &str| s.parse::<u32>().unwrap_or(0)),
            ),
            RevisionRef::HeadRelative,
        ),
        // branch~N (unquoted identifier followed by ~N)
        map(
            tuple((
                identifier,
                char('~'),
                map(digit1, |s: &str| s.parse::<u32>().unwrap_or(0)),
            )),
            |(branch, _, offset)| RevisionRef::BranchRelative {
                branch: branch.to_string(),
                offset,
            },
        ),
        // HLC: timestamp_counter format or just a numeric string
        map(hlc_string, |s| RevisionRef::Hlc(s.to_string())),
    ))
    .parse(input)
}

/// Parse an HLC string: numeric with optional underscore (e.g., 1734567890123_42 or 1734567890123)
fn hlc_string(input: &str) -> IResult<&str, &str> {
    take_while1(|c: char| c.is_ascii_digit() || c == '_').parse(input)
}

/// Parse a boolean value: TRUE or FALSE (case insensitive)
pub(crate) fn boolean_value(input: &str) -> IResult<&str, bool> {
    alt((
        value(true, tag_no_case("TRUE")),
        value(false, tag_no_case("FALSE")),
    ))
    .parse(input)
}
