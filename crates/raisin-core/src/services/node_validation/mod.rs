//! Node validation service.
//!
//! Validates nodes against their NodeType schemas including:
//! - Required properties
//! - Strict mode (no undefined properties)
//! - Unique properties
//! - Property type validation
//! - Archetype field constraints
//! - Element type field constraints

mod archetype_validation;
mod core;
mod element_validation;
mod property_checks;

#[cfg(test)]
mod tests;

pub use self::core::NodeValidator;
