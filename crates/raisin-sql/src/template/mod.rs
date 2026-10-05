//! Statement templates: a query analyzed and optimized ONCE with its `$n`
//! placeholders in place, then bound to each execution's values (plan Phase
//! 13d).
//!
//! # The contract
//!
//! Binding values into a template must produce exactly what analyzing and
//! optimizing the SUBSTITUTED text would (`raisin_sql::substitute_params`
//! renders each value as a SQL literal into the text). Three things make
//! that hold:
//!
//! 1. **Types.** A placeholder is analyzed with the type of the literal its
//!    value renders to ([`param_literal`] — the analyzer's own literal rule,
//!    `literal_from_sql_value`), so every type check and coercion decision is
//!    made as on the literal. The one coercion that re-spells a literal
//!    (Text → Path for a function argument) types the placeholder `Path`,
//!    and binding applies the same `coerce_text_to_path`.
//! 2. **Positions.** A literal's VALUE also steers analysis and optimization
//!    in places a type cannot: `__revision`/`__branch` extraction, LIMIT,
//!    `@>` JSON parsing, regex validation, analysis-time folding, LIKE, …
//!    [`check`] admits a template only when every placeholder sits where its
//!    value steers nothing at analysis: a comparison, IN list or BETWEEN
//!    against a column or a JSON extraction of one, or the path argument of
//!    `CHILD_OF` / `DESCENDANT_OF` / `PATH_STARTS_WITH` / `REFERENCES`. The
//!    one extraction that IS supported is `locale = $n` (and `IN`): the
//!    analyzer records a [`locale_marker`] that binding replaces. Anything
//!    else makes the statement VALUE-DEPENDENT: it is planned per value,
//!    exactly as before.
//! 3. **Value-driven optimizer passes.** Constant folding and the hierarchy
//!    rewrite (`CHILD_OF` → canonical, JSON property equality, …) read
//!    literal values; `Optimizer::rebind` re-runs both on the bound plan.
//!    The physical planner always runs on the bound plan, so its literal-
//!    driven choices (point lookup, compound index, LIMIT pushdown, the
//!    localized lookup) see the values.
//!
//! The executing side (`raisin-sql-execution`'s prepared cache) also proves
//! the contract per template: on a template's first use it plans the
//! substituted text the ordinary way and caches the template only if the
//! bound template is IDENTICAL to it.

mod bind;
mod check;
mod literal;

#[cfg(test)]
mod tests;

pub use bind::{bind_plan, bind_statement, count_parameters};
pub use check::{check, placeholders_are_tokens};
pub use literal::param_literal;

/// Prefix of the locale a template's `locale = $n` is extracted into. NUL
/// cannot occur in a locale code; the binder replaces the marker with the
/// bound value.
const LOCALE_MARKER_PREFIX: &str = "\u{0}raisin-param:";

/// The `locales` entry recording "the locale is parameter `placeholder`".
pub fn locale_marker(placeholder: &str) -> String {
    format!("{LOCALE_MARKER_PREFIX}{placeholder}")
}

/// The placeholder a `locales` entry stands for, if it is a marker.
pub(crate) fn locale_marker_param(locale: &str) -> Option<&str> {
    locale.strip_prefix(LOCALE_MARKER_PREFIX)
}

/// `$n` → `n - 1`.
pub(crate) fn param_index(placeholder: &str) -> Option<usize> {
    placeholder
        .strip_prefix('$')
        .and_then(|n| n.parse::<usize>().ok())
        .filter(|n| *n >= 1)
        .map(|n| n - 1)
}
