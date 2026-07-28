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

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditLog {
    pub id: String,
    pub node_id: String,
    pub path: String,
    pub workspace: String,
    pub user_id: Option<String>,
    pub action: AuditLogAction,
    pub timestamp: DateTime<Utc>,
    pub details: Option<String>,

    /// The non-human principal the write was made *through*, when there was one
    /// — e.g. `mcp:<server-slug>` for a write that arrived over an MCP tool
    /// call, or `agent:<agent-node-path>` for an AI-agent-driven write.
    ///
    /// Orthogonal to `user_id`, which stays the human on whose behalf the agent
    /// acted: an MCP write by Alice records `user_id = "alice"` **and**
    /// `agent = "mcp:studio-admin"`. `None` means the write came straight from a
    /// human (or an unattributed system job).
    ///
    /// Namespaced string rather than an enum so new agent kinds need no schema
    /// change. Optional + defaulted so records persisted before this field
    /// existed still decode, and omitted from the wire when absent so older
    /// clients see the exact JSON they saw before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AuditLogAction {
    Create,
    Update,
    UpdateProperty,
    Delete,
    Read,
    Viewed,
    Publish,
    Unpublish,
    Share,
    Unshare,
    Move,
    Copy,
    Rename,
    Restore,
    DeleteVersion,
}
