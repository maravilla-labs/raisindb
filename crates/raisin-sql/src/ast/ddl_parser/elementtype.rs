//! ElementType DDL parsers
//!
//! Parsers for CREATE ELEMENTTYPE, ALTER ELEMENTTYPE, and DROP ELEMENTTYPE statements.

use nom::{
    branch::alt,
    bytes::complete::tag_no_case,
    character::complete::{char, multispace0, multispace1},
    combinator::{map, opt},
    multi::many0,
    sequence::preceded,
    IResult, Parser,
};

use super::super::ddl::{
    AlterElementType, CreateElementType, DropElementType, ElementTypeAlteration,
};
use super::primitives::{boolean_literal, identifier, quoted_string, ws_and_comments};
use super::property::{property_def, property_list};

/// Parse CREATE ELEMENTTYPE statement
/// Supports both syntaxes:
///   CREATE ELEMENTTYPE 'name' FIELDS (...);
///   CREATE ELEMENTTYPE 'name' (FIELDS (...));
pub(crate) fn create_elementtype(input: &str) -> IResult<&str, CreateElementType> {
    let (input, _) = (
        tag_no_case("CREATE"),
        multispace1,
        tag_no_case("ELEMENTTYPE"),
        multispace1,
    )
        .parse(input)?;

    let (input, name) = quoted_string(input)?;
    let (input, _) = multispace0.parse(input)?;

    // Check for optional opening paren (SQL-conformant syntax)
    let (input, has_paren) = opt(char('(')).parse(input)?;
    let (input, _) = multispace0.parse(input)?;

    let mut result = CreateElementType {
        name: name.to_string(),
        ..Default::default()
    };

    let (input, _) = parse_elementtype_clauses(input, &mut result)?;

    // If we had opening paren, require closing paren
    let input = if has_paren.is_some() {
        let (input, _) = multispace0.parse(input)?;
        let (input, _) = char(')').parse(input)?;
        input
    } else {
        input
    };

    Ok((input, result))
}

/// Parse ElementType clauses
fn parse_elementtype_clauses<'a>(
    mut input: &'a str,
    result: &mut CreateElementType,
) -> IResult<&'a str, ()> {
    loop {
        // Skip whitespace AND comments between clauses
        let (remaining, _) = ws_and_comments(input)?;

        if let Ok((new_input, desc)) =
            preceded((tag_no_case("DESCRIPTION"), multispace1), quoted_string).parse(remaining)
        {
            result.description = Some(desc.to_string());
            input = new_input;
            continue;
        }

        if let Ok((new_input, icon)) =
            preceded((tag_no_case("ICON"), multispace1), quoted_string).parse(remaining)
        {
            result.icon = Some(icon.to_string());
            input = new_input;
            continue;
        }

        if let Ok((new_input, fields)) =
            preceded((tag_no_case("FIELDS"), multispace0), property_list).parse(remaining)
        {
            result.fields = fields;
            input = new_input;
            continue;
        }

        if let Ok((new_input, _)) =
            tag_no_case::<_, _, nom::error::Error<&str>>("PUBLISHABLE").parse(remaining)
        {
            result.publishable = true;
            input = new_input;
            continue;
        }

        break;
    }

    Ok((input, ()))
}

// =============================================================================
// ALTER ELEMENTTYPE
// =============================================================================

/// Parse ALTER ELEMENTTYPE statement
pub(crate) fn alter_elementtype(input: &str) -> IResult<&str, AlterElementType> {
    let (input, _) = (
        tag_no_case("ALTER"),
        multispace1,
        tag_no_case("ELEMENTTYPE"),
        multispace1,
    )
        .parse(input)?;

    let (input, name) = quoted_string(input)?;
    let (input, _) = multispace0.parse(input)?;

    let (input, alterations) = many0(preceded(multispace0, elementtype_alteration)).parse(input)?;

    Ok((
        input,
        AlterElementType {
            name: name.to_string(),
            alterations,
        },
    ))
}

/// Parse individual ElementType alteration
pub(crate) fn elementtype_alteration(input: &str) -> IResult<&str, ElementTypeAlteration> {
    alt((
        map(
            preceded(
                (
                    tag_no_case("ADD"),
                    multispace1,
                    tag_no_case("FIELD"),
                    multispace1,
                ),
                property_def,
            ),
            ElementTypeAlteration::AddField,
        ),
        map(
            preceded(
                (
                    tag_no_case("DROP"),
                    multispace1,
                    tag_no_case("FIELD"),
                    multispace1,
                ),
                identifier,
            ),
            |name| ElementTypeAlteration::DropField(name.to_string()),
        ),
        map(
            preceded(
                (
                    tag_no_case("MODIFY"),
                    multispace1,
                    tag_no_case("FIELD"),
                    multispace1,
                ),
                property_def,
            ),
            ElementTypeAlteration::ModifyField,
        ),
        map(
            preceded(
                (
                    tag_no_case("SET"),
                    multispace1,
                    tag_no_case("DESCRIPTION"),
                    multispace0,
                    char('='),
                    multispace0,
                ),
                quoted_string,
            ),
            |s| ElementTypeAlteration::SetDescription(s.to_string()),
        ),
        map(
            preceded(
                (
                    tag_no_case("SET"),
                    multispace1,
                    tag_no_case("ICON"),
                    multispace0,
                    char('='),
                    multispace0,
                ),
                quoted_string,
            ),
            |s| ElementTypeAlteration::SetIcon(s.to_string()),
        ),
        map(
            preceded(
                (
                    tag_no_case("SET"),
                    multispace1,
                    tag_no_case("PUBLISHABLE"),
                    multispace0,
                    char('='),
                    multispace0,
                ),
                boolean_literal,
            ),
            ElementTypeAlteration::SetPublishable,
        ),
    ))
    .parse(input)
}

// =============================================================================
// DROP ELEMENTTYPE
// =============================================================================

/// Parse DROP ELEMENTTYPE statement
pub(crate) fn drop_elementtype(input: &str) -> IResult<&str, DropElementType> {
    let (input, _) = (
        tag_no_case("DROP"),
        multispace1,
        tag_no_case("ELEMENTTYPE"),
        multispace1,
    )
        .parse(input)?;

    let (input, name) = quoted_string(input)?;
    let (input, _) = multispace0.parse(input)?;

    let (input, cascade) = opt(tag_no_case("CASCADE")).parse(input)?;

    Ok((
        input,
        DropElementType {
            name: name.to_string(),
            cascade: cascade.is_some(),
        },
    ))
}
