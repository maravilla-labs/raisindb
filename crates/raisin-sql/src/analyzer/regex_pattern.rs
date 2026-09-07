//! SQL `SIMILAR TO` pattern → Rust regex translation.
//!
//! Lives in the analyzer crate so the analyzer can validate a constant
//! pattern and the executor can translate a runtime one with the SAME
//! rules — two translators would disagree on some edge and the analyzer
//! would accept a pattern the executor rejects per row.

/// Translate a `SIMILAR TO` pattern to an anchored regex source.
///
/// `%` → `.*`, `_` → `.`; the SQL:2003 regex operators (`|`, `*`, `+`,
/// `?`, `{m,n}`, `(...)`, `[...]`) pass through; every other regex
/// metacharacter is escaped so it matches literally. A backslash escapes
/// the following character.
pub fn similar_to_regex(pattern: &str) -> String {
    let mut out = String::with_capacity(pattern.len() + 8);
    out.push_str("^(?:");
    let mut chars = pattern.chars().peekable();
    let mut in_class = false;
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(next) = chars.next() {
                    out.push_str(&regex::escape(&next.to_string()));
                }
            }
            '[' => {
                in_class = true;
                out.push('[');
            }
            ']' => {
                in_class = false;
                out.push(']');
            }
            _ if in_class => out.push(c),
            '%' => out.push_str(".*"),
            '_' => out.push('.'),
            '|' | '*' | '+' | '?' | '{' | '}' | '(' | ')' => out.push(c),
            '.' | '^' | '$' | '#' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out.push_str(")$");
    out
}

#[cfg(test)]
mod tests {
    use super::similar_to_regex;

    #[test]
    fn translates_wildcards_and_keeps_alternation() {
        assert_eq!(similar_to_regex("a%"), "^(?:a.*)$");
        assert_eq!(similar_to_regex("a_c"), "^(?:a.c)$");
        assert_eq!(similar_to_regex("(a|b)%"), "^(?:(a|b).*)$");
        assert_eq!(similar_to_regex("v1.0"), r"^(?:v1\.0)$");
        assert_eq!(similar_to_regex(r"100\%"), "^(?:100%)$");
    }
}
