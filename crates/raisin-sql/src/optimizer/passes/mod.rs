//! Optimization Pass Implementations
//!
//! This module contains the implementations of optimization passes as methods
//! on the `Optimizer` struct. Each pass is implemented in a separate file
//! for better code organization.
//!
//! # Passes
//!
//! - `constant_folding_pass` - Evaluates constant expressions at compile time
//! - `hierarchy_rewriting_pass` - Transforms hierarchy functions to canonical predicates

mod constant_folding_pass;
mod hierarchy_rewriting_pass;
