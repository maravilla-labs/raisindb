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

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::nodes::properties::schema::CompoundIndexDefinition;
use crate::nodes::types::initial_structure::InitialNodeStructure;
use crate::timestamp::StorageTimestamp;

pub mod builtin_indexes;
pub mod delta;
pub use builtin_indexes::BuiltinIndexes;
pub use delta::DeltaOp;

/// Workspace configuration for branch and NodeType version management
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct WorkspaceConfig {
    /// Default branch for this workspace
    #[serde(default = "default_branch_name")]
    pub default_branch: String,

    /// NodeType revision pinning: maps NodeType name to specific revision (HLC)
    /// None means "track latest", Some(hlc) means "pin to this HLC revision"
    #[serde(default, rename = "node_type_pins", alias = "node_type_refs")]
    pub node_type_pins: HashMap<String, Option<raisin_hlc::HLC>>,

    /// Spatial index defaults for this workspace, with per-property overrides.
    ///
    /// This is *replicated intent* — it travels with the workspace record via
    /// `OperationType::UpdateWorkspace`, so `ALTER SPATIAL INDEX` on one node
    /// reaches every node, and each node then schedules its own local reindex
    /// when it notices the policy fingerprint changed. That is the entire
    /// cluster-wide fan-out mechanism; nothing is broadcast separately.
    ///
    /// `None` means "server defaults", and serializes to nothing, so adding this
    /// field does not rewrite any stored workspace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spatial: Option<crate::nodes::properties::spatial_policy::SpatialWorkspaceSchema>,

    /// Built-in workspace indexes (plan Phase 13f). `None` means every
    /// built-in index is ON (the default); a workspace opts out per index,
    /// e.g. `builtin_indexes: { children_by_created_at: false }`. Replicated
    /// with the workspace record like `spatial`, so each node notices the
    /// change and builds or drops its own entries.
    ///
    /// Skipped when `None`, so adding it rewrites no stored workspace. (Every
    /// persisted and network encoding of a workspace is NAMED msgpack or
    /// JSON; a positional encoding would shift around a skipped field.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builtin_indexes: Option<BuiltinIndexes>,
}

impl WorkspaceConfig {
    /// The built-in index switches in force (defaults when unset).
    pub fn effective_builtin_indexes(&self) -> BuiltinIndexes {
        self.builtin_indexes.clone().unwrap_or_default()
    }
}

fn default_branch_name() -> String {
    "main".to_string()
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            default_branch: default_branch_name(),
            node_type_pins: HashMap::new(),
            spatial: None,
            builtin_indexes: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Workspace {
    pub name: String, // Name of the workspace
    #[serde(default)]
    pub description: Option<String>, // Description of the workspace
    pub allowed_node_types: Vec<String>, // NodeTypes that are allowed in this workspace (namespace:node_type)
    pub allowed_root_node_types: Vec<String>, // NodeTypes that can be root-level types in this workspace (namespace:node_type)
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub initial_structure: Option<InitialNodeStructure>, // Initial root-level nodes to create when workspace is created
    #[serde(default = "default_created_at")]
    pub created_at: StorageTimestamp, // Timestamp for when the workspace was created (i64 nanos in binary, RFC3339 in JSON)
    #[serde(default)]
    pub updated_at: Option<StorageTimestamp>, // Timestamp for when the workspace was last updated (i64 nanos in binary, RFC3339 in JSON)
    #[serde(default)]
    pub config: WorkspaceConfig, // Workspace configuration
    /// Compound indexes owned by THIS workspace (plan Phase 13e): each covers
    /// every node of the workspace, whatever its node type, so an untyped
    /// listing (`CHILD_OF('/a') ORDER BY created_at DESC LIMIT n`) can be
    /// index-served. Authored like a NodeType's `compound_indexes`; stored
    /// under a workspace keyspace name — see [`Self::owned_compound_indexes`].
    ///
    /// LAST and skipped when absent: a workspace without workspace indexes
    /// serializes byte-for-byte as before, in the named and the compact
    /// (replication) encodings alike, so an older peer still decodes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compound_indexes: Option<Vec<CompoundIndexDefinition>>,
}
fn default_created_at() -> StorageTimestamp {
    StorageTimestamp::now()
}

impl Workspace {
    pub fn new(name: String) -> Self {
        Workspace {
            name,
            allowed_node_types: Vec::new(),
            allowed_root_node_types: Vec::new(),
            depends_on: Vec::new(),
            initial_structure: None,
            created_at: StorageTimestamp::now(),
            description: None,
            updated_at: None,
            config: WorkspaceConfig::default(),
            compound_indexes: None,
        }
    }

    /// This workspace's compound indexes as the writers, builds and planner
    /// see them: each stored under the workspace keyspace name
    /// (`@{name}`, [`CompoundIndexDefinition::owned_by_workspace`]) with the
    /// owner stamped. A nameless declaration is ignored; a repeated name
    /// keeps its first declaration (one keyspace per name).
    ///
    /// The BUILT-IN indexes the config leaves on (plan Phase 13f,
    /// [`builtin_indexes`]) come last, derived from the config and never
    /// from the stored declarations: a user declaration with a reserved name
    /// (`__…`) is ignored here (and refused when the workspace is written).
    pub fn owned_compound_indexes(&self) -> Vec<CompoundIndexDefinition> {
        let mut out: Vec<CompoundIndexDefinition> = Vec::new();
        let builtin = self.config.effective_builtin_indexes().declarations();
        for index in self.compound_indexes.iter().flatten().chain(builtin.iter()) {
            if index.name.is_empty() || index.columns.is_empty() {
                continue;
            }
            let is_builtin = builtin.iter().any(|b| std::ptr::eq(b, index));
            if !is_builtin && builtin_indexes::is_reserved_authored_name(&index.name) {
                continue;
            }
            let owned = index.owned_by_workspace(&self.name);
            if !out.iter().any(|o| o.name == owned.name) {
                out.push(owned);
            }
        }
        out
    }

    /// The user declarations whose authored name is reserved for built-in
    /// indexes (`__…`): a workspace write carrying one is refused.
    pub fn reserved_compound_index_names(&self) -> Vec<String> {
        self.compound_indexes
            .iter()
            .flatten()
            .filter(|index| builtin_indexes::is_reserved_authored_name(&index.name))
            .map(|index| index.name.clone())
            .collect()
    }

    pub fn update_allowed_node_types(
        &mut self,
        allowed_node_types: Vec<String>,
        allowed_root_node_types: Vec<String>,
    ) {
        self.allowed_node_types = allowed_node_types;
        self.allowed_root_node_types = allowed_root_node_types;
        self.updated_at = Some(StorageTimestamp::now());
    }
}

#[cfg(test)]
mod tests;
