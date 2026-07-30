//! Keyword matching helpers for the PGQ parser.
//!
//! `tag_no_case` alone is not enough for selector and restrictor keywords: it
//! would match the `ANY` prefix of a variable called `anyone`. These helpers
//! additionally require that the match is not followed by an identifier
//! character.

use nom::{bytes::complete::tag_no_case, character::complete::multispace1, IResult, Parser};

/// Match `word` case-insensitively, but only as a whole token.
pub fn keyword<'a>(input: &'a str, word: &str) -> IResult<&'a str, &'a str> {
    let (rest, matched) = tag_no_case(word).parse(input)?;
    match rest.chars().next() {
        Some(c) if c.is_alphanumeric() || c == '_' => Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Tag,
        ))),
        _ => Ok((rest, matched)),
    }
}

/// Match two whitespace-separated keywords as one token pair.
pub fn two_words<'a>(input: &'a str, first: &str, second: &str) -> IResult<&'a str, ()> {
    let (rest, _) = keyword(input, first)?;
    let (rest, _) = multispace1.parse(rest)?;
    let (rest, _) = keyword(rest, second)?;
    Ok((rest, ()))
}
