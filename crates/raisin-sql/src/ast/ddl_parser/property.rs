//! Property definition parsing
//!
//! Parsers for property definitions, types, and modifiers used in
//! CREATE/ALTER statements for NodeTypes, Archetypes, and ElementTypes.

use nom::{
    branch::alt,
    bytes::complete::tag_no_case,
    character::complete::{char, multispace0, multispace1},
    combinator::{cut, map, opt, value},
    multi::separated_list0,
    sequence::{delimited, preceded},
    IResult, Parser,
};

use super::super::ddl::{IndexTypeDef, PropertyDef, PropertyTypeDef};
use super::primitives::{default_value, identifier, integer_literal, quoted_string};

/// Parse a list of properties: (prop1 Type MOD, prop2 Type MOD, ...)
///
/// When the list stops short of its `)` the error is reported at the token that
/// actually broke rather than at the head of the list. `separated_list0`
/// backtracks the whole element on failure, so the naive `delimited(...)` form
/// blamed the property NAME for a bad TYPE - `PROPERTIES (body Strng)` came
/// back as "Invalid property type 'body'".
///
/// The error stays SOFT (`nom::Err::Error`, not `Failure`): the paren-wrapper
/// syntax and the implied-PROPERTIES shorthand both depend on this parser
/// backtracking cleanly. `preceded_property_list` is where the `cut` lives.
pub(crate) fn property_list(input: &str) -> IResult<&str, Vec<PropertyDef>> {
    let (rest, _) = (char('('), multispace0).parse(input)?;
    let (rest, props) =
        separated_list0((multispace0, char(','), multispace0), property_def).parse(rest)?;
    let (rest, _) = (multispace0, opt(char(',')), multispace0).parse(rest)?;

    match char::<&str, nom::error::Error<&str>>(')').parse(rest) {
        Ok((rest, _)) => Ok((rest, props)),
        Err(_) => Err(nom::Err::Error(nom::error::Error::new(
            blame_position(rest),
            nom::error::ErrorKind::Char,
        ))),
    }
}

/// Where to point when a property list stops short of its closing paren.
///
/// If what is left starts with a valid property NAME followed by whitespace,
/// the name was fine and the type is the problem, so blame what comes after it.
fn blame_position(rest: &str) -> &str {
    if let Ok((after_name, _)) = property_name_or_path(rest) {
        let after_space = after_name.trim_start();
        if after_space.len() < after_name.len() && !after_space.is_empty() {
            return after_space;
        }
    }
    rest
}

/// Parse a single property definition: name Type [MODIFIERS] [DEFAULT value]
/// Name can be an identifier or a quoted path (for ALTER nested properties)
pub(crate) fn property_def(input: &str) -> IResult<&str, PropertyDef> {
    let (input, name) = property_name_or_path(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, prop_type) = property_type(input)?;
    let (input, _) = multispace0.parse(input)?;

    let mut def = PropertyDef {
        name,
        property_type: prop_type,
        ..Default::default()
    };

    // Parse modifiers
    let (input, _) = parse_property_modifiers(input, &mut def)?;

    Ok((input, def))
}

/// Parse a property name: either a simple identifier or a quoted dotted path
/// - Simple: `title`
/// - Quoted path: `'specs.dimensions.width'`
pub(crate) fn property_name_or_path(input: &str) -> IResult<&str, String> {
    alt((
        // Quoted path (can contain dots): 'specs.dimensions.width'
        map(quoted_string, |s| s.to_string()),
        // Simple identifier: title
        map(identifier, |s| s.to_string()),
    ))
    .parse(input)
}

