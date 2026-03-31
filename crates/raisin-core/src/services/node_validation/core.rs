//! Core NodeValidator struct definition and constructors.
//!
//! The `NodeValidator` validates nodes against their NodeType schemas,
//! including required properties, strict mode, unique constraints,
//! archetype associations, and element type validation.

use raisin_error::Result;
use raisin_indexer::IndexManager;
use raisin_models::nodes::Node;
use raisin_storage::{scope::BranchScope, NodeTypeRepository, Storage};
use std::collections::HashMap;
use std::sync::Arc;

use crate::services::archetype_resolver::ArchetypeResolver;
use crate::services::element_type_resolver::{ElementTypeResolver, ResolvedElementType};
use crate::services::node_type_resolver::{NodeTypeResolver, ResolvedNodeType};

/// Validates nodes against their NodeType, Archetype, and ElementType schemas.
#[derive(Clone)]
pub struct NodeValidator<S: Storage> {
    pub(super) storage: Arc<S>,
    pub(super) resolver: NodeTypeResolver<S>,
    pub(super) archetype_resolver: ArchetypeResolver<S>,
    pub(super) element_type_resolver: ElementTypeResolver<S>,
    pub(super) index_manager: Option<Arc<IndexManager>>,
    pub(super) tenant_id: String,
    pub(super) repo_id: String,
    pub(super) branch: String,
}

impl<S: Storage> NodeValidator<S> {
    /// Create a new NodeValidator without index support
    pub fn new(storage: Arc<S>, tenant_id: String, repo_id: String, branch: String) -> Self {
        Self {
            resolver: NodeTypeResolver::new(
                storage.clone(),
                tenant_id.clone(),
                repo_id.clone(),
                branch.clone(),
            ),
            archetype_resolver: ArchetypeResolver::new(
                storage.clone(),
                tenant_id.clone(),
                repo_id.clone(),
                branch.clone(),
            ),
            element_type_resolver: ElementTypeResolver::new(
                storage.clone(),
                tenant_id.clone(),
                repo_id.clone(),
                branch.clone(),
            ),
            storage,
            index_manager: None,
            tenant_id,
            repo_id,
            branch,
        }
    }

    /// Create a new NodeValidator with index support
    pub fn with_index_manager(
        storage: Arc<S>,
        index_manager: Arc<IndexManager>,
        tenant_id: String,
        repo_id: String,
        branch: String,
    ) -> Self {
        Self {
            resolver: NodeTypeResolver::new(
                storage.clone(),
                tenant_id.clone(),
                repo_id.clone(),
                branch.clone(),
            ),
            archetype_resolver: ArchetypeResolver::new(
                storage.clone(),
                tenant_id.clone(),
                repo_id.clone(),
                branch.clone(),
            ),
            element_type_resolver: ElementTypeResolver::new(
                storage.clone(),
                tenant_id.clone(),
                repo_id.clone(),
                branch.clone(),
            ),
            storage,
            index_manager: Some(index_manager),
            tenant_id,
            repo_id,
            branch,
        }
    }

    /// Validate a node against its NodeType schema
    pub async fn validate_node(&self, workspace: &str, node: &Node) -> Result<()> {
        // Resolve the NodeType with full inheritance (now repository-level)
        let resolved = self
            .resolver
            .resolve_for_workspace(workspace, &node.node_type)
            .await?;

        // Check required properties
        self.check_required_properties(node, &resolved)?;

        // Check strict mode
        if resolved.node_type.strict.unwrap_or(false) {
            self.check_strict_mode(node, &resolved)?;
        }

        // Check unique properties
        self.check_unique_properties(workspace, node, &resolved)
            .await?;

        // Validate archetype association and element usage
        let mut element_type_cache: HashMap<String, ResolvedElementType> = HashMap::new();
        self.validate_archetype(node, &mut element_type_cache)
            .await?;
        self.validate_element_types(node, &mut element_type_cache)
            .await?;

        Ok(())
    }

    /// Validate that the NodeType exists (without checking if published)
    /// Use this for draft content creation where unpublished NodeTypes are allowed
    pub async fn validate_node_type_exists(&self, node_type_name: &str) -> Result<()> {
        // Just check that the NodeType exists (repository-level), don't require it to be published
        self.storage
            .node_types()
            .get(
                BranchScope::new(&self.tenant_id, &self.repo_id, &self.branch),
                node_type_name,
                None,
            )
            .await?
            .ok_or_else(|| {
                raisin_error::Error::NotFound(format!("NodeType '{}' not found", node_type_name))
            })?;
        Ok(())
    }

    /// Validate that the NodeType exists and is published
    /// Use this when publishing content to ensure only published NodeTypes are used
    pub async fn validate_node_type_published(&self, node_type_name: &str) -> Result<()> {
        self.resolver
            .validate_exists_and_published(node_type_name)
            .await
    }
}
