// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! nom parser for the `SPATIAL INDEX` admin statements.
//!
//! Structure mirrors [`crate::ast::ai_config_parser`]: a cheap `is_*_statement`
//! guard so the analyzer can skip the parser entirely, then a `parse_*` entry point
//! that insists on consuming the whole statement — trailing junk is an error rather
//! than being silently ignored, because a half-understood admin command is worse
//! than a rejected one.

use nom::{
    branch::alt,
    bytes::complete::{tag_no_case, take_until, take_while1},
    character::complete::{char, digit1, multispace0, multispace1},
    combinator::{map, opt},
    IResult, Parser,
};

use super::spatial_admin::{CoverModeSpec, SpatialAdminStatement, SpatialIndexSettings};

/// Valid geohash precision range.
///
/// Duplicated from `raisin_models::nodes::properties::PRECISION_RANGE` rather than
/// imported: `raisin-sql` deliberately does not depend on `raisin-models` (it is
/// WASM-compatible and parser-only). The model layer re-validates on the way to
/// disk, so a drift here can only make the parser stricter, never let a bad value
/// through.
const PRECISION_RANGE: std::ops::RangeInclusive<usize> = 1..=12;

/// Parse failure, carrying a byte offset into the original SQL when known.
#[derive(Debug, Clone, PartialEq)]
pub struct SpatialAdminParseError {
    pub message: String,
    pub position: Option<usize>,
}

impl std::fmt::Display for SpatialAdminParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.position {
            Some(pos) => write!(
                f,
                "spatial index admin parse error at position {}: {}",
                pos, self.message
            ),
            None => write!(f, "spatial index admin parse error: {}", self.message),
        }
    }
}

impl std::error::Error for SpatialAdminParseError {}

/// Cheap prefix test, so the analyzer only pays for the parser on a real match.
pub fn is_spatial_admin_statement(sql: &str) -> bool {
    // Collapse whitespace so `SHOW   SPATIAL\n INDEX` is recognised.
    let upper: String = sql.trim().to_uppercase();
    let normalized: String = upper.split_whitespace().collect::<Vec<_>>().join(" ");
    normalized.starts_with("SHOW SPATIAL INDEX")
        || normalized.starts_with("ALTER SPATIAL INDEX")
        || normalized.starts_with("REBUILD SPATIAL INDEX")
        || normalized.starts_with("VERIFY SPATIAL INDEX")
}

/// Parse a spatial-index admin statement.
///
/// `Ok(None)` means "not one of these", so the analyzer can fall through to the
/// next statement family.
pub fn parse_spatial_admin(
    sql: &str,
) -> Result<Option<SpatialAdminStatement>, SpatialAdminParseError> {
    let trimmed = sql.trim();
    let statement_start = super::ddl_parser::strip_leading_comments(trimmed);

    if !is_spatial_admin_statement(statement_start) {
        return Ok(None);
    }

    let offset = statement_start.as_ptr() as usize - sql.as_ptr() as usize;

    match spatial_admin_statement(statement_start) {
        Ok((remaining, stmt)) => {
            let rest = remaining.trim().trim_end_matches(';').trim();
            if rest.is_empty() {
                validate(&stmt)?;
                Ok(Some(stmt))
            } else {
                Err(SpatialAdminParseError {
                    message: format!("Unexpected trailing content: '{}'", rest),
                    position: Some(offset + (statement_start.len() - remaining.len())),
                })
            }
        }
        Err(e) => {
            let (position, message) = match &e {
                nom::Err::Failure(err) | nom::Err::Error(err) => {
                    let near: String = err.input.trim().chars().take(40).collect();
                    (
                        Some(offset + (statement_start.len() - err.input.len())),
                        format!("Parse error near: '{}'", near.trim()),
                    )
                }
                nom::Err::Incomplete(_) => (None, "Incomplete statement".to_string()),
            };
            Err(SpatialAdminParseError { message, position })
        }
    }
}

