//! Simple SQL LIKE pattern matching.

/// Simple LIKE pattern matching
///
/// Supports:
/// - `%` matches any sequence of characters
/// - `_` matches any single character
pub(super) fn like_match(s: &str, pattern: &str) -> bool {
    let mut pattern_chars = pattern.chars().peekable();
    let mut string_chars = s.chars().peekable();

    like_match_impl(&mut pattern_chars, &mut string_chars)
}

fn like_match_impl(
    pattern: &mut std::iter::Peekable<std::str::Chars>,
    string: &mut std::iter::Peekable<std::str::Chars>,
) -> bool {
    while let Some(p) = pattern.next() {
        match p {
            '%' => {
                // Skip consecutive %
                while pattern.peek() == Some(&'%') {
                    pattern.next();
                }

                // If % is at end, match rest of string
                if pattern.peek().is_none() {
                    return true;
                }

                // Try matching rest of pattern at each position
                loop {
                    let mut pattern_copy = pattern.clone();
                    let mut string_copy = string.clone();
                    if like_match_impl(&mut pattern_copy, &mut string_copy) {
                        return true;
                    }
                    if string.next().is_none() {
                        return false;
                    }
                }
            }
            '_' => {
                // Match any single character
                if string.next().is_none() {
                    return false;
                }
            }
            c => {
                // Match exact character (case insensitive)
                match string.next() {
                    Some(s) if s.to_lowercase().next() == c.to_lowercase().next() => {}
                    _ => return false,
                }
            }
        }
    }

    // Pattern exhausted, string should also be exhausted
    string.peek().is_none()
}
