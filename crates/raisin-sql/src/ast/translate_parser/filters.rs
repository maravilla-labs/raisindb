//! Value and filter parsers for TRANSLATE statements
//!
//! Contains parsers for translation values (strings, numbers, booleans, NULL)
//! and WHERE filter clauses (path, id, node_type combinations).

use nom::{
    branch::alt,
    bytes::complete::{tag_no_case, take_while1},
    character::complete::{char, multispace0, multispace1},
    combinator::{map, opt},
    sequence::tuple,
    IResult, Parser,
};

use super::helpers::quoted_string;
use crate::ast::translate::{TranslateFilter, TranslationValue};

/// Parse a translation value: string literal, number, boolean, or NULL
pub(crate) fn translation_value(input: &str) -> IResult<&str, TranslationValue> {
    alt((
        // NULL
        map(tag_no_case("NULL"), |_| TranslationValue::Null),
        // Boolean
        map(tag_no_case("true"), |_| TranslationValue::Boolean(true)),
        map(tag_no_case("false"), |_| TranslationValue::Boolean(false)),
        // Float (must come before integer to capture decimal point)
        map(float_literal, TranslationValue::Float),
        // Integer
        map(integer_literal, TranslationValue::Integer),
        // String (quoted)
        map(quoted_string, |s| TranslationValue::String(s.to_string())),
    ))
    .parse(input)
}

/// Parse an integer literal
fn integer_literal(input: &str) -> IResult<&str, i64> {
    let (input, sign) = opt(alt((char('-'), char('+')))).parse(input)?;
    let (input, digits) = take_while1(|c: char| c.is_ascii_digit()).parse(input)?;

    let value: i64 = digits.parse().map_err(|_| {
        nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Digit))
    })?;

    let value = if sign == Some('-') { -value } else { value };
    Ok((input, value))
}

/// Parse a float literal
fn float_literal(input: &str) -> IResult<&str, f64> {
    let (input, sign) = opt(alt((char('-'), char('+')))).parse(input)?;
    let (input, integer_part) = take_while1(|c: char| c.is_ascii_digit()).parse(input)?;
    let (input, _) = char('.').parse(input)?;
    let (input, decimal_part) = take_while1(|c: char| c.is_ascii_digit()).parse(input)?;

    let num_str = format!("{}.{}", integer_part, decimal_part);
    let value: f64 = num_str.parse().map_err(|_| {
        nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Float))
    })?;

    let value = if sign == Some('-') { -value } else { value };
    Ok((input, value))
}

/// Parse the WHERE filter clause
pub(crate) fn translate_filter(input: &str) -> IResult<&str, TranslateFilter> {
    alt((
        // path = '...' AND node_type = '...'
        path_and_type_filter,
        // id = '...' AND node_type = '...'
        id_and_type_filter,
        // path = '...'
        path_filter,
        // id = '...'
        id_filter,
        // node_type = '...'
        node_type_filter,
    ))
    .parse(input)
}

/// Parse: path = '...'
fn path_filter(input: &str) -> IResult<&str, TranslateFilter> {
    let (input, _) = tag_no_case("path").parse(input)?;
    let (input, _) = tuple((multispace0, char('='), multispace0)).parse(input)?;
    let (input, path) = quoted_string(input)?;

    Ok((input, TranslateFilter::Path(path.to_string())))
}

/// Parse: id = '...'
fn id_filter(input: &str) -> IResult<&str, TranslateFilter> {
    let (input, _) = tag_no_case("id").parse(input)?;
    let (input, _) = tuple((multispace0, char('='), multispace0)).parse(input)?;
    let (input, id) = quoted_string(input)?;

    Ok((input, TranslateFilter::Id(id.to_string())))
}

/// Parse: node_type = '...'
fn node_type_filter(input: &str) -> IResult<&str, TranslateFilter> {
    let (input, _) = tag_no_case("node_type").parse(input)?;
    let (input, _) = tuple((multispace0, char('='), multispace0)).parse(input)?;
    let (input, node_type) = quoted_string(input)?;

    Ok((input, TranslateFilter::NodeType(node_type.to_string())))
}

/// Parse: path = '...' AND node_type = '...'
fn path_and_type_filter(input: &str) -> IResult<&str, TranslateFilter> {
    let (input, _) = tag_no_case("path").parse(input)?;
    let (input, _) = tuple((multispace0, char('='), multispace0)).parse(input)?;
    let (input, path) = quoted_string(input)?;
    let (input, _) = tuple((multispace1, tag_no_case("AND"), multispace1)).parse(input)?;
    let (input, _) = tag_no_case("node_type").parse(input)?;
    let (input, _) = tuple((multispace0, char('='), multispace0)).parse(input)?;
    let (input, node_type) = quoted_string(input)?;

    Ok((
        input,
        TranslateFilter::PathAndType {
            path: path.to_string(),
            node_type: node_type.to_string(),
        },
    ))
}

/// Parse: id = '...' AND node_type = '...'
fn id_and_type_filter(input: &str) -> IResult<&str, TranslateFilter> {
    let (input, _) = tag_no_case("id").parse(input)?;
    let (input, _) = tuple((multispace0, char('='), multispace0)).parse(input)?;
    let (input, id) = quoted_string(input)?;
    let (input, _) = tuple((multispace1, tag_no_case("AND"), multispace1)).parse(input)?;
    let (input, _) = tag_no_case("node_type").parse(input)?;
    let (input, _) = tuple((multispace0, char('='), multispace0)).parse(input)?;
    let (input, node_type) = quoted_string(input)?;

    Ok((
        input,
        TranslateFilter::IdAndType {
            id: id.to_string(),
            node_type: node_type.to_string(),
        },
    ))
}
