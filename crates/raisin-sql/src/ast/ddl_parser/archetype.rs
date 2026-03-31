//! Archetype DDL parsers
//!
//! Parsers for CREATE ARCHETYPE, ALTER ARCHETYPE, and DROP ARCHETYPE statements.
//!
// NOTE: File intentionally exceeds 300 lines - single parser with tightly coupled alt() match arms is idiomatic Rust

use nom::{
    branch::alt,
    bytes::complete::tag_no_case,
    character::complete::{char, multispace0, multispace1},
    combinator::{map, opt},
    multi::many0,
    sequence::preceded,
    IResult, Parser,
};

use super::super::ddl::{AlterArchetype, ArchetypeAlteration, CreateArchetype, DropArchetype};
use super::primitives::{boolean_literal, identifier, quoted_string, ws_and_comments};
use super::property::{property_def, property_list};

/// Parse CREATE ARCHETYPE statement
/// Supports both syntaxes:
///   CREATE ARCHETYPE 'name' BASE_NODE_TYPE '...' FIELDS (...);
///   CREATE ARCHETYPE 'name' (BASE_NODE_TYPE '...' FIELDS (...));
pub(crate) fn create_archetype(input: &str) -> IResult<&str, CreateArchetype> {
    let (input, _) = (
        tag_no_case("CREATE"),
        multispace1,
        tag_no_case("ARCHETYPE"),
        multispace1,
    )
        .parse(input)?;

    let (input, name) = quoted_string(input)?;
    let (input, _) = multispace0.parse(input)?;

    // Check for optional opening paren (SQL-conformant syntax)
    let (input, has_paren) = opt(char('(')).parse(input)?;
    let (input, _) = multispace0.parse(input)?;

    let mut result = CreateArchetype {
        name: name.to_string(),
        ..Default::default()
    };

    let (input, _) = parse_archetype_clauses(input, &mut result)?;

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

/// Parse Archetype clauses
fn parse_archetype_clauses<'a>(
    mut input: &'a str,
    result: &mut CreateArchetype,
) -> IResult<&'a str, ()> {
    loop {
        // Skip whitespace AND comments between clauses
        let (remaining, _) = ws_and_comments(input)?;

        if let Ok((new_input, extends)) =
            preceded((tag_no_case("EXTENDS"), multispace1), quoted_string).parse(remaining)
        {
            result.extends = Some(extends.to_string());
            input = new_input;
            continue;
        }

        if let Ok((new_input, base)) =
            preceded((tag_no_case("BASE_NODE_TYPE"), multispace1), quoted_string).parse(remaining)
        {
            result.base_node_type = Some(base.to_string());
            input = new_input;
            continue;
        }

        if let Ok((new_input, title)) =
            preceded((tag_no_case("TITLE"), multispace1), quoted_string).parse(remaining)
        {
            result.title = Some(title.to_string());
            input = new_input;
            continue;
        }

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
// ALTER ARCHETYPE
// =============================================================================

/// Parse ALTER ARCHETYPE statement
pub(crate) fn alter_archetype(input: &str) -> IResult<&str, AlterArchetype> {
    let (input, _) = (
        tag_no_case("ALTER"),
        multispace1,
        tag_no_case("ARCHETYPE"),
        multispace1,
    )
        .parse(input)?;

    let (input, name) = quoted_string(input)?;
    let (input, _) = multispace0.parse(input)?;

    let (input, alterations) = many0(preceded(multispace0, archetype_alteration)).parse(input)?;

    Ok((
        input,
        AlterArchetype {
            name: name.to_string(),
            alterations,
        },
    ))
}

/// Parse individual Archetype alteration
pub(crate) fn archetype_alteration(input: &str) -> IResult<&str, ArchetypeAlteration> {
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
            ArchetypeAlteration::AddField,
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
            |name| ArchetypeAlteration::DropField(name.to_string()),
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
            ArchetypeAlteration::ModifyField,
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
            |s| ArchetypeAlteration::SetDescription(s.to_string()),
        ),
        map(
            preceded(
                (
                    tag_no_case("SET"),
                    multispace1,
                    tag_no_case("TITLE"),
                    multispace0,
                    char('='),
                    multispace0,
                ),
                quoted_string,
            ),
            |s| ArchetypeAlteration::SetTitle(s.to_string()),
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
            |s| ArchetypeAlteration::SetIcon(s.to_string()),
        ),
        map(
            preceded(
                (
                    tag_no_case("SET"),
                    multispace1,
                    tag_no_case("BASE_NODE_TYPE"),
                    multispace0,
                    char('='),
                    multispace0,
                ),
                alt((
                    map(tag_no_case("NULL"), |_| None),
                    map(quoted_string, |s| Some(s.to_string())),
                )),
            ),
            ArchetypeAlteration::SetBaseNodeType,
        ),
        map(
            preceded(
                (
                    tag_no_case("SET"),
                    multispace1,
                    tag_no_case("EXTENDS"),
                    multispace0,
                    char('='),
                    multispace0,
                ),
                alt((
                    map(tag_no_case("NULL"), |_| None),
                    map(quoted_string, |s| Some(s.to_string())),
                )),
            ),
            ArchetypeAlteration::SetExtends,
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
            ArchetypeAlteration::SetPublishable,
        ),
    ))
    .parse(input)
}

// =============================================================================
// DROP ARCHETYPE
// =============================================================================

/// Parse DROP ARCHETYPE statement
pub(crate) fn drop_archetype(input: &str) -> IResult<&str, DropArchetype> {
    let (input, _) = (
        tag_no_case("DROP"),
        multispace1,
        tag_no_case("ARCHETYPE"),
        multispace1,
    )
        .parse(input)?;

    let (input, name) = quoted_string(input)?;
    let (input, _) = multispace0.parse(input)?;

    let (input, cascade) = opt(tag_no_case("CASCADE")).parse(input)?;

    Ok((
        input,
        DropArchetype {
            name: name.to_string(),
            cascade: cascade.is_some(),
        },
    ))
}
