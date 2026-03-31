//! Schema operations: NodeType, Archetype, ElementType CRUD

use crate::{cf, keys};
use raisin_error::Result;
use raisin_models::nodes::element::element_type::ElementType;
use raisin_models::nodes::types::archetype::Archetype;
use raisin_models::nodes::types::node_type::NodeType;
use raisin_replication::Operation;

use super::super::schema_operations::{apply_schema_update, delete_schema};
use super::OperationApplicator;

impl OperationApplicator {
    /// Apply a NodeType update operation
    ///
    /// This writes the NodeType directly to the NODE_TYPES column family.
    /// We bypass the NodeTypeRepository to avoid triggering operation capture.
    pub(in crate::replication::application) async fn apply_update_nodetype(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        node_type_id: &str,
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

        let revision = Self::op_revision(op)?;

        apply_schema_update(
            &self.db,
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

        self.emit_schema_event(
            tenant_id,
            repo_id,
            branch,
            &node_type.name,
            "NodeType",
            raisin_events::SchemaEventKind::NodeTypeUpdated,
        );

        Ok(())
    }

    /// Apply an Archetype update operation
    pub(in crate::replication::application) async fn apply_update_archetype(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        archetype_id: &str,
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

        let revision = Self::op_revision(op)?;

        apply_schema_update(
            &self.db,
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

        self.emit_schema_event(
            tenant_id,
            repo_id,
            branch,
            &archetype.name,
            "Archetype",
            raisin_events::SchemaEventKind::ArchetypeUpdated,
        );

        Ok(())
    }

    /// Apply an ElementType update operation
    pub(in crate::replication::application) async fn apply_update_element_type(
        &self,
        tenant_id: &str,
        repo_id: &str,
        branch: &str,
        element_type_id: &str,
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

        let revision = Self::op_revision(op)?;

        apply_schema_update(
            &self.db,
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

        self.emit_schema_event(
            tenant_id,
            repo_id,
            branch,
            &element_type.name,
            "ElementType",
            raisin_events::SchemaEventKind::ElementTypeUpdated,
        );

        Ok(())
    }

    /// Apply a NodeType delete operation
    pub(in crate::replication::application) async fn apply_delete_nodetype(
        &self,
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
            &self.db,
            cf::NODE_TYPES,
            tenant_id,
            repo_id,
            branch,
            node_type_id,
            keys::nodetype_name_prefix,
        )?;

        self.emit_schema_event(
            tenant_id,
            repo_id,
            branch,
            node_type_id,
            "NodeType",
            raisin_events::SchemaEventKind::NodeTypeDeleted,
        );

        Ok(())
    }

    /// Apply an Archetype delete operation
    pub(in crate::replication::application) async fn apply_delete_archetype(
        &self,
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
            &self.db,
            cf::ARCHETYPES,
            tenant_id,
            repo_id,
            branch,
            archetype_id,
            keys::archetype_name_prefix,
        )?;

        self.emit_schema_event(
            tenant_id,
            repo_id,
            branch,
            archetype_id,
            "Archetype",
            raisin_events::SchemaEventKind::ArchetypeDeleted,
        );

        Ok(())
    }

    /// Apply an ElementType delete operation
    pub(in crate::replication::application) async fn apply_delete_element_type(
        &self,
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
            &self.db,
            cf::ELEMENT_TYPES,
            tenant_id,
            repo_id,
            branch,
            element_type_id,
            keys::element_type_name_prefix,
        )?;

        self.emit_schema_event(
            tenant_id,
            repo_id,
            branch,
            element_type_id,
            "ElementType",
            raisin_events::SchemaEventKind::ElementTypeDeleted,
        );

        Ok(())
    }
}