/// Parse property modifiers (REQUIRED, UNIQUE, FULLTEXT, etc.)
fn parse_property_modifiers<'a>(mut input: &'a str, def: &mut PropertyDef) -> IResult<&'a str, ()> {
    loop {
        let (remaining, _) = multispace0.parse(input)?;

        if let Ok((new_input, _)) =
            tag_no_case::<_, _, nom::error::Error<&str>>("REQUIRED").parse(remaining)
        {
            def.required = true;
            input = new_input;
            continue;
        }

        if let Ok((new_input, _)) =
            tag_no_case::<_, _, nom::error::Error<&str>>("UNIQUE").parse(remaining)
        {
            def.unique = true;
            input = new_input;
            continue;
        }

        if let Ok((new_input, _)) =
            tag_no_case::<_, _, nom::error::Error<&str>>("FULLTEXT").parse(remaining)
        {
            if !def.index.contains(&IndexTypeDef::Fulltext) {
                def.index.push(IndexTypeDef::Fulltext);
            }
            input = new_input;
            continue;
        }

        if let Ok((new_input, _)) =
            tag_no_case::<_, _, nom::error::Error<&str>>("VECTOR").parse(remaining)
        {
            if !def.index.contains(&IndexTypeDef::Vector) {
                def.index.push(IndexTypeDef::Vector);
            }
            input = new_input;
            continue;
        }

        if let Ok((new_input, _)) =
            tag_no_case::<_, _, nom::error::Error<&str>>("PROPERTY_INDEX").parse(remaining)
        {
            if !def.index.contains(&IndexTypeDef::Property) {
                def.index.push(IndexTypeDef::Property);
            }
            input = new_input;
            continue;
        }

        if let Ok((new_input, _)) =
            tag_no_case::<_, _, nom::error::Error<&str>>("TRANSLATABLE").parse(remaining)
        {
            def.translatable = true;
            input = new_input;
            continue;
        }

        // DEFAULT value
        if let Ok((new_input, default_val)) =
            preceded((tag_no_case("DEFAULT"), multispace1), default_value).parse(remaining)
        {
            def.default = Some(default_val);
            input = new_input;
            continue;
        }

        // LABEL 'Human Readable Label'
        if let Ok((new_input, label)) =
            preceded((tag_no_case("LABEL"), multispace1), quoted_string).parse(remaining)
        {
            def.label = Some(label.to_string());
            input = new_input;
            continue;
        }

        // DESCRIPTION 'Description text'
        if let Ok((new_input, desc)) =
            preceded((tag_no_case("DESCRIPTION"), multispace1), quoted_string).parse(remaining)
        {
            def.description = Some(desc.to_string());
            input = new_input;
            continue;
        }

        // ORDER 1 (display order hint)
        if let Ok((new_input, order)) =
            preceded((tag_no_case("ORDER"), multispace1), integer_literal).parse(remaining)
        {
            def.order = Some(order);
            input = new_input;
            continue;
        }

        // ALLOW_ADDITIONAL_PROPERTIES (for Object types)
        if let Ok((new_input, _)) =
            tag_no_case::<_, _, nom::error::Error<&str>>("ALLOW_ADDITIONAL_PROPERTIES")
                .parse(remaining)
        {
            def.allow_additional_properties = true;
            input = new_input;
            continue;
        }

        break;
    }

    Ok((input, ()))
}

/// Parse a property type
pub(crate) fn property_type(input: &str) -> IResult<&str, PropertyTypeDef> {
    alt((
        // Array OF Type must come before simple types
        array_type,
        // Object { ... }
        object_type,
        // Simple types
        value(PropertyTypeDef::String, tag_no_case("String")),
        value(PropertyTypeDef::Number, tag_no_case("Number")),
        value(PropertyTypeDef::Boolean, tag_no_case("Boolean")),
        value(PropertyTypeDef::Date, tag_no_case("Date")),
        value(PropertyTypeDef::URL, tag_no_case("URL")),
        value(PropertyTypeDef::Reference, tag_no_case("Reference")),
        value(PropertyTypeDef::Resource, tag_no_case("Resource")),
        value(PropertyTypeDef::Composite, tag_no_case("Composite")),
        value(PropertyTypeDef::Element, tag_no_case("Element")),
        value(PropertyTypeDef::NodeType, tag_no_case("NodeType")),
    ))
    .parse(input)
}

/// Parse Array OF Type
pub(crate) fn array_type(input: &str) -> IResult<&str, PropertyTypeDef> {
    let (input, _) = tag_no_case("Array").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("OF").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, item_type) = property_type(input)?;

    Ok((
        input,
        PropertyTypeDef::Array {
            items: Box::new(item_type),
        },
    ))
}

/// Parse Object { field Type, ... }
pub(crate) fn object_type(input: &str) -> IResult<&str, PropertyTypeDef> {
    let (input, _) = tag_no_case("Object").parse(input)?;
    let (input, _) = multispace0.parse(input)?;
    let (input, fields) = delimited(
        (char('{'), multispace0),
        separated_list0((multispace0, char(','), multispace0), property_def),
        (multispace0, opt(char(',')), multispace0, char('}')),
    )
    .parse(input)?;

    Ok((input, PropertyTypeDef::Object { fields }))
}

/// Parse a property list preceded by PROPERTIES keyword with cut for error propagation
pub(crate) fn preceded_property_list(input: &str) -> IResult<&str, Vec<PropertyDef>> {
    preceded((tag_no_case("PROPERTIES"), multispace0), cut(property_list)).parse(input)
}

/// The SQL-conformant shorthand: a bare `(col Type MODIFIERS, ...)` directly
/// after the object name, with the `PROPERTIES` / `FIELDS` keyword implied —
/// i.e. what `CREATE TABLE t (...)` looks like in every other dialect.
///
/// This is the SAME `property_list` the keyword forms use, so a modifier only
/// has to be taught to one parser. It requires at least one property, which is
/// what keeps it from colliding with the paren-WRAPPER form
/// (`CREATE NODETYPE 'n' (PROPERTIES (...) VERSIONABLE)`): a clause keyword is
/// never a `name Type` pair, so the wrapper always yields an empty list here
/// and falls through to the clause parser untouched.
pub(crate) fn implicit_property_list(input: &str) -> IResult<&str, Vec<PropertyDef>> {
    let (rest, props) = property_list(input)?;
    if props.is_empty() {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::SeparatedList,
        )));
    }
    Ok((rest, props))
}
