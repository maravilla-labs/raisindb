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

//! Built-in, SYSTEM-declared workspace compound indexes (plan Phase 13f).
//!
//! `CHILD_OF` is core functionality, so a newest- or oldest-first folder
//! listing — `CHILD_OF($p) ORDER BY created_at [ASC|DESC] [LIMIT n]` — must be
//! index-served out of the box. Every workspace therefore carries one
//! workspace-owned compound index on `(__parent_path, __created_at)` unless it
//! opts out (`config.builtin_indexes.children_by_created_at: false`).
//!
//! The index is DERIVED from the workspace config, never stored among the
//! user declarations: [`super::Workspace::owned_compound_indexes`] appends it,
//! and that derivation is the one the writers, the builds and the planner all
//! call — so turning it off and on is a config change and nothing else. Its
//! authored name starts with the reserved [`RESERVED_INDEX_NAME_PREFIX`],
//! which a user declaration may not use (refused at workspace write, ignored
//! by the derivation), so no user index can share its keyspace or build state.

use crate::nodes::properties::schema::{
    CompoundColumnType, CompoundIndexColumn, CompoundIndexDefinition, WORKSPACE_INDEX_PREFIX,
};
use serde::{Deserialize, Serialize};

/// Authored names starting with this are reserved for built-in indexes.
pub const RESERVED_INDEX_NAME_PREFIX: &str = "__";

/// Authored name of the built-in `(__parent_path, __created_at)` index.
pub const CHILDREN_BY_CREATED_AT: &str = "__children_by_created_at";

/// Stored (keyspace) name of the built-in `(__parent_path, __created_at)`
/// index: `@__children_by_created_at`.
pub fn children_by_created_at_stored_name() -> String {
    format!("{WORKSPACE_INDEX_PREFIX}{CHILDREN_BY_CREATED_AT}")
}

/// Whether an AUTHORED workspace index name is reserved for the system.
pub fn is_reserved_authored_name(name: &str) -> bool {
    name.starts_with(RESERVED_INDEX_NAME_PREFIX)
}

/// Whether a STORED (keyspace) name is a built-in workspace index.
pub fn is_builtin_stored_name(name: &str) -> bool {
    name.strip_prefix(WORKSPACE_INDEX_PREFIX)
        .is_some_and(is_reserved_authored_name)
}

/// Which built-in indexes a workspace carries (`config.builtin_indexes`).
/// Every switch defaults to ON; a workspace opts OUT explicitly.
///
/// ```yaml
/// config:
///   builtin_indexes:
///     children_by_created_at: false
/// ```
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct BuiltinIndexes {
    /// `(__parent_path, __created_at)`: serves `CHILD_OF(..) ORDER BY
    /// created_at [ASC|DESC] [LIMIT n]`. Costs one index entry per node
    /// (written on create, re-keyed on move, ended on delete; an update that
    /// changes neither the parent nor `created_at` writes nothing).
    #[serde(default = "enabled")]
    pub children_by_created_at: bool,
}

fn enabled() -> bool {
    true
}

impl Default for BuiltinIndexes {
    fn default() -> Self {
        Self {
            children_by_created_at: true,
        }
    }
}

impl BuiltinIndexes {
    /// The AUTHORED built-in declarations these switches turn on (the
    /// workspace stamps its owner and keyspace name on them).
    pub fn declarations(&self) -> Vec<CompoundIndexDefinition> {
        let mut out = Vec::new();
        if self.children_by_created_at {
            out.push(children_by_created_at());
        }
        out
    }
}

/// The authored `(__parent_path, __created_at)` declaration. Its hash is part
/// of the build state: do not change it without accepting a rebuild of every
/// workspace on every node.
pub fn children_by_created_at() -> CompoundIndexDefinition {
    CompoundIndexDefinition {
        name: CHILDREN_BY_CREATED_AT.to_string(),
        columns: vec![
            CompoundIndexColumn {
                property: "__parent_path".to_string(),
                ascending: None,
                column_type: CompoundColumnType::String,
            },
            CompoundIndexColumn {
                property: "__created_at".to_string(),
                ascending: None,
                column_type: CompoundColumnType::Timestamp,
            },
        ],
        has_order_column: true,
        owner: None,
    }
}
