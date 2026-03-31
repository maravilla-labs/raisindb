//! Expression evaluation module

mod async_evaluator;
mod comparison;
mod context;
mod evaluator;
mod methods;
pub mod resolver;

#[cfg(test)]
mod evaluator_tests;

pub use async_evaluator::{evaluate_async, requires_async};
pub use context::EvalContext;
pub use evaluator::evaluate;
pub use resolver::{NoOpResolver, RelationResolver};

// Re-export RelDirection from ast for convenience
pub use crate::ast::RelDirection;
