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

//! Repository management storage traits
//!
//! This module provides storage traits for managing repositories, branches, and revisions
//! in the repository-first architecture.

mod branch;
mod gc;
mod repo_management;
mod revision;
mod tag;
mod types;

pub use branch::BranchRepository;
pub use gc::GarbageCollectionRepository;
pub use repo_management::RepositoryManagementRepository;
pub use revision::RevisionRepository;
pub use tag::TagRepository;
pub use types::{
    ArchetypeChangeInfo, ElementTypeChangeInfo, GarbageCollectionStats, NodeChangeInfo,
    NodeTypeChangeInfo, RevisionMeta,
};

#[cfg(test)]
mod tests {
    use super::*;
    use raisin_hlc::HLC;

    #[test]
    fn test_revision_meta_serde() {
        let meta = RevisionMeta {
            revision: HLC::new(42, 0),
            parent: Some(HLC::new(41, 0)),
            merge_parent: None,
            branch: "main".to_string(),
            timestamp: chrono::Utc::now(),
            actor: "user-123".to_string(),
            message: "Update homepage".to_string(),
            is_system: false,
            changed_nodes: Vec::new(),
            changed_node_types: Vec::new(),
            changed_archetypes: Vec::new(),
            changed_element_types: Vec::new(),
            operation: None,
        };

        let json = serde_json::to_string(&meta).unwrap();
        let deserialized: RevisionMeta = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.revision, HLC::new(42, 0));
        assert_eq!(deserialized.parent, Some(HLC::new(41, 0)));
        assert_eq!(deserialized.branch, "main");
        assert_eq!(deserialized.actor, "user-123");
        assert_eq!(deserialized.message, "Update homepage");
        assert!(!deserialized.is_system);
        assert!(deserialized.changed_nodes.is_empty());
        assert!(deserialized.changed_node_types.is_empty());
        assert!(deserialized.changed_archetypes.is_empty());
        assert!(deserialized.changed_element_types.is_empty());
        assert!(deserialized.operation.is_none());
    }
}
