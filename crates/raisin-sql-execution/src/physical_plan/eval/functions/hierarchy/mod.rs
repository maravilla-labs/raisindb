//! Hierarchical path manipulation functions
//!
//! This module contains functions for working with hierarchical paths:
//! - DEPTH: Calculate hierarchy depth
//! - PARENT: Get parent path (with optional levels up)
//! - ANCESTOR: Get ancestor at absolute depth from root
//! - PATH_STARTS_WITH: Check if path starts with prefix
//! - CHILD_OF: Check if path is a direct child of parent
//! - DESCENDANT_OF: Check if path is a descendant of parent (any depth or limited)
//! - REFERENCES: Check if node references a target (uses reverse reference index)
//!
//! Helper functions for path manipulation are in the helpers module.

mod ancestor;
mod child_of;
mod depth;
mod descendant_of;
pub mod helpers;
mod parent;
mod path_starts_with;
mod references;

pub use ancestor::AncestorFunction;
pub use child_of::ChildOfFunction;
pub use depth::DepthFunction;
pub use descendant_of::DescendantOfFunction;
pub use parent::ParentFunction;
pub use path_starts_with::PathStartsWithFunction;
pub use references::ReferencesFunction;

use super::registry::FunctionRegistry;

/// Register all hierarchy functions in the provided registry
///
/// This function is called during registry initialization to register
/// all hierarchical path manipulation functions.
pub fn register_functions(registry: &mut FunctionRegistry) {
    registry.register(Box::new(DepthFunction));
    registry.register(Box::new(ParentFunction));
    registry.register(Box::new(AncestorFunction));
    registry.register(Box::new(PathStartsWithFunction));
    registry.register(Box::new(ChildOfFunction));
    registry.register(Box::new(DescendantOfFunction));
    registry.register(Box::new(ReferencesFunction));
}
