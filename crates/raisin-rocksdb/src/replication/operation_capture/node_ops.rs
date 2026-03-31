//! Convenience methods for node-level operation capture
//!
//! These are thin wrappers around `capture_operation` that construct the
//! appropriate `OpType` variants for each node operation.

use raisin_error::Result;
use raisin_replication::{OpType, Operation};

use super::core::OperationCapture;

impl OperationCapture {
    /// Capture a CreateNode operation with full node data
    pub async fn capture_create_node(
        &self,
        tenant_id: String,
        repo_id: String,
        branch: String,
        node_id: String,
        name: String,
        node_type: String,
        archetype: Option<String>,
        parent_id: Option<String>,
        order_key: String,
        properties: serde_json::Value,
        owner_id: Option<String>,
        workspace: Option<String>,
        path: String,
        actor: String,
    ) -> Result<Operation> {
        let properties = serde_json::from_value(properties).unwrap_or_default();
        self.capture_operation(
            tenant_id,
            repo_id,
            branch,
            OpType::CreateNode {
                node_id,
                name,
                node_type,
                archetype,
                parent_id,
                order_key,
                properties,
                owner_id,
                workspace,
                path,
            },
            actor,
            None,
            false,
        )
        .await
    }

    /// Capture a DeleteNode operation
    pub async fn capture_delete_node(
        &self,
        tenant_id: String,
        repo_id: String,
        branch: String,
        node_id: String,
        actor: String,
    ) -> Result<Operation> {
        self.capture_operation(
            tenant_id,
            repo_id,
            branch,
            OpType::DeleteNode { node_id },
            actor,
            None,
            false,
        )
        .await
    }

    /// Capture a RenameNode operation
    pub async fn capture_rename_node(
        &self,
        tenant_id: String,
        repo_id: String,
        branch: String,
        node_id: String,
        old_name: String,
        new_name: String,
        actor: String,
    ) -> Result<Operation> {
        self.capture_operation(
            tenant_id,
            repo_id,
            branch,
            OpType::RenameNode {
                node_id,
                old_name,
                new_name,
            },
            actor,
            None,
            false,
        )
        .await
    }

    /// Capture a MoveNode operation
    pub async fn capture_move_node(
        &self,
        tenant_id: String,
        repo_id: String,
        branch: String,
        node_id: String,
        old_parent_id: Option<String>,
        new_parent_id: Option<String>,
        position: Option<String>,
        actor: String,
    ) -> Result<Operation> {
        self.capture_operation(
            tenant_id,
            repo_id,
            branch,
            OpType::MoveNode {
                node_id,
                old_parent_id,
                new_parent_id,
                position,
            },
            actor,
            None,
            false,
        )
        .await
    }

    /// Capture a SetArchetype operation
    pub async fn capture_set_archetype(
        &self,
        tenant_id: String,
        repo_id: String,
        branch: String,
        node_id: String,
        old_archetype: Option<String>,
        new_archetype: Option<String>,
        actor: String,
    ) -> Result<Operation> {
        self.capture_operation(
            tenant_id,
            repo_id,
            branch,
            OpType::SetArchetype {
                node_id,
                old_archetype,
                new_archetype,
            },
            actor,
            None,
            false,
        )
        .await
    }

    /// Capture a SetOrderKey operation
    pub async fn capture_set_order_key(
        &self,
        tenant_id: String,
        repo_id: String,
        branch: String,
        node_id: String,
        old_order_key: String,
        new_order_key: String,
        actor: String,
    ) -> Result<Operation> {
        self.capture_operation(
            tenant_id,
            repo_id,
            branch,
            OpType::SetOrderKey {
                node_id,
                old_order_key,
                new_order_key,
            },
            actor,
            None,
            false,
        )
        .await
    }

    /// Capture a SetOwner operation
    pub async fn capture_set_owner(
        &self,
        tenant_id: String,
        repo_id: String,
        branch: String,
        node_id: String,
        old_owner_id: Option<String>,
        new_owner_id: Option<String>,
        actor: String,
    ) -> Result<Operation> {
        self.capture_operation(
            tenant_id,
            repo_id,
            branch,
            OpType::SetOwner {
                node_id,
                old_owner_id,
                new_owner_id,
            },
            actor,
            None,
            false,
        )
        .await
    }

