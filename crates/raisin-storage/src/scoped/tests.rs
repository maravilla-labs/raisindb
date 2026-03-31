use super::*;
use raisin_models as models;

// Mock storage for testing
struct MockStorage;

use crate::Transaction;

impl Storage for MockStorage {
    type Tx = MockTx;
    type Nodes = MockNodeRepo;
    type NodeTypes = MockNodeTypeRepo;
    type Workspaces = MockWorkspaceRepo;
    type Registry = MockRegistryRepo;
    type PropertyIndex = MockPropertyIndexRepo;
    type ReferenceIndex = MockReferenceIndexRepo;
    type Versioning = MockVersioningRepo;

    fn nodes(&self) -> &Self::Nodes {
        unimplemented!()
    }
    fn node_types(&self) -> &Self::NodeTypes {
        unimplemented!()
    }
    fn workspaces(&self) -> &Self::Workspaces {
        unimplemented!()
    }
    fn registry(&self) -> &Self::Registry {
        unimplemented!()
    }
    fn property_index(&self) -> &Self::PropertyIndex {
        unimplemented!()
    }
    fn reference_index(&self) -> &Self::ReferenceIndex {
        unimplemented!()
    }
    fn versioning(&self) -> &Self::Versioning {
        unimplemented!()
    }
    async fn begin(&self) -> Result<Self::Tx> {
        unimplemented!()
    }
}

struct MockTx;
impl Transaction for MockTx {
    async fn commit(self: Box<Self>) -> Result<()> {
        Ok(())
    }
    async fn rollback(self: Box<Self>) -> Result<()> {
        Ok(())
    }
}

struct MockNodeRepo;
impl crate::NodeRepository for MockNodeRepo {
    async fn get(&self, _workspace: &str, _id: &str) -> Result<Option<models::nodes::Node>> {
        unimplemented!()
    }
    async fn get_by_path(
        &self,
        _workspace: &str,
        _path: &str,
    ) -> Result<Option<models::nodes::Node>> {
        unimplemented!()
    }
    async fn put(&self, _workspace: &str, _node: models::nodes::Node) -> Result<()> {
        unimplemented!()
    }
    async fn delete(&self, _workspace: &str, _id: &str) -> Result<bool> {
        unimplemented!()
    }
    async fn list_children(
        &self,
        _workspace: &str,
        _parent_path: &str,
    ) -> Result<Vec<models::nodes::Node>> {
        unimplemented!()
    }
    async fn list_root(&self, _workspace: &str) -> Result<Vec<models::nodes::Node>> {
        unimplemented!()
    }
    async fn publish(&self, _workspace: &str, _node_path: &str) -> Result<()> {
        unimplemented!()
    }
    async fn unpublish(&self, _workspace: &str, _node_path: &str) -> Result<()> {
        unimplemented!()
    }
    async fn publish_tree(&self, _workspace: &str, _node_path: &str) -> Result<()> {
        unimplemented!()
    }
    async fn unpublish_tree(&self, _workspace: &str, _node_path: &str) -> Result<()> {
        unimplemented!()
    }
    async fn get_published(
        &self,
        _workspace: &str,
        _id: &str,
    ) -> Result<Option<models::nodes::Node>> {
        unimplemented!()
    }
    async fn get_published_by_path(
        &self,
        _workspace: &str,
        _path: &str,
    ) -> Result<Option<models::nodes::Node>> {
        unimplemented!()
    }
    async fn list_published_children(
        &self,
        _workspace: &str,
        _parent_path: &str,
    ) -> Result<Vec<models::nodes::Node>> {
        unimplemented!()
    }
    async fn list_published_root(&self, _workspace: &str) -> Result<Vec<models::nodes::Node>> {
        unimplemented!()
    }
    async fn list_by_type(
        &self,
        _workspace: &str,
        _node_type: &str,
    ) -> Result<Vec<models::nodes::Node>> {
        unimplemented!()
    }
    async fn list_by_parent(
        &self,
        _workspace: &str,
        _parent: &str,
    ) -> Result<Vec<models::nodes::Node>> {
        unimplemented!()
    }
    async fn list_all(&self, _workspace: &str) -> Result<Vec<models::nodes::Node>> {
        unimplemented!()
    }
    async fn delete_by_path(&self, _workspace: &str, _path: &str) -> Result<bool> {
        unimplemented!()
    }
    async fn move_node(&self, _workspace: &str, _id: &str, _new_path: &str) -> Result<()> {
        unimplemented!()
    }
    async fn rename_node(
        &self,
        _workspace: &str,
        _old_path: &str,
        _new_name: &str,
    ) -> Result<()> {
        unimplemented!()
    }
    async fn deep_children_nested(
        &self,
        _workspace: &str,
        _parent_path: &str,
        _max_depth: u32,
    ) -> Result<std::collections::HashMap<String, models::nodes::DeepNode>> {
        unimplemented!()
    }
    async fn deep_children_flat(
        &self,
        _workspace: &str,
        _parent_path: &str,
        _max_depth: u32,
    ) -> Result<Vec<models::nodes::Node>> {
        unimplemented!()
    }
    async fn deep_children_array(
        &self,
        _workspace: &str,
        _parent_path: &str,
        _max_depth: u32,
    ) -> Result<Vec<models::nodes::NodeWithChildren>> {
        unimplemented!()
    }
    async fn reorder_child(
        &self,
        _workspace: &str,
        _parent_path: &str,
        _child_name: &str,
        _new_position: usize,
    ) -> Result<()> {
        unimplemented!()
    }
    async fn move_child_before(
        &self,
        _workspace: &str,
        _parent_path: &str,
        _child_name: &str,
        _before_child_name: &str,
    ) -> Result<()> {
        unimplemented!()
    }
    async fn move_child_after(
        &self,
        _workspace: &str,
        _parent_path: &str,
        _child_name: &str,
        _after_child_name: &str,
    ) -> Result<()> {
        unimplemented!()
    }
    async fn get_property_by_path(
        &self,
        _workspace: &str,
        _node_path: &str,
        _property_path: &str,
    ) -> Result<Option<models::nodes::properties::PropertyValue>> {
        unimplemented!()
    }
    async fn update_property_by_path(
        &self,
        _workspace: &str,
        _node_path: &str,
        _property_path: &str,
        _value: models::nodes::properties::PropertyValue,
    ) -> Result<()> {
        unimplemented!()
    }
    async fn copy_node(
        &self,
        _workspace: &str,
        _source_path: &str,
        _target_parent: &str,
        _new_name: Option<&str>,
    ) -> Result<models::nodes::Node> {
        unimplemented!()
    }
    async fn copy_node_tree(
        &self,
        _workspace: &str,
        _source_path: &str,
        _target_parent: &str,
        _new_name: Option<&str>,
    ) -> Result<models::nodes::Node> {
        unimplemented!()
    }
}

