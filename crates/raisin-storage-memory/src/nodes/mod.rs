use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models as models;
use raisin_models::nodes::properties::PropertyValue;
use raisin_storage::{
    BranchScope, CreateNodeOptions, DeleteNodeOptions, ListOptions, NodeRepository,
    NodeWithPopulatedChildren, StorageScope, UpdateNodeOptions,
};

use crate::property_index::InMemoryPropertyIndexRepo;
use crate::reference_index::InMemoryReferenceIndexRepo;

mod basic_ops;
mod copy;
mod deep_children;
mod list_ops;
mod properties;
mod publish;
mod reorder;
mod stubs;
mod tree_ops;

/// In-memory implementation of the NodeRepository trait.
///
/// This implementation stores all nodes in a HashMap keyed by full storage path.
/// It provides fast in-memory access to nodes and is suitable for testing,
/// development, or small-scale deployments.
///
/// # Repository-First Architecture
///
/// Keys follow the format: `/{tenant_id}/repo/{repo_id}/branch/{branch}/workspace/{ws}/nodes/{id}`
///
/// # Thread Safety
///
/// All operations use async RwLock for thread-safe concurrent access.
///
/// # Performance Notes
///
/// This implementation uses `.clone()` extensively for several reasons:
/// - Data must be cloned out of `Arc<RwLock<HashMap>>` to avoid holding locks across await points
/// - String keys are cloned to avoid complex lifetime management with the HashMap
/// - Node values are cloned when returned to provide ownership to callers
///
/// For large-scale production deployments where cloning overhead is a concern,
/// consider using a persistent backend like RocksDB or PostgreSQL.
#[derive(Clone)]
pub struct InMemoryNodeRepo {
    /// Storage for nodes, keyed by full path (tenant/repo/branch/workspace/node)
    pub(crate) nodes: Arc<RwLock<HashMap<String, models::nodes::Node>>>,
    /// Property index for fast property-based lookups
    pub(crate) property_index: Arc<InMemoryPropertyIndexRepo>,
    /// Reference index for fast reference-based lookups
    pub(crate) reference_index: Arc<InMemoryReferenceIndexRepo>,
}

impl InMemoryNodeRepo {
    /// Creates a new empty in-memory node repository.
    pub fn new() -> Self {
        Self {
            nodes: Default::default(),
            property_index: Arc::new(InMemoryPropertyIndexRepo::new()),
            reference_index: Arc::new(InMemoryReferenceIndexRepo::new()),
        }
    }

    /// Creates a new in-memory node repository with a shared property index
    pub fn with_property_index(property_index: Arc<InMemoryPropertyIndexRepo>) -> Self {
        Self {
            nodes: Default::default(),
            property_index,
            reference_index: Arc::new(InMemoryReferenceIndexRepo::new()),
        }
    }

    /// Creates a new in-memory node repository with shared property and reference indexes
    pub fn with_indexes(
        property_index: Arc<InMemoryPropertyIndexRepo>,
        reference_index: Arc<InMemoryReferenceIndexRepo>,
    ) -> Self {
        Self {
            nodes: Default::default(),
            property_index,
            reference_index,
        }
    }
}

impl Default for InMemoryNodeRepo {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeRepository for InMemoryNodeRepo {
    // --- Core CRUD ---

