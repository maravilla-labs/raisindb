//! Convenience methods for schema and registry operation capture
//!
//! These are thin wrappers around `capture_operation_with_revision` for
//! NodeType, Archetype, ElementType, User, Tenant, Deployment, and Repository operations.

use raisin_error::Result;
use raisin_replication::{OpType, Operation};

use super::core::OperationCapture;

impl OperationCapture {
    /// Capture an UpdateNodeType operation
    pub async fn capture_upsert_nodetype(
        &self,
        tenant_id: String,
        repo_id: String,
        branch: String,
        node_type_id: String,
        node_type: raisin_models::nodes::types::node_type::NodeType,
        actor: String,
        revision: raisin_hlc::HLC,
    ) -> Result<Operation> {
        self.capture_operation_with_revision(
            tenant_id,
            repo_id,
            branch,
            OpType::UpdateNodeType {
                node_type_id,
                node_type,
            },
            actor,
            None,
            false,
            Some(revision),
        )
        .await
    }

    /// Capture a DeleteNodeType operation
    pub async fn capture_delete_nodetype(
        &self,
        tenant_id: String,
        repo_id: String,
        branch: String,
        node_type_id: String,
        actor: String,
        revision: raisin_hlc::HLC,
    ) -> Result<Operation> {
        self.capture_operation_with_revision(
            tenant_id,
            repo_id,
            branch,
            OpType::DeleteNodeType { node_type_id },
            actor,
            None,
            false,
            Some(revision),
        )
        .await
    }

    /// Capture an UpdateArchetype operation
    pub async fn capture_upsert_archetype(
        &self,
        tenant_id: String,
        repo_id: String,
        branch: String,
        archetype_id: String,
        archetype: raisin_models::nodes::types::archetype::Archetype,
        actor: String,
        revision: raisin_hlc::HLC,
    ) -> Result<Operation> {
        self.capture_operation_with_revision(
            tenant_id,
            repo_id,
            branch,
            OpType::UpdateArchetype {
                archetype_id,
                archetype,
            },
            actor,
            None,
            false,
            Some(revision),
        )
        .await
    }

    /// Capture a DeleteArchetype operation
    pub async fn capture_delete_archetype(
        &self,
        tenant_id: String,
        repo_id: String,
        branch: String,
        archetype_id: String,
        actor: String,
        revision: raisin_hlc::HLC,
    ) -> Result<Operation> {
        self.capture_operation_with_revision(
            tenant_id,
            repo_id,
            branch,
            OpType::DeleteArchetype { archetype_id },
            actor,
            None,
            false,
            Some(revision),
        )
        .await
    }

    /// Capture an UpdateElementType operation
    pub async fn capture_upsert_element_type(
        &self,
        tenant_id: String,
        repo_id: String,
        branch: String,
        element_type_id: String,
        element_type: raisin_models::nodes::element::element_type::ElementType,
        actor: String,
        revision: raisin_hlc::HLC,
    ) -> Result<Operation> {
        self.capture_operation_with_revision(
            tenant_id,
            repo_id,
            branch,
            OpType::UpdateElementType {
                element_type_id,
                element_type,
            },
            actor,
            None,
            false,
            Some(revision),
        )
        .await
    }

    /// Capture a DeleteElementType operation
    pub async fn capture_delete_element_type(
        &self,
        tenant_id: String,
        repo_id: String,
        branch: String,
        element_type_id: String,
        actor: String,
        revision: raisin_hlc::HLC,
    ) -> Result<Operation> {
        self.capture_operation_with_revision(
            tenant_id,
            repo_id,
            branch,
            OpType::DeleteElementType { element_type_id },
            actor,
            None,
            false,
            Some(revision),
        )
        .await
    }

    /// Capture an UpdateUser operation
    pub async fn capture_update_user(
        &self,
        tenant_id: String,
        repo_id: String,
        branch: String,
        user_id: String,
        data: serde_json::Value,
        actor: String,
    ) -> Result<Operation> {
        let user: raisin_models::admin_user::DatabaseAdminUser =
            serde_json::from_value(data).map_err(|e| anyhow::anyhow!(e))?;
        self.capture_operation(
            tenant_id,
            repo_id,
            branch,
            OpType::UpdateUser { user_id, user },
            actor,
            None,
            false,
        )
        .await
    }

    /// Capture a DeleteUser operation
    pub async fn capture_delete_user(
        &self,
        tenant_id: String,
        repo_id: String,
        branch: String,
        user_id: String,
        actor: String,
    ) -> Result<Operation> {
        self.capture_operation(
            tenant_id,
            repo_id,
            branch,
            OpType::DeleteUser { user_id },
            actor,
            None,
            false,
        )
        .await
    }

    /// Capture a tenant registration/update operation
    ///
    /// Note: Tenant operations don't have repo/branch context, so we use special values
    pub async fn capture_update_tenant(
        &self,
        tenant_id: String,
        tenant: raisin_models::registry::TenantRegistration,
        actor: String,
    ) -> Result<Operation> {
        self.capture_operation(
            tenant_id.clone(),
            "_registry".to_string(),
            "main".to_string(),
            OpType::UpdateTenant { tenant_id, tenant },
            actor,
            None,
            true,
        )
        .await
    }

    /// Capture a deployment registration/update operation
    pub async fn capture_update_deployment(
        &self,
        tenant_id: String,
        deployment_id: String,
        deployment: raisin_models::registry::DeploymentRegistration,
        actor: String,
    ) -> Result<Operation> {
        self.capture_operation(
            tenant_id,
            "_registry".to_string(),
            "main".to_string(),
            OpType::UpdateDeployment {
                deployment_id,
                deployment,
            },
            actor,
            None,
            true,
        )
        .await
    }

    /// Capture a repository creation/update operation
    pub async fn capture_update_repository(
        &self,
        tenant_id: String,
        repo_id: String,
        repository: raisin_context::RepositoryInfo,
        actor: String,
    ) -> Result<Operation> {
        self.capture_operation(
            tenant_id.clone(),
            repo_id.clone(),
            "main".to_string(),
            OpType::UpdateRepository {
                tenant_id,
                repo_id,
                repository,
            },
            actor,
            None,
            true,
        )
        .await
    }
}
