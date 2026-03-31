// SPDX-License-Identifier: BSL-1.1

//! Package dependency graph validation
//!
//! Provides functionality for:
//! - Building dependency graphs from packages
//! - Topological sorting (Kahn's algorithm)
//! - Circular dependency detection with cycle path reporting
//! - Validation of type references before installation

mod graph;
mod validation;

#[cfg(test)]
mod tests;

pub use graph::{DependencyGraph, DependencyGraphError, PackageNode};
pub use validation::{
    AvailableTypes, ContentValidationResult, ContentValidationWarning, ContentValidator,
};