/// Semantic validation the grammar cannot express.
///
/// Done here rather than at execution so a bad precision set is rejected before it
/// can be written to a replicated record — a workspace record carrying precision 40
/// would be replicated to every peer before anyone noticed.
fn validate(stmt: &SpatialAdminStatement) -> Result<(), SpatialAdminParseError> {
    let SpatialAdminStatement::Alter { settings, .. } = stmt else {
        return Ok(());
    };

    if settings.is_empty() {
        return Err(SpatialAdminParseError {
            message: "ALTER SPATIAL INDEX requires at least one SET clause, or RESET".to_string(),
            position: None,
        });
    }

    if let Some(precisions) = &settings.precisions {
        if precisions.is_empty() {
            return Err(SpatialAdminParseError {
                message: "SET PRECISIONS requires at least one precision".to_string(),
                position: None,
            });
        }
        for p in precisions {
            if !PRECISION_RANGE.contains(p) {
                return Err(SpatialAdminParseError {
                    message: format!(
                        "precision {} is out of range: geohash precisions run {}..={}",
                        p,
                        PRECISION_RANGE.start(),
                        PRECISION_RANGE.end()
                    ),
                    position: None,
                });
            }
        }
        let mut sorted = precisions.clone();
        sorted.sort_unstable();
        let before = sorted.len();
        sorted.dedup();
        if sorted.len() != before {
            return Err(SpatialAdminParseError {
                message: "SET PRECISIONS contains duplicates".to_string(),
                position: None,
            });
        }
        // The coarsest cell must be able to answer a city-scale query inside the
        // cell budget. Precision 6 cells are ~1.2 km; anything finer than that as
        // the *coarsest* entry cannot cover a 50 km radius and would silently push
        // every wide query onto a full scan.
        let coarsest = sorted.first().copied().unwrap_or(0);
        if coarsest > 6 {
            return Err(SpatialAdminParseError {
                message: format!(
                    "coarsest precision {} is too fine: include a precision of 6 or less so \
                     city-scale queries can use the index (precision 6 cells are ~1.2 km)",
                    coarsest
                ),
                position: None,
            });
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Statement dispatch
// ---------------------------------------------------------------------------

fn spatial_admin_statement(input: &str) -> IResult<&str, SpatialAdminStatement> {
    alt((show_config, show_health, alter_or_reset, rebuild, verify)).parse(input)
}

/// `SPATIAL INDEX`, the shared middle of every statement in this family.
fn spatial_index_kw(input: &str) -> IResult<&str, ()> {
    let (input, _) = tag_no_case("SPATIAL").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("INDEX").parse(input)?;
    Ok((input, ()))
}

fn quoted_or_bare(input: &str) -> IResult<&str, String> {
    alt((
        map(
            nom::sequence::delimited(char('\''), take_until("'"), char('\'')),
            |s: &str| s.to_string(),
        ),
        map(
            nom::sequence::delimited(char('"'), take_until("\""), char('"')),
            |s: &str| s.to_string(),
        ),
        map(
            take_while1(|c: char| c.is_alphanumeric() || c == '_' || c == ':' || c == '-'),
            |s: &str| s.to_string(),
        ),
    ))
    .parse(input)
}

/// `FOR 'workspace' [ PROPERTY 'prop' ]`
fn target(input: &str) -> IResult<&str, (String, Option<String>)> {
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("FOR").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, workspace) = quoted_or_bare(input)?;
    let (input, property) = opt(property_clause).parse(input)?;
    Ok((input, (workspace, property)))
}

fn property_clause(input: &str) -> IResult<&str, String> {
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("PROPERTY").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    quoted_or_bare(input)
}

// ---------------------------------------------------------------------------
// SHOW
// ---------------------------------------------------------------------------

fn show_config(input: &str) -> IResult<&str, SpatialAdminStatement> {
    let (input, _) = tag_no_case("SHOW").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, ()) = spatial_index_kw(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("CONFIG").parse(input)?;
    let (input, t) = opt(target).parse(input)?;
    let (workspace, property) = split_target(t);
    Ok((
        input,
        SpatialAdminStatement::ShowConfig {
            workspace,
            property,
        },
    ))
}

fn show_health(input: &str) -> IResult<&str, SpatialAdminStatement> {
    let (input, _) = tag_no_case("SHOW").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, ()) = spatial_index_kw(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("HEALTH").parse(input)?;
    let (input, t) = opt(target).parse(input)?;
    let (workspace, property) = split_target(t);
    Ok((
        input,
        SpatialAdminStatement::ShowHealth {
            workspace,
            property,
        },
    ))
}

fn split_target(t: Option<(String, Option<String>)>) -> (Option<String>, Option<String>) {
    match t {
        Some((ws, prop)) => (Some(ws), prop),
        None => (None, None),
    }
}

// ---------------------------------------------------------------------------
// ALTER / RESET
// ---------------------------------------------------------------------------

fn alter_or_reset(input: &str) -> IResult<&str, SpatialAdminStatement> {
    let (input, _) = tag_no_case("ALTER").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, ()) = spatial_index_kw(input)?;
    let (input, (workspace, property)) = target(input)?;

    // RESET first: `RESET` and `SET …` are mutually exclusive tails.
    let after_reset = {
        let trimmed = input.trim_start();
        match tag_no_case::<_, _, nom::error::Error<&str>>("RESET").parse(trimmed) {
            Ok((rest, _)) => Some(rest),
            Err(_) => None,
        }
    };
    if let Some(rest) = after_reset {
        return Ok((
            rest,
            SpatialAdminStatement::Reset {
                workspace,
                property,
            },
        ));
    }

    let (input, settings) = set_clauses(input)?;
    Ok((
        input,
        SpatialAdminStatement::Alter {
            workspace,
            property,
            settings,
        },
    ))
}

fn set_clauses(input: &str) -> IResult<&str, SpatialIndexSettings> {
    let mut settings = SpatialIndexSettings::default();
    let mut remaining = input;
    loop {
        let trimmed = remaining.trim_start();
        match set_clause(trimmed, &mut settings) {
            Ok(rest) => remaining = rest,
            Err(_) => break,
        }
    }
    if settings.is_empty() {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Tag,
        )));
    }
    Ok((remaining, settings))
}