    /// Capture a PublishNode operation
    pub async fn capture_publish_node(
        &self,
        tenant_id: String,
        repo_id: String,
        branch: String,
        node_id: String,
        published_by: String,
        actor: String,
    ) -> Result<Operation> {
        let published_at = chrono::Utc::now().timestamp_millis() as u64;
        self.capture_operation(
            tenant_id,
            repo_id,
            branch,
            OpType::PublishNode {
                node_id,
                published_by,
                published_at,
            },
            actor,
            None,
            false,
        )
        .await
    }

    /// Capture an UnpublishNode operation
    pub async fn capture_unpublish_node(
        &self,
        tenant_id: String,
        repo_id: String,
        branch: String,
        node_id: String,
        actor: String,
    ) -> Result<Operation> {
        self.capture_operation(
            tenant_id,
            repo_id,
            branch,
            OpType::UnpublishNode { node_id },
            actor,
            None,
            false,
        )
        .await
    }

    /// Capture an AddRelation operation.
    /// Uses Last-Write-Wins (LWW) semantics based on HLC timestamps.
    pub async fn capture_add_relation(
        &self,
        tenant_id: String,
        repo_id: String,
        branch: String,
        source_id: String,
        source_workspace: String,
        relation_type: String,
        target_id: String,
        target_workspace: String,
        properties: serde_json::Value,
        actor: String,
    ) -> Result<Operation> {
        let props: std::collections::HashMap<String, serde_json::Value> =
            serde_json::from_value(properties).unwrap_or_default();

        let target_node_type = props
            .get("target_node_type")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let weight = props
            .get("weight")
            .and_then(|v| v.as_f64())
            .map(|w| w as f32);

        let relation = raisin_models::nodes::RelationRef::new(
            target_id,
            target_workspace,
            target_node_type,
            relation_type.clone(),
            weight,
        );

        self.capture_operation(
            tenant_id,
            repo_id,
            branch,
            OpType::AddRelation {
                source_id,
                source_workspace,
                relation_type,
                target_id: relation.target.clone(),
                target_workspace: relation.workspace.clone(),
                relation,
            },
            actor,
            None,
            false,
        )
        .await
    }

    /// Capture a RemoveRelation operation.
    /// Uses Last-Write-Wins (LWW) semantics based on HLC timestamps.
    pub async fn capture_remove_relation(
        &self,
        tenant_id: String,
        repo_id: String,
        branch: String,
        source_id: String,
        source_workspace: String,
        relation_type: String,
        target_id: String,
        target_workspace: String,
        actor: String,
    ) -> Result<Operation> {
        self.capture_operation(
            tenant_id,
            repo_id,
            branch,
            OpType::RemoveRelation {
                source_id,
                source_workspace,
                relation_type,
                target_id,
                target_workspace,
            },
            actor,
            None,
            false,
        )
        .await
    }

    /// Capture a SetTranslation operation
    pub async fn capture_set_translation(
        &self,
        tenant_id: String,
        repo_id: String,
        branch: String,
        node_id: String,
        locale: String,
        property_name: String,
        value: serde_json::Value,
        actor: String,
    ) -> Result<Operation> {
        let value = serde_json::from_value(value).unwrap_or(
            raisin_models::nodes::properties::PropertyValue::String(String::new()),
        );
        self.capture_operation(
            tenant_id,
            repo_id,
            branch,
            OpType::SetTranslation {
                node_id,
                locale,
                property_name,
                value,
            },
            actor,
            None,
            false,
        )
        .await
    }

    /// Capture a DeleteTranslation operation
    pub async fn capture_delete_translation(
        &self,
        tenant_id: String,
        repo_id: String,
        branch: String,
        node_id: String,
        locale: String,
        property_name: String,
        actor: String,
    ) -> Result<Operation> {
        self.capture_operation(
            tenant_id,
            repo_id,
            branch,
            OpType::DeleteTranslation {
                node_id,
                locale,
                property_name,
            },
            actor,
            None,
            false,
        )
        .await
    }
}
