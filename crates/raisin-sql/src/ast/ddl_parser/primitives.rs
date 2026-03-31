//! Primitive parsers for DDL parsing
//!
//! Low-level nom combinators for parsing quoted strings, identifiers,
//! booleans, numbers, and whitespace/comments.

use nom::{
    branch::alt,
    bytes::complete::{tag_no_case, take_until, take_while, take_while1},
    character::complete::{char, multispace0},
    combinator::{map, map_res, opt, recognize, value},
    multi::separated_list0,
    sequence::{delimited, pair},
    IResult, Parser,
};

use super::super::ddl::DefaultValue;

/// Strip leading SQL comments (-- and /* */) to find the actual statement start
pub(crate) fn strip_leading_comments(sql: &str) -> &str {
    let mut s = sql.trim();

    loop {
        // Skip single-line comments
        if s.starts_with("--") {
            if let Some(newline_pos) = s.find('\n') {
                s = s[newline_pos + 1..].trim_start();
                continue;
            } else {
                // Entire remaining string is a comment
                return "";
            }
        }

        // Skip multi-line comments
        if s.starts_with("/*") {
            if let Some(end_pos) = s.find("*/") {
                s = s[end_pos + 2..].trim_start();
                continue;
            } else {
                // Unclosed comment
                return "";
            }
        }

        // No more comments
        break;
    }

    s
}

/// Parse a quoted string: 'content' or "content"
pub(crate) fn quoted_string(input: &str) -> IResult<&str, &str> {
    alt((
        delimited(char('\''), take_until("'"), char('\'')),
        delimited(char('"'), take_until("\""), char('"')),
    ))
    .parse(input)
}

/// Parse a list of quoted strings: ('a', 'b', 'c')
pub(crate) fn quoted_string_list(input: &str) -> IResult<&str, Vec<&str>> {
    delimited(
        (char('('), multispace0),
        separated_list0((multispace0, char(','), multispace0), quoted_string),
        (multispace0, opt(char(',')), multispace0, char(')')),
    )
    .parse(input)
}

/// Parse an identifier (alphanumeric + underscore, starting with letter or underscore)
pub(crate) fn identifier(input: &str) -> IResult<&str, &str> {
    recognize(pair(
        take_while1(|c: char| c.is_alphabetic() || c == '_'),
        take_while(|c: char| c.is_alphanumeric() || c == '_'),
    ))
    .parse(input)
}

/// Parse a boolean literal: true or false
pub(crate) fn boolean_literal(input: &str) -> IResult<&str, bool> {
    alt((
        value(true, tag_no_case("true")),
        value(false, tag_no_case("false")),
    ))
    .parse(input)
}

/// Parse a default value: 'string', number, true/false, or NULL
pub(crate) fn default_value(input: &str) -> IResult<&str, DefaultValue> {
    alt((
        map(tag_no_case("NULL"), |_| DefaultValue::Null),
        map(boolean_literal, DefaultValue::Boolean),
        map(number_literal, DefaultValue::Number),
        map(quoted_string, |s| DefaultValue::String(s.to_string())),
    ))
    .parse(input)
}

/// Parse a number literal (integer or float)
pub(crate) fn number_literal(input: &str) -> IResult<&str, f64> {
    map_res(
        recognize((
            opt(char('-')),
            take_while1(|c: char| c.is_ascii_digit()),
            opt((char('.'), take_while1(|c: char| c.is_ascii_digit()))),
        )),
        |s: &str| s.parse::<f64>(),
    )
    .parse(input)
}

/// Parse an integer literal
pub(crate) fn integer_literal(input: &str) -> IResult<&str, i32> {
    map_res(
        recognize((opt(char('-')), take_while1(|c: char| c.is_ascii_digit()))),
        |s: &str| s.parse::<i32>(),
    )
    .parse(input)
}

/// Parse whitespace and SQL comments (-- line comments and /* block comments */)
/// This is used between clauses to allow comments in DDL statements
pub(crate) fn ws_and_comments(input: &str) -> IResult<&str, &str> {
    let start = input;
    let result = strip_leading_comments(input);
    // Calculate how much we consumed
    let consumed_len = result.as_ptr() as usize - start.as_ptr() as usize;
    Ok((result, &start[..consumed_len]))
}
