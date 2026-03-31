//! Schema operation handlers for replication
//!
//! This module provides:
//! 1. Common schema operation helpers (apply_schema_update, delete_schema)
//! 2. Specific handlers for NodeType, Archetype, and ElementType operations:
//!    - apply_update_nodetype, apply_delete_nodetype
//!    - apply_update_archetype, apply_delete_archetype
//!    - apply_update_element_type, apply_delete_element_type

use super::super::OperationApplicator;
use super::db_helpers::{delete_with_prefix, serialize_and_write};
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_events::SchemaEventKind;
use raisin_hlc::HLC;
use raisin_models::nodes::element::element_type::ElementType;
use raisin_models::nodes::types::archetype::Archetype;
use raisin_models::nodes::types::node_type::NodeType;
use raisin_replication::Operation;
use rocksdb::DB;
use serde::Serialize;
use std::sync::Arc;

/// Trait for schema types that can be versioned
pub trait VersionedSchema: Serialize {
    fn name(&self) -> &str;
    fn version(&self) -> Option<i32>;
}

impl VersionedSchema for raisin_models::nodes::types::node_type::NodeType {
    fn name(&self) -> &str {
        &self.name
    }
    fn version(&self) -> Option<i32> {
        self.version
    }
}

impl VersionedSchema for raisin_models::nodes::types::archetype::Archetype {
    fn name(&self) -> &str {
        &self.name
    }
    fn version(&self) -> Option<i32> {
        self.version
    }
}

impl VersionedSchema for raisin_models::nodes::element::element_type::ElementType {
    fn name(&self) -> &str {
        &self.name
    }
    fn version(&self) -> Option<i32> {
        self.version
    }
}

/// Apply a schema update operation (NodeType, Archetype, or ElementType)
///
/// This encapsulates the common pattern:
/// 1. Create versioned key
/// 2. Serialize and write schema
/// 3. Create version index if version is present
pub fn apply_schema_update<T: VersionedSchema>(
    db: &Arc<DB>,
    cf_name: &str,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    schema: &T,
    revision: &HLC,
    key_fn: impl FnOnce(&str, &str, &str, &str, &HLC) -> Vec<u8>,
    version_index_fn: impl FnOnce(&str, &str, &str, &str, i32) -> Vec<u8>,
    op_id: &str,
) -> Result<()> {
    let cf = cf_handle(db, cf_name)?;
    let key = key_fn(tenant_id, repo_id, branch, schema.name(), revision);

    tracing::debug!(
        op_id = op_id,
        tenant_id = tenant_id,
        repo_id = repo_id,
        schema_name = schema.name(),
        version = ?schema.version(),
        "Applying schema update"
    );

    // Serialize and write
    serialize_and_write(
        db,
        cf,
        key,
        schema,
        &format!("apply_schema_update_{}", schema.name()),
    )?;

    // Create version index if version is present
    if let Some(version) = schema.version() {
        let index_key = version_index_fn(tenant_id, repo_id, branch, schema.name(), version);
        db.put_cf(cf, index_key, revision.encode_descending())
            .map_err(|e| {
                tracing::error!(
                    op_id = op_id,
                    schema_name = schema.name(),
                    version = version,
                    error = %e,
                    "Failed to write schema version index"
                );
                raisin_error::Error::storage(e.to_string())
            })?;
    }

    tracing::debug!(
        op_id = op_id,
        schema_name = schema.name(),
        "Successfully applied schema update"
    );

    Ok(())
}

/// Delete a schema entity (all versions)
///
/// This encapsulates the common pattern for schema deletions:
/// 1. Get prefix for the schema name
/// 2. Scan and delete all keys with that prefix
pub fn delete_schema(
    db: &Arc<DB>,
    cf_name: &str,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    schema_id: &str,
    prefix_fn: impl FnOnce(&str, &str, &str, &str) -> Vec<u8>,
) -> Result<()> {
    let cf = cf_handle(db, cf_name)?;
    let prefix = prefix_fn(tenant_id, repo_id, branch, schema_id);

    let count = delete_with_prefix(db, cf, &prefix, &format!("delete_schema_{}", schema_id))?;

    tracing::info!(
        tenant_id = tenant_id,
        repo_id = repo_id,
        branch = branch,
        schema_id = schema_id,
        deleted_count = count,
        "Deleted schema with all versions"
    );

    Ok(())
}

// ========== OPERATION HANDLERS ==========

/// Apply a NodeType update operation
pub(super) async fn apply_update_nodetype(
    applicator: &OperationApplicator,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    _node_type_id: &str,
    node_type: &NodeType,
    op: &Operation,
) -> Result<()> {
    tracing::info!(
        "📥 Applying NodeType update: {}/{}/{}/{} from node {}",
        tenant_id,
        repo_id,
        branch,
        node_type.name,
        op.cluster_node_id
    );

    let revision = OperationApplicator::op_revision(op)?;

    apply_schema_update(
        &applicator.db,
        cf::NODE_TYPES,
        tenant_id,
        repo_id,
        branch,
        node_type,
        &revision,
        keys::nodetype_key_versioned,
        keys::nodetype_version_index_key,
        &op.op_id.to_string(),
    )?;

    tracing::info!(
        "✅ NodeType applied successfully: {}/{}/{}/{}",
        tenant_id,
        repo_id,
        branch,
        node_type.name
    );

    applicator.emit_schema_event(
        tenant_id,
        repo_id,
        branch,
        &node_type.name,
        "NodeType",
        SchemaEventKind::NodeTypeUpdated,
    );

    Ok(())
}

