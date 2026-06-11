//! NodeType inheritance resolution service
//!
//! Handles resolution of NodeType inheritance chains including:
//! - `extends` - single parent inheritance
//! - `mixins` - multiple trait-like composition
//! - `overrides` - property value overrides
//!
//! The resolution algorithm:
//! 1. Fetch the current NodeType
//! 2. Recursively resolve parent (if extends is set)
//! 3. Merge parent properties (parent first, child overrides)
//! 4. Apply each mixin in order
//! 5. Apply overrides last
//! 6. Detect circular dependencies

mod resolution;

use raisin_error::{Error, Result};
use raisin_hlc::HLC;
use raisin_models::nodes::properties::schema::{IndexType, PropertyValueSchema};
use raisin_models::nodes::types::NodeType;
use raisin_storage::{
    scope::{BranchScope, RepoScope},
    NodeTypeRepository, Storage, WorkspaceRepository,
};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Resolved NodeType with all inheritance applied
#[derive(Debug, Clone)]
pub struct ResolvedNodeType {
    /// The original NodeType
    pub node_type: NodeType,
    /// All properties including inherited ones
    pub resolved_properties: Vec<PropertyValueSchema>,
    /// All allowed children including inherited ones
    pub resolved_allowed_children: Vec<String>,
    /// Effective mixin names applied to this NodeType, including those inherited
    /// from `extends` and transitively from other mixins (deduped, order preserved)
    pub resolved_mixins: Vec<String>,
    /// Whether this node type is indexable (merged from inheritance)
    pub resolved_indexable: bool,
    /// Which index types are enabled (merged from inheritance)
    pub resolved_index_types: Vec<IndexType>,
    /// Inheritance chain (for debugging)
    pub inheritance_chain: Vec<String>,
}

impl ResolvedNodeType {
    /// The full "is-a" membership set for this NodeType: the type itself, its
    /// `extends` ancestors (from `inheritance_chain`), and every effective mixin
    /// (deduped, order preserved). Used to materialize `$supertypes` on nodes.
    pub fn effective_supertypes(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for name in self
            .inheritance_chain
            .iter()
            .chain(self.resolved_mixins.iter())
        {
            if !out.iter().any(|x| x == name) {
                out.push(name.clone());
            }
        }
        out
    }
}

/// Maximum depth of inheritance chain to prevent stack overflow
const MAX_INHERITANCE_DEPTH: usize = 20;

#[derive(Clone)]
pub struct NodeTypeResolver<S: Storage> {
    storage: Arc<S>,
    tenant_id: String,
    repo_id: String,
    branch: String,
}

impl<S: Storage> NodeTypeResolver<S> {
    pub fn new(storage: Arc<S>, tenant_id: String, repo_id: String, branch: String) -> Self {
        Self {
            storage,
            tenant_id,
            repo_id,
            branch,
        }
    }

    /// Resolve a NodeType with all inheritance applied
    pub async fn resolve(&self, node_type_name: &str) -> Result<ResolvedNodeType> {
        let mut visited = HashSet::new();
        let mut chain = Vec::new();
        let mut revision_cache = HashMap::new();
        self.resolve_recursive(
            node_type_name,
            &mut visited,
            &mut chain,
            None,
            &mut revision_cache,
        )
        .await
    }

    /// Resolve a NodeType for a specific workspace, honoring NodeType pins.
    pub async fn resolve_for_workspace(
        &self,
        workspace: &str,
        node_type_name: &str,
    ) -> Result<ResolvedNodeType> {
        let pins = self.load_workspace_pins(workspace).await?;
        let mut visited = HashSet::new();
        let mut chain = Vec::new();
        let mut revision_cache = HashMap::new();
        self.resolve_recursive(
            node_type_name,
            &mut visited,
            &mut chain,
            Some(&pins),
            &mut revision_cache,
        )
        .await
    }

    async fn load_workspace_pins(&self, workspace: &str) -> Result<HashMap<String, Option<HLC>>> {
        let workspaces = self.storage.workspaces();
        if let Some(ws) = workspaces
            .get(RepoScope::new(&self.tenant_id, &self.repo_id), workspace)
            .await?
        {
            Ok(ws.config.node_type_pins.clone())
        } else {
            Ok(HashMap::new())
        }
    }

    /// Check if a NodeType exists and is published
    pub async fn validate_exists_and_published(&self, node_type_name: &str) -> Result<()> {
        let repo = self.storage.node_types();

        let node_type = repo
            .get(
                BranchScope::new(&self.tenant_id, &self.repo_id, &self.branch),
                node_type_name,
                None,
            )
            .await?
            .ok_or_else(|| Error::NotFound(format!("NodeType not found: {}", node_type_name)))?;

        if !node_type.is_published() {
            return Err(Error::Validation(format!(
                "NodeType '{}' is not published",
                node_type_name
            )));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests;
