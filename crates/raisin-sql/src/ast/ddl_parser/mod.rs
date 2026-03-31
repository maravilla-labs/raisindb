//! DDL Parser for NodeTypes, Archetypes, and ElementTypes
//!
//! Parses DDL statements for schema management using nom combinators.
//!
//! # Supported Statements
//!
//! - CREATE NODETYPE / ALTER NODETYPE / DROP NODETYPE
//! - CREATE ARCHETYPE / ALTER ARCHETYPE / DROP ARCHETYPE
//! - CREATE ELEMENTTYPE / ALTER ELEMENTTYPE / DROP ELEMENTTYPE

mod archetype;
mod compound_index;
mod elementtype;
mod mixin;
mod nodetype;
mod primitives;
mod property;

#[cfg(test)]
mod tests;

use nom::{branch::alt, combinator::map, IResult, Parser};

use super::ddl::DdlStatement;
use archetype::{alter_archetype, create_archetype, drop_archetype};
use elementtype::{alter_elementtype, create_elementtype, drop_elementtype};
use mixin::{alter_mixin, create_mixin, drop_mixin};
use nodetype::{alter_nodetype, create_nodetype, drop_nodetype};

/// Error type for DDL parsing
#[derive(Debug, Clone, PartialEq)]
pub struct DdlParseError {
    pub message: String,
    pub position: Option<usize>,
}

impl std::fmt::Display for DdlParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(pos) = self.position {
            write!(f, "DDL parse error at position {}: {}", pos, self.message)
        } else {
            write!(f, "DDL parse error: {}", self.message)
        }
    }
}

impl std::error::Error for DdlParseError {}

// =============================================================================
// Main Entry Point
// =============================================================================

/// Try to parse a DDL statement from SQL string
///
/// Returns `Some(DdlStatement)` if the input is a valid DDL statement,
/// `None` if it's not a DDL statement (should be handled by sqlparser).
pub fn parse_ddl(sql: &str) -> Result<Option<DdlStatement>, DdlParseError> {
    let trimmed = sql.trim();

    // Strip leading SQL comments to find the actual statement
    let statement_start = strip_leading_comments(trimmed);
    let upper = statement_start.to_uppercase();

    // Check if this looks like a DDL statement
    if !is_ddl_statement(&upper) {
        return Ok(None);
    }

    // Calculate offset from original SQL to statement_start for position mapping
    let offset_to_statement_start = statement_start.as_ptr() as usize - sql.as_ptr() as usize;

    // Parse the DDL statement (starting from after comments)
    match ddl_statement(statement_start) {
        Ok((remaining, stmt)) => {
            // Verify we consumed all input (except whitespace and semicolon)
            let remaining_trimmed = remaining.trim().trim_end_matches(';').trim();
            if remaining_trimmed.is_empty() {
                Ok(Some(stmt))
            } else {
                // Calculate position of unexpected trailing content
                let position_in_statement = statement_start.len() - remaining.len();
                let position = offset_to_statement_start + position_in_statement;
                Err(DdlParseError {
                    message: format!("Unexpected trailing content: '{}'", remaining_trimmed),
                    position: Some(position),
                })
            }
        }
        Err(e) => {
            // Extract position and generate helpful error message
            let (position, remaining_input) = match &e {
                // Failure errors (from cut()) have reliable position
                nom::Err::Failure(err) => {
                    let pos_in_statement = statement_start.len() - err.input.len();
                    (
                        Some(offset_to_statement_start + pos_in_statement),
                        Some(err.input),
                    )
                }
                // Regular errors may have unreliable position due to backtracking
                nom::Err::Error(err) => {
                    let pos_in_statement = statement_start.len() - err.input.len();
                    (
                        Some(offset_to_statement_start + pos_in_statement),
                        Some(err.input),
                    )
                }
                nom::Err::Incomplete(_) => (None, None),
            };

            // Generate a helpful error message
            let message = if let Some(remaining) = remaining_input {
                let remaining_trimmed = remaining.trim();
                // Extract the problematic token/area
                let problematic: String = remaining_trimmed
                    .chars()
                    .take(40)
                    .take_while(|c| *c != '\n' && *c != ')')
                    .collect();
                let problematic = problematic.trim();

                // Check if this looks like an invalid property type
                if !problematic.is_empty() {
                    // Check for common property type typos
                    let first_word: String = problematic
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();

                    if !first_word.is_empty()
                        && !matches!(
                            first_word.to_uppercase().as_str(),
                            "STRING"
                                | "NUMBER"
                                | "BOOLEAN"
                                | "DATE"
                                | "URL"
                                | "REFERENCE"
                                | "RESOURCE"
                                | "OBJECT"
                                | "ARRAY"
                                | "DEFAULT"
                                | "REQUIRED"
                                | "LABEL"
                                | "DESCRIPTION"
                                | "ORDER"
                                | ","
                        )
                    {
                        format!(
                            "Invalid property type '{}'. Expected: String, Number, Boolean, Date, URL, Reference, Resource, Object, or Array",
                            first_word
                        )
                    } else {
                        format!("Parse error near: '{}'", problematic)
                    }
                } else {
                    "Parse error: unexpected end of input".to_string()
                }
            } else {
                "Incomplete DDL statement".to_string()
            };

            Err(DdlParseError { message, position })
        }
    }
}

/// Re-export strip_leading_comments for use by sibling parser modules
pub(crate) use primitives::strip_leading_comments;

/// Check if the SQL looks like a DDL statement we should parse
fn is_ddl_statement(upper: &str) -> bool {
    upper.starts_with("CREATE NODETYPE")
        || upper.starts_with("ALTER NODETYPE")
        || upper.starts_with("DROP NODETYPE")
        || upper.starts_with("CREATE MIXIN")
        || upper.starts_with("ALTER MIXIN")
        || upper.starts_with("DROP MIXIN")
        || upper.starts_with("CREATE ARCHETYPE")
        || upper.starts_with("ALTER ARCHETYPE")
        || upper.starts_with("DROP ARCHETYPE")
        || upper.starts_with("CREATE ELEMENTTYPE")
        || upper.starts_with("ALTER ELEMENTTYPE")
        || upper.starts_with("DROP ELEMENTTYPE")
}

/// Parse any DDL statement
fn ddl_statement(input: &str) -> IResult<&str, DdlStatement> {
    alt((
        // NodeType
        map(create_nodetype, DdlStatement::CreateNodeType),
        map(alter_nodetype, DdlStatement::AlterNodeType),
        map(drop_nodetype, DdlStatement::DropNodeType),
        // Mixin
        map(create_mixin, DdlStatement::CreateMixin),
        map(alter_mixin, DdlStatement::AlterMixin),
        map(drop_mixin, DdlStatement::DropMixin),
        // Archetype
        map(create_archetype, DdlStatement::CreateArchetype),
        map(alter_archetype, DdlStatement::AlterArchetype),
        map(drop_archetype, DdlStatement::DropArchetype),
        // ElementType
        map(create_elementtype, DdlStatement::CreateElementType),
        map(alter_elementtype, DdlStatement::AlterElementType),
        map(drop_elementtype, DdlStatement::DropElementType),
    ))
    .parse(input)
}
