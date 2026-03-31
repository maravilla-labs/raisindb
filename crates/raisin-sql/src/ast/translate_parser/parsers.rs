//! Core TRANSLATE statement parsers
//!
//! Contains the main statement parser, assignment parser, and path parsers
//! for translation-aware UPDATE statements.

use nom::{
    branch::alt,
    bytes::complete::tag_no_case,
    character::complete::{char, multispace0, multispace1},
    combinator::opt,
    multi::separated_list1,
    sequence::{preceded, tuple},
    IResult, Parser,
};

use super::filters::{translate_filter, translation_value};
use super::helpers::{identifier, quoted_string};
use crate::ast::translate::{TranslateStatement, TranslationAssignment, TranslationPath};

/// Error type for TRANSLATE statement parsing
#[derive(Debug, Clone, PartialEq)]
pub struct TranslateParseError {
    pub message: String,
    pub position: Option<usize>,
}

impl std::fmt::Display for TranslateParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(pos) = self.position {
            write!(
                f,
                "TRANSLATE parse error at position {}: {}",
                pos, self.message
            )
        } else {
            write!(f, "TRANSLATE parse error: {}", self.message)
        }
    }
}

impl std::error::Error for TranslateParseError {}

/// Check if a SQL statement is a translation-aware UPDATE statement
///
/// Translation statement starts with "UPDATE" and contains "FOR LOCALE"
pub fn is_translate_statement(sql: &str) -> bool {
    let trimmed = sql.trim();
    let upper = trimmed.to_uppercase();

    // Must start with UPDATE
    if !upper.starts_with("UPDATE") {
        return false;
    }

    // Must contain FOR LOCALE somewhere after UPDATE
    upper.contains("FOR LOCALE")
}

/// Parse a translation-aware UPDATE statement from SQL string
///
/// Returns `Some(TranslateStatement)` if the input is a valid translation statement,
/// `None` if it's not a translation statement (should be handled by other parsers).
pub fn parse_translate(sql: &str) -> Result<Option<TranslateStatement>, TranslateParseError> {
    let trimmed = sql.trim();

    // Strip leading SQL comments
    let statement_start = crate::ast::ddl_parser::strip_leading_comments(trimmed);

    // Check if this is a translation statement
    if !is_translate_statement(statement_start) {
        return Ok(None);
    }

    // Calculate offset for error position mapping
    let offset_to_statement_start = statement_start.as_ptr() as usize - sql.as_ptr() as usize;

    match translate_statement(statement_start) {
        Ok((remaining, stmt)) => {
            // Verify we consumed all input (except whitespace and semicolon)
            let remaining_trimmed = remaining.trim().trim_end_matches(';').trim();
            if remaining_trimmed.is_empty() {
                Ok(Some(stmt))
            } else {
                let position_in_statement = statement_start.len() - remaining.len();
                let position = offset_to_statement_start + position_in_statement;
                Err(TranslateParseError {
                    message: format!("Unexpected trailing content: '{}'", remaining_trimmed),
                    position: Some(position),
                })
            }
        }
        Err(e) => {
            let (position, message) = match &e {
                nom::Err::Failure(err) | nom::Err::Error(err) => {
                    let pos_in_statement = statement_start.len() - err.input.len();
                    let remaining = err.input.trim();
                    let problematic: String = remaining
                        .chars()
                        .take(30)
                        .take_while(|c| *c != '\n')
                        .collect();
                    (
                        Some(offset_to_statement_start + pos_in_statement),
                        format!("Parse error near: '{}'", problematic.trim()),
                    )
                }
                nom::Err::Incomplete(_) => (None, "Incomplete TRANSLATE statement".to_string()),
            };
            Err(TranslateParseError { message, position })
        }
    }
}

/// Parse the full translation statement:
/// UPDATE Table FOR LOCALE 'xx' [IN BRANCH 'yy'] SET assignments [WHERE filter]
fn translate_statement(input: &str) -> IResult<&str, TranslateStatement> {
    let (input, _) = tag_no_case("UPDATE").parse(input)?;
    let (input, _) = multispace1.parse(input)?;

    // Parse table name (identifier)
    let (input, table) = identifier(input)?;
    let (input, _) = multispace1.parse(input)?;

    // Parse FOR LOCALE 'xx'
    let (input, _) = tag_no_case("FOR").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("LOCALE").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, locale) = quoted_string(input)?;

    // Parse optional IN BRANCH 'xx' clause
    let (input, branch) = opt(preceded(
        tuple((
            multispace1,
            tag_no_case("IN"),
            multispace1,
            tag_no_case("BRANCH"),
            multispace1,
        )),
        quoted_string,
    ))
    .parse(input)?;

    let (input, _) = multispace1.parse(input)?;

    // Parse SET keyword
    let (input, _) = tag_no_case("SET").parse(input)?;
    let (input, _) = multispace1.parse(input)?;

    // Parse comma-separated assignments
    let (input, assignments) = separated_list1(
        tuple((multispace0, char(','), multispace0)),
        translation_assignment,
    )
    .parse(input)?;

    // Parse optional WHERE clause
    let (input, filter) = opt(preceded(
        tuple((multispace1, tag_no_case("WHERE"), multispace1)),
        translate_filter,
    ))
    .parse(input)?;

    Ok((
        input,
        TranslateStatement::with_branch(
            table,
            locale,
            branch.map(|s| s.to_string()),
            assignments,
            filter,
        ),
    ))
}

/// Parse a translation assignment: path = value
fn translation_assignment(input: &str) -> IResult<&str, TranslationAssignment> {
    let (input, path) = translation_path(input)?;
    let (input, _) = tuple((multispace0, char('='), multispace0)).parse(input)?;
    let (input, value) = translation_value(input)?;

    Ok((input, TranslationAssignment::new(path, value)))
}

/// Parse a translation path: either simple property or block property
fn translation_path(input: &str) -> IResult<&str, TranslationPath> {
    alt((block_property_path, simple_property_path)).parse(input)
}

/// Parse simple property path: `field` or `field.nested.deep`
fn simple_property_path(input: &str) -> IResult<&str, TranslationPath> {
    let (input, first) = identifier(input)?;
    let (input, rest) = nom::multi::many0(preceded(char('.'), identifier)).parse(input)?;

    let mut segments = vec![first.to_string()];
    segments.extend(rest.into_iter().map(|s| s.to_string()));

    Ok((input, TranslationPath::Property(segments)))
}

/// Parse block property path: `blocks[uuid='...'].content.text`
fn block_property_path(input: &str) -> IResult<&str, TranslationPath> {
    // Parse array field name
    let (input, array_field) = identifier(input)?;

    // Parse [uuid='...']
    let (input, _) = char('[').parse(input)?;
    let (input, _) = multispace0.parse(input)?;
    let (input, _) = tag_no_case("uuid").parse(input)?;
    let (input, _) = multispace0.parse(input)?;
    let (input, _) = char('=').parse(input)?;
    let (input, _) = multispace0.parse(input)?;
    let (input, block_uuid) = quoted_string(input)?;
    let (input, _) = multispace0.parse(input)?;
    let (input, _) = char(']').parse(input)?;

    // Parse property path within the block: .content.text
    let (input, property_segments) =
        nom::multi::many1(preceded(char('.'), identifier)).parse(input)?;

    Ok((
        input,
        TranslationPath::BlockProperty {
            array_field: array_field.to_string(),
            block_uuid: block_uuid.to_string(),
            property_path: property_segments
                .into_iter()
                .map(|s| s.to_string())
                .collect(),
        },
    ))
}