struct MockNodeTypeRepo;
impl crate::NodeTypeRepository for MockNodeTypeRepo {
    async fn get(&self, _name: &str) -> Result<Option<models::nodes::types::NodeType>> {
        unimplemented!()
    }
    async fn get_by_id(&self, _id: &str) -> Result<Option<models::nodes::types::NodeType>> {
        unimplemented!()
    }
    async fn get_by_names(
        &self,
        _names: &[String],
    ) -> Result<Vec<models::nodes::types::NodeType>> {
        unimplemented!()
    }
    async fn put(&self, _node_type: models::nodes::types::NodeType) -> Result<()> {
        unimplemented!()
    }
    async fn delete(&self, _name: &str) -> Result<bool> {
        unimplemented!()
    }
    async fn list(&self) -> Result<Vec<models::nodes::types::NodeType>> {
        unimplemented!()
    }
    async fn list_published(&self) -> Result<Vec<models::nodes::types::NodeType>> {
        unimplemented!()
    }
    async fn publish(&self, _name: &str) -> Result<()> {
        unimplemented!()
    }
    async fn unpublish(&self, _name: &str) -> Result<()> {
        unimplemented!()
    }
    async fn is_published(&self, _name: &str) -> Result<bool> {
        unimplemented!()
    }
    async fn validate_published(&self, _node_type_name: &str) -> Result<()> {
        unimplemented!()
    }
}

struct MockWorkspaceRepo;
impl crate::WorkspaceRepository for MockWorkspaceRepo {
    async fn get(&self, _id: &str) -> Result<Option<models::workspace::Workspace>> {
        unimplemented!()
    }
    async fn put(&self, _ws: models::workspace::Workspace) -> Result<()> {
        unimplemented!()
    }
    async fn list(&self) -> Result<Vec<models::workspace::Workspace>> {
        unimplemented!()
    }
    async fn get_root_children(&self, _id: &str) -> Result<Option<Vec<String>>> {
        unimplemented!()
    }
    async fn set_root_children(&self, _id: &str, _order: Vec<String>) -> Result<()> {
        unimplemented!()
    }
}

struct MockRegistryRepo;
impl crate::RegistryRepository for MockRegistryRepo {
    async fn register_tenant(
        &self,
        _tenant_id: &str,
        _metadata: std::collections::HashMap<String, String>,
    ) -> Result<()> {
        unimplemented!()
    }
    async fn get_tenant(
        &self,
        _tenant_id: &str,
    ) -> Result<Option<models::registry::TenantRegistration>> {
        unimplemented!()
    }
    async fn list_tenants(&self) -> Result<Vec<models::registry::TenantRegistration>> {
        unimplemented!()
    }
    async fn update_tenant_last_seen(&self, _tenant_id: &str) -> Result<()> {
        unimplemented!()
    }
    async fn register_deployment(&self, _tenant_id: &str, _deployment_key: &str) -> Result<()> {
        unimplemented!()
    }
    async fn get_deployment(
        &self,
        _tenant_id: &str,
        _deployment_key: &str,
    ) -> Result<Option<models::registry::DeploymentRegistration>> {
        unimplemented!()
    }
    async fn list_deployments(
        &self,
        _tenant_id: Option<&str>,
    ) -> Result<Vec<models::registry::DeploymentRegistration>> {
        unimplemented!()
    }
    async fn update_deployment_nodetype_version(
        &self,
        _tenant_id: &str,
        _deployment_key: &str,
        _version: &str,
    ) -> Result<()> {
        unimplemented!()
    }
    async fn update_deployment_last_seen(
        &self,
        _tenant_id: &str,
        _deployment_key: &str,
    ) -> Result<()> {
        unimplemented!()
    }
}

