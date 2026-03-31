//! Primitive parsers for identifiers, literals, and basic tokens

use nom::{
    branch::alt,
    bytes::complete::{take_while, take_while1},
    character::complete::char,
    combinator::recognize,
    number::complete::double,
    sequence::pair,
    IResult, Parser,
};

/// Parse identifier (variable name, label, etc.)
///
/// Supports:
/// - Regular identifiers: starts with letter/underscore, then alphanumeric/underscore
/// - Backtick-quoted identifiers: `any-name-here` (can contain hyphens, etc.)
pub fn parse_identifier(input: &str) -> IResult<&str, String> {
    alt((parse_backtick_identifier, parse_regular_identifier)).parse(input)
}

/// Parse backtick-quoted identifier: `any-name`
fn parse_backtick_identifier(input: &str) -> IResult<&str, String> {
    let (input, _) = char('`').parse(input)?;
    let (input, content) = take_while1(|c: char| c != '`').parse(input)?;
    let (input, _) = char('`').parse(input)?;
    Ok((input, content.to_string()))
}

/// Parse regular (unquoted) identifier
fn parse_regular_identifier(input: &str) -> IResult<&str, String> {
    let (input, first) = take_while1(|c: char| c.is_alphabetic() || c == '_').parse(input)?;
    let (input, rest) = take_while(|c: char| c.is_alphanumeric() || c == '_').parse(input)?;

    let name = format!("{}{}", first, rest);
    let upper = name.to_uppercase();
    if matches!(
        upper.as_str(),
        "MATCH"
            | "WHERE"
            | "COLUMNS"
            | "AND"
            | "OR"
            | "NOT"
            | "AS"
            | "IN"
            | "BETWEEN"
            | "LIKE"
            | "IS"
            | "NULL"
            | "TRUE"
            | "FALSE"
    ) {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Tag,
        )));
    }

    Ok((input, name))
}

/// Parse identifier or star (for wildcards)
pub fn parse_identifier_or_star(input: &str) -> IResult<&str, &str> {
    alt((
        nom::bytes::complete::tag("*"),
        recognize(pair(
            take_while1(|c: char| c.is_alphabetic() || c == '_'),
            take_while(|c: char| c.is_alphanumeric() || c == '_'),
        )),
    ))
    .parse(input)
}

/// Parse identifier as &str (for use in JSON parsing where we need borrowed str)
pub fn parse_identifier_str(input: &str) -> IResult<&str, &str> {
    recognize(pair(
        take_while1(|c: char| c.is_alphabetic() || c == '_'),
        take_while(|c: char| c.is_alphanumeric() || c == '_'),
    ))
    .parse(input)
}

/// Parse string literal: 'value'
pub fn parse_string_literal(input: &str) -> IResult<&str, String> {
    let (input, _) = char('\'').parse(input)?;
    let (input, content) = take_while(|c| c != '\'').parse(input)?;
    let (input, _) = char('\'').parse(input)?;
    Ok((input, content.to_string()))
}

/// Parse number literal
pub fn parse_number_literal(input: &str) -> IResult<&str, f64> {
    double.parse(input)
}
