//! Literal value parsing

use super::common::{
    close_brace, close_bracket, colon, comma, identifier, kw_false, kw_null, kw_true, open_brace,
    open_bracket, ws, ws0, PResult, Span,
};
use crate::ast::Literal;
use nom::{
    branch::alt,
    character::complete::{char, digit1, one_of},
    combinator::{map, opt, recognize},
    multi::separated_list0,
    sequence::{pair, separated_pair},
    Parser,
};

/// Parse any literal value
pub fn literal(input: Span) -> PResult<Literal> {
    alt((
        null_literal,
        boolean_literal,
        number_literal,
        string_literal,
        array_literal,
        object_literal,
    ))
    .parse(input)
}

/// Parse null literal
pub fn null_literal(input: Span) -> PResult<Literal> {
    map(kw_null, |_| Literal::Null).parse(input)
}

/// Parse boolean literal (true or false)
pub fn boolean_literal(input: Span) -> PResult<Literal> {
    alt((
        map(kw_true, |_| Literal::Boolean(true)),
        map(kw_false, |_| Literal::Boolean(false)),
    ))
    .parse(input)
}

/// Parse number literal (integer or float)
pub fn number_literal(input: Span) -> PResult<Literal> {
    let (input, num_str) = recognize((
        opt(char('-')),
        digit1,
        opt(pair(char('.'), digit1)),
        opt((one_of("eE"), opt(one_of("+-")), digit1)),
    ))
    .parse(input)?;

    let num_str = *num_str.fragment();

    // Try to parse as integer first, then float
    if num_str.contains('.') || num_str.contains('e') || num_str.contains('E') {
        match num_str.parse::<f64>() {
            Ok(f) => Ok((input, Literal::Float(f))),
            Err(_) => Err(nom::Err::Error(nom::error::Error::new(
                input,
                nom::error::ErrorKind::Float,
            ))),
        }
    } else {
        match num_str.parse::<i64>() {
            Ok(i) => Ok((input, Literal::Integer(i))),
            Err(_) => Err(nom::Err::Error(nom::error::Error::new(
                input,
                nom::error::ErrorKind::Digit,
            ))),
        }
    }
}

/// Parse a string literal with single quotes
fn single_quoted_string(input: Span) -> PResult<String> {
    let (input, _) = char('\'')(input)?;
    let (input, content) = parse_string_content('\'')(input)?;
    let (input, _) = char('\'')(input)?;
    Ok((input, content))
}

/// Parse a string literal with double quotes
fn double_quoted_string(input: Span) -> PResult<String> {
    let (input, _) = char('"')(input)?;
    let (input, content) = parse_string_content('"')(input)?;
    let (input, _) = char('"')(input)?;
    Ok((input, content))
}

/// Parse string content handling escape sequences
fn parse_string_content(quote: char) -> impl FnMut(Span) -> PResult<String> {
    move |input: Span| {
        let mut result = String::new();
        let mut remaining = input;

        loop {
            // Check for end of string or escape
            if remaining.fragment().is_empty() {
                return Err(nom::Err::Error(nom::error::Error::new(
                    remaining,
                    nom::error::ErrorKind::Eof,
                )));
            }

            let first_char = remaining.fragment().chars().next().unwrap();

            if first_char == quote {
                // End of string
                return Ok((remaining, result));
            } else if first_char == '\\' {
                // Escape sequence
                let (new_remaining, _) = char('\\')(remaining)?;
                if new_remaining.fragment().is_empty() {
                    return Err(nom::Err::Error(nom::error::Error::new(
                        new_remaining,
                        nom::error::ErrorKind::Eof,
                    )));
                }
                let escape_char = new_remaining.fragment().chars().next().unwrap();
                let (new_remaining, _) = nom::character::complete::anychar::<
                    Span,
                    nom::error::Error<Span>,
                >(new_remaining)?;
                remaining = new_remaining;

                match escape_char {
                    'n' => result.push('\n'),
                    'r' => result.push('\r'),
                    't' => result.push('\t'),
                    '\\' => result.push('\\'),
                    '\'' => result.push('\''),
                    '"' => result.push('"'),
                    _ => {
                        // Unknown escape, keep as-is
                        result.push('\\');
                        result.push(escape_char);
                    }
                }
            } else {
                // Regular character
                let (new_remaining, c) =
                    nom::character::complete::anychar::<Span, nom::error::Error<Span>>(remaining)?;
                remaining = new_remaining;
                result.push(c);
            }
        }
    }
}

/// Parse a string literal (single or double quoted)
pub fn string_literal(input: Span) -> PResult<Literal> {
    map(
        alt((single_quoted_string, double_quoted_string)),
        Literal::String,
    )
    .parse(input)
}

/// Parse an array literal: [1, 2, 3]
pub fn array_literal(input: Span) -> PResult<Literal> {
    let (input, _) = open_bracket(input)?;
    let (input, _) = ws0(input)?;
    let (input, items) = separated_list0(comma, ws(literal)).parse(input)?;
    let (input, _) = ws0(input)?;
    let (input, _) = close_bracket(input)?;

    Ok((input, Literal::Array(items)))
}

