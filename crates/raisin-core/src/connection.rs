//! Server-side connection API for RaisinDB.
//!
//! This module provides the main entry point for server-side operations with RaisinDB.
//! It implements a MongoDB-inspired fluent API with repository-first architecture.
//!
//! # Architecture
//!
//! ```text
//! RaisinConnection<S: Storage>  (server connection with storage backend)
//!   |
//! TenantScope<'c, S>            (tenant isolation - always required)
//!   |
//! Repository<S>                 (repository/database handle)
//!   |
//! Workspace<S>                  (workspace within repository)
//!   |
//! NodeServiceBuilder            (fluent API with .branch() and .revision())
//!   |
//! NodeService                   (CRUD operations on nodes)
//! ```
//!
//! # Example
//!
//! ```rust,no_run
//! use raisin_core::RaisinConnection;
//! use raisin_storage_memory::InMemoryStorage;
//! use std::sync::Arc;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let storage = Arc::new(InMemoryStorage::default());
//! let connection = RaisinConnection::with_storage(storage);
//!
//! let tenant = connection.tenant("default");
//! let repo = tenant.repository("my-app");
//! let workspace = repo.workspace("main");
//! let node = workspace.nodes().get("node-id").await?;
//! # Ok(())
//! # }
//! ```

mod core;
mod management;
mod node_crud;
mod repository;
mod tenant;
mod tree_publish;
mod workspace;

#[cfg(test)]
mod tests;

pub use self::core::{RaisinConnection, ServerConfig};
pub use management::RepositoryManagement;
pub use repository::Repository;
pub use tenant::TenantScope;
pub use workspace::{NodeServiceBuilder, Workspace};
