//! Property index operations (custom and system properties)

use super::super::super::helpers::hash_property_value;
use super::super::super::NodeRepositoryImpl;
use crate::{cf, cf_handle, keys};
use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models::nodes::Node;
use rocksdb::WriteBatch;

impl NodeRepositoryImpl {
    /// Add property indexes for all node properties
    pub(in crate::repositories) fn add_property_indexes(
        &self,
        batch: &mut WriteBatch,
        node: &Node,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        revision: &HLC,
    ) -> Result<()> {
        let cf_property = cf_handle(&self.db, cf::PROPERTY_INDEX)?;
        let is_published = node.published_at.is_some();

        for (prop_name, prop_value) in &node.properties {
            let value_hash = hash_property_value(prop_value);
            let prop_key = keys::property_index_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                prop_name,
                &value_hash,
                revision,
                &node.id,
                is_published,
            );
            batch.put_cf(cf_property, prop_key, node.id.as_bytes());
        }

        Ok(())
    }

    /// Add system property indexes (__name, __node_type, __archetype, etc.)
    pub(in crate::repositories) fn add_system_property_indexes(
        &self,
        batch: &mut WriteBatch,
        node: &Node,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        workspace: &str,
        revision: &HLC,
    ) -> Result<()> {
        let cf_property = cf_handle(&self.db, cf::PROPERTY_INDEX)?;
        let is_published = node.published_at.is_some();

        // Index node_type as a pseudo-property for efficient list_by_type queries
        let node_type_key = keys::property_index_key_versioned(
            tenant_id,
            repo_id,
            branch,
            workspace,
            "__node_type",
            &node.node_type,
            revision,
            &node.id,
            is_published,
        );
        batch.put_cf(cf_property, node_type_key, node.id.as_bytes());

        // Index: name
        if !node.name.is_empty() {
            let name_key = keys::property_index_key_versioned(
                tenant_id,
                repo_id,
                branch,
                workspace,
                "__name",
                &node.name,
                revision,
                &node.id,
                is_published,
            );
            batch.put_cf(cf_property, name_key, node.id.as_bytes());
        }

        // Index: archetype
        if let Some(ref archetype) = node.archetype {
            if !archetype.is_empty() {
                let archetype_key = keys::property_index_key_versioned(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    "__archetype",
                    archetype,
                    revision,
                    &node.id,
                    is_published,
                );
                batch.put_cf(cf_property, archetype_key, node.id.as_bytes());
            }
        }

        // Index: created_by
        if let Some(ref created_by) = node.created_by {
            if !created_by.is_empty() {
                let created_by_key = keys::property_index_key_versioned(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    "__created_by",
                    created_by,
                    revision,
                    &node.id,
                    is_published,
                );
                batch.put_cf(cf_property, created_by_key, node.id.as_bytes());
            }
        }

        // Index: updated_by
        if let Some(ref updated_by) = node.updated_by {
            if !updated_by.is_empty() {
                let updated_by_key = keys::property_index_key_versioned(
                    tenant_id,
                    repo_id,
                    branch,
                    workspace,
                    "__updated_by",
                    updated_by,
                    revision,
                    &node.id,
                    is_published,
                );
                batch.put_cf(cf_property, updated_by_key, node.id.as_bytes());
            }
        }

        // Index: created_at (as i64 microseconds for efficient range queries)
        if let Some(created_at) = node.created_at {
            let created_at_key = keys::property_index_key_versioned_timestamp(
                tenant_id,
                repo_id,
                branch,
                workspace,
                "__created_at",
                created_at.timestamp_micros(),
                revision,
                &node.id,
                is_published,
            );
            batch.put_cf(cf_property, created_at_key, node.id.as_bytes());
        }

        // Index: updated_at (as i64 microseconds for efficient range queries)
        if let Some(updated_at) = node.updated_at {
            let updated_at_key = keys::property_index_key_versioned_timestamp(
                tenant_id,
                repo_id,
                branch,
                workspace,
                "__updated_at",
                updated_at.timestamp_micros(),
                revision,
                &node.id,
                is_published,
            );
            batch.put_cf(cf_property, updated_at_key, node.id.as_bytes());
        }

        Ok(())
    }
}
