//! Error types for PGQ parsing
//!
//! # Actionable diagnostics
//!
//! nom's default error type carries only an [`nom::error::ErrorKind`], which
//! renders as "Expected keyword" — useless for a construct we deliberately
//! reject (`SIMPLE`, an unscoped `*`, a `COST` on an anonymous edge). Rather
//! than thread a custom error type through every combinator, the parser calls
//! [`fail`], which stores a message in a thread-local **and** returns
//! [`nom::Err::Failure`].
//!
//! `Failure` is what makes this reliable: unlike `Err::Error` it is not
//! backtracked by `alt` / `opt` / `many0`, so it aborts straight out to
//! [`crate::ast::pgq_parser::parse_graph_table`], which clears the slot before
//! each parse and takes it afterwards. There is no window in which a stale
//! message can be attributed to a later parse.

/// Parse error with location information
#[derive(Debug, Clone)]
pub struct PgqParseError {
    /// Error message
    pub message: String,
    /// Line number (1-indexed)
    pub line: usize,
    /// Column number (1-indexed)
    pub column: usize,
    /// Byte offset in source
    pub offset: usize,
    /// Context snippet
    pub context: Option<String>,
}

impl std::fmt::Display for PgqParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Parse error at line {}, column {}: {}",
            self.line, self.column, self.message
        )?;
        if let Some(ctx) = &self.context {
            write!(f, "\n  --> {}", ctx)?;
        }
        Ok(())
    }
}

impl std::error::Error for PgqParseError {}

/// Calculate line and column from byte offset
pub fn offset_to_location(input: &str, offset: usize) -> (usize, usize) {
    let prefix = &input[..offset.min(input.len())];
    let line = prefix.chars().filter(|c| *c == '\n').count() + 1;
    let column = prefix
        .rfind('\n')
        .map(|pos| offset - pos)
        .unwrap_or(offset + 1);
    (line, column)
}

/// Create error with location
pub fn make_error(input: &str, original: &str, message: impl Into<String>) -> PgqParseError {
    let offset = original.len() - input.len();
    let (line, column) = offset_to_location(original, offset);
    let context = input
        .lines()
        .next()
        .map(|l| l.chars().take(40).collect::<String>());

    PgqParseError {
        message: message.into(),
        line,
        column,
        offset,
        context,
    }
}

thread_local! {
    /// Remaining-input length plus message for the most recent hard failure.
    static DIAGNOSTIC: std::cell::RefCell<Option<(usize, String)>> =
        const { std::cell::RefCell::new(None) };
}

/// Abort the parse with an actionable message pinned to `input`'s position.
///
/// Returns [`nom::Err::Failure`] so no combinator backtracks past it.
pub fn fail<T>(input: &str, message: impl Into<String>) -> nom::IResult<&str, T> {
    let message = message.into();
    DIAGNOSTIC.with(|slot| *slot.borrow_mut() = Some((input.len(), message)));
    Err(nom::Err::Failure(nom::error::Error::new(
        input,
        nom::error::ErrorKind::Verify,
    )))
}

/// Drop any pending diagnostic. Called before every top-level parse.
pub fn clear_diagnostic() {
    DIAGNOSTIC.with(|slot| *slot.borrow_mut() = None);
}

/// Take the pending diagnostic, if the parse raised one.
///
/// Returns the remaining-input length at the failure point and the message.
pub fn take_diagnostic() -> Option<(usize, String)> {
    DIAGNOSTIC.with(|slot| slot.borrow_mut().take())
}

/// Describe nom error kind for user-friendly messages
pub fn describe_error_kind(kind: &nom::error::ErrorKind) -> &'static str {
    use nom::error::ErrorKind::*;
    match kind {
        Tag => "keyword",
        Char => "character",
        Alpha => "alphabetic character",
        Digit => "digit",
        TakeWhile1 => "identifier",
        _ => "valid syntax",
    }
}
