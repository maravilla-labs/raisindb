//! Mixin DDL parsers
//!
//! Parsers for CREATE MIXIN, ALTER MIXIN, and DROP MIXIN statements.

use nom::{
    branch::alt,
    bytes::complete::tag_no_case,
    character::complete::{char, multispace0, multispace1},
    combinator::{map, opt},
    multi::many0,
    sequence::preceded,
    IResult, Parser,
};

use super::super::ddl::{AlterMixin, CreateMixin, DropMixin, MixinAlteration};
use super::primitives::{quoted_string, ws_and_comments};
use super::property::{preceded_property_list, property_def, property_name_or_path};

/// Parse CREATE MIXIN statement
pub(crate) fn create_mixin(input: &str) -> IResult<&str, CreateMixin> {
    let (input, _) = (
        tag_no_case("CREATE"),
        multispace1,
        tag_no_case("MIXIN"),
        multispace1,
    )
        .parse(input)?;

    let (input, name) = quoted_string(input)?;
    let (input, _) = multispace0.parse(input)?;

    let mut result = CreateMixin {
        name: name.to_string(),
        ..Default::default()
    };

    let (input, _) = parse_mixin_clauses(input, &mut result)?;

    Ok((input, result))
}

/// Parse Mixin clauses (DESCRIPTION, ICON, PROPERTIES)
fn parse_mixin_clauses<'a>(mut input: &'a str, result: &mut CreateMixin) -> IResult<&'a str, ()> {
    loop {
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

        if remaining.to_uppercase().starts_with("PROPERTIES") {
            let (new_input, props) = preceded_property_list(remaining)?;
            result.properties = props;
            input = new_input;
            continue;
        }

        break;
    }

    Ok((input, ()))
}

// =============================================================================
// ALTER MIXIN
// =============================================================================

/// Parse ALTER MIXIN statement
pub(crate) fn alter_mixin(input: &str) -> IResult<&str, AlterMixin> {
    let (input, _) = (
        tag_no_case("ALTER"),
        multispace1,
        tag_no_case("MIXIN"),
        multispace1,
    )
        .parse(input)?;

    let (input, name) = quoted_string(input)?;
    let (input, _) = multispace0.parse(input)?;

    let (input, alterations) = many0(preceded(multispace0, mixin_alteration)).parse(input)?;

    Ok((
        input,
        AlterMixin {
            name: name.to_string(),
            alterations,
        },
    ))
}

/// Parse individual Mixin alteration
fn mixin_alteration(input: &str) -> IResult<&str, MixinAlteration> {
    alt((
        // ADD PROPERTY
        map(
            preceded(
                (
                    tag_no_case("ADD"),
                    multispace1,
                    tag_no_case("PROPERTY"),
                    multispace1,
                ),
                property_def,
            ),
            MixinAlteration::AddProperty,
        ),
        // DROP PROPERTY
        map(
            preceded(
                (
                    tag_no_case("DROP"),
                    multispace1,
                    tag_no_case("PROPERTY"),
                    multispace1,
                ),
                property_name_or_path,
            ),
            MixinAlteration::DropProperty,
        ),
        // MODIFY PROPERTY
        map(
            preceded(
                (
                    tag_no_case("MODIFY"),
                    multispace1,
                    tag_no_case("PROPERTY"),
                    multispace1,
                ),
                property_def,
            ),
            MixinAlteration::ModifyProperty,
        ),
        // SET DESCRIPTION = '...'
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
            |s| MixinAlteration::SetDescription(s.to_string()),
        ),
        // SET ICON = '...'
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
            |s| MixinAlteration::SetIcon(s.to_string()),
        ),
    ))
    .parse(input)
}

// =============================================================================
// DROP MIXIN
// =============================================================================

/// Parse DROP MIXIN statement
pub(crate) fn drop_mixin(input: &str) -> IResult<&str, DropMixin> {
    let (input, _) = (
        tag_no_case("DROP"),
        multispace1,
        tag_no_case("MIXIN"),
        multispace1,
    )
        .parse(input)?;

    let (input, name) = quoted_string(input)?;
    let (input, _) = multispace0.parse(input)?;

    let (input, cascade) = opt(tag_no_case("CASCADE")).parse(input)?;

    Ok((
        input,
        DropMixin {
            name: name.to_string(),
            cascade: cascade.is_some(),
        },
    ))
}
