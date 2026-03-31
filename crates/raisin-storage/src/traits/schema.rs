// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Schema repository trait definitions.
//!
//! This module contains traits for managing schema definitions:
//! - `NodeTypeRepository` - For managing NodeType schemas
//! - `ArchetypeRepository` - For managing Archetype definitions
//! - `ElementTypeRepository` - For managing ElementType definitions

use raisin_error::Result;
use raisin_hlc::HLC;
use raisin_models as models;
use raisin_models::nodes::types::element::element_type::ElementType as ElementTypeModel;

use crate::scope::BranchScope;
use crate::types::CommitMetadata;

/// Repository for managing NodeType schemas
///
/// NodeTypes define the structure and validation rules for nodes.
/// In repository-first architecture, NodeTypes are scoped by tenant/repo/branch,
/// allowing different branches to evolve their schemas independently.
///
/// # Scoped Architecture
///
/// All methods take a `BranchScope` parameter that bundles:
/// - **tenant_id**: Multi-tenant isolation
/// - **repo_id**: Repository (project/database) scoping
/// - **branch**: Git-like branch operations - each branch maintains its own NodeType history
///
/// # Key Storage Format
///
/// `/{tenant_id}/repo/{repo_id}/{branch}/nodetypes/{name}/{~revision}`
///
/// NodeTypes are persisted per branch and revision. Listing APIs return the latest
/// revision unless `max_revision` is provided to enable time-travel semantics.
pub trait NodeTypeRepository: Send + Sync {
    fn get(
        &self,
        scope: BranchScope<'_>,
        name: &str,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Option<models::nodes::types::NodeType>>> + Send;

    fn get_by_id(
        &self,
        scope: BranchScope<'_>,
        id: &str,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Option<models::nodes::types::NodeType>>> + Send;

    /// Batch fetch multiple NodeTypes by name. More efficient than multiple get() calls.
    fn get_by_names(
        &self,
        scope: BranchScope<'_>,
        names: &[String],
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Vec<models::nodes::types::NodeType>>> + Send;

    /// Resolve the revision number associated with a specific NodeType version.
    ///
    /// Returns `Ok(Some(revision))` when the version is known, `Ok(None)` when the
    /// version does not exist.
    fn resolve_version_revision(
        &self,
        scope: BranchScope<'_>,
        name: &str,
        version: i32,
    ) -> impl std::future::Future<Output = Result<Option<HLC>>> + Send;

    /// Create a new NodeType. Fails if the NodeType already exists.
    ///
    /// Use this for SQL INSERT semantics where creating a duplicate is an error.
    fn create(
        &self,
        scope: BranchScope<'_>,
        node_type: models::nodes::types::NodeType,
        commit: CommitMetadata,
    ) -> impl std::future::Future<Output = Result<HLC>> + Send;

    /// Update an existing NodeType. Fails if the NodeType does not exist.
    ///
    /// Use this for SQL UPDATE semantics where updating a non-existent entity is an error.
    fn update(
        &self,
        scope: BranchScope<'_>,
        node_type: models::nodes::types::NodeType,
        commit: CommitMetadata,
    ) -> impl std::future::Future<Output = Result<HLC>> + Send;

    /// Upsert a NodeType (create if not exists, update if exists).
    ///
    /// Use this for SQL UPSERT/MERGE semantics or when you don't care about existence.
    fn upsert(
        &self,
        scope: BranchScope<'_>,
        node_type: models::nodes::types::NodeType,
        commit: CommitMetadata,
    ) -> impl std::future::Future<Output = Result<HLC>> + Send;

    /// Deprecated: Use `upsert` instead. This method is an alias for `upsert`.
    #[deprecated(since = "0.1.0", note = "Use `upsert` instead")]
    fn put(
        &self,
        scope: BranchScope<'_>,
        node_type: models::nodes::types::NodeType,
        commit: CommitMetadata,
    ) -> impl std::future::Future<Output = Result<HLC>> + Send {
        self.upsert(scope, node_type, commit)
    }

    fn delete(
        &self,
        scope: BranchScope<'_>,
        name: &str,
        commit: CommitMetadata,
    ) -> impl std::future::Future<Output = Result<Option<HLC>>> + Send;

    fn list(
        &self,
        scope: BranchScope<'_>,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Vec<models::nodes::types::NodeType>>> + Send;

    fn list_published(
        &self,
        scope: BranchScope<'_>,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Vec<models::nodes::types::NodeType>>> + Send;

    fn publish(
        &self,
        scope: BranchScope<'_>,
        name: &str,
        commit: CommitMetadata,
    ) -> impl std::future::Future<Output = Result<HLC>> + Send;

    fn unpublish(
        &self,
        scope: BranchScope<'_>,
        name: &str,
        commit: CommitMetadata,
    ) -> impl std::future::Future<Output = Result<HLC>> + Send;

    fn is_published(
        &self,
        scope: BranchScope<'_>,
        name: &str,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<bool>> + Send;

    fn validate_published(
        &self,
        scope: BranchScope<'_>,
        node_type_name: &str,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<()>> + Send;
}

/// Repository for managing Archetype definitions.
///
/// Archetypes encapsulate reusable content structures that can extend base node types.
/// They follow the same branching and revision semantics as NodeTypes to ensure
/// consistent schema evolution across branches.
///
/// All methods take a `BranchScope` (tenant + repo + branch).
pub trait ArchetypeRepository: Send + Sync {
    fn get(
        &self,
        scope: BranchScope<'_>,
        name: &str,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Option<models::nodes::types::archetype::Archetype>>>
           + Send;

    fn get_by_id(
        &self,
        scope: BranchScope<'_>,
        id: &str,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Option<models::nodes::types::archetype::Archetype>>>
           + Send;

    fn get_by_names(
        &self,
        scope: BranchScope<'_>,
        names: &[String],
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Vec<models::nodes::types::archetype::Archetype>>> + Send;

    fn resolve_version_revision(
        &self,
        scope: BranchScope<'_>,
        name: &str,
        version: i32,
    ) -> impl std::future::Future<Output = Result<Option<HLC>>> + Send;

    /// Create a new Archetype. Fails if the Archetype already exists.
    fn create(
        &self,
        scope: BranchScope<'_>,
        archetype: models::nodes::types::archetype::Archetype,
        commit: CommitMetadata,
    ) -> impl std::future::Future<Output = Result<HLC>> + Send;

    /// Update an existing Archetype. Fails if the Archetype does not exist.
    fn update(
        &self,
        scope: BranchScope<'_>,
        archetype: models::nodes::types::archetype::Archetype,
        commit: CommitMetadata,
    ) -> impl std::future::Future<Output = Result<HLC>> + Send;

    /// Upsert an Archetype (create if not exists, update if exists).
    fn upsert(
        &self,
        scope: BranchScope<'_>,
        archetype: models::nodes::types::archetype::Archetype,
        commit: CommitMetadata,
    ) -> impl std::future::Future<Output = Result<HLC>> + Send;

    /// Deprecated: Use `upsert` instead.
    #[deprecated(since = "0.1.0", note = "Use `upsert` instead")]
    fn put(
        &self,
        scope: BranchScope<'_>,
        archetype: models::nodes::types::archetype::Archetype,
        commit: CommitMetadata,
    ) -> impl std::future::Future<Output = Result<HLC>> + Send {
        self.upsert(scope, archetype, commit)
    }

    fn delete(
        &self,
        scope: BranchScope<'_>,
        name: &str,
        commit: CommitMetadata,
    ) -> impl std::future::Future<Output = Result<Option<HLC>>> + Send;

    fn list(
        &self,
        scope: BranchScope<'_>,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Vec<models::nodes::types::archetype::Archetype>>> + Send;

    fn list_published(
        &self,
        scope: BranchScope<'_>,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Vec<models::nodes::types::archetype::Archetype>>> + Send;

    fn publish(
        &self,
        scope: BranchScope<'_>,
        name: &str,
        commit: CommitMetadata,
    ) -> impl std::future::Future<Output = Result<HLC>> + Send;

    fn unpublish(
        &self,
        scope: BranchScope<'_>,
        name: &str,
        commit: CommitMetadata,
    ) -> impl std::future::Future<Output = Result<HLC>> + Send;

    fn is_published(
        &self,
        scope: BranchScope<'_>,
        name: &str,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<bool>> + Send;

    fn validate_published(
        &self,
        scope: BranchScope<'_>,
        archetype_name: &str,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<()>> + Send;
}

/// Repository for managing ElementType definitions used inside composite properties.
///
/// All methods take a `BranchScope` (tenant + repo + branch).
pub trait ElementTypeRepository: Send + Sync {
    fn get(
        &self,
        scope: BranchScope<'_>,
        name: &str,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Option<ElementTypeModel>>> + Send;

    fn get_by_id(
        &self,
        scope: BranchScope<'_>,
        id: &str,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Option<ElementTypeModel>>> + Send;

    fn get_by_names(
        &self,
        scope: BranchScope<'_>,
        names: &[String],
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Vec<ElementTypeModel>>> + Send;

    fn resolve_version_revision(
        &self,
        scope: BranchScope<'_>,
        name: &str,
        version: i32,
    ) -> impl std::future::Future<Output = Result<Option<HLC>>> + Send;

    /// Create a new ElementType. Fails if the ElementType already exists.
    fn create(
        &self,
        scope: BranchScope<'_>,
        element_type: ElementTypeModel,
        commit: CommitMetadata,
    ) -> impl std::future::Future<Output = Result<HLC>> + Send;

    /// Update an existing ElementType. Fails if the ElementType does not exist.
    fn update(
        &self,
        scope: BranchScope<'_>,
        element_type: ElementTypeModel,
        commit: CommitMetadata,
    ) -> impl std::future::Future<Output = Result<HLC>> + Send;

    /// Upsert an ElementType (create if not exists, update if exists).
    fn upsert(
        &self,
        scope: BranchScope<'_>,
        element_type: ElementTypeModel,
        commit: CommitMetadata,
    ) -> impl std::future::Future<Output = Result<HLC>> + Send;

    /// Deprecated: Use `upsert` instead.
    #[deprecated(since = "0.1.0", note = "Use `upsert` instead")]
    fn put(
        &self,
        scope: BranchScope<'_>,
        element_type: ElementTypeModel,
        commit: CommitMetadata,
    ) -> impl std::future::Future<Output = Result<HLC>> + Send {
        self.upsert(scope, element_type, commit)
    }

    fn delete(
        &self,
        scope: BranchScope<'_>,
        name: &str,
        commit: CommitMetadata,
    ) -> impl std::future::Future<Output = Result<Option<HLC>>> + Send;

    fn list(
        &self,
        scope: BranchScope<'_>,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Vec<ElementTypeModel>>> + Send;

    fn list_published(
        &self,
        scope: BranchScope<'_>,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<Vec<ElementTypeModel>>> + Send;

    fn publish(
        &self,
        scope: BranchScope<'_>,
        name: &str,
        commit: CommitMetadata,
    ) -> impl std::future::Future<Output = Result<HLC>> + Send;

    fn unpublish(
        &self,
        scope: BranchScope<'_>,
        name: &str,
        commit: CommitMetadata,
    ) -> impl std::future::Future<Output = Result<HLC>> + Send;

    fn is_published(
        &self,
        scope: BranchScope<'_>,
        name: &str,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<bool>> + Send;

    fn validate_published(
        &self,
        scope: BranchScope<'_>,
        element_type_name: &str,
        max_revision: Option<&HLC>,
    ) -> impl std::future::Future<Output = Result<()>> + Send;
}
