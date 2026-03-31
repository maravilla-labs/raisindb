//\! MERGE BRANCH statement parser
//\!
//\! Handles parsing of MERGE BRANCH statements including optional merge strategy,
//\! commit message, and conflict resolution clauses.

use nom::{
    branch::alt,
    bytes::complete::tag_no_case,
    character::complete::{char, multispace0, multispace1},
    combinator::{opt, value},
    multi::separated_list0,
    sequence::preceded,
    IResult, Parser,
};

use super::super::branch::{MergeBranch, MergeStrategy, SqlConflictResolution, SqlResolutionType};
use super::helpers::{branch_name, quoted_string};

/// Parse MERGE BRANCH statement
pub(crate) fn merge_branch(input: &str) -> IResult<&str, MergeBranch> {
    let (input, _) = tag_no_case("MERGE").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("BRANCH").parse(input)?;
    let (input, _) = multispace1.parse(input)?;

    // Parse source branch
    let (input, source) = branch_name(input)?;
    let (input, _) = multispace1.parse(input)?;

    // Parse INTO keyword
    let (input, _) = tag_no_case("INTO").parse(input)?;
    let (input, _) = multispace1.parse(input)?;

    // Parse target branch
    let (input, target) = branch_name(input)?;

    // Parse optional USING strategy
    let (input, strategy) = opt(preceded(
        (multispace1, tag_no_case("USING"), multispace1),
        merge_strategy,
    ))
    .parse(input)?;

    // Parse optional MESSAGE
    let (input, message) = opt(preceded(
        (multispace1, tag_no_case("MESSAGE"), multispace1),
        quoted_string,
    ))
    .parse(input)?;

    // Parse optional RESOLVE CONFLICTS clause
    let (input, resolutions) = opt(preceded(
        (
            multispace1,
            tag_no_case("RESOLVE"),
            multispace1,
            tag_no_case("CONFLICTS"),
            multispace0,
        ),
        parse_conflict_resolutions,
    ))
    .parse(input)?;

    Ok((
        input,
        MergeBranch {
            source_branch: source,
            target_branch: target,
            strategy,
            message: message.map(|s| s.to_string()),
            resolutions: resolutions.unwrap_or_default(),
        },
    ))
}

/// Parse a list of conflict resolutions: ( (res1), (res2), ... )
fn parse_conflict_resolutions(input: &str) -> IResult<&str, Vec<SqlConflictResolution>> {
    let (input, _) = char('(').parse(input)?;
    let (input, _) = multispace0.parse(input)?;

    let (input, resolutions) = separated_list0(
        (multispace0, char(','), multispace0),
        parse_single_resolution,
    )
    .parse(input)?;

    let (input, _) = multispace0.parse(input)?;
    let (input, _) = char(')').parse(input)?;

    Ok((input, resolutions))
}

/// Parse a single conflict resolution: (node_id, [locale,] RESOLUTION_TYPE)
fn parse_single_resolution(input: &str) -> IResult<&str, SqlConflictResolution> {
    let (input, _) = char('(').parse(input)?;
    let (input, _) = multispace0.parse(input)?;

    // Parse node_id (required)
    let (input, node_id) = quoted_string(input)?;
    let (input, _) = multispace0.parse(input)?;
    let (input, _) = char(',').parse(input)?;
    let (input, _) = multispace0.parse(input)?;

    // Try to parse optional locale (second quoted string before resolution type)
    // Look ahead to check if we have locale or resolution type
    let (input, locale_and_resolution) =
        alt((parse_locale_and_resolution, parse_resolution_only)).parse(input)?;

    let (locale, resolution) = locale_and_resolution;

    let (input, _) = multispace0.parse(input)?;
    let (input, _) = char(')').parse(input)?;

    Ok((
        input,
        SqlConflictResolution {
            node_id: node_id.to_string(),
            translation_locale: locale,
            resolution,
        },
    ))
}

/// Parse locale followed by resolution: 'locale', RESOLUTION_TYPE
fn parse_locale_and_resolution(input: &str) -> IResult<&str, (Option<String>, SqlResolutionType)> {
    let (input, locale) = quoted_string(input)?;
    let (input, _) = multispace0.parse(input)?;
    let (input, _) = char(',').parse(input)?;
    let (input, _) = multispace0.parse(input)?;
    let (input, resolution) = parse_resolution_type(input)?;
    Ok((input, (Some(locale.to_string()), resolution)))
}

/// Parse resolution only (no locale)
fn parse_resolution_only(input: &str) -> IResult<&str, (Option<String>, SqlResolutionType)> {
    let (input, resolution) = parse_resolution_type(input)?;
    Ok((input, (None, resolution)))
}

/// Parse resolution type: KEEP_OURS | KEEP_THEIRS | DELETE | USE_VALUE 'json'
fn parse_resolution_type(input: &str) -> IResult<&str, SqlResolutionType> {
    alt((
        value(SqlResolutionType::KeepOurs, tag_no_case("KEEP_OURS")),
        value(SqlResolutionType::KeepTheirs, tag_no_case("KEEP_THEIRS")),
        value(SqlResolutionType::Delete, tag_no_case("DELETE")),
        parse_use_value,
    ))
    .parse(input)
}

/// Parse USE_VALUE 'json' resolution
fn parse_use_value(input: &str) -> IResult<&str, SqlResolutionType> {
    let (input, _) = tag_no_case("USE_VALUE").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, json_str) = quoted_string(input)?;
    let val = serde_json::from_str(json_str).unwrap_or(serde_json::Value::Null);
    Ok((input, SqlResolutionType::UseValue(val)))
}

/// Parse merge strategy: FAST_FORWARD or THREE_WAY
fn merge_strategy(input: &str) -> IResult<&str, MergeStrategy> {
    alt((
        value(MergeStrategy::FastForward, tag_no_case("FAST_FORWARD")),
        value(MergeStrategy::ThreeWay, tag_no_case("THREE_WAY")),
    ))
    .parse(input)
}