struct MockPropertyIndexRepo;
impl crate::PropertyIndexRepository for MockPropertyIndexRepo {
    async fn index_properties(
        &self,
        _workspace: &str,
        _node_id: &str,
        _properties: &std::collections::HashMap<String, models::nodes::properties::PropertyValue>,
        _is_published: bool,
    ) -> Result<()> {
        unimplemented!()
    }

    async fn unindex_properties(&self, _workspace: &str, _node_id: &str) -> Result<()> {
        unimplemented!()
    }

    async fn update_publish_status(
        &self,
        _workspace: &str,
        _node_id: &str,
        _properties: &std::collections::HashMap<String, models::nodes::properties::PropertyValue>,
        _is_published: bool,
    ) -> Result<()> {
        unimplemented!()
    }

    async fn find_by_property(
        &self,
        _workspace: &str,
        _property_name: &str,
        _property_value: &models::nodes::properties::PropertyValue,
        _published_only: bool,
    ) -> Result<Vec<String>> {
        unimplemented!()
    }

    async fn find_nodes_with_property(
        &self,
        _workspace: &str,
        _property_name: &str,
        _published_only: bool,
    ) -> Result<Vec<String>> {
        unimplemented!()
    }
}

struct MockReferenceIndexRepo;
impl crate::ReferenceIndexRepository for MockReferenceIndexRepo {
    async fn index_references(
        &self,
        _workspace: &str,
        _node_id: &str,
        _properties: &std::collections::HashMap<String, models::nodes::properties::PropertyValue>,
        _is_published: bool,
    ) -> Result<()> {
        unimplemented!()
    }

    async fn unindex_references(&self, _workspace: &str, _node_id: &str) -> Result<()> {
        unimplemented!()
    }

    async fn update_reference_publish_status(
        &self,
        _workspace: &str,
        _node_id: &str,
        _properties: &std::collections::HashMap<String, models::nodes::properties::PropertyValue>,
        _is_published: bool,
    ) -> Result<()> {
        unimplemented!()
    }

    async fn find_referencing_nodes(
        &self,
        _workspace: &str,
        _target_workspace: &str,
        _target_path: &str,
        _published_only: bool,
    ) -> Result<Vec<(String, String)>> {
        unimplemented!()
    }

    async fn get_node_references(
        &self,
        _workspace: &str,
        _node_id: &str,
        _published_only: bool,
    ) -> Result<Vec<(String, models::nodes::properties::RaisinReference)>> {
        unimplemented!()
    }

    async fn get_unique_references(
        &self,
        _workspace: &str,
        _node_id: &str,
        _published_only: bool,
    ) -> Result<std::collections::HashMap<String, (Vec<String>, models::nodes::properties::RaisinReference)>> {
        unimplemented!()
    }
}

struct MockVersioningRepo;
impl crate::VersioningRepository for MockVersioningRepo {
    async fn create_version(&self, _node: &models::nodes::Node) -> Result<i32> {
        unimplemented!()
    }

    async fn list_versions(&self, _node_id: &str) -> Result<Vec<models::nodes::NodeVersion>> {
        unimplemented!()
    }

    async fn get_version(
        &self,
        _node_id: &str,
        _version: i32,
    ) -> Result<Option<models::nodes::NodeVersion>> {
        unimplemented!()
    }

    async fn delete_all_versions(&self, _node_id: &str) -> Result<usize> {
        unimplemented!()
    }
}

#[test]
fn test_scoped_storage_creation() {
    let storage = Arc::new(MockStorage);
    let ctx = TenantContext::new("test-tenant", "dev");

    let scoped = ScopedStorage::new(storage.clone(), ctx.clone());
    assert_eq!(scoped.context().tenant_id(), "test-tenant");
    assert_eq!(scoped.context().deployment(), "dev");
}

#[test]
fn test_storage_ext() {
    let storage = Arc::new(MockStorage);
    let ctx = TenantContext::new("test-tenant", "prod");

    let scoped = storage.scoped(ctx);
    assert_eq!(scoped.context().tenant_id(), "test-tenant");
}

#[test]
fn test_with_isolation_single() {
    let storage = Arc::new(MockStorage);
    let scoped = storage.with_isolation(IsolationMode::Single);
    assert!(scoped.is_none());
}

#[test]
fn test_with_isolation_shared() {
    let storage = Arc::new(MockStorage);
    let ctx = TenantContext::new("tenant", "preview");
    let scoped = storage.with_isolation(IsolationMode::Shared(ctx));
    assert!(scoped.is_some());
}