/// Parse an object key (identifier or string)
fn object_key(input: Span) -> PResult<String> {
    alt((identifier, single_quoted_string, double_quoted_string)).parse(input)
}

/// Parse an object literal: {key: 'value', num: 42}
pub fn object_literal(input: Span) -> PResult<Literal> {
    let (input, _) = open_brace(input)?;
    let (input, _) = ws0(input)?;

    let (input, fields) =
        separated_list0(comma, ws(separated_pair(object_key, colon, literal))).parse(input)?;

    let (input, _) = ws0(input)?;
    let (input, _) = close_brace(input)?;

    Ok((input, Literal::Object(fields)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(s: &str) -> Span {
        Span::new(s)
    }

    #[test]
    fn test_null() {
        let (_, lit) = null_literal(span("null")).unwrap();
        assert_eq!(lit, Literal::Null);
    }

    #[test]
    fn test_boolean() {
        let (_, lit) = boolean_literal(span("true")).unwrap();
        assert_eq!(lit, Literal::Boolean(true));

        let (_, lit) = boolean_literal(span("false")).unwrap();
        assert_eq!(lit, Literal::Boolean(false));
    }

    #[test]
    fn test_integer() {
        let (_, lit) = number_literal(span("42")).unwrap();
        assert_eq!(lit, Literal::Integer(42));

        let (_, lit) = number_literal(span("-10")).unwrap();
        assert_eq!(lit, Literal::Integer(-10));

        let (_, lit) = number_literal(span("0")).unwrap();
        assert_eq!(lit, Literal::Integer(0));
    }

    #[test]
    fn test_float() {
        let (_, lit) = number_literal(span("3.14")).unwrap();
        assert_eq!(lit, Literal::Float(3.14));

        let (_, lit) = number_literal(span("-0.5")).unwrap();
        assert_eq!(lit, Literal::Float(-0.5));

        let (_, lit) = number_literal(span("1e10")).unwrap();
        assert_eq!(lit, Literal::Float(1e10));

        let (_, lit) = number_literal(span("2.5E-3")).unwrap();
        assert_eq!(lit, Literal::Float(2.5e-3));
    }

    #[test]
    fn test_string() {
        let (_, lit) = string_literal(span("'hello'")).unwrap();
        assert_eq!(lit, Literal::String("hello".to_string()));

        let (_, lit) = string_literal(span("\"world\"")).unwrap();
        assert_eq!(lit, Literal::String("world".to_string()));
    }

    #[test]
    fn test_string_escapes() {
        let (_, lit) = string_literal(span("'hello\\nworld'")).unwrap();
        assert_eq!(lit, Literal::String("hello\nworld".to_string()));

        let (_, lit) = string_literal(span("'it\\'s'")).unwrap();
        assert_eq!(lit, Literal::String("it's".to_string()));

        let (_, lit) = string_literal(span("\"tab\\there\"")).unwrap();
        assert_eq!(lit, Literal::String("tab\there".to_string()));
    }

    #[test]
    fn test_array() {
        let (_, lit) = array_literal(span("[]")).unwrap();
        assert_eq!(lit, Literal::Array(vec![]));

        let (_, lit) = array_literal(span("[1, 2, 3]")).unwrap();
        assert_eq!(
            lit,
            Literal::Array(vec![
                Literal::Integer(1),
                Literal::Integer(2),
                Literal::Integer(3)
            ])
        );

        let (_, lit) = array_literal(span("['a', 'b']")).unwrap();
        assert_eq!(
            lit,
            Literal::Array(vec![
                Literal::String("a".to_string()),
                Literal::String("b".to_string())
            ])
        );
    }

    #[test]
    fn test_object() {
        let (_, lit) = object_literal(span("{}")).unwrap();
        assert_eq!(lit, Literal::Object(vec![]));

        let (_, lit) = object_literal(span("{name: 'test'}")).unwrap();
        assert_eq!(
            lit,
            Literal::Object(vec![(
                "name".to_string(),
                Literal::String("test".to_string())
            )])
        );

        let (_, lit) = object_literal(span("{a: 1, b: 2}")).unwrap();
        assert_eq!(
            lit,
            Literal::Object(vec![
                ("a".to_string(), Literal::Integer(1)),
                ("b".to_string(), Literal::Integer(2))
            ])
        );
    }

    #[test]
    fn test_nested_structures() {
        let (_, lit) = array_literal(span("[[1, 2], [3, 4]]")).unwrap();
        assert_eq!(
            lit,
            Literal::Array(vec![
                Literal::Array(vec![Literal::Integer(1), Literal::Integer(2)]),
                Literal::Array(vec![Literal::Integer(3), Literal::Integer(4)])
            ])
        );

        let (_, lit) = object_literal(span("{items: [1, 2]}")).unwrap();
        assert_eq!(
            lit,
            Literal::Object(vec![(
                "items".to_string(),
                Literal::Array(vec![Literal::Integer(1), Literal::Integer(2)])
            )])
        );
    }
}
