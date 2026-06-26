//! Helper parsers for TRANSLATE statements
//!
//! Shared low-level parsers for identifiers and quoted strings.

use nom::{
    branch::alt,
    bytes::complete::{take_until, take_while1},
    character::complete::char,
    error::{Error, ErrorKind},
    sequence::delimited,
    Err, IResult, Parser,
};

/// Parse an identifier (table name, field name): alphanumeric + underscore
pub(crate) fn identifier(input: &str) -> IResult<&str, &str> {
    take_while1(|c: char| c.is_alphanumeric() || c == '_').parse(input)
}

/// Parse a quoted string: 'content' or "content".
///
/// Zero-copy, escape-UNAWARE — for structural tokens that never contain the
/// delimiter (paths, ids, locale codes, uuids). For string VALUES (which may
/// hold an apostrophe, e.g. a French translation) use [`quoted_string_value`].
pub(crate) fn quoted_string(input: &str) -> IResult<&str, &str> {
    alt((
        delimited(char('\''), take_until("'"), char('\'')),
        delimited(char('"'), take_until("\""), char('"')),
    ))
    .parse(input)
}

/// Parse a quoted string VALUE honoring SQL-standard quote-doubling: an embedded
/// delimiter is written twice (`''` → `'`, `""` → `"`). Returns an OWNED string
/// with the doubled delimiters collapsed. This is what lets a translation value
/// contain an apostrophe — `'c''est'` parses to `c'est` — which the zero-copy
/// `take_until` parser cannot do (it stops at the first quote, leaving
/// "Unexpected trailing content"). The HTTP layer escapes bound params to this
/// same `''` form (see params.rs), so both inline literals and `$N` params work.
pub(crate) fn quoted_string_value(input: &str) -> IResult<&str, String> {
    let mut chars = input.char_indices();
    let quote = match chars.next() {
        Some((_, c)) if c == '\'' || c == '"' => c,
        _ => return Err(Err::Error(Error::new(input, ErrorKind::Char))),
    };
    let mut out = String::new();
    while let Some((i, c)) = chars.next() {
        if c == quote {
            // A doubled delimiter is a literal one; anything else closes the string.
            if matches!(chars.clone().next(), Some((_, n)) if n == quote) {
                out.push(quote);
                chars.next(); // consume the second delimiter
            } else {
                return Ok((&input[i + quote.len_utf8()..], out));
            }
        } else {
            out.push(c);
        }
    }
    // Ran off the end without a closing delimiter.
    Err(Err::Error(Error::new(input, ErrorKind::Char)))
}
