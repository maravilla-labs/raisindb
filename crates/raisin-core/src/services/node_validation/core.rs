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
        self.validate_node_resolved(workspace, node)
            .await
            .map(|_| ())
    }

    /// Validate a node against its NodeType schema and return the resolved NodeType.
    ///
    /// Callers on the write path use the returned [`ResolvedNodeType`] to materialize
    /// the node's effective mixin / supertype sets without resolving a second time.
    pub async fn validate_node_resolved(
        &self,
        workspace: &str,
        node: &Node,
    ) -> Result<ResolvedNodeType> {
        // Resolve the NodeType with full inheritance (now repository-level)
        let resolved = self
            .resolver
            .resolve_for_workspace(workspace, &node.node_type)
            .await?;

        self.validate_against_resolved(workspace, node, &resolved)
            .await?;

        Ok(resolved)
    }

    /// The checks themselves, against an ALREADY-RESOLVED NodeType.
    ///
    /// Split out so the write path can resolve once and then coerce, validate
    /// and stamp from the same resolution instead of resolving three times.
    pub async fn validate_against_resolved(
        &self,
        workspace: &str,
        node: &Node,
        resolved: &ResolvedNodeType,
    ) -> Result<()> {
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

        // Every declared PropertyType is enforced against the value present.
        super::property_checks::check_property_types(node, resolved)?;

        Ok(())
    }

    /// Validate a node against its NodeType AND stamp the materialized
    /// effective-mixin / supertype membership sets onto it.
    ///
    /// THE single validate-and-stamp function. Every write path funnels through
    /// it — the service layer (`NodeService::validate_and_stamp`) and the
    /// transaction layer's `add_node` / `put_node`, which is what SQL DML and
    /// the WebSocket create handler reach. Before this existed the service
    /// stamped and the transaction only validated, so a node created through a
    /// child POST or from `psql` carried no `$mixins` / `$supertypes` and
    /// `has_mixin()` / `is_a()` answered false for it.
    ///
    /// Client-supplied reserved (`$`) properties are stripped first: they are
    /// server-computed metadata and must never be trusted from input.
    pub async fn validate_and_stamp(&self, workspace: &str, node: &mut Node) -> Result<()> {
        // Never trust client-supplied membership sets.
        node.strip_reserved_properties();

        self.validate_node_type_exists(&node.node_type).await?;

        // ONE resolution, then: coerce, validate, stamp.
        let resolved = self
            .resolver
            .resolve_for_workspace(workspace, &node.node_type)
            .await?;

        // Coercion runs FIRST. A property declared `Decimal` arrives as a
        // string and is not yet a Decimal for the type check to accept, so
        // validating before coercing would refuse every well-formed decimal.
        super::property_checks::coerce_declared_decimals(node, &resolved)?;

        self.validate_against_resolved(workspace, node, &resolved)
            .await?;

        let supertypes = resolved.effective_supertypes();
        node.set_effective_types(resolved.resolved_mixins.clone(), supertypes);
        Ok(())
    }

    /// Stamp the materialized effective-mixin / supertype sets WITHOUT running
    /// schema-shape validation.
    ///
    /// Type membership is ENGINE METADATA, not user schema: `is_a()` /
    /// `has_mixin()`, `allowed_children` family matching and the type-membership
    /// index all read it, and a node that lacks it does not merely skip a check
    /// — it silently answers `false` to questions about itself. So it must be
    /// stamped on every write, including the paths that deliberately turn
    /// schema validation off (bulk import, replication, `psql`).
    ///
    /// The same reasoning the immutability check in `put_node.rs` already uses:
    /// not everything on the write path is schema-shape validation, and the
    /// parts that are not must not ride on that toggle.
    ///
    /// FAILS OPEN. If the NodeType cannot be resolved — it does not exist yet,
    /// or this is a replication stream carrying nodes ahead of their schema —
    /// the node is left unstamped rather than rejected. Turning validation off
    /// must never turn a write that used to succeed into an error.
    pub async fn stamp_effective_types(&self, workspace: &str, node: &mut Node) -> Result<()> {
        // Never trust client-supplied membership sets.
        node.strip_reserved_properties();

        match self
            .resolver
            .resolve_for_workspace(workspace, &node.node_type)
            .await
        {
            Ok(resolved) => {
                // Coerce declared decimals here too — storing "19.90" as a
                // string on a Decimal property would defeat the type. Fails
                // open, like the rest of this function: a value that cannot be
                // coerced is left as it came rather than rejecting a write on a
                // path that deliberately turned validation off.
                if let Err(e) = super::property_checks::coerce_declared_decimals(node, &resolved) {
                    tracing::debug!(
                        node_type = %node.node_type,
                        error = %e,
                        "decimal coercion skipped on a validation-disabled write"
                    );
                }
                let supertypes = resolved.effective_supertypes();
                node.set_effective_types(resolved.resolved_mixins.clone(), supertypes);
            }
            Err(e) => {
                tracing::debug!(
                    node_type = %node.node_type,
                    workspace = %workspace,
                    error = %e,
                    "type membership not stamped: NodeType could not be resolved"
                );
            }
        }
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
