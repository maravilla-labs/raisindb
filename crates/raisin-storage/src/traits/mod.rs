// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Storage trait definitions for RaisinDB.
//!
//! This module contains all the repository trait definitions used by RaisinDB.
//! Different storage backends (in-memory, RocksDB, PostgreSQL, MongoDB)
//! implement these traits to provide data persistence.
//!
//! # Main Traits
//!
//! - [`Storage`] - Main storage abstraction providing access to all repositories
//! - [`NodeRepository`] - CRUD operations for nodes
//! - [`NodeTypeRepository`] - CRUD operations for node types
//! - [`WorkspaceRepository`] - CRUD operations for workspaces
//! - [`RegistryRepository`] - Multi-tenant deployment and tenant registration
//!
//! # Index Traits
//!
//! - [`PropertyIndexRepository`] - For fast property-based lookups
//! - [`ReferenceIndexRepository`] - For tracking PropertyValue::Reference relationships
//! - [`CompoundIndexRepository`] - For multi-column queries with ORDER BY
//!
//! # Schema Traits
//!
//! - [`NodeTypeRepository`] - For managing NodeType schemas
//! - [`ArchetypeRepository`] - For managing Archetype definitions
//! - [`ElementTypeRepository`] - For managing ElementType definitions

pub mod index;
pub mod node;
pub mod processing_rules;
pub mod registry;
pub mod relation;
pub mod schema;
pub mod storage;
pub mod workspace;

// Re-export all traits at the module level for convenience
pub use index::{
    CompoundColumnValue, CompoundIndexRepository, CompoundIndexScanEntry, PropertyIndexRepository,
    PropertyScanEntry, ReferenceIndexRepository,
};
pub use node::NodeRepository;
pub use processing_rules::ProcessingRulesRepository;
pub use registry::{RegistryRepository, TreeRepository};
pub use relation::{GlobalRelationEntry, RelationRepository};
pub use schema::{ArchetypeRepository, ElementTypeRepository, NodeTypeRepository};
pub use storage::{Storage, Transaction};
pub use workspace::{VersioningRepository, WorkspaceRepository};