/// One `SET <field> = <value>` clause, folded into `settings`.
fn set_clause<'a>(
    input: &'a str,
    settings: &mut SpatialIndexSettings,
) -> Result<&'a str, nom::Err<nom::error::Error<&'a str>>> {
    let (input, _) = tag_no_case("SET").parse(input)?;
    let (input, _) = multispace1.parse(input)?;

    if let Ok((rest, _)) = tag_no_case::<_, _, nom::error::Error<&str>>("PRECISIONS").parse(input) {
        let (rest, list) = precision_list(rest)?;
        settings.precisions = Some(list);
        return Ok(rest);
    }
    if let Ok((rest, _)) = tag_no_case::<_, _, nom::error::Error<&str>>("SRID").parse(input) {
        let (rest, _) = eq(rest)?;
        let (rest, digits) = digit1.parse(rest)?;
        settings.srid = digits.parse::<u32>().ok();
        return Ok(rest);
    }
    if let Ok((rest, _)) = tag_no_case::<_, _, nom::error::Error<&str>>("BUCKET").parse(input) {
        let (rest, _) = multispace1.parse(rest)?;
        let (rest, _) = tag_no_case("PROPERTY").parse(rest)?;
        let (rest, _) = eq(rest)?;
        let (rest, name) = quoted_or_bare(rest)?;
        settings.bucket_property = Some(name);
        return Ok(rest);
    }
    if let Ok((rest, _)) = tag_no_case::<_, _, nom::error::Error<&str>>("COVER").parse(input) {
        let (rest, _) = eq(rest)?;
        let (rest, mode) = alt((
            map(tag_no_case("CENTROID"), |_| CoverModeSpec::Centroid),
            map(tag_no_case("EXTENT"), |_| CoverModeSpec::Extent),
        ))
        .parse(rest)?;
        settings.cover = Some(mode);
        return Ok(rest);
    }

    Err(nom::Err::Error(nom::error::Error::new(
        input,
        nom::error::ErrorKind::Tag,
    )))
}

fn eq(input: &str) -> IResult<&str, ()> {
    let (input, _) = multispace0.parse(input)?;
    let (input, _) = char('=').parse(input)?;
    let (input, _) = multispace0.parse(input)?;
    Ok((input, ()))
}

/// `= ( 11, 10, 9 )`, also accepting a bare list without parentheses.
fn precision_list(input: &str) -> IResult<&str, Vec<usize>> {
    let (input, _) = eq(input)?;
    let (input, open) = opt(char('(')).parse(input)?;

    let mut values = Vec::new();
    let mut remaining = input;
    loop {
        let (rest, _) = multispace0.parse(remaining)?;
        let Ok((rest, digits)) = digit1::<_, nom::error::Error<&str>>.parse(rest) else {
            break;
        };
        let Ok(value) = digits.parse::<usize>() else {
            break;
        };
        values.push(value);
        let (rest, _) = multispace0.parse(rest)?;
        match char::<_, nom::error::Error<&str>>(',').parse(rest) {
            Ok((after_comma, _)) => remaining = after_comma,
            Err(_) => {
                remaining = rest;
                break;
            }
        }
    }

    if values.is_empty() {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Digit,
        )));
    }

    let remaining = if open.is_some() {
        let (rest, _) = multispace0.parse(remaining)?;
        let (rest, _) = char(')').parse(rest)?;
        rest
    } else {
        remaining
    };

    Ok((remaining, values))
}

// ---------------------------------------------------------------------------
// REBUILD / VERIFY
// ---------------------------------------------------------------------------

fn rebuild(input: &str) -> IResult<&str, SpatialAdminStatement> {
    let (input, _) = tag_no_case("REBUILD").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, ()) = spatial_index_kw(input)?;
    let (input, (workspace, property)) = target(input)?;
    Ok((
        input,
        SpatialAdminStatement::Rebuild {
            workspace,
            property,
        },
    ))
}

fn verify(input: &str) -> IResult<&str, SpatialAdminStatement> {
    let (input, _) = tag_no_case("VERIFY").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, ()) = spatial_index_kw(input)?;
    let (input, (workspace, property)) = target(input)?;
    Ok((
        input,
        SpatialAdminStatement::Verify {
            workspace,
            property,
        },
    ))
}

#[cfg(test)]
mod tests;
