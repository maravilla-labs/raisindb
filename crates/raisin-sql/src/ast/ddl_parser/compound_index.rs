//! Compound index parsing
//!
//! Parsers for compound index definitions in CREATE NODETYPE statements.

use nom::{
    branch::alt,
    bytes::complete::{tag, tag_no_case},
    character::complete::{char, multispace0, multispace1},
    combinator::{opt, recognize, value},
    multi::separated_list0,
    sequence::{delimited, pair},
    IResult, Parser,
};

use super::super::ddl::{CompoundIndexColumnDef, CompoundIndexDef};
use super::primitives::{identifier, quoted_string};

/// Parse a compound index definition:
/// ```sql
/// COMPOUND_INDEX 'idx_name' ON (column1, column2, column3 DESC)
/// ```
pub(crate) fn compound_index(input: &str) -> IResult<&str, CompoundIndexDef> {
    let (input, _) = tag_no_case("COMPOUND_INDEX").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, name) = quoted_string(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("ON").parse(input)?;
    let (input, _) = multispace0.parse(input)?;
    let (input, columns) = compound_index_columns(input)?;

    // Determine if last column has explicit ordering (ASC/DESC)
    // If the last column has explicit ordering, it's used for ORDER BY
    let has_order_column = columns
        .last()
        .map(|c| {
            !c.ascending || {
                // Check if this was explicitly set (we need to track this)
                // For simplicity, assume any column with DESC is an order column
                // or if it's the last column with system time fields
                c.property.starts_with("__created_at")
                    || c.property.starts_with("__updated_at")
                    || !c.ascending
            }
        })
        .unwrap_or(false);

    Ok((
        input,
        CompoundIndexDef {
            name: name.to_string(),
            columns,
            has_order_column,
        },
    ))
}

/// Parse compound index column list: (col1, col2, col3 DESC)
pub(crate) fn compound_index_columns(input: &str) -> IResult<&str, Vec<CompoundIndexColumnDef>> {
    delimited(
        (char('('), multispace0),
        separated_list0((multispace0, char(','), multispace0), compound_index_column),
        (multispace0, opt(char(',')), multispace0, char(')')),
    )
    .parse(input)
}

/// Parse a single compound index column: column_name [ASC|DESC]
pub(crate) fn compound_index_column(input: &str) -> IResult<&str, CompoundIndexColumnDef> {
    let (input, property) = alt((
        // System fields with __ prefix
        recognize(pair(tag("__"), identifier)),
        // Regular property name
        identifier,
    ))
    .parse(input)?;
    let (input, _) = multispace0.parse(input)?;

    // Parse optional ASC/DESC
    let (input, direction) = opt(alt((
        value(true, tag_no_case("ASC")),
        value(false, tag_no_case("DESC")),
    )))
    .parse(input)?;

    Ok((
        input,
        CompoundIndexColumnDef {
            property: property.to_string(),
            ascending: direction.unwrap_or(true), // Default to ASC
        },
    ))
}
