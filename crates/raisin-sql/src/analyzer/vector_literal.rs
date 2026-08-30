//! Recognising a vector that arrived as SQL text.
//!
//! # Why this exists
//!
//! A caller with a 1024-float image embedding does not type it into a query.
//! It BINDS it: `KNN($1, 10, workspaces => 'assets')` with the floats in the
//! parameter array. Parameters are substituted into the SQL text before it is
//! parsed (`raisin_sql::substitute_params`), and the two substituters in this
//! tree render a JSON array two different ways:
//!
//! | surface | `[0.1, 0.2]` becomes | parses as |
//! |---|---|---|
//! | HTTP / pgwire (`params::format_param_value`) | `'[0.1,0.2]'` | `Literal::Text` |
//! | functions runtime (`callbacks::sql_params`) | `ARRAY[0.1, 0.2]` | `SqlExpr::Array` |
//!
//! Neither reached a vector. The text form was EMBEDDED as the literal string
//! `"[0.1,0.2]"` — a silent wrong answer, ranked plausibly, with nothing logged;
//! the `ARRAY[...]` form was `UnsupportedExpression`. This module is the one
//! place that turns either into `Vec<f32>`, so the two surfaces cannot come to
//! mean different things by the same parameter.
//!
//! # The text form is pgvector's own literal
//!
//! `'[1,2,3]'` is how pgvector writes a vector, and it is what
//! `format_param_value` already produces for a bound array. Accepting it costs
//! one ambiguity: a caller whose search TEXT is literally `"[1, 2, 3]"` now
//! searches by vector. That trade is taken deliberately, because the two
//! failure modes are not symmetric — treating a bound vector as text is silent
//! and unfixable from the outside, while treating a 3-word bracketed phrase as
//! a 3-dimensional vector fails LOUDLY on the first dimension check. Prefer the
//! loud one.
//!
//! Recognition is strict on purpose: brackets, at least one element, and EVERY
//! element a finite number. `'[a, b]'` stays text; `'[]'` stays text;
//! `'[1, NaN]'` stays text. Anything that is not unambiguously a vector is not
//! one.

/// Parse the pgvector / JSON text form of a vector — `[0.1, -0.2, 3]`.
///
/// Returns `None` for anything that is not unambiguously a vector, in which
/// case the caller must go on treating the string as text.
pub fn parse_vector_text(raw: &str) -> Option<Vec<f32>> {
    let trimmed = raw.trim();
    let inner = trimmed.strip_prefix('[')?.strip_suffix(']')?.trim();
    if inner.is_empty() {
        // `'[]'` is a zero-dimension vector, which no index can be searched
        // with. Left as text so the error names the real problem.
        return None;
    }

    let mut out = Vec::new();
    for element in inner.split(',') {
        let value: f32 = element.trim().parse().ok()?;
        if !value.is_finite() {
            // NaN and the infinities are parseable f32s and would poison every
            // distance they touch. A vector containing one is not a vector.
            return None;
        }
        out.push(value);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pgvector_text_form_is_a_vector() {
        assert_eq!(
            parse_vector_text("[0.1,0.2,0.3]"),
            Some(vec![0.1, 0.2, 0.3])
        );
        // Whitespace, as a human would type it.
        assert_eq!(
            parse_vector_text("  [1, -2.5, 3e-2]  "),
            Some(vec![1.0, -2.5, 3e-2])
        );
        // Integers, which is what a JSON array of whole numbers renders as.
        assert_eq!(parse_vector_text("[1,2]"), Some(vec![1.0, 2.0]));
    }

    /// The whole reason the recognition is strict: a search for a phrase that
    /// happens to start with `[` must not become a vector search.
    #[test]
    fn prose_stays_prose() {
        assert_eq!(parse_vector_text("kittens"), None);
        assert_eq!(parse_vector_text("[draft] release notes"), None);
        assert_eq!(parse_vector_text("[a, b, c]"), None);
        assert_eq!(parse_vector_text("[1, two]"), None);
        assert_eq!(parse_vector_text("[]"), None);
        assert_eq!(parse_vector_text("[1,2"), None);
        assert_eq!(parse_vector_text("1,2,3"), None);
    }

    /// A non-finite element would make every distance it touches meaningless,
    /// and `"NaN".parse::<f32>()` succeeds — so it has to be rejected here.
    #[test]
    fn a_non_finite_element_is_not_a_vector() {
        assert_eq!(parse_vector_text("[1, NaN]"), None);
        assert_eq!(parse_vector_text("[1, inf]"), None);
        assert_eq!(parse_vector_text("[1, -inf]"), None);
    }
}
