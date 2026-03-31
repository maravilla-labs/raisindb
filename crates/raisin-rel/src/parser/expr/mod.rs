//! Expression parsing with proper operator precedence
//!
//! Operator precedence (lowest to highest):
//! 1. || (OR)
//! 2. && (AND)
//! 3. ==, !=, <, >, <=, >= (comparison), RELATES
//! 4. +, - (additive)
//! 5. *, /, % (multiplicative)
//! 6. !, - (unary NOT, unary minus)
//! 7. . and [...] (property/index access), method calls (.method())
//! 8. Atoms (literals, variables, parentheses)

mod parsers;

#[cfg(test)]
mod tests;

pub use parsers::expr;
