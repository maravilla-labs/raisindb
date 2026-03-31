//! In-memory storage backend for RaisinDB.
//!
//! This crate provides an in-memory implementation of the RaisinDB storage traits,
//! suitable for testing, development, and small-scale deployments.
//!
//! # Features
//!
//! - Fast in-memory operations
//! - No external dependencies
//! - Thread-safe with async RwLock
//! - Suitable for testing and prototyping
//!
//! # Usage
//!
//! ```rust
//! use raisin_storage_memory::InMemoryStorage;
//! use std::sync::Arc;
//!
//! let storage = Arc::new(InMemoryStorage::default());
//! // Use with NodeService, etc.
//! ```
//!
//! # Warning
//!
//! All data is stored in memory and will be lost when the process terminates.
//! For persistent storage, use a different backend (RocksDB, PostgreSQL, etc.).

mod archetypes;
mod compound_index;
mod element_types;
mod gc;
pub(crate) mod index_types;
mod keys;
mod management;
mod node_types;
mod nodes;
mod property_index;
mod reference_index;
mod registry;
mod relations;
mod repository;
mod spatial_index;
mod storage;
mod tags;
mod trees;
mod tx;
mod versioning;
mod workspaces;

pub use archetypes::InMemoryArchetypeRepo;
pub use compound_index::InMemoryCompoundIndexRepo;
pub use element_types::InMemoryElementTypeRepo;
pub use gc::InMemoryGarbageCollector;
pub use keys::{NodeKey, NodeTypeKey, WorkspaceKey};
pub use node_types::InMemoryNodeTypeRepo;
pub use nodes::InMemoryNodeRepo;
pub use property_index::InMemoryPropertyIndexRepo;
pub use reference_index::InMemoryReferenceIndexRepo;
pub use registry::InMemoryRegistryRepo;
pub use relations::InMemoryRelationRepo;
pub use repository::{InMemoryBranchRepo, InMemoryRepositoryManagement, InMemoryRevisionRepo};
pub use spatial_index::InMemorySpatialIndexRepo;
pub use storage::{InMemoryStorage, NoopTranslationRepo};
pub use tags::InMemoryTagRepo;
pub use trees::InMemoryTreeRepo;
pub use tx::InMemoryTx;
pub use versioning::InMemoryVersioningRepo;
pub use workspaces::InMemoryWorkspaceRepo;
