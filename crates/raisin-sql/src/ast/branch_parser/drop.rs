//\! DROP BRANCH statement parser

use nom::{
    bytes::complete::tag_no_case, character::complete::multispace1, combinator::opt,
    sequence::tuple, IResult, Parser,
};

use super::super::branch::DropBranch;
use super::helpers::branch_name;

/// Parse DROP BRANCH statement
pub(crate) fn drop_branch(input: &str) -> IResult<&str, DropBranch> {
    let (input, _) = tag_no_case("DROP").parse(input)?;
    let (input, _) = multispace1.parse(input)?;
    let (input, _) = tag_no_case("BRANCH").parse(input)?;
    let (input, _) = multispace1.parse(input)?;

    // Check for optional IF EXISTS
    let (input, if_exists) = opt(tuple((
        tag_no_case("IF"),
        multispace1,
        tag_no_case("EXISTS"),
        multispace1,
    )))
    .parse(input)?;

    // Parse branch name
    let (input, name) = branch_name(input)?;

    Ok((
        input,
        DropBranch {
            name,
            if_exists: if_exists.is_some(),
        },
    ))
}
