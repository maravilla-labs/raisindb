//! JSON_QUERY function - extract JSON objects/arrays from JSON using JSONPath
//!
//! Implements SQL:2016 (ISO/IEC 9075-2:2016) JSON_QUERY function.
//! Complements JSON_VALUE by extracting structured data (objects/arrays) instead of scalars.

mod clauses;
mod evaluate;

pub use evaluate::JsonQueryFunction;

#[cfg(test)]
mod tests;
