//! Per-statement bounds on what RESOLVE may read and inline.
//!
//! A depth-10 RESOLVE over a densely linked graph fans out exponentially, and a
//! listing multiplies that by its row count. Without a bound that is an
//! unbounded read and an unbounded allocation behind one innocuous function
//! call. Exceeding a bound is an ERROR that names RESOLVE — never a document
//! with some references quietly left bare, which a renderer cannot tell from
//! broken links.

use raisin_error::Error;

/// The bounds one statement's RESOLVE calls share.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolveBudget {
    /// Distinct targets read (a missing or denied target still costs a read).
    pub max_targets: usize,
    /// References replaced by a node, counted at every nesting level.
    pub max_occurrences: usize,
    /// Approximate serialized bytes inlined.
    pub max_bytes: usize,
}

impl Default for ResolveBudget {
    fn default() -> Self {
        Self {
            max_targets: 5_000,
            max_occurrences: 50_000,
            max_bytes: 32 * 1024 * 1024,
        }
    }
}

const HINT: &str = "Lower RESOLVE's depth, pass a `fields` list, or select fewer rows.";

pub(super) fn targets_exceeded(limit: usize) -> Error {
    Error::Validation(format!(
        "RESOLVE() budget exceeded: this statement would read more than {limit} distinct \
         referenced nodes. {HINT}"
    ))
}

pub(super) fn occurrences_exceeded(limit: usize) -> Error {
    Error::Validation(format!(
        "RESOLVE() budget exceeded: this statement would inline more than {limit} referenced \
         nodes. {HINT}"
    ))
}

pub(super) fn bytes_exceeded(limit: usize) -> Error {
    Error::Validation(format!(
        "RESOLVE() budget exceeded: this statement would inline more than {limit} bytes of \
         referenced content. {HINT}"
    ))
}
