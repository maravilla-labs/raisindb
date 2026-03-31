//! One-time data format migrations
//!
//! This module handles migrating existing RocksDB data between serialization
//! formats and schema versions.
//!
//! **TEMPORARY MODULE** - Should be deleted after successful deployment and migration.
//!
//! # Migration submodules
//!
//! - `json_to_msgpack` - JSON to MessagePack serialization migration
//! - `relation_schema` - Relation schema migration (old 3/5-field to new 5/8-field)
//! - `job_metadata` - Job metadata and type serialization migrations
//! - `embedding_config` - Tenant embedding config migration
//! - `revision_meta` - Revision metadata migrations (v2 and v3)

mod embedding_config;
mod job_metadata;
mod json_to_msgpack;
mod old_types;
mod relation_schema;
mod revision_meta;

pub use embedding_config::run_tenant_embedding_config_migration;
pub use job_metadata::{run_job_metadata_migration, run_job_type_serialization_migration};
pub use json_to_msgpack::run_migration;
pub use relation_schema::run_relation_schema_migration;
pub use revision_meta::{run_revision_meta_migration, run_revision_meta_migration_v3};

/// Batch size for iterating over column family entries during migration
const BATCH_SIZE: usize = 1000;
