//! Projection and aggregation logic for Cypher queries
//!
//! This module contains the logic for projecting variable bindings into result rows,
//! supporting both simple projections and complex aggregations with grouping.
//!
//! # Modules
//! - `accumulator` - Accumulator types for aggregate functions (COUNT, SUM, AVG, etc.)
//! - `grouping` - GroupKey type for efficient grouping
//! - `projector` - Main projection logic and ProjectionEngine

mod accumulator;
mod grouping;
mod projector;

pub(crate) use projector::ProjectionEngine;
