//! NodeType DDL parsers
//!
//! Parsers for CREATE NODETYPE, ALTER NODETYPE, and DROP NODETYPE statements.
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

use super::super::ddl::{AlterNodeType, CreateNodeType, DropNodeType, NodeTypeAlteration};
use super::compound_index::compound_index;
use super::primitives::{boolean_literal, quoted_string, quoted_string_list, ws_and_comments};
use super::property::{preceded_property_list, property_def, property_name_or_path};

/// Parse CREATE NODETYPE statement
/// Supports both syntaxes:
///   CREATE NODETYPE 'name' PROPERTIES (...) FLAGS;
///   CREATE NODETYPE 'name' (PROPERTIES (...) FLAGS);
pub(crate) fn create_nodetype(input: &str) -> IResult<&str, CreateNodeType> {
    let (input, _) = (
        tag_no_case("CREATE"),
        multispace1,
        tag_no_case("NODETYPE"),
        multispace1,
    )
        .parse(input)?;

    let (input, name) = quoted_string(input)?;
    let (input, _) = multispace0.parse(input)?;

    // Check for optional opening paren (SQL-conformant syntax)
    let (input, has_paren) = opt(char('(')).parse(input)?;
    let (input, _) = multispace0.parse(input)?;

    // Parse optional clauses in any order
    let mut result = CreateNodeType {
        name: name.to_string(),
        ..Default::default()
    };

    let (input, _) = parse_nodetype_clauses(input, &mut result)?;

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

/// Parse NodeType clauses (EXTENDS, MIXINS, PROPERTIES, etc.)
fn parse_nodetype_clauses<'a>(
    mut input: &'a str,
    result: &mut CreateNodeType,
) -> IResult<&'a str, ()> {
    loop {
        // Skip whitespace AND comments between clauses
        let (remaining, _) = ws_and_comments(input)?;

        // Try each clause type
        if let Ok((new_input, extends)) =
            preceded((tag_no_case("EXTENDS"), multispace1), quoted_string).parse(remaining)
        {
            result.extends = Some(extends.to_string());
            input = new_input;
            continue;
        }

        if let Ok((new_input, mixins)) =
            preceded((tag_no_case("MIXINS"), multispace0), quoted_string_list).parse(remaining)
        {
            result.mixins = mixins.into_iter().map(|s| s.to_string()).collect();
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

        // PROPERTIES clause - use cut() to ensure errors propagate once keyword matches
        if remaining.to_uppercase().starts_with("PROPERTIES") {
            let (new_input, props) = preceded_property_list(remaining)?;
            result.properties = props;
            input = new_input;
            continue;
        }

        if let Ok((new_input, children)) = preceded(
            (tag_no_case("ALLOWED_CHILDREN"), multispace0),
            quoted_string_list,
        )
        .parse(remaining)
        {
            result.allowed_children = children.into_iter().map(|s| s.to_string()).collect();
            input = new_input;
            continue;
        }

        if let Ok((new_input, required)) = preceded(
            (tag_no_case("REQUIRED_NODES"), multispace0),
            quoted_string_list,
        )
        .parse(remaining)
        {
            result.required_nodes = required.into_iter().map(|s| s.to_string()).collect();
            input = new_input;
            continue;
        }

        // COMPOUND_INDEX 'name' ON (columns...)
        if remaining.to_uppercase().starts_with("COMPOUND_INDEX") {
            let (new_input, idx) = compound_index(remaining)?;
            result.compound_indexes.push(idx);
            input = new_input;
            continue;
        }

        // Boolean flags
        if let Ok((new_input, _)) =
            tag_no_case::<_, _, nom::error::Error<&str>>("VERSIONABLE").parse(remaining)
        {
            result.versionable = true;
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

        if let Ok((new_input, _)) =
            tag_no_case::<_, _, nom::error::Error<&str>>("AUDITABLE").parse(remaining)
        {
            result.auditable = true;
            input = new_input;
            continue;
        }

        if let Ok((new_input, _)) =
            tag_no_case::<_, _, nom::error::Error<&str>>("INDEXABLE").parse(remaining)
        {
            result.indexable = true;
            input = new_input;
            continue;
        }

        if let Ok((new_input, _)) =
            tag_no_case::<_, _, nom::error::Error<&str>>("STRICT").parse(remaining)
        {
            result.strict = true;
            input = new_input;
            continue;
        }

        // No more clauses found
        break;
    }

    Ok((input, ()))
}

// =============================================================================
// ALTER NODETYPE
// =============================================================================

/// Parse ALTER NODETYPE statement
pub(crate) fn alter_nodetype(input: &str) -> IResult<&str, AlterNodeType> {
    let (input, _) = (
        tag_no_case("ALTER"),
        multispace1,
        tag_no_case("NODETYPE"),
        multispace1,
    )
        .parse(input)?;

    let (input, name) = quoted_string(input)?;
    let (input, _) = multispace0.parse(input)?;

    let (input, alterations) = many0(preceded(multispace0, nodetype_alteration)).parse(input)?;

    Ok((
        input,
        AlterNodeType {
            name: name.to_string(),
            alterations,
        },
    ))
}

/// Parse individual NodeType alteration
pub(crate) fn nodetype_alteration(input: &str) -> IResult<&str, NodeTypeAlteration> {
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
            NodeTypeAlteration::AddProperty,
        ),
        // DROP PROPERTY name or 'nested.path'
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
            NodeTypeAlteration::DropProperty,
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
            NodeTypeAlteration::ModifyProperty,
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
            |s| NodeTypeAlteration::SetDescription(s.to_string()),
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
            |s| NodeTypeAlteration::SetIcon(s.to_string()),
        ),
        // SET EXTENDS = '...' or SET EXTENDS = NULL
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
            NodeTypeAlteration::SetExtends,
        ),
        // SET ALLOWED_CHILDREN = (...)
        map(
            preceded(
                (
                    tag_no_case("SET"),
                    multispace1,
                    tag_no_case("ALLOWED_CHILDREN"),
                    multispace0,
                    char('='),
                    multispace0,
                ),
                quoted_string_list,
            ),
            |list| {
                NodeTypeAlteration::SetAllowedChildren(
                    list.into_iter().map(|s| s.to_string()).collect(),
                )
            },
        ),
        // SET REQUIRED_NODES = (...)
        map(
            preceded(
                (
                    tag_no_case("SET"),
                    multispace1,
                    tag_no_case("REQUIRED_NODES"),
                    multispace0,
                    char('='),
                    multispace0,
                ),
                quoted_string_list,
            ),
            |list| {
                NodeTypeAlteration::SetRequiredNodes(
                    list.into_iter().map(|s| s.to_string()).collect(),
                )
            },
        ),
        // ADD MIXIN '...'
        map(
            preceded(
                (
                    tag_no_case("ADD"),
                    multispace1,
                    tag_no_case("MIXIN"),
                    multispace1,
                ),
                quoted_string,
            ),
            |s| NodeTypeAlteration::AddMixin(s.to_string()),
        ),
        // DROP MIXIN '...'
        map(
            preceded(
                (
                    tag_no_case("DROP"),
                    multispace1,
                    tag_no_case("MIXIN"),
                    multispace1,
                ),
                quoted_string,
            ),
            |s| NodeTypeAlteration::DropMixin(s.to_string()),
        ),
        // SET VERSIONABLE = true/false
        map(
            preceded(
                (
                    tag_no_case("SET"),
                    multispace1,
                    tag_no_case("VERSIONABLE"),
                    multispace0,
                    char('='),
                    multispace0,
                ),
                boolean_literal,
            ),
            NodeTypeAlteration::SetVersionable,
        ),
        // SET PUBLISHABLE = true/false
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
            NodeTypeAlteration::SetPublishable,
        ),
        // SET AUDITABLE = true/false
        map(
            preceded(
                (
                    tag_no_case("SET"),
                    multispace1,
                    tag_no_case("AUDITABLE"),
                    multispace0,
                    char('='),
                    multispace0,
                ),
                boolean_literal,
            ),
            NodeTypeAlteration::SetAuditable,
        ),
        // SET INDEXABLE = true/false
        map(
            preceded(
                (
                    tag_no_case("SET"),
                    multispace1,
                    tag_no_case("INDEXABLE"),
                    multispace0,
                    char('='),
                    multispace0,
                ),
                boolean_literal,
            ),
            NodeTypeAlteration::SetIndexable,
        ),
        // SET STRICT = true/false
        map(
            preceded(
                (
                    tag_no_case("SET"),
                    multispace1,
                    tag_no_case("STRICT"),
                    multispace0,
                    char('='),
                    multispace0,
                ),
                boolean_literal,
            ),
            NodeTypeAlteration::SetStrict,
        ),
    ))
    .parse(input)
}

// =============================================================================
// DROP NODETYPE
// =============================================================================

/// Parse DROP NODETYPE statement
pub(crate) fn drop_nodetype(input: &str) -> IResult<&str, DropNodeType> {
    let (input, _) = (
        tag_no_case("DROP"),
        multispace1,
        tag_no_case("NODETYPE"),
        multispace1,
    )
        .parse(input)?;

    let (input, name) = quoted_string(input)?;
    let (input, _) = multispace0.parse(input)?;

    let (input, cascade) = opt(tag_no_case("CASCADE")).parse(input)?;

    Ok((
        input,
        DropNodeType {
            name: name.to_string(),
            cascade: cascade.is_some(),
        },
    ))
}