    fn get(
        &self,
        scope: StorageScope<'_>,
        id: &str,
        _max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Option<models::nodes::Node>>> + Send {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        basic_ops::get(self, tenant_id, repo_id, branch, workspace, id)
    }

    async fn get_with_children(
        &self,
        scope: StorageScope<'_>,
        id: &str,
        max_revision: Option<&HLC>,
    ) -> Result<Option<NodeWithPopulatedChildren>> {
        let node = match self.get(scope, id, max_revision).await? {
            Some(n) => n,
            None => return Ok(None),
        };
        let children = self
            .list_children(
                scope,
                &node.path,
                ListOptions {
                    compute_has_children: false,
                    max_revision: max_revision.copied(),
                },
            )
            .await?;
        Ok(Some(NodeWithPopulatedChildren {
            node,
            children_nodes: children,
        }))
    }

    async fn create(
        &self,
        scope: StorageScope<'_>,
        node: models::nodes::Node,
        options: CreateNodeOptions,
    ) -> Result<()> {
        if self.get(scope, &node.id, None).await?.is_some() {
            return Err(raisin_error::Error::Conflict(format!(
                "Node '{}' already exists",
                node.id
            )));
        }
        if options.validate_schema {
            // TODO: Implement schema validation
        }
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        basic_ops::put(self, tenant_id, repo_id, branch, workspace, node).await
    }

    fn create_deep_node(
        &self,
        scope: StorageScope<'_>,
        path: &str,
        node: models::nodes::Node,
        parent_node_type: &str,
        _options: CreateNodeOptions,
    ) -> impl std::future::Future<Output = Result<models::nodes::Node>> + Send {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        stubs::create_deep_node(
            self,
            tenant_id,
            repo_id,
            branch,
            workspace,
            path,
            node,
            parent_node_type,
        )
    }

    async fn update(
        &self,
        scope: StorageScope<'_>,
        node: models::nodes::Node,
        options: UpdateNodeOptions,
    ) -> Result<()> {
        let existing_node = self.get(scope, &node.id, None).await?.ok_or_else(|| {
            raisin_error::Error::NotFound(format!("Node '{}' not found", node.id))
        })?;
        if !options.allow_type_change && existing_node.node_type != node.node_type {
            return Err(raisin_error::Error::Validation(format!(
                "Cannot change node type from '{}' to '{}'. Set allow_type_change=true to override",
                existing_node.node_type, node.node_type
            )));
        }
        if options.validate_schema {
            // TODO: Implement schema validation
        }
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        basic_ops::put(self, tenant_id, repo_id, branch, workspace, node).await
    }

    fn delete(
        &self,
        scope: StorageScope<'_>,
        id: &str,
        _options: DeleteNodeOptions,
    ) -> impl std::future::Future<Output = Result<bool>> + Send {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        basic_ops::delete(self, tenant_id, repo_id, branch, workspace, id)
    }

    fn has_children(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        _max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<bool>> + Send {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        basic_ops::has_children(self, tenant_id, repo_id, branch, workspace, node_id)
    }

    // --- List operations ---

    fn list_by_type(
        &self,
        scope: StorageScope<'_>,
        node_type: &str,
        options: ListOptions,
    ) -> impl std::future::Future<Output = Result<Vec<models::nodes::Node>>> + Send {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        list_ops::list_by_type(
            self, tenant_id, repo_id, branch, workspace, node_type, options,
        )
    }

    fn list_by_parent(
        &self,
        scope: StorageScope<'_>,
        parent: &str,
        options: ListOptions,
    ) -> impl std::future::Future<Output = Result<Vec<models::nodes::Node>>> + Send {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        list_ops::list_by_parent(self, tenant_id, repo_id, branch, workspace, parent, options)
    }

    fn get_by_path(
        &self,
        scope: StorageScope<'_>,
        path: &str,
        _max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Option<models::nodes::Node>>> + Send {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        basic_ops::get_by_path(self, tenant_id, repo_id, branch, workspace, path)
    }

    fn get_node_id_by_path(
        &self,
        scope: StorageScope<'_>,
        path: &str,
        _max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Option<String>>> + Send {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        async move {
            let node =
                basic_ops::get_by_path(self, tenant_id, repo_id, branch, workspace, path).await?;
            Ok(node.map(|n| n.id))
        }
    }

    fn list_all(
        &self,
        scope: StorageScope<'_>,
        options: ListOptions,
    ) -> impl std::future::Future<Output = Result<Vec<models::nodes::Node>>> + Send {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        list_ops::list_all(self, tenant_id, repo_id, branch, workspace, options)
    }

    fn count_all(
        &self,
        scope: StorageScope<'_>,
        _max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<usize>> + Send {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        basic_ops::count_all(self, tenant_id, repo_id, branch, workspace)
    }

    fn list_root(
        &self,
        scope: StorageScope<'_>,
        options: ListOptions,
    ) -> impl std::future::Future<Output = Result<Vec<models::nodes::Node>>> + Send {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        list_ops::list_root(self, tenant_id, repo_id, branch, workspace, options)
    }

    fn list_children(
        &self,
        scope: StorageScope<'_>,
        parent_path: &str,
        options: ListOptions,
    ) -> impl std::future::Future<Output = Result<Vec<models::nodes::Node>>> + Send {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        list_ops::list_children(
            self,
            tenant_id,
            repo_id,
            branch,
            workspace,
            parent_path,
            options,
        )
    }

    // --- Tree operations ---

    fn delete_by_path(
        &self,
        scope: StorageScope<'_>,
        path: &str,
        _options: DeleteNodeOptions,
    ) -> impl std::future::Future<Output = Result<bool>> + Send {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        tree_ops::delete_by_path(self, tenant_id, repo_id, branch, workspace, path)
    }

    fn move_node(
        &self,
        scope: StorageScope<'_>,
        id: &str,
        new_path: &str,
        operation_meta: Option<raisin_models::operations::OperationMeta>,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        tree_ops::move_node(
            self,
            tenant_id,
            repo_id,
            branch,
            workspace,
            id,
            new_path,
            operation_meta,
        )
    }

    fn move_node_tree(
        &self,
        scope: StorageScope<'_>,
        id: &str,
        new_path: &str,
        _operation_meta: Option<raisin_models::operations::OperationMeta>,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        stubs::move_node_tree(self, tenant_id, repo_id, branch, workspace, id, new_path)
    }

    fn rename_node(
        &self,
        scope: StorageScope<'_>,
        old_path: &str,
        new_name: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        tree_ops::rename_node(
            self, tenant_id, repo_id, branch, workspace, old_path, new_name,
        )
    }

    // --- Deep children ---

    fn deep_children_nested(
        &self,
        scope: StorageScope<'_>,
        parent_path: &str,
        max_depth: u32,
        _max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<HashMap<String, models::nodes::DeepNode>>> + Send
    {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        deep_children::deep_children_nested(
            self,
            tenant_id,
            repo_id,
            branch,
            workspace,
            parent_path,
            max_depth,
        )
    }

    fn deep_children_flat(
        &self,
        scope: StorageScope<'_>,
        parent_path: &str,
        max_depth: u32,
        _max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Vec<models::nodes::Node>>> + Send {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        deep_children::deep_children_flat(
            self,
            tenant_id,
            repo_id,
            branch,
            workspace,
            parent_path,
            max_depth,
        )
    }

    fn deep_children_array(
        &self,
        scope: StorageScope<'_>,
        parent_path: &str,
        max_depth: u32,
        _max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Vec<models::nodes::NodeWithChildren>>> + Send
    {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        deep_children::deep_children_array(
            self,
            tenant_id,
            repo_id,
            branch,
            workspace,
            parent_path,
            max_depth,
        )
    }

    // --- Reordering ---

    fn reorder_child(
        &self,
        scope: StorageScope<'_>,
        parent_path: &str,
        child_name: &str,
        new_position: usize,
        message: Option<&str>,
        actor: Option<&str>,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        reorder::reorder_child(
            self,
            tenant_id,
            repo_id,
            branch,
            workspace,
            parent_path,
            child_name,
            new_position,
            message,
            actor,
        )
    }

    fn move_child_before(
        &self,
        scope: StorageScope<'_>,
        parent_path: &str,
        child_name: &str,
        before_child_name: &str,
        message: Option<&str>,
        actor: Option<&str>,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        reorder::move_child_before(
            self,
            tenant_id,
            repo_id,
            branch,
            workspace,
            parent_path,
            child_name,
            before_child_name,
            message,
            actor,
        )
    }

    fn move_child_after(
        &self,
        scope: StorageScope<'_>,
        parent_path: &str,
        child_name: &str,
        after_child_name: &str,
        message: Option<&str>,
        actor: Option<&str>,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        reorder::move_child_after(
            self,
            tenant_id,
            repo_id,
            branch,
            workspace,
            parent_path,
            child_name,
            after_child_name,
            message,
            actor,
        )
    }

    // --- Property access ---

    fn get_property_by_path(
        &self,
        scope: StorageScope<'_>,
        node_path: &str,
        property_path: &str,
        _max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Option<PropertyValue>>> + Send {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        properties::get_property_by_path(
            self,
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_path,
            property_path,
        )
    }

    fn update_property_by_path(
        &self,
        scope: StorageScope<'_>,
        node_path: &str,
        property_path: &str,
        value: PropertyValue,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        properties::update_property_by_path(
            self,
            tenant_id,
            repo_id,
            branch,
            workspace,
            node_path,
            property_path,
            value,
        )
    }

    // --- Copy operations ---

    fn copy_node(
        &self,
        scope: StorageScope<'_>,
        source_path: &str,
        target_parent: &str,
        new_name: Option<&str>,
        operation_meta: Option<raisin_models::operations::OperationMeta>,
    ) -> impl std::future::Future<Output = Result<models::nodes::Node>> + Send {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        copy::copy_node(
            self,
            tenant_id,
            repo_id,
            branch,
            workspace,
            source_path,
            target_parent,
            new_name,
            operation_meta,
        )
    }

    fn copy_node_tree(
        &self,
        scope: StorageScope<'_>,
        source_path: &str,
        target_parent: &str,
        new_name: Option<&str>,
        operation_meta: Option<raisin_models::operations::OperationMeta>,
    ) -> impl std::future::Future<Output = Result<models::nodes::Node>> + Send {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        copy::copy_node_tree(
            self,
            tenant_id,
            repo_id,
            branch,
            workspace,
            source_path,
            target_parent,
            new_name,
            operation_meta,
        )
    }

    // --- Publish/unpublish ---

    fn publish(
        &self,
        scope: StorageScope<'_>,
        node_path: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        publish::publish(self, tenant_id, repo_id, branch, workspace, node_path)
    }

    fn publish_tree(
        &self,
        scope: StorageScope<'_>,
        node_path: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        publish::publish_tree(self, tenant_id, repo_id, branch, workspace, node_path)
    }

    fn unpublish(
        &self,
        scope: StorageScope<'_>,
        node_path: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        publish::unpublish(self, tenant_id, repo_id, branch, workspace, node_path)
    }

    fn unpublish_tree(
        &self,
        scope: StorageScope<'_>,
        node_path: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        publish::unpublish_tree(self, tenant_id, repo_id, branch, workspace, node_path)
    }

    fn get_published(
        &self,
        scope: StorageScope<'_>,
        id: &str,
    ) -> impl std::future::Future<Output = Result<Option<models::nodes::Node>>> + Send {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        publish::get_published(self, tenant_id, repo_id, branch, workspace, id)
    }

    fn get_published_by_path(
        &self,
        scope: StorageScope<'_>,
        path: &str,
    ) -> impl std::future::Future<Output = Result<Option<models::nodes::Node>>> + Send {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        publish::get_published_by_path(self, tenant_id, repo_id, branch, workspace, path)
    }

    fn list_published_children(
        &self,
        scope: StorageScope<'_>,
        parent_path: &str,
    ) -> impl std::future::Future<Output = Result<Vec<models::nodes::Node>>> + Send {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        publish::list_published_children(self, tenant_id, repo_id, branch, workspace, parent_path)
    }

    fn list_published_root(
        &self,
        scope: StorageScope<'_>,
    ) -> impl std::future::Future<Output = Result<Vec<models::nodes::Node>>> + Send {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        publish::list_published_root(self, tenant_id, repo_id, branch, workspace)
    }

    // --- Stub/placeholder methods ---

    fn scan_by_path_prefix(
        &self,
        scope: StorageScope<'_>,
        path_prefix: &str,
        options: ListOptions,
    ) -> impl std::future::Future<Output = Result<Vec<models::nodes::Node>>> + Send {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        stubs::scan_by_path_prefix(
            self,
            tenant_id,
            repo_id,
            branch,
            workspace,
            path_prefix,
            options,
        )
    }

    fn scan_descendants_ordered(
        &self,
        scope: StorageScope<'_>,
        parent_node_id: &str,
        options: ListOptions,
    ) -> impl std::future::Future<Output = Result<Vec<models::nodes::Node>>> + Send {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        stubs::scan_descendants_ordered(
            self,
            tenant_id,
            repo_id,
            branch,
            workspace,
            parent_node_id,
            options,
        )
    }

    fn get_descendants_bulk(
        &self,
        scope: StorageScope<'_>,
        parent_path: &str,
        max_depth: u32,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<HashMap<String, models::nodes::Node>>> + Send
    {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        stubs::get_descendants_bulk(
            self,
            tenant_id,
            repo_id,
            branch,
            workspace,
            parent_path,
            max_depth,
            max_revision,
        )
    }

    fn validate_parent_allows_child(
        &self,
        scope: BranchScope<'_>,
        parent_node_type: &str,
        child_node_type: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        let BranchScope {
            tenant_id,
            repo_id,
            branch,
        } = scope;
        stubs::validate_parent_allows_child(
            self,
            tenant_id,
            repo_id,
            branch,
            parent_node_type,
            child_node_type,
        )
    }

    fn validate_workspace_allows_node_type(
        &self,
        scope: StorageScope<'_>,
        node_type: &str,
        is_root_node: bool,
    ) -> impl std::future::Future<Output = Result<()>> + Send {
        let StorageScope {
            tenant_id,
            repo_id,
            workspace,
            ..
        } = scope;
        stubs::validate_workspace_allows_node_type(
            self,
            tenant_id,
            repo_id,
            workspace,
            node_type,
            is_root_node,
        )
    }

    fn stream_ordered_child_ids(
        &self,
        scope: StorageScope<'_>,
        parent_id: &str,
        max_revision: Option<&raisin_hlc::HLC>,
    ) -> impl std::future::Future<Output = Result<Vec<String>>> + Send {
        let StorageScope {
            tenant_id,
            repo_id,
            branch,
            workspace,
        } = scope;
        stubs::stream_ordered_child_ids(
            self,
            tenant_id,
            repo_id,
            branch,
            workspace,
            parent_id,
            max_revision,
        )
    }
}