/// Apply an Archetype update operation
pub(super) async fn apply_update_archetype(
    applicator: &OperationApplicator,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    _archetype_id: &str,
    archetype: &Archetype,
    op: &Operation,
) -> Result<()> {
    tracing::info!(
        "📥 Applying Archetype update: {}/{}/{}/{} from node {}",
        tenant_id,
        repo_id,
        branch,
        archetype.name,
        op.cluster_node_id
    );

    let revision = OperationApplicator::op_revision(op)?;

    apply_schema_update(
        &applicator.db,
        cf::ARCHETYPES,
        tenant_id,
        repo_id,
        branch,
        archetype,
        &revision,
        keys::archetype_key_versioned,
        keys::archetype_version_index_key,
        &op.op_id.to_string(),
    )?;

    tracing::info!(
        "✅ Archetype applied successfully: {}/{}/{}/{}",
        tenant_id,
        repo_id,
        branch,
        archetype.name
    );

    applicator.emit_schema_event(
        tenant_id,
        repo_id,
        branch,
        &archetype.name,
        "Archetype",
        SchemaEventKind::ArchetypeUpdated,
    );

    Ok(())
}

/// Apply an ElementType update operation
pub(super) async fn apply_update_element_type(
    applicator: &OperationApplicator,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    _element_type_id: &str,
    element_type: &ElementType,
    op: &Operation,
) -> Result<()> {
    tracing::info!(
        "📥 Applying ElementType update: {}/{}/{}/{} from node {}",
        tenant_id,
        repo_id,
        branch,
        element_type.name,
        op.cluster_node_id
    );

    let revision = OperationApplicator::op_revision(op)?;

    apply_schema_update(
        &applicator.db,
        cf::ELEMENT_TYPES,
        tenant_id,
        repo_id,
        branch,
        element_type,
        &revision,
        keys::element_type_key_versioned,
        keys::element_type_version_index_key,
        &op.op_id.to_string(),
    )?;

    tracing::info!(
        "✅ ElementType applied successfully: {}/{}/{}/{}",
        tenant_id,
        repo_id,
        branch,
        element_type.name
    );

    applicator.emit_schema_event(
        tenant_id,
        repo_id,
        branch,
        &element_type.name,
        "ElementType",
        SchemaEventKind::ElementTypeUpdated,
    );

    Ok(())
}

/// Apply a NodeType delete operation
pub(super) async fn apply_delete_nodetype(
    applicator: &OperationApplicator,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    node_type_id: &str,
    op: &Operation,
) -> Result<()> {
    tracing::info!(
        "📥 Applying NodeType delete: {}/{}/{}/{} from node {}",
        tenant_id,
        repo_id,
        branch,
        node_type_id,
        op.cluster_node_id
    );

    delete_schema(
        &applicator.db,
        cf::NODE_TYPES,
        tenant_id,
        repo_id,
        branch,
        node_type_id,
        keys::nodetype_name_prefix,
    )?;

    applicator.emit_schema_event(
        tenant_id,
        repo_id,
        branch,
        node_type_id,
        "NodeType",
        SchemaEventKind::NodeTypeDeleted,
    );

    Ok(())
}

/// Apply an Archetype delete operation
pub(super) async fn apply_delete_archetype(
    applicator: &OperationApplicator,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    archetype_id: &str,
    op: &Operation,
) -> Result<()> {
    tracing::info!(
        "📥 Applying Archetype delete: {}/{}/{}/{} from node {}",
        tenant_id,
        repo_id,
        branch,
        archetype_id,
        op.cluster_node_id
    );

    delete_schema(
        &applicator.db,
        cf::ARCHETYPES,
        tenant_id,
        repo_id,
        branch,
        archetype_id,
        keys::archetype_name_prefix,
    )?;

    applicator.emit_schema_event(
        tenant_id,
        repo_id,
        branch,
        archetype_id,
        "Archetype",
        SchemaEventKind::ArchetypeDeleted,
    );

    Ok(())
}

/// Apply an ElementType delete operation
pub(super) async fn apply_delete_element_type(
    applicator: &OperationApplicator,
    tenant_id: &str,
    repo_id: &str,
    branch: &str,
    element_type_id: &str,
    op: &Operation,
) -> Result<()> {
    tracing::info!(
        "📥 Applying ElementType delete: {}/{}/{}/{} from node {}",
        tenant_id,
        repo_id,
        branch,
        element_type_id,
        op.cluster_node_id
    );

    delete_schema(
        &applicator.db,
        cf::ELEMENT_TYPES,
        tenant_id,
        repo_id,
        branch,
        element_type_id,
        keys::element_type_name_prefix,
    )?;

    applicator.emit_schema_event(
        tenant_id,
        repo_id,
        branch,
        element_type_id,
        "ElementType",
        SchemaEventKind::ElementTypeDeleted,
    );

    Ok(())
}
