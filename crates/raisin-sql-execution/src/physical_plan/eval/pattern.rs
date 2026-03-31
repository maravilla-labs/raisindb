//! SQL LIKE pattern matching implementation

/// Matches text against a SQL LIKE pattern
///
/// Supports:
/// - `%` matches zero or more characters
/// - `_` matches exactly one character
/// - All other characters match literally
///
/// # Examples
/// ```
/// # use raisin_sql::physical_plan::eval::pattern::sql_like_match;
/// assert!(sql_like_match("hello", "h%"));
/// assert!(sql_like_match("hello", "h_llo"));
/// assert!(!sql_like_match("hello", "h_l"));
/// ```
pub(super) fn sql_like_match(text: &str, pattern: &str) -> bool {
    let mut text_chars = text.chars().peekable();
    let mut pattern_chars = pattern.chars().peekable();

    sql_like_match_recursive(&mut text_chars, &mut pattern_chars, false)
}

/// Case-insensitive SQL ILIKE pattern matching
///
/// Same as sql_like_match but ignores case differences.
///
/// # Examples
/// ```
/// # use raisin_sql::physical_plan::eval::pattern::sql_ilike_match;
/// assert!(sql_ilike_match("Hello", "h%"));
/// assert!(sql_ilike_match("HELLO", "hello"));
/// assert!(sql_ilike_match("Hello World", "%WORLD"));
/// ```
pub(super) fn sql_ilike_match(text: &str, pattern: &str) -> bool {
    let mut text_chars = text.chars().peekable();
    let mut pattern_chars = pattern.chars().peekable();

    sql_like_match_recursive(&mut text_chars, &mut pattern_chars, true)
}

/// Recursive helper for SQL LIKE pattern matching
///
/// When `case_insensitive` is true, performs case-insensitive matching (ILIKE).
fn sql_like_match_recursive(
    text: &mut std::iter::Peekable<std::str::Chars>,
    pattern: &mut std::iter::Peekable<std::str::Chars>,
    case_insensitive: bool,
) -> bool {
    loop {
        match pattern.next() {
            None => {
                // Pattern exhausted - matches if text also exhausted
                return text.peek().is_none();
            }
            Some('%') => {
                // % matches zero or more characters
                // Try matching the rest of the pattern at each position
                if pattern.peek().is_none() {
                    // % at end of pattern always matches
                    return true;
                }

                // Try matching at current position (% matches zero chars)
                let text_clone = text.clone();
                let pattern_clone = pattern.clone();
                if sql_like_match_recursive(
                    &mut text.clone(),
                    &mut pattern.clone(),
                    case_insensitive,
                ) {
                    return true;
                }

                // Try advancing text and matching again (% matches one or more chars)
                let mut text = text_clone;
                while text.next().is_some() {
                    if sql_like_match_recursive(
                        &mut text.clone(),
                        &mut pattern_clone.clone(),
                        case_insensitive,
                    ) {
                        return true;
                    }
                }
                return false;
            }
            Some('_') => {
                // _ matches exactly one character
                if text.next().is_none() {
                    return false;
                }
            }
            Some(p) => {
                // Literal character - must match exactly (or case-insensitively)
                match text.next() {
                    Some(t) => {
                        let matches = if case_insensitive {
                            t.to_lowercase().eq(p.to_lowercase())
                        } else {
                            t == p
                        };
                        if matches {
                            continue;
                        } else {
                            return false;
                        }
                    }
                    None => return false,
                }
            }
        }
    }
}
