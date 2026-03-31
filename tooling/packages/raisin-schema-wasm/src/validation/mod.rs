//! Core validation logic for manifest, nodetype, workspace, and content files
//!
//! This module is split into submodules for maintainability:
//! - `context` - ValidationContext and constants (regex, valid types)
//! - `field_resolution` - Inheritance-aware field resolution for archetypes and element types
//! - `manifest` - Manifest file validation
//! - `nodetype` - NodeType file validation
//! - `workspace` - Workspace file validation
//! - `content` - Content file validation (references, elements, archetypes)
//! - `archetype` - Archetype file validation (serde-based)
//! - `elementtype` - ElementType file validation (serde-based)
//! - `helpers` - String conversion and error formatting utilities

mod archetype;
mod content;
mod context;
mod elementtype;
mod field_resolution;
mod helpers;
mod manifest;
mod nodetype;
mod workspace;

// Re-export public API
pub use archetype::validate_archetype;
pub use content::validate_content;
pub use context::ValidationContext;
pub use elementtype::validate_elementtype;
pub use manifest::validate_manifest;
pub use nodetype::validate_nodetype;
pub use workspace::validate_workspace;

#[cfg(test)]
mod tests;
