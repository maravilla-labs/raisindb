//! Error types for PGQ parsing

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
