// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland

//! Main Storage trait definition.
//!
//! This module contains the main `Storage` trait which provides access
//! to all repository types and the `Transaction` trait for atomic operations.

use raisin_error::Result;
use raisin_events::EventBus;
use std::sync::Arc;

use crate::fulltext::FullTextJobStore;
use crate::repository::{
    BranchRepository, GarbageCollectionRepository, RepositoryManagementRepository,
    RevisionRepository, TagRepository,
};
use crate::scope::StorageScope;
use crate::spatial::SpatialIndexRepository;
use crate::translations::TranslationRepository;

use super::index::{CompoundIndexRepository, PropertyIndexRepository, ReferenceIndexRepository};
use super::node::NodeRepository;
use super::registry::{RegistryRepository, TreeRepository};
use super::relation::RelationRepository;
use super::schema::{ArchetypeRepository, ElementTypeRepository, NodeTypeRepository};
use super::workspace::{VersioningRepository, WorkspaceRepository};

/// Transaction support for storage backends.
///
/// Implementations should provide atomic commit/rollback semantics.
pub trait Transaction: Send + Sync {
    /// Commits all changes made within this transaction.
    fn commit(&self) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Rolls back all changes made within this transaction.
    fn rollback(&self) -> impl std::future::Future<Output = Result<()>> + Send;
}

/// Main storage trait providing access to all repositories.
///
/// This is the primary abstraction layer for RaisinDB storage backends.
/// Different implementations (RocksDB, InMemory, PostgreSQL, etc.) provide
/// the actual data persistence while conforming to this interface.
pub trait Storage: Send + Sync {
    type Tx: Transaction;
    type Nodes: NodeRepository;
    type NodeTypes: NodeTypeRepository;
    type Archetypes: ArchetypeRepository;
    type ElementTypes: ElementTypeRepository;
    type Workspaces: WorkspaceRepository;
    type Registry: RegistryRepository;
    type PropertyIndex: PropertyIndexRepository;
    type ReferenceIndex: ReferenceIndexRepository;
    type Versioning: VersioningRepository;
    type RepositoryManagement: RepositoryManagementRepository;
    type Branches: BranchRepository;
    type Tags: TagRepository;
    type Revisions: RevisionRepository;
    type GarbageCollection: GarbageCollectionRepository;
    type Trees: TreeRepository;
    type Relations: RelationRepository;
    type Translations: TranslationRepository + Clone;
    type FullTextJobStore: FullTextJobStore;
    type SpatialIndex: SpatialIndexRepository;
    type CompoundIndex: CompoundIndexRepository;

    fn nodes(&self) -> &Self::Nodes;
    fn node_types(&self) -> &Self::NodeTypes;
    fn archetypes(&self) -> &Self::Archetypes;
    fn element_types(&self) -> &Self::ElementTypes;
    fn workspaces(&self) -> &Self::Workspaces;
    fn registry(&self) -> &Self::Registry;
    fn property_index(&self) -> &Self::PropertyIndex;
    fn reference_index(&self) -> &Self::ReferenceIndex;
    fn versioning(&self) -> &Self::Versioning;
    fn repository_management(&self) -> &Self::RepositoryManagement;
    fn branches(&self) -> &Self::Branches;
    fn tags(&self) -> &Self::Tags;
    fn revisions(&self) -> &Self::Revisions;
    fn garbage_collection(&self) -> &Self::GarbageCollection;
    fn trees(&self) -> &Self::Trees;
    fn relations(&self) -> &Self::Relations;
    fn translations(&self) -> &Self::Translations;
    fn fulltext_job_store(&self) -> &Self::FullTextJobStore;
    fn spatial_index(&self) -> &Self::SpatialIndex;
    fn compound_index(&self) -> &Self::CompoundIndex;
    fn begin(&self) -> impl std::future::Future<Output = Result<Self::Tx>> + Send;

    /// Get the event bus for subscribing to storage events
    fn event_bus(&self) -> Arc<dyn EventBus>;

    // Workspace delta operations (branch-scoped draft storage)

    /// Put a node into workspace delta (draft storage)
    fn put_workspace_delta(
        &self,
        scope: StorageScope<'_>,
        node: &raisin_models::nodes::Node,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Get a node from workspace delta by path
    fn get_workspace_delta(
        &self,
        scope: StorageScope<'_>,
        path: &str,
    ) -> impl std::future::Future<Output = Result<Option<raisin_models::nodes::Node>>> + Send;

    /// Get a node from workspace delta by ID
    fn get_workspace_delta_by_id(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
    ) -> impl std::future::Future<Output = Result<Option<raisin_models::nodes::Node>>> + Send;

    /// List all delta operations in workspace delta
    fn list_workspace_deltas(
        &self,
        scope: StorageScope<'_>,
    ) -> impl std::future::Future<Output = Result<Vec<raisin_models::workspace::DeltaOp>>> + Send;

    /// Clear all workspace deltas (called after commit)
    fn clear_workspace_deltas(
        &self,
        scope: StorageScope<'_>,
    ) -> impl std::future::Future<Output = Result<()>> + Send;

    /// Delete specific node from workspace delta (creates tombstone)
    fn delete_workspace_delta(
        &self,
        scope: StorageScope<'_>,
        node_id: &str,
        path: &str,
    ) -> impl std::future::Future<Output = Result<()>> + Send;
}
