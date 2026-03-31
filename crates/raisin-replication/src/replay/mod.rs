//! Operation replay engine for applying operations in causal order
//!
//! The replay engine is responsible for:
//! - Sorting operations by causal dependencies (vector clocks)
//! - Grouping operations by target entity
//! - Applying CRDT merge rules
//! - Ensuring idempotency

mod engine;
pub mod idempotency;
#[cfg(test)]
mod tests;
pub mod types;

pub use engine::ReplayEngine;
pub use idempotency::{IdempotencyTracker, InMemoryIdempotencyTracker};
pub use types::{ConflictInfo, ReplayResult};
