//! Helper parsers for TRANSLATE statements
//!
//! Shared low-level parsers for identifiers and quoted strings.

use nom::{
    branch::alt,
    bytes::complete::{take_until, take_while1},
    character::complete::char,
    sequence::delimited,
    IResult, Parser,
};

/// Parse an identifier (table name, field name): alphanumeric + underscore
pub(crate) fn identifier(input: &str) -> IResult<&str, &str> {
    take_while1(|c: char| c.is_alphanumeric() || c == '_').parse(input)
}

/// Parse a quoted string: 'content' or "content"
pub(crate) fn quoted_string(input: &str) -> IResult<&str, &str> {
    alt((
        delimited(char('\''), take_until("'"), char('\'')),
        delimited(char('"'), take_until("\""), char('"')),
    ))
    .parse(input)
}
