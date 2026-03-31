// SPDX-License-Identifier: BSL-1.1
//
// RaisinDB - Git-like hierarchical multi model database
// Copyright (C) 2019-2025 SOLUTAS GmbH, Switzerland
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file at the root of this repository.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

// TODO(v0.2): Update deprecated API usages and clean up unused code
#![allow(deprecated)]
#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_variables)]

//! Storage trait definitions for RaisinDB.
//!
//! This crate defines the storage abstraction layer used by RaisinDB.
//! Different storage backends (in-memory, RocksDB, PostgreSQL, MongoDB)
//! implement these traits to provide data persistence.
//!
//! # Main Traits
//!
//! - [`Storage`] - Main storage abstraction providing access to all repositories
//! - [`NodeRepository`] - CRUD operations for nodes
//! - [`NodeTypeRepository`] - CRUD operations for node types
//! - [`WorkspaceRepository`] - CRUD operations for workspaces
//! - [`RegistryRepository`] - Multi-tenant deployment and tenant registration

pub mod fulltext;
pub mod jobs;
pub mod management;
pub mod node_operations;
mod repository;
pub mod scope;
pub mod spatial;
pub mod system_updates;
mod tenant_init;
pub mod traits;
pub mod transactional;
pub mod translations;
pub mod types;
pub mod upload_sessions;

// Re-export TenantContext for convenience
pub use raisin_context::{IsolationMode, TenantContext};

// Re-export event types
pub use raisin_events::{
    Event, EventBus, EventBusExt, EventFilter, EventHandler, FnEventHandler, InMemoryEventBus,
    NodeEvent, NodeEventKind, RepositoryEvent, RepositoryEventKind, WorkspaceEvent,
    WorkspaceEventKind,
};
pub use tenant_init::init_tenant_nodetypes;

// Re-export repository management types
pub use repository::{
    ArchetypeChangeInfo, BranchRepository, ElementTypeChangeInfo, GarbageCollectionRepository,
    GarbageCollectionStats, NodeChangeInfo, NodeTypeChangeInfo, RepositoryManagementRepository,
    RevisionMeta, RevisionRepository, TagRepository,
};

// Re-export translation types
pub use translations::TranslationRepository;

// Re-export management types
pub use management::{
    BackgroundJobs, BackupInfo, CategoryQueueDepthStats, CompactionStats, HealthCheck, HealthLevel,
    HealthStatus, IndexHealth, IndexIssue, IndexManagement, IndexReport, IndexStatus, IndexType,
    IntegrityReport, Issue, JobQueueStats, ManagementOps, Metrics, OptimizeStats, PersistedStats,
    QueueDepthStats, RebuildStats, RepairResult, RestoreStats, WorkerStats,
};

// Re-export job types from the new jobs module
pub use jobs::{JobHandle, JobId, JobInfo, JobLogEntry, JobStatus, JobType, LogEmitter};

// Re-export fulltext types
pub use fulltext::{
    FullTextIndexJob, FullTextJobStore, FullTextSearchQuery, FullTextSearchResult, IndexingEngine,
    JobKind,
};

// Re-export spatial types
pub use spatial::{ProximityResult, SpatialIndexEntry, SpatialIndexRepository};

// Re-export system update types
pub use system_updates::{
    AppliedDefinition, ApplyResult, ApplyResultDetail, BreakingChange, BreakingChangeType,
    PendingUpdate, PendingUpdatesSummary, ResourceType, SystemUpdateRepository,
};

// Re-export node operation types
pub use node_operations::{
    CreateNodeOptions, DeleteNodeOptions, ListOptions, NodeWithPopulatedChildren, UpdateNodeOptions,
};

// Re-export upload session types
pub use upload_sessions::{UploadSession, UploadSessionStatus};

// Re-export common types
pub use types::CommitMetadata;

// Re-export scope types
pub use scope::{
    BranchScope, OwnedBranchScope, OwnedRepoScope, OwnedStorageScope, RepoScope, StorageScope,
};

// Re-export all storage traits from the traits module
pub use traits::{
    ArchetypeRepository, CompoundColumnValue, CompoundIndexRepository, CompoundIndexScanEntry,
    ElementTypeRepository, NodeRepository, NodeTypeRepository, ProcessingRulesRepository,
    PropertyIndexRepository, PropertyScanEntry, ReferenceIndexRepository, RegistryRepository,
    RelationRepository, Storage, Transaction, TreeRepository, VersioningRepository,
    WorkspaceRepository,
};
